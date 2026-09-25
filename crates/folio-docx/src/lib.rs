//! DOCX/OOXML input adapter.
//!
//! The crate deliberately stops at the format-neutral Folio Semantic IR.  ZIP
//! parts, relationship ids, Word style ids, numbering ids and WML element
//! names are confined to this crate; target writers only receive `Book`.

use std::{
    collections::{btree_map::Entry, BTreeMap, BTreeSet},
    fs,
    io::{Cursor, Read},
    path::Path,
    sync::Arc,
};

use folio_format::{
    DetectionConfidence, DetectionResult, FormatAdapter, FormatError, FormatSupport, ImportContext,
    ImportedBook,
};
use folio_input::{BookSource, DetectedFormat, SourceKind};
use folio_model::{
    Anchor, AnchorEdge, AnchorGraph, AnchorId, AnchorRelation, Book, ComputedStyle, Confidence,
    Diagnostic, Document, DocumentId, MemoryResourceLoader, Metadata, NavPoint, Node, NodeId,
    NodeKind, PresentationFeature, PresentationIntent, Resource, ResourceId, ResourceKind,
    SemanticRole, StyleId,
};
use roxmltree::{Document as XmlDocument, Node as XmlNode};
use thiserror::Error;
use zip::{result::ZipError, ZipArchive};

const MAX_ENTRIES: usize = 100_000;
const MAX_TOTAL_UNCOMPRESSED: u64 = 1 << 30;
const MAX_ENTRY_SIZE: u64 = 256 << 20;
const MAX_XML_SIZE: u64 = 64 << 20;
const MAX_XML_NODES: usize = 2_000_000;
const MAX_XML_DEPTH: usize = 128;
const DOCX_MAIN_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const REL_OFFICE_DOCUMENT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const REL_HYPERLINK: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
const REL_HEADER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header";
const REL_FOOTER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer";

#[derive(Clone, Debug)]
pub struct DocxLimits {
    pub max_entries: usize,
    pub max_total_uncompressed: u64,
    pub max_entry_size: u64,
    pub max_xml_size: u64,
    pub max_xml_nodes: usize,
    pub max_xml_depth: usize,
}

impl Default for DocxLimits {
    fn default() -> Self {
        Self {
            max_entries: MAX_ENTRIES,
            max_total_uncompressed: MAX_TOTAL_UNCOMPRESSED,
            max_entry_size: MAX_ENTRY_SIZE,
            max_xml_size: MAX_XML_SIZE,
            max_xml_nodes: MAX_XML_NODES,
            max_xml_depth: MAX_XML_DEPTH,
        }
    }
}

#[derive(Debug, Error)]
pub enum DocxError {
    #[error("DOCX adapter expects one regular .docx or OOXML file")]
    Source,
    #[error("DOCX source I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("DOCX ZIP container is invalid: {0}")]
    Zip(#[from] ZipError),
    #[error("DOCX XML in {path} is invalid: {message}")]
    Xml { path: String, message: String },
    #[error("DOCX package is invalid: {0}")]
    Invalid(String),
    #[error("DOCX safety limit exceeded: {0}")]
    Limit(String),
    #[error("DOCX macro-enabled package is not supported")]
    MacroEnabled,
}

#[derive(Clone, Debug)]
struct DocxPackage {
    parts: BTreeMap<String, Arc<[u8]>>,
    content_types: BTreeMap<String, String>,
    relationships: BTreeMap<String, Relationship>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug)]
struct Relationship {
    id: String,
    kind: String,
    target: String,
    target_mode: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct DocxNativeDocument {
    metadata: Metadata,
    body: Vec<DocxBlock>,
    footnotes: BTreeMap<i32, Vec<DocxBlock>>,
    endnotes: BTreeMap<i32, Vec<DocxBlock>>,
    styles: StyleCatalog,
    numbering: NumberingCatalog,
    relationships: BTreeMap<String, Relationship>,
    images: BTreeMap<String, Arc<[u8]>>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug)]
enum DocxBlock {
    Paragraph(DocxParagraph),
    Table(DocxTable),
}

#[derive(Clone, Debug, Default)]
struct DocxParagraph {
    style_id: Option<String>,
    outline_level: Option<u8>,
    num_id: Option<String>,
    ilvl: u8,
    direct: StyleProperties,
    runs: Vec<DocxInline>,
    bookmarks: Vec<String>,
    section_break: bool,
}

#[derive(Clone, Debug)]
enum DocxInline {
    Run(DocxRun),
    Link {
        href: String,
        children: Vec<DocxInline>,
    },
    FootnoteReference(i32),
    EndnoteReference(i32),
    PageBreak,
    Image(DocxImage),
    Tab,
}

#[derive(Clone, Debug, Default)]
struct DocxRun {
    text: String,
    character_style: Option<String>,
    direct: StyleProperties,
}

#[derive(Clone, Debug, Default)]
struct DocxImage {
    target: String,
    alt: String,
    width_emu: Option<i64>,
    height_emu: Option<i64>,
    floating: bool,
}

#[derive(Clone, Debug, Default)]
struct DocxTable {
    rows: Vec<Vec<Vec<DocxBlock>>>,
    has_merge: bool,
}

#[derive(Clone, Debug, Default)]
struct StyleCatalog {
    defaults: StyleProperties,
    styles: BTreeMap<String, DocxStyle>,
}

#[derive(Clone, Debug, Default)]
struct DocxStyle {
    name: Option<String>,
    based_on: Option<String>,
    linked: Option<String>,
    paragraph: StyleProperties,
    run: StyleProperties,
    outline_level: Option<u8>,
}

#[derive(Clone, Debug, Default)]
struct StyleProperties {
    values: BTreeMap<String, String>,
}

impl StyleProperties {
    fn merge(&mut self, other: &Self) {
        for (key, value) in &other.values {
            self.values.insert(key.clone(), value.clone());
        }
    }
}

#[derive(Clone, Debug, Default)]
struct NumberingCatalog {
    abstract_nums: BTreeMap<String, AbstractNumbering>,
    nums: BTreeMap<String, NumberingInstance>,
}

#[derive(Clone, Debug, Default)]
struct AbstractNumbering {
    levels: BTreeMap<u8, NumberingLevel>,
}

#[derive(Clone, Debug, Default)]
struct NumberingInstance {
    abstract_id: String,
    overrides: BTreeMap<u8, u32>,
}

#[derive(Clone, Debug, Default)]
struct NumberingLevel {
    format: String,
    level_text: Option<String>,
    start: u32,
}

#[derive(Clone, Debug)]
struct ListInfo {
    num_id: String,
    level: u8,
    ordered: bool,
    start: u32,
    level_text: Option<String>,
}

impl DocxPackage {
    fn open(bytes: &[u8], limits: &DocxLimits) -> Result<Self, DocxError> {
        let mut archive = ZipArchive::new(Cursor::new(bytes))?;
        if archive.len() > limits.max_entries {
            return Err(DocxError::Limit(format!(
                "ZIP entry count exceeds {}",
                limits.max_entries
            )));
        }
        let mut parts = BTreeMap::new();
        let mut total = 0u64;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let raw_name = entry.name().to_owned();
            if raw_name.ends_with('/') {
                continue;
            }
            let name = normalize_package_path(&raw_name)?;
            let slot = match parts.entry(name) {
                Entry::Occupied(existing) => {
                    return Err(DocxError::Invalid(format!(
                        "duplicate normalized ZIP member: {}",
                        existing.key()
                    )));
                }
                Entry::Vacant(slot) => slot,
            };
            let name = slot.key().clone();
            if entry.size() > limits.max_entry_size {
                return Err(DocxError::Limit(format!(
                    "ZIP entry {name} exceeds {} bytes",
                    limits.max_entry_size
                )));
            }
            total = total
                .checked_add(entry.size())
                .ok_or_else(|| DocxError::Limit("ZIP total size overflow".to_owned()))?;
            if total > limits.max_total_uncompressed {
                return Err(DocxError::Limit(format!(
                    "ZIP total uncompressed size exceeds {}",
                    limits.max_total_uncompressed
                )));
            }
            let mut value =
                Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0).min(16 << 20));
            entry.read_to_end(&mut value)?;
            slot.insert(Arc::<[u8]>::from(value.into_boxed_slice()));
        }

        let content_type_bytes = parts
            .get("[Content_Types].xml")
            .ok_or_else(|| DocxError::Invalid("missing [Content_Types].xml".to_owned()))?;
        if content_type_bytes.len() as u64 > limits.max_xml_size {
            return Err(DocxError::Limit(
                "[Content_Types].xml exceeds the XML safety limit".to_owned(),
            ));
        }
        let content_types = parse_content_types(content_type_bytes, limits)?;
        if content_types
            .get("word/document.xml")
            .is_some_and(|value| value.contains("macroEnabled"))
        {
            return Err(DocxError::MacroEnabled);
        }
        if content_types.get("word/document.xml") != Some(&DOCX_MAIN_CONTENT_TYPE.to_owned()) {
            return Err(DocxError::Invalid(
                "word/document.xml is not declared as a WordprocessingML main document".to_owned(),
            ));
        }
        if !parts.contains_key("word/document.xml") {
            return Err(DocxError::Invalid(
                "missing required word/document.xml".to_owned(),
            ));
        }

        let mut diagnostics = Vec::new();
        let root_relationships = if let Some(bytes) = parts.get("_rels/.rels") {
            parse_relationships(bytes, "_rels/.rels", limits)?
        } else {
            diagnostics.push(Diagnostic::warning(
                "DOCX-P001",
                "DOCX package has no root relationships part; using word/document.xml by convention",
            ));
            Vec::new()
        };
        let main_from_root = root_relationships
            .iter()
            .find(|relationship| relationship.kind == REL_OFFICE_DOCUMENT)
            .and_then(|relationship| {
                if relationship.target_mode.as_deref() == Some("External") {
                    None
                } else {
                    resolve_package_target("", &relationship.target).ok()
                }
            });
        if main_from_root.as_deref() != Some("word/document.xml") {
            diagnostics.push(Diagnostic::warning(
                "DOCX-P002",
                "root relationship does not explicitly target word/document.xml; convention fallback used",
            ));
        }

        let relationships = if let Some(bytes) = parts.get("word/_rels/document.xml.rels") {
            parse_relationships(bytes, "word/_rels/document.xml.rels", limits)?
                .into_iter()
                .map(|relationship| (relationship.id.clone(), relationship))
                .collect()
        } else {
            diagnostics.push(Diagnostic::warning(
                "DOCX-P003",
                "DOCX package has no word/document.xml.rels; relationship-backed media and links may be unavailable",
            ));
            BTreeMap::new()
        };

        for (name, content_type) in &content_types {
            if name.starts_with("word/comments")
                || name.starts_with("word/embeddings/")
                || name.starts_with("word/activeX/")
                || name.starts_with("word/diagrams/")
                || name.starts_with("customXml/")
            {
                diagnostics.push(Diagnostic::warning(
                    unsupported_part_code(name),
                    format!("unsupported DOCX part detected: {name} ({content_type})"),
                ));
            }
        }

        Ok(Self {
            parts,
            content_types,
            relationships,
            diagnostics,
        })
    }
}

fn parse_content_types(
    bytes: &[u8],
    limits: &DocxLimits,
) -> Result<BTreeMap<String, String>, DocxError> {
    let document = parse_xml("[Content_Types].xml", bytes, limits)?;
    let mut result = BTreeMap::new();
    for node in document
        .root_element()
        .children()
        .filter(|node| node.is_element())
    {
        if local_name(node) != "Override" {
            continue;
        }
        let Some(raw_part) = attr_local(node, "PartName") else {
            continue;
        };
        let Some(content_type) = attr_local(node, "ContentType") else {
            continue;
        };
        let part = normalize_package_path(raw_part.trim_start_matches('/'))?;
        result.insert(part, content_type.to_owned());
    }
    Ok(result)
}

fn parse_relationships(
    bytes: &[u8],
    path: &str,
    limits: &DocxLimits,
) -> Result<Vec<Relationship>, DocxError> {
    let document = parse_xml(path, bytes, limits)?;
    let mut relationships = Vec::new();
    for node in document
        .root_element()
        .children()
        .filter(|node| node.is_element())
    {
        if local_name(node) != "Relationship" {
            continue;
        }
        let Some(id) = attr_local(node, "Id") else {
            return Err(DocxError::Invalid(format!(
                "relationship in {path} has no Id"
            )));
        };
        let Some(kind) = attr_local(node, "Type") else {
            return Err(DocxError::Invalid(format!(
                "relationship {id} in {path} has no Type"
            )));
        };
        let Some(target) = attr_local(node, "Target") else {
            return Err(DocxError::Invalid(format!(
                "relationship {id} in {path} has no Target"
            )));
        };
        relationships.push(Relationship {
            id: id.to_owned(),
            kind: kind.to_owned(),
            target: target.to_owned(),
            target_mode: attr_local(node, "TargetMode").map(ToOwned::to_owned),
        });
    }
    Ok(relationships)
}

fn parse_xml<'a>(
    path: &str,
    bytes: &'a [u8],
    limits: &DocxLimits,
) -> Result<XmlDocument<'a>, DocxError> {
    if bytes.len() as u64 > limits.max_xml_size {
        return Err(DocxError::Limit(format!(
            "XML part {path} exceeds {} bytes",
            limits.max_xml_size
        )));
    }
    if bytes
        .windows(9)
        .any(|window| window.eq_ignore_ascii_case(b"<!DOCTYPE"))
        || bytes
            .windows(8)
            .any(|window| window.eq_ignore_ascii_case(b"<!ENTITY"))
    {
        return Err(DocxError::Invalid(format!(
            "external entities/DOCTYPE are not allowed in {path}"
        )));
    }
    let text = std::str::from_utf8(bytes).map_err(|error| DocxError::Xml {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    let document = XmlDocument::parse(text).map_err(|error| DocxError::Xml {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    let count = document.descendants().count();
    if count > limits.max_xml_nodes {
        return Err(DocxError::Limit(format!(
            "XML part {path} has more than {} nodes",
            limits.max_xml_nodes
        )));
    }
    let mut max_depth = 0usize;
    for node in document.descendants() {
        let mut depth = 0usize;
        let mut current = node;
        while let Some(parent) = current.parent() {
            depth = depth.saturating_add(1);
            current = parent;
        }
        max_depth = max_depth.max(depth);
    }
    if max_depth > limits.max_xml_depth {
        return Err(DocxError::Limit(format!(
            "XML part {path} exceeds depth {}",
            limits.max_xml_depth
        )));
    }
    Ok(document)
}

fn normalize_package_path(value: &str) -> Result<String, DocxError> {
    if value.contains('\0') {
        return Err(DocxError::Invalid("ZIP path contains NUL".to_owned()));
    }
    let path = folio_input::normalize_relative_path(Path::new(&value.replace('\\', "/")))
        .map_err(|error| DocxError::Invalid(format!("unsafe ZIP path {value}: {error}")))?;
    Ok(path.to_string_lossy().replace('\\', "/"))
}

fn resolve_package_target(base_dir: &str, target: &str) -> Result<String, DocxError> {
    if target.starts_with("http://") || target.starts_with("https://") {
        return Err(DocxError::Invalid(
            "external relationship target".to_owned(),
        ));
    }
    let target = target.trim_start_matches('/').replace('\\', "/");
    let combined = if base_dir.is_empty() {
        target
    } else {
        format!("{base_dir}/{target}")
    };
    normalize_package_path(&combined)
}

fn unsupported_part_code(path: &str) -> &'static str {
    if path.starts_with("word/comments") {
        "DOCX-X001"
    } else if path.starts_with("word/embeddings/") || path.starts_with("word/activeX/") {
        "DOCX-X002"
    } else if path.starts_with("word/diagrams/") {
        "DOCX-X003"
    } else {
        "DOCX-X004"
    }
}

fn local_name<'a, 'input>(node: XmlNode<'a, 'input>) -> &'input str {
    node.tag_name()
        .name()
        .rsplit(':')
        .next()
        .unwrap_or(node.tag_name().name())
}

fn attr_local<'a>(node: XmlNode<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes().find_map(|attribute| {
        (attribute.name() == name
            || attribute
                .name()
                .rsplit(':')
                .next()
                .is_some_and(|value| value == name))
        .then_some(attribute.value())
    })
}

fn element_text(node: XmlNode<'_, '_>) -> String {
    node.descendants()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect::<String>()
}

pub struct DocxAdapter;

impl FormatAdapter for DocxAdapter {
    fn format(&self) -> DetectedFormat {
        DetectedFormat::Docx
    }

    fn support(&self) -> FormatSupport {
        FormatSupport {
            detect: true,
            inspect: true,
            import: true,
            metadata_read: true,
            metadata_write: true,
            edit: true,
            preview: true,
            ..FormatSupport::default()
        }
    }

    fn detect(&self, source: &BookSource) -> Result<DetectionResult, FormatError> {
        let bytes = source_bytes(source)?;
        DocxPackage::open(&bytes, &DocxLimits::default())
            .map_err(|error| FormatError::Unsupported(error.to_string()))?;
        Ok(DetectionResult {
            format: DetectedFormat::Docx.name().to_owned(),
            confidence: DetectionConfidence::Exact,
            evidence: vec![
                "ZIP/OPC container signature".to_owned(),
                "[Content_Types].xml declares WordprocessingML main document".to_owned(),
                "word/document.xml exists".to_owned(),
            ],
            warnings: Vec::new(),
        })
    }

    fn import(
        &self,
        source: &BookSource,
        context: &ImportContext,
    ) -> Result<ImportedBook, FormatError> {
        let bytes = source_bytes(source)?;
        let package = DocxPackage::open(&bytes, &DocxLimits::default())
            .map_err(|error| FormatError::Invalid(error.to_string()))?;
        let decoder = Decoder::new(package, context.strict);
        let (book, diagnostics) = decoder
            .decode()
            .map_err(|error| FormatError::Invalid(error.to_string()))?;
        let input_loss = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.starts_with("DOCX-"))
            .filter(|diagnostic| diagnostic.severity != folio_model::Severity::Info)
            .map(|diagnostic| diagnostic.message.clone())
            .collect();
        Ok(ImportedBook {
            format: DetectedFormat::Docx,
            parser: "folio-docx/1".to_owned(),
            book,
            diagnostics,
            input_loss,
            text: None,
        })
    }
}

fn source_bytes(source: &BookSource) -> Result<Vec<u8>, FormatError> {
    if source.kind() != SourceKind::SingleFile {
        return Err(FormatError::Unsupported(
            "DOCX adapter expects one regular file".to_owned(),
        ));
    }
    let file = source
        .files()
        .first()
        .ok_or_else(|| FormatError::Unsupported("source contains no files".to_owned()))?;
    fs::read(&file.path).map_err(FormatError::from)
}

fn parse_styles(
    package: &DocxPackage,
    diagnostics: &mut Vec<Diagnostic>,
    limits: &DocxLimits,
) -> Result<StyleCatalog, DocxError> {
    let Some(bytes) = package.parts.get("word/styles.xml") else {
        diagnostics.push(Diagnostic::warning(
            "DOCX-S000",
            "DOCX has no word/styles.xml; direct formatting and default styles are used",
        ));
        return Ok(StyleCatalog::default());
    };
    let document = parse_xml("word/styles.xml", bytes, limits)?;
    let root = document.root_element();
    let mut catalog = StyleCatalog::default();
    if let Some(defaults) = child_named(root, "docDefaults") {
        if let Some(rpr) = child_named(
            child_named(defaults, "rPrDefault").unwrap_or(defaults),
            "rPr",
        ) {
            catalog.defaults.merge(&parse_run_properties(rpr));
        }
        if let Some(ppr) = child_named(
            child_named(defaults, "pPrDefault").unwrap_or(defaults),
            "pPr",
        ) {
            catalog.defaults.merge(&parse_paragraph_properties(ppr));
        }
    }
    for style in root.children().filter(|node| node.is_element()) {
        if local_name(style) != "style" {
            continue;
        }
        let Some(id) = attr_local(style, "styleId") else {
            diagnostics.push(Diagnostic::warning(
                "DOCX-S002",
                "DOCX style without styleId was ignored",
            ));
            continue;
        };
        let mut value = DocxStyle {
            name: child_named(style, "name")
                .and_then(|node| attr_local(node, "val"))
                .map(ToOwned::to_owned),
            based_on: child_named(style, "basedOn")
                .and_then(|node| attr_local(node, "val"))
                .map(ToOwned::to_owned),
            linked: child_named(style, "link")
                .and_then(|node| attr_local(node, "val"))
                .map(ToOwned::to_owned),
            paragraph: StyleProperties::default(),
            run: StyleProperties::default(),
            outline_level: None,
        };
        if let Some(ppr) = child_named(style, "pPr") {
            value.paragraph = parse_paragraph_properties(ppr);
            value.outline_level = child_named(ppr, "outlineLvl")
                .and_then(|node| attr_local(node, "val"))
                .and_then(|value| value.parse::<u8>().ok());
        }
        if let Some(rpr) = child_named(style, "rPr") {
            value.run = parse_run_properties(rpr);
        }
        catalog.styles.insert(id.to_owned(), value);
    }
    Ok(catalog)
}

impl StyleCatalog {
    fn chain<'a>(
        &'a self,
        style_id: Option<&str>,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Vec<&'a DocxStyle> {
        let Some(style_id) = style_id else {
            return Vec::new();
        };
        let mut result = Vec::new();
        let mut current = Some(style_id);
        let mut seen = BTreeSet::new();
        while let Some(id) = current {
            if !seen.insert(id.to_owned()) {
                diagnostics.push(Diagnostic::warning(
                    "DOCX-S001",
                    format!("style basedOn cycle detected at '{id}'; inheritance stopped safely"),
                ));
                break;
            }
            let Some(style) = self.styles.get(id) else {
                diagnostics.push(Diagnostic::warning(
                    "DOCX-S003",
                    format!("style '{id}' is referenced but not defined"),
                ));
                break;
            };
            current = style.based_on.as_deref();
            result.push(style);
        }
        result.reverse();
        result
    }

    fn resolve_paragraph(
        &self,
        style_id: Option<&str>,
        direct: &StyleProperties,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> StyleProperties {
        let mut result = self.defaults.clone();
        for style in self.chain(style_id, diagnostics) {
            result.merge(&style.paragraph);
        }
        result.merge(direct);
        result
    }

    fn resolve_run(
        &self,
        paragraph_style: Option<&str>,
        character_style: Option<&str>,
        paragraph_direct: &StyleProperties,
        run_direct: &StyleProperties,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> StyleProperties {
        let mut result = self.resolve_paragraph(paragraph_style, paragraph_direct, diagnostics);
        for style in self.chain(paragraph_style, diagnostics) {
            result.merge(&style.run);
        }
        let linked_styles = self
            .chain(paragraph_style, diagnostics)
            .into_iter()
            .filter_map(|style| style.linked.clone())
            .collect::<Vec<_>>();
        for linked_style in linked_styles {
            for style in self.chain(Some(&linked_style), diagnostics) {
                result.merge(&style.run);
            }
        }
        for style in self.chain(character_style, diagnostics) {
            result.merge(&style.run);
        }
        result.merge(run_direct);
        result
    }

    fn heading_level(&self, style_id: Option<&str>, outline: Option<u8>) -> Option<u8> {
        if let Some(level) = outline {
            return Some(level.saturating_add(1).clamp(1, 6));
        }
        let style = style_id.and_then(|id| self.styles.get(id));
        if let Some(level) = style.and_then(|value| value.outline_level) {
            return Some(level.saturating_add(1).clamp(1, 6));
        }
        let id = style_id?.to_ascii_lowercase();
        if let Some(suffix) = id.strip_prefix("heading") {
            if let Ok(level) = suffix.parse::<u8>() {
                return Some(level.clamp(1, 6));
            }
        }
        if id == "title" || id == "subtitle" {
            return Some(if id == "title" { 1 } else { 2 });
        }
        let name = style
            .and_then(|value| value.name.as_deref())?
            .to_ascii_lowercase();
        if name.starts_with("heading") || name.starts_with("标题") {
            let level = name
                .chars()
                .filter(|character| character.is_ascii_digit())
                .collect::<String>()
                .parse::<u8>()
                .unwrap_or(1);
            return Some(level.clamp(1, 6));
        }
        None
    }
}

fn parse_numbering(
    package: &DocxPackage,
    diagnostics: &mut Vec<Diagnostic>,
    limits: &DocxLimits,
) -> Result<NumberingCatalog, DocxError> {
    let Some(bytes) = package.parts.get("word/numbering.xml") else {
        diagnostics.push(Diagnostic::warning(
            "DOCX-L000",
            "DOCX has no word/numbering.xml; numbered paragraphs remain ordinary paragraphs",
        ));
        return Ok(NumberingCatalog::default());
    };
    let document = parse_xml("word/numbering.xml", bytes, limits)?;
    let root = document.root_element();
    let mut catalog = NumberingCatalog::default();
    for abstract_node in root.children().filter(|node| node.is_element()) {
        if local_name(abstract_node) != "abstractNum" {
            continue;
        }
        let Some(id) = attr_local(abstract_node, "abstractNumId") else {
            continue;
        };
        let mut value = AbstractNumbering::default();
        for level_node in abstract_node.children().filter(|node| node.is_element()) {
            if local_name(level_node) != "lvl" {
                continue;
            }
            let level = attr_local(level_node, "ilvl")
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or(0);
            let num_fmt = child_named(level_node, "numFmt")
                .and_then(|node| attr_local(node, "val"))
                .unwrap_or("bullet")
                .to_owned();
            let level_text = child_named(level_node, "lvlText")
                .and_then(|node| attr_local(node, "val"))
                .map(ToOwned::to_owned);
            let start = child_named(level_node, "start")
                .and_then(|node| attr_local(node, "val"))
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(1);
            value.levels.insert(
                level,
                NumberingLevel {
                    format: num_fmt,
                    level_text,
                    start,
                },
            );
        }
        catalog.abstract_nums.insert(id.to_owned(), value);
    }
    for num_node in root.children().filter(|node| node.is_element()) {
        if local_name(num_node) != "num" {
            continue;
        }
        let Some(id) = attr_local(num_node, "numId") else {
            continue;
        };
        let Some(abstract_id) = child_named(num_node, "abstractNumId")
            .and_then(|node| attr_local(node, "val"))
            .map(ToOwned::to_owned)
        else {
            diagnostics.push(Diagnostic::warning(
                "DOCX-L001",
                format!("numbering instance '{id}' has no abstractNumId"),
            ));
            continue;
        };
        let mut value = NumberingInstance {
            abstract_id,
            overrides: BTreeMap::new(),
        };
        for override_node in num_node.children().filter(|node| node.is_element()) {
            if local_name(override_node) != "lvlOverride" {
                continue;
            }
            let Some(level) =
                attr_local(override_node, "ilvl").and_then(|value| value.parse::<u8>().ok())
            else {
                continue;
            };
            if let Some(start) = child_named(override_node, "startOverride")
                .and_then(|node| attr_local(node, "val"))
                .and_then(|value| value.parse::<u32>().ok())
            {
                value.overrides.insert(level, start);
            }
        }
        catalog.nums.insert(id.to_owned(), value);
    }
    Ok(catalog)
}

impl NumberingCatalog {
    fn info(
        &self,
        num_id: Option<&str>,
        level: u8,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Option<ListInfo> {
        let num_id = num_id?;
        let Some(instance) = self.nums.get(num_id) else {
            diagnostics.push(Diagnostic::warning(
                "DOCX-L001",
                format!("paragraph references missing numbering instance '{num_id}'"),
            ));
            return None;
        };
        let Some(abstract_num) = self.abstract_nums.get(&instance.abstract_id) else {
            diagnostics.push(Diagnostic::warning(
                "DOCX-L001",
                format!("numbering instance '{num_id}' references missing abstract numbering"),
            ));
            return None;
        };
        let level_data = abstract_num
            .levels
            .get(&level)
            .or_else(|| abstract_num.levels.get(&0))?;
        Some(ListInfo {
            num_id: num_id.to_owned(),
            level,
            ordered: !matches!(level_data.format.as_str(), "bullet" | "none" | "picture"),
            start: instance
                .overrides
                .get(&level)
                .copied()
                .unwrap_or(level_data.start),
            level_text: level_data.level_text.clone(),
        })
    }
}

fn child_named<'a, 'input>(node: XmlNode<'a, 'input>, name: &str) -> Option<XmlNode<'a, 'input>> {
    node.children()
        .find(|child| child.is_element() && local_name(*child) == name)
}

fn bool_property(node: XmlNode<'_, '_>) -> bool {
    attr_local(node, "val")
        .map(|value| {
            !matches!(
                value.to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "none"
            )
        })
        .unwrap_or(true)
}

fn twips_to_pt(value: &str) -> Option<String> {
    let twips = value.parse::<i64>().ok()?;
    if twips % 20 == 0 {
        Some(format!("{}pt", twips / 20))
    } else {
        Some(format!("{:.2}pt", twips as f64 / 20.0))
    }
}

fn half_points_to_pt(value: &str) -> Option<String> {
    let half_points = value.parse::<i64>().ok()?;
    if half_points % 2 == 0 {
        Some(format!("{}pt", half_points / 2))
    } else {
        Some(format!("{:.1}pt", half_points as f64 / 2.0))
    }
}

fn parse_run_properties(node: XmlNode<'_, '_>) -> StyleProperties {
    let mut result = StyleProperties::default();
    for child in node.children().filter(|child| child.is_element()) {
        let name = local_name(child);
        match name {
            "rStyle" => {
                if let Some(value) = attr_local(child, "val") {
                    result
                        .values
                        .insert("character-style".to_owned(), value.to_owned());
                }
            }
            "b" if bool_property(child) => {
                result
                    .values
                    .insert("font-weight".to_owned(), "bold".to_owned());
            }
            "i" if bool_property(child) => {
                result
                    .values
                    .insert("font-style".to_owned(), "italic".to_owned());
            }
            "u" => {
                if attr_local(child, "val") != Some("none") {
                    result
                        .values
                        .insert("text-decoration".to_owned(), "underline".to_owned());
                }
            }
            "strike" if bool_property(child) => {
                result
                    .values
                    .insert("text-decoration-line".to_owned(), "line-through".to_owned());
            }
            "vertAlign" => {
                if let Some(value) = attr_local(child, "val") {
                    let value = match value {
                        "superscript" => "super",
                        "subscript" => "sub",
                        other => other,
                    };
                    result
                        .values
                        .insert("vertical-align".to_owned(), value.to_owned());
                }
            }
            "sz" | "szCs" => {
                if let Some(value) = attr_local(child, "val").and_then(half_points_to_pt) {
                    result.values.insert("font-size".to_owned(), value);
                }
            }
            "rFonts" => {
                for key in ["ascii", "hAnsi", "eastAsia", "cs"] {
                    if let Some(value) = attr_local(child, key) {
                        result
                            .values
                            .insert("font-family".to_owned(), value.to_owned());
                        break;
                    }
                }
            }
            "color" => {
                if let Some(value) = attr_local(child, "val") {
                    result
                        .values
                        .insert("color".to_owned(), format!("#{value}"));
                }
            }
            "highlight" => {
                if let Some(value) = attr_local(child, "val") {
                    result
                        .values
                        .insert("background-color".to_owned(), value.to_owned());
                }
            }
            "caps" if bool_property(child) => {
                result
                    .values
                    .insert("text-transform".to_owned(), "uppercase".to_owned());
            }
            "smallCaps" if bool_property(child) => {
                result
                    .values
                    .insert("font-variant".to_owned(), "small-caps".to_owned());
            }
            _ => {}
        }
    }
    result
}

fn parse_paragraph_properties(node: XmlNode<'_, '_>) -> StyleProperties {
    let mut result = StyleProperties::default();
    for child in node.children().filter(|child| child.is_element()) {
        match local_name(child) {
            "jc" => {
                if let Some(value) = attr_local(child, "val") {
                    let value = match value {
                        "both" => "justify",
                        "start" => "left",
                        "end" => "right",
                        other => other,
                    };
                    result
                        .values
                        .insert("text-align".to_owned(), value.to_owned());
                }
            }
            "ind" => {
                if let Some(value) = attr_local(child, "left").and_then(twips_to_pt) {
                    result.values.insert("margin-left".to_owned(), value);
                }
                if let Some(value) = attr_local(child, "right").and_then(twips_to_pt) {
                    result.values.insert("margin-right".to_owned(), value);
                }
                if let Some(value) = attr_local(child, "firstLine").and_then(twips_to_pt) {
                    result.values.insert("text-indent".to_owned(), value);
                } else if let Some(value) = attr_local(child, "hanging").and_then(twips_to_pt) {
                    result
                        .values
                        .insert("text-indent".to_owned(), format!("-{value}"));
                }
            }
            "spacing" => {
                if let Some(value) = attr_local(child, "before").and_then(twips_to_pt) {
                    result.values.insert("margin-top".to_owned(), value);
                }
                if let Some(value) = attr_local(child, "after").and_then(twips_to_pt) {
                    result.values.insert("margin-bottom".to_owned(), value);
                }
                if let Some(value) = attr_local(child, "line") {
                    result
                        .values
                        .insert("line-height-twips".to_owned(), value.to_owned());
                }
            }
            "keepNext" if bool_property(child) => {
                result
                    .values
                    .insert("keep-next".to_owned(), "true".to_owned());
            }
            "keepLines" if bool_property(child) => {
                result
                    .values
                    .insert("keep-lines".to_owned(), "true".to_owned());
            }
            "pageBreakBefore" if bool_property(child) => {
                result
                    .values
                    .insert("page-break-before".to_owned(), "true".to_owned());
            }
            "outlineLvl" => {
                if let Some(value) = attr_local(child, "val") {
                    result
                        .values
                        .insert("outline-level".to_owned(), value.to_owned());
                }
            }
            _ => {}
        }
    }
    result
}

struct Decoder {
    package: DocxPackage,
    strict: bool,
    limits: DocxLimits,
    native: DocxNativeDocument,
}

impl Decoder {
    fn new(package: DocxPackage, strict: bool) -> Self {
        Self {
            package,
            strict,
            limits: DocxLimits::default(),
            native: DocxNativeDocument::default(),
        }
    }

    fn decode(mut self) -> Result<(Book, Vec<Diagnostic>), DocxError> {
        self.native.diagnostics = self.package.diagnostics.clone();
        self.native.relationships = self.package.relationships.clone();
        self.native.styles =
            parse_styles(&self.package, &mut self.native.diagnostics, &self.limits)?;
        self.native.numbering =
            parse_numbering(&self.package, &mut self.native.diagnostics, &self.limits)?;
        self.native.metadata = self.parse_metadata()?;
        self.collect_media();
        let document_bytes = self
            .package
            .parts
            .get("word/document.xml")
            .cloned()
            .ok_or_else(|| DocxError::Invalid("missing required word/document.xml".to_owned()))?;
        let document = parse_xml("word/document.xml", document_bytes.as_ref(), &self.limits)?;
        self.native.body = self.parse_blocks(document.root_element())?;
        self.parse_notes("word/footnotes.xml", true)?;
        self.parse_notes("word/endnotes.xml", false)?;
        self.record_non_body_parts();
        self.decode_semantic()
    }

    fn parse_metadata(&mut self) -> Result<Metadata, DocxError> {
        let mut metadata = Metadata::default();
        if let Some(bytes) = self.package.parts.get("docProps/core.xml") {
            let document = parse_xml("docProps/core.xml", bytes, &self.limits)?;
            for node in document
                .root_element()
                .descendants()
                .filter(|node| node.is_element())
            {
                match local_name(node) {
                    "title" => metadata.title = non_empty_text(node),
                    "creator" => {
                        if let Some(value) = non_empty_text(node) {
                            metadata.add_author(value);
                        }
                    }
                    "subject" => metadata.subjects.extend(non_empty_text(node)),
                    "description" => metadata.description = non_empty_text(node),
                    "keywords" => {
                        if let Some(value) = non_empty_text(node) {
                            metadata.subjects.extend(
                                value
                                    .split([',', ';'])
                                    .map(str::trim)
                                    .filter(|value| !value.is_empty())
                                    .map(ToOwned::to_owned),
                            );
                        }
                    }
                    "language" => metadata.language = non_empty_text(node),
                    "created" | "modified" => {
                        if let Some(value) = non_empty_text(node) {
                            if local_name(node) == "created" && metadata.date.is_none() {
                                metadata.date = Some(value.clone());
                            }
                            metadata.dates.push(value);
                        }
                    }
                    "lastModifiedBy" if non_empty_text(node).is_some() => {
                        self.native.diagnostics.push(Diagnostic::info(
                            "DOCX-P004",
                            "docProps/core.xml lastModifiedBy was read as provenance, not as ebook author",
                        ));
                    }
                    _ => {}
                }
            }
        } else {
            self.native.diagnostics.push(Diagnostic::warning(
                "DOCX-P005",
                "DOCX has no docProps/core.xml; metadata defaults were used",
            ));
        }
        if let Some(bytes) = self.package.parts.get("docProps/app.xml") {
            let document = parse_xml("docProps/app.xml", bytes, &self.limits)?;
            if document
                .root_element()
                .descendants()
                .any(|node| node.is_element() && local_name(node) == "Application")
            {
                self.native.diagnostics.push(Diagnostic::info(
                    "DOCX-P006",
                    "DOCX application provenance was recognized and kept outside ebook metadata",
                ));
            }
        }
        Ok(metadata)
    }

    fn collect_media(&mut self) {
        for (path, bytes) in &self.package.parts {
            if path.starts_with("word/media/") {
                self.native.images.insert(path.clone(), Arc::clone(bytes));
            }
        }
    }

    fn parse_notes(&mut self, part: &str, footnote: bool) -> Result<(), DocxError> {
        let Some(bytes) = self.package.parts.get(part).cloned() else {
            return Ok(());
        };
        let document = match parse_xml(part, bytes.as_ref(), &self.limits) {
            Ok(document) => document,
            Err(error) => {
                self.native.diagnostics.push(Diagnostic::warning(
                    if footnote { "DOCX-N001" } else { "DOCX-N002" },
                    format!("{part} could not be read: {error}"),
                ));
                if self.strict {
                    return Err(error);
                }
                return Ok(());
            }
        };
        for note in document
            .root_element()
            .children()
            .filter(|node| node.is_element())
        {
            let name = local_name(note);
            if name != if footnote { "footnote" } else { "endnote" } {
                continue;
            }
            let Some(id) = attr_local(note, "id").and_then(|value| value.parse::<i32>().ok())
            else {
                self.native.diagnostics.push(Diagnostic::warning(
                    if footnote { "DOCX-N003" } else { "DOCX-N004" },
                    format!("{part} contains a note without a numeric id"),
                ));
                continue;
            };
            if id < 0 {
                continue;
            }
            let blocks = self.parse_blocks(note)?;
            if footnote {
                self.native.footnotes.insert(id, blocks);
            } else {
                self.native.endnotes.insert(id, blocks);
            }
        }
        Ok(())
    }

    fn parse_blocks(&mut self, parent: XmlNode<'_, '_>) -> Result<Vec<DocxBlock>, DocxError> {
        let mut blocks = Vec::new();
        for child in parent.children().filter(|node| node.is_element()) {
            match local_name(child) {
                "body" => blocks.extend(self.parse_blocks(child)?),
                "p" => blocks.push(DocxBlock::Paragraph(self.parse_paragraph(child)?)),
                "tbl" => blocks.push(DocxBlock::Table(self.parse_table(child)?)),
                "tcPr" | "trPr" | "tblPr" | "tblPrEx" | "tblGrid" => {}
                "sdt" => {
                    if let Some(content) = child_named(child, "sdtContent") {
                        blocks.extend(self.parse_blocks(content)?);
                    } else {
                        self.native.diagnostics.push(Diagnostic::warning(
                            "DOCX-X006",
                            "content control without sdtContent was ignored",
                        ));
                    }
                }
                "sectPr" => {}
                "altChunk" => self.native.diagnostics.push(Diagnostic::warning(
                    "DOCX-X007",
                    "altChunk external content is unsupported and was not imported",
                )),
                name if name == "customXml" || name == "smartTag" => {
                    self.native.diagnostics.push(Diagnostic::warning(
                        "DOCX-X004",
                        format!("unsupported body container '{name}' was not imported"),
                    ));
                }
                name => {
                    if self.strict && name == "object" {
                        return Err(DocxError::Invalid(
                            "strict mode rejects unsupported embedded object".to_owned(),
                        ));
                    }
                    self.native.diagnostics.push(Diagnostic::warning(
                        "DOCX-X005",
                        format!("unsupported body element '{name}' was not imported"),
                    ));
                }
            }
        }
        Ok(blocks)
    }

    fn parse_paragraph(&mut self, paragraph: XmlNode<'_, '_>) -> Result<DocxParagraph, DocxError> {
        let properties = child_named(paragraph, "pPr");
        let direct = properties
            .map(parse_paragraph_properties)
            .unwrap_or_default();
        let style_id = properties
            .and_then(|node| child_named(node, "pStyle"))
            .and_then(|node| attr_local(node, "val"))
            .map(ToOwned::to_owned);
        let outline_level = properties
            .and_then(|node| child_named(node, "outlineLvl"))
            .and_then(|node| attr_local(node, "val"))
            .and_then(|value| value.parse::<u8>().ok());
        let num_pr = properties.and_then(|node| child_named(node, "numPr"));
        let num_id = num_pr
            .and_then(|node| child_named(node, "numId"))
            .and_then(|node| attr_local(node, "val"))
            .map(ToOwned::to_owned);
        let ilvl = num_pr
            .and_then(|node| child_named(node, "ilvl"))
            .and_then(|node| attr_local(node, "val"))
            .and_then(|value| value.parse::<u8>().ok())
            .unwrap_or(0);
        let section_break = properties.is_some_and(|node| child_named(node, "sectPr").is_some());
        let mut result = DocxParagraph {
            style_id,
            outline_level,
            num_id,
            ilvl,
            direct,
            runs: Vec::new(),
            bookmarks: Vec::new(),
            section_break,
        };
        for child in paragraph.children().filter(|node| node.is_element()) {
            match local_name(child) {
                "pPr" => {}
                "r" => result.runs.extend(self.parse_run(child)?),
                "hyperlink" => result.runs.push(self.parse_hyperlink(child)?),
                "bookmarkStart" => {
                    if let Some(name) = attr_local(child, "name").filter(|value| !value.is_empty())
                    {
                        result.bookmarks.push(name.to_owned());
                    }
                }
                "bookmarkEnd" | "proofErr" | "permStart" | "permEnd" => {}
                "fldSimple" => {
                    let instruction = attr_local(child, "instr")
                        .unwrap_or_default()
                        .to_ascii_uppercase();
                    if instruction.contains("PAGE") || instruction.contains("TOC") {
                        self.native.diagnostics.push(Diagnostic::warning(
                            "DOCX-X008",
                            format!(
                                "generated field '{instruction}' was not imported as body text"
                            ),
                        ));
                    } else {
                        result.runs.extend(self.parse_inline_children(child)?);
                    }
                }
                "ins" => {
                    self.native.diagnostics.push(Diagnostic::info(
                        "DOCX-X005",
                        "tracked insertion imported using accepted-view content",
                    ));
                    result.runs.extend(self.parse_inline_children(child)?);
                }
                "del" => {
                    self.native.diagnostics.push(Diagnostic::warning(
                        "DOCX-X005",
                        "tracked deletion detected and omitted from accepted-view content",
                    ));
                }
                "sdt" => {
                    if let Some(content) = child_named(child, "sdtContent") {
                        result.runs.extend(self.parse_inline_children(content)?);
                    }
                }
                name => self.native.diagnostics.push(Diagnostic::warning(
                    "DOCX-X009",
                    format!("unsupported paragraph child '{name}' was not imported"),
                )),
            }
        }
        Ok(result)
    }

    fn parse_inline_children(
        &mut self,
        parent: XmlNode<'_, '_>,
    ) -> Result<Vec<DocxInline>, DocxError> {
        let mut result = Vec::new();
        for child in parent.children().filter(|node| node.is_element()) {
            match local_name(child) {
                "r" => result.extend(self.parse_run(child)?),
                "hyperlink" => result.push(self.parse_hyperlink(child)?),
                "ins" => result.extend(self.parse_inline_children(child)?),
                "del" => {}
                "fldSimple" => result.extend(self.parse_inline_children(child)?),
                "footnoteReference" => {
                    if let Some(id) = attr_local(child, "id").and_then(|value| value.parse().ok()) {
                        result.push(DocxInline::FootnoteReference(id));
                    }
                }
                "endnoteReference" => {
                    if let Some(id) = attr_local(child, "id").and_then(|value| value.parse().ok()) {
                        result.push(DocxInline::EndnoteReference(id));
                    }
                }
                "bookmarkStart" | "bookmarkEnd" => {}
                _ => {}
            }
        }
        Ok(result)
    }

    fn parse_run(&mut self, run: XmlNode<'_, '_>) -> Result<Vec<DocxInline>, DocxError> {
        let direct = child_named(run, "rPr")
            .map(parse_run_properties)
            .unwrap_or_default();
        let character_style = direct.values.get("character-style").cloned();
        let mut result = Vec::new();
        for child in run.children().filter(|node| node.is_element()) {
            match local_name(child) {
                "rPr" => {}
                "t" => {
                    result.push(DocxInline::Run(DocxRun {
                        text: child.text().unwrap_or_default().to_owned(),
                        character_style: character_style.clone(),
                        direct: direct.clone(),
                    }));
                }
                "delText" => self.native.diagnostics.push(Diagnostic::warning(
                    "DOCX-X005",
                    "deleted run text was omitted from accepted-view content",
                )),
                "tab" => result.push(DocxInline::Tab),
                "br" => {
                    let kind = attr_local(child, "type").unwrap_or("textWrapping");
                    if matches!(kind, "page" | "column") {
                        result.push(DocxInline::PageBreak);
                    } else {
                        result.push(DocxInline::Run(DocxRun {
                            text: "\n".to_owned(),
                            character_style: character_style.clone(),
                            direct: direct.clone(),
                        }));
                    }
                }
                "lastRenderedPageBreak" => result.push(DocxInline::PageBreak),
                "drawing" | "pict" => {
                    if let Some(image) = self.parse_image(child) {
                        result.push(DocxInline::Image(image));
                    }
                }
                "footnoteReference" => {
                    if let Some(id) = attr_local(child, "id").and_then(|value| value.parse().ok()) {
                        result.push(DocxInline::FootnoteReference(id));
                    }
                }
                "endnoteReference" => {
                    if let Some(id) = attr_local(child, "id").and_then(|value| value.parse().ok()) {
                        result.push(DocxInline::EndnoteReference(id));
                    }
                }
                "instrText" => self.native.diagnostics.push(Diagnostic::warning(
                    "DOCX-X008",
                    "field instruction text was not emitted as body text",
                )),
                "fldChar" => {}
                "noBreakHyphen" => result.push(DocxInline::Run(DocxRun {
                    text: "\u{2011}".to_owned(),
                    character_style: character_style.clone(),
                    direct: direct.clone(),
                })),
                "softHyphen" => result.push(DocxInline::Run(DocxRun {
                    text: "\u{00ad}".to_owned(),
                    character_style: character_style.clone(),
                    direct: direct.clone(),
                })),
                "continuationSeparator" => {}
                name => self.native.diagnostics.push(Diagnostic::warning(
                    "DOCX-X009",
                    format!("unsupported run child '{name}' was not imported"),
                )),
            }
        }
        Ok(result)
    }

    fn parse_hyperlink(&mut self, hyperlink: XmlNode<'_, '_>) -> Result<DocxInline, DocxError> {
        let href = if let Some(anchor) = attr_local(hyperlink, "anchor") {
            format!("#{}", anchor)
        } else if let Some(id) = attr_local(hyperlink, "id") {
            if let Some(relationship) = self.native.relationships.get(id) {
                if relationship.kind != REL_HYPERLINK {
                    self.native.diagnostics.push(Diagnostic::warning(
                        "DOCX-R002",
                        format!("relationship '{id}' is not a hyperlink relationship"),
                    ));
                }
                if relationship.target_mode.as_deref() == Some("External") {
                    relationship.target.clone()
                } else {
                    resolve_package_target("word", &relationship.target)
                        .map(|target| format!("docx.xhtml#{target}"))
                        .unwrap_or_else(|_| relationship.target.clone())
                }
            } else {
                self.native.diagnostics.push(Diagnostic::warning(
                    "DOCX-R001",
                    format!("hyperlink references missing relationship '{id}'"),
                ));
                format!("#unresolved-{id}")
            }
        } else {
            self.native.diagnostics.push(Diagnostic::warning(
                "DOCX-R003",
                "hyperlink has neither an external relationship nor an anchor",
            ));
            "#unresolved-hyperlink".to_owned()
        };
        Ok(DocxInline::Link {
            href,
            children: self.parse_inline_children(hyperlink)?,
        })
    }

    fn parse_image(&mut self, drawing: XmlNode<'_, '_>) -> Option<DocxImage> {
        let blip = drawing
            .descendants()
            .find(|node| node.is_element() && local_name(*node) == "blip")
            .or_else(|| {
                drawing
                    .descendants()
                    .find(|node| node.is_element() && local_name(*node) == "imagedata")
            });
        let relation_id = blip
            .and_then(|node| {
                attr_local(node, "embed")
                    .or_else(|| attr_local(node, "link"))
                    .or_else(|| attr_local(node, "id"))
            })
            .map(ToOwned::to_owned);
        if let Some(id) = relation_id.as_deref() {
            if self
                .native
                .relationships
                .get(id)
                .is_some_and(|relationship| relationship.kind != REL_IMAGE)
            {
                self.native.diagnostics.push(Diagnostic::warning(
                    "DOCX-M003",
                    format!("drawing relationship '{id}' is not an image relationship"),
                ));
            }
        }
        let target = relation_id
            .as_deref()
            .and_then(|id| self.native.relationships.get(id))
            .map(|relationship| {
                if relationship.target_mode.as_deref() == Some("External") {
                    relationship.target.clone()
                } else {
                    resolve_package_target("word", &relationship.target)
                        .unwrap_or_else(|_| relationship.target.clone())
                }
            })
            .or(relation_id);
        let Some(target) = target else {
            self.native.diagnostics.push(Diagnostic::warning(
                "DOCX-M002",
                "drawing has no resolvable image relationship",
            ));
            return None;
        };
        let doc_properties = drawing
            .descendants()
            .find(|node| node.is_element() && local_name(*node) == "docPr");
        let alt = doc_properties
            .and_then(|node| attr_local(node, "descr").or_else(|| attr_local(node, "title")))
            .unwrap_or_default()
            .to_owned();
        let extent = drawing
            .descendants()
            .find(|node| node.is_element() && local_name(*node) == "extent");
        let width_emu = extent
            .and_then(|node| attr_local(node, "cx"))
            .and_then(|value| value.parse::<i64>().ok());
        let height_emu = extent
            .and_then(|node| attr_local(node, "cy"))
            .and_then(|value| value.parse::<i64>().ok());
        let floating = drawing
            .descendants()
            .any(|node| node.is_element() && local_name(node) == "anchor");
        Some(DocxImage {
            target,
            alt,
            width_emu,
            height_emu,
            floating,
        })
    }

    fn parse_table(&mut self, table: XmlNode<'_, '_>) -> Result<DocxTable, DocxError> {
        let mut result = DocxTable::default();
        for row in table.children().filter(|node| node.is_element()) {
            if local_name(row) != "tr" {
                continue;
            }
            let mut cells = Vec::new();
            for cell in row.children().filter(|node| node.is_element()) {
                if local_name(cell) != "tc" {
                    continue;
                }
                if let Some(properties) = child_named(cell, "tcPr") {
                    if child_named(properties, "gridSpan").is_some()
                        || child_named(properties, "vMerge").is_some()
                    {
                        result.has_merge = true;
                    }
                }
                cells.push(self.parse_blocks(cell)?);
            }
            result.rows.push(cells);
        }
        if result.has_merge {
            self.native.diagnostics.push(Diagnostic::warning(
                "DOCX-T001",
                "merged table cells were preserved in source order; exact Word grid geometry is approximate",
            ));
        }
        Ok(result)
    }

    fn record_non_body_parts(&mut self) {
        let mut recorded = BTreeSet::new();
        for relationship in self.native.relationships.values() {
            let (code, message) = match relationship.kind.as_str() {
                REL_HEADER => (
                    "DOCX-X010",
                    "header part recognized and excluded from ebook body",
                ),
                REL_FOOTER => (
                    "DOCX-X011",
                    "footer part recognized and excluded from ebook body",
                ),
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" => (
                    "DOCX-X012",
                    "comments part recognized and excluded from ebook body",
                ),
                _ => continue,
            };
            if recorded.insert(code) {
                self.native
                    .diagnostics
                    .push(Diagnostic::warning(code, message));
            }
        }
        if self
            .package
            .parts
            .keys()
            .any(|path| path.starts_with("word/comments"))
            && recorded.insert("DOCX-X012")
        {
            self.native.diagnostics.push(Diagnostic::warning(
                "DOCX-X012",
                "comments part recognized and excluded from ebook body",
            ));
        }
    }

    fn decode_semantic(self) -> Result<(Book, Vec<Diagnostic>), DocxError> {
        let mut book = Book::new();
        book.metadata = self.native.metadata.clone();
        let mut loader = MemoryResourceLoader::default();
        let mut resources = Vec::new();
        let mut resource_ids = BTreeMap::new();
        for (path, bytes) in &self.native.images {
            let id = ResourceId::new(resources.len() as u32);
            let media_type = self
                .package
                .content_types
                .get(path)
                .cloned()
                .unwrap_or_else(|| guess_media_type(path, bytes));
            loader.insert(path.clone(), Arc::clone(bytes));
            resources.push(Resource {
                id,
                path: path.clone(),
                media_type: media_type.clone(),
                kind: resource_kind(&media_type, path),
                properties: Vec::new(),
                size: Some(bytes.len() as u64),
            });
            resource_ids.insert(path.clone(), id);
        }
        book.resources = resources;
        let mut state = SemanticDecoder {
            styles: self.native.styles.clone(),
            numbering: self.native.numbering.clone(),
            style_pool: folio_model::StylePool::new(),
            diagnostics: self.native.diagnostics,
            next_node: 0,
            next_anchor: 0,
            anchors: Vec::new(),
            navigation: Vec::new(),
            edges: Vec::new(),
            resource_ids,
            resources: book.resources.clone(),
            loader,
            footnotes: self.native.footnotes,
            endnotes: self.native.endnotes,
        };
        let document_id = DocumentId::new(0);
        let nodes = state.decode_blocks(&self.native.body, document_id)?;
        let title = nodes.iter().find_map(|node| match &node.kind {
            NodeKind::Heading { .. } => Some(node.text_content().trim().to_owned()),
            _ => None,
        });
        book.documents.push(Document {
            id: document_id,
            href: "docx.xhtml".to_owned(),
            media_type: "application/xhtml+xml".to_owned(),
            title,
            nodes,
        });
        if !state.footnotes.is_empty() {
            let note_document = DocumentId::new(book.documents.len() as u32);
            let nodes = state.decode_note_blocks(true, note_document)?;
            book.documents.push(Document {
                id: note_document,
                href: "footnotes.xhtml".to_owned(),
                media_type: "application/xhtml+xml".to_owned(),
                title: Some("Footnotes".to_owned()),
                nodes,
            });
        }
        if !state.endnotes.is_empty() {
            let note_document = DocumentId::new(book.documents.len() as u32);
            let nodes = state.decode_note_blocks(false, note_document)?;
            book.documents.push(Document {
                id: note_document,
                href: "endnotes.xhtml".to_owned(),
                media_type: "application/xhtml+xml".to_owned(),
                title: Some("Endnotes".to_owned()),
                nodes,
            });
        }
        let edges = state.anchor_edges();
        book.resources = state.resources;
        book.styles = state.style_pool;
        book.anchors = state.anchors;
        book.navigation.toc = state.navigation;
        book.navigation.anchor_graph = AnchorGraph { edges };
        book = book.with_resource_loader(Arc::new(state.loader));
        Ok((book, state.diagnostics))
    }
}

fn non_empty_text(node: XmlNode<'_, '_>) -> Option<String> {
    let value = element_text(node).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn guess_media_type(path: &str, bytes: &[u8]) -> String {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png".to_owned()
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg".to_owned()
    } else if bytes.starts_with(b"GIF8") {
        "image/gif".to_owned()
    } else {
        match path
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "png" => "image/png".to_owned(),
            "jpg" | "jpeg" => "image/jpeg".to_owned(),
            "gif" => "image/gif".to_owned(),
            "svg" => "image/svg+xml".to_owned(),
            "webp" => "image/webp".to_owned(),
            _ => "application/octet-stream".to_owned(),
        }
    }
}

fn resource_kind(media_type: &str, path: &str) -> ResourceKind {
    match media_type.to_ascii_lowercase().as_str() {
        "image/png" => ResourceKind::Png,
        "image/jpeg" | "image/jpg" => ResourceKind::Jpeg,
        "image/gif" => ResourceKind::Gif,
        "image/svg+xml" => ResourceKind::Svg,
        _ => match path
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "png" => ResourceKind::Png,
            "jpg" | "jpeg" => ResourceKind::Jpeg,
            "gif" => ResourceKind::Gif,
            "svg" => ResourceKind::Svg,
            _ => ResourceKind::Unknown,
        },
    }
}

struct SemanticDecoder {
    styles: StyleCatalog,
    numbering: NumberingCatalog,
    style_pool: folio_model::StylePool,
    diagnostics: Vec<Diagnostic>,
    next_node: u32,
    next_anchor: u32,
    anchors: Vec<Anchor>,
    navigation: Vec<NavPoint>,
    edges: Vec<AnchorEdge>,
    resource_ids: BTreeMap<String, ResourceId>,
    resources: Vec<Resource>,
    loader: MemoryResourceLoader,
    footnotes: BTreeMap<i32, Vec<DocxBlock>>,
    endnotes: BTreeMap<i32, Vec<DocxBlock>>,
}

impl SemanticDecoder {
    fn allocate_node(&mut self) -> NodeId {
        let id = NodeId::new(self.next_node);
        self.next_node = self.next_node.saturating_add(1);
        id
    }

    fn style_id(&mut self, properties: &StyleProperties) -> StyleId {
        let mut computed = ComputedStyle::default();
        for (key, value) in &properties.values {
            if key != "character-style" {
                computed.properties.insert(key.clone(), value.clone());
            }
        }
        self.style_pool.intern(computed)
    }

    fn default_style(&mut self) -> StyleId {
        self.style_id(&StyleProperties::default())
    }

    fn decode_blocks(
        &mut self,
        blocks: &[DocxBlock],
        document: DocumentId,
    ) -> Result<Vec<Node>, DocxError> {
        let mut result = Vec::new();
        let mut index = 0usize;
        while index < blocks.len() {
            if let DocxBlock::Paragraph(paragraph) = &blocks[index] {
                if paragraph.num_id.is_some()
                    && self
                        .numbering
                        .info(
                            paragraph.num_id.as_deref(),
                            paragraph.ilvl,
                            &mut self.diagnostics,
                        )
                        .is_some()
                {
                    let num_id = paragraph.num_id.clone().unwrap_or_default();
                    let start = index;
                    while index < blocks.len() {
                        let Some(DocxBlock::Paragraph(candidate)) = blocks.get(index) else {
                            break;
                        };
                        if candidate.num_id.as_deref() != Some(num_id.as_str()) {
                            break;
                        }
                        index += 1;
                    }
                    let list = self.decode_list_group(&blocks[start..index], document, &num_id)?;
                    result.push(list);
                    continue;
                }
            }
            result.push(self.decode_block(&blocks[index], document)?);
            index += 1;
        }
        Ok(result)
    }

    fn decode_block(&mut self, block: &DocxBlock, document: DocumentId) -> Result<Node, DocxError> {
        match block {
            DocxBlock::Paragraph(paragraph) => self.decode_paragraph(paragraph, document),
            DocxBlock::Table(table) => self.decode_table(table, document),
        }
    }

    fn decode_list_group(
        &mut self,
        blocks: &[DocxBlock],
        document: DocumentId,
        num_id: &str,
    ) -> Result<Node, DocxError> {
        let first = blocks
            .iter()
            .find_map(|block| match block {
                DocxBlock::Paragraph(paragraph) if paragraph.num_id.as_deref() == Some(num_id) => {
                    self.numbering.info(
                        paragraph.num_id.as_deref(),
                        paragraph.ilvl,
                        &mut self.diagnostics,
                    )
                }
                _ => None,
            })
            .ok_or_else(|| {
                DocxError::Invalid("list group has no numbering definition".to_owned())
            })?;
        let mut paragraphs = Vec::new();
        for block in blocks {
            if let DocxBlock::Paragraph(paragraph) = block {
                paragraphs.push(paragraph.clone());
            }
        }
        let mut position = 0usize;
        let node =
            self.decode_list_level(&paragraphs, &mut position, first.level, num_id, document)?;
        if position < paragraphs.len() {
            self.diagnostics.push(Diagnostic::warning(
                "DOCX-L002",
                "some numbered paragraphs could not be nested exactly and were retained in list order",
            ));
        }
        Ok(node)
    }

    fn decode_list_level(
        &mut self,
        paragraphs: &[DocxParagraph],
        position: &mut usize,
        level: u8,
        num_id: &str,
        document: DocumentId,
    ) -> Result<Node, DocxError> {
        let first_info = paragraphs
            .get(*position)
            .and_then(|paragraph| {
                self.numbering.info(
                    paragraph.num_id.as_deref(),
                    paragraph.ilvl,
                    &mut self.diagnostics,
                )
            })
            .unwrap_or(ListInfo {
                num_id: num_id.to_owned(),
                level,
                ordered: false,
                start: 1,
                level_text: None,
            });
        let mut list_style = StyleProperties::default();
        list_style
            .values
            .insert("list-start".to_owned(), first_info.start.to_string());
        if let Some(level_text) = &first_info.level_text {
            list_style
                .values
                .insert("list-level-text".to_owned(), level_text.clone());
        }
        let list_style_id = self.style_id(&list_style);
        let mut items: Vec<Node> = Vec::new();
        while let Some(paragraph) = paragraphs.get(*position) {
            let Some(info) = self.numbering.info(
                paragraph.num_id.as_deref(),
                paragraph.ilvl,
                &mut self.diagnostics,
            ) else {
                break;
            };
            if info.num_id != num_id || info.level < level {
                break;
            }
            if info.level > level {
                if let Some(last) = items.last_mut() {
                    let nested =
                        self.decode_list_level(paragraphs, position, info.level, num_id, document)?;
                    last.children.push(nested);
                } else {
                    *position += 1;
                }
                continue;
            }
            let paragraph_node = self.decode_paragraph(paragraph, document)?;
            *position += 1;
            let item = Node::new(
                self.allocate_node(),
                NodeKind::ListItem,
                paragraph_node.style,
                vec![paragraph_node],
            )
            .with_semantics(
                SemanticRole::List,
                Default::default(),
                Confidence::Explicit,
            );
            items.push(item);
        }
        Ok(Node::new(
            self.allocate_node(),
            if first_info.ordered {
                NodeKind::OrderedList
            } else {
                NodeKind::UnorderedList
            },
            list_style_id,
            items,
        )
        .with_semantics(SemanticRole::List, Default::default(), Confidence::Explicit))
    }

    fn decode_paragraph(
        &mut self,
        paragraph: &DocxParagraph,
        document: DocumentId,
    ) -> Result<Node, DocxError> {
        let paragraph_style = self.styles.resolve_paragraph(
            paragraph.style_id.as_deref(),
            &paragraph.direct,
            &mut self.diagnostics,
        );
        let style_id = self.style_id(&paragraph_style);
        let heading_level = self
            .styles
            .heading_level(paragraph.style_id.as_deref(), paragraph.outline_level);
        let node_id = self.allocate_node();
        let mut children = self.decode_inlines(
            &paragraph.runs,
            paragraph.style_id.as_deref(),
            &paragraph.direct,
        )?;
        if paragraph
            .direct
            .values
            .get("page-break-before")
            .is_some_and(|value| value == "true")
        {
            children.insert(
                0,
                Node::new(
                    self.allocate_node(),
                    NodeKind::PageBreak,
                    style_id,
                    Vec::new(),
                )
                .with_semantics(
                    SemanticRole::PageBreak,
                    Default::default(),
                    Confidence::Explicit,
                ),
            );
        }
        if paragraph.section_break {
            children.push(
                Node::new(
                    self.allocate_node(),
                    NodeKind::PageBreak,
                    style_id,
                    Vec::new(),
                )
                .with_semantics(
                    SemanticRole::PageBreak,
                    Default::default(),
                    Confidence::Explicit,
                ),
            );
        }
        let mut presentation = PresentationIntent::default();
        if paragraph_style.values.get("text-align") == Some(&"center".to_owned()) {
            presentation = presentation.with(PresentationFeature::Centered);
        }
        if paragraph_style.values.contains_key("margin-left")
            || paragraph_style.values.contains_key("text-indent")
        {
            presentation = presentation.with(PresentationFeature::Indented);
        }
        let (kind, role, confidence) = if let Some(level) = heading_level {
            (
                NodeKind::Heading { level },
                SemanticRole::Heading,
                Confidence::Explicit,
            )
        } else {
            (
                NodeKind::Paragraph,
                SemanticRole::Paragraph,
                Confidence::Explicit,
            )
        };
        let node = Node::new(node_id, kind, style_id, children).with_semantics(
            role,
            presentation,
            confidence,
        );
        for name in &paragraph.bookmarks {
            self.add_anchor(document, node_id, name.clone());
        }
        if let NodeKind::Heading { level } = node.kind {
            let name = format!("docx-heading-{}", node_id.get());
            self.add_anchor(document, node_id, name.clone());
            self.add_navigation(
                NavPoint {
                    label: node.text_content().trim().to_owned(),
                    href: format!("docx.xhtml#{name}"),
                    children: Vec::new(),
                },
                level,
            );
        }
        Ok(node)
    }

    fn decode_inlines(
        &mut self,
        inlines: &[DocxInline],
        paragraph_style: Option<&str>,
        paragraph_direct: &StyleProperties,
    ) -> Result<Vec<Node>, DocxError> {
        let mut result = Vec::new();
        for inline in inlines {
            match inline {
                DocxInline::Run(run) => {
                    if run.text.is_empty() {
                        continue;
                    }
                    let style = self.styles.resolve_run(
                        paragraph_style,
                        run.character_style.as_deref(),
                        paragraph_direct,
                        &run.direct,
                        &mut self.diagnostics,
                    );
                    let style_id = self.style_id(&style);
                    let text = Node::new(
                        self.allocate_node(),
                        NodeKind::Text {
                            value: run.text.clone(),
                        },
                        style_id,
                        Vec::new(),
                    );
                    let text = self.wrap_inline_style(text, &style);
                    result.push(text);
                }
                DocxInline::Tab => result.push(Node::new(
                    self.allocate_node(),
                    NodeKind::Text {
                        value: "\t".to_owned(),
                    },
                    self.default_style(),
                    Vec::new(),
                )),
                DocxInline::PageBreak => result.push(
                    Node::new(
                        self.allocate_node(),
                        NodeKind::PageBreak,
                        self.default_style(),
                        Vec::new(),
                    )
                    .with_semantics(
                        SemanticRole::PageBreak,
                        Default::default(),
                        Confidence::Explicit,
                    ),
                ),
                DocxInline::Link { href, children } => {
                    let child_nodes =
                        self.decode_inlines(children, paragraph_style, paragraph_direct)?;
                    let node_id = self.allocate_node();
                    let href = if href.starts_with('#') {
                        format!("docx.xhtml{href}")
                    } else {
                        href.clone()
                    };
                    self.edges.push(AnchorEdge {
                        source: format!("docx.xhtml#node-{}", node_id.get()),
                        target: href.clone(),
                        relation: AnchorRelation::Link,
                    });
                    result.push(
                        Node::new(
                            node_id,
                            NodeKind::Link { href },
                            self.default_style(),
                            child_nodes,
                        )
                        .with_semantics(
                            SemanticRole::Link,
                            Default::default(),
                            Confidence::Explicit,
                        ),
                    );
                }
                DocxInline::FootnoteReference(id) => {
                    let href = format!("footnotes.xhtml#footnote-{id}");
                    let node_id = self.allocate_node();
                    if !self.footnotes.contains_key(id) {
                        self.diagnostics.push(Diagnostic::warning(
                            "DOCX-N001",
                            format!("unresolved footnote reference {id}"),
                        ));
                    }
                    self.edges.push(AnchorEdge {
                        source: format!("docx.xhtml#node-{}", node_id.get()),
                        target: href.clone(),
                        relation: AnchorRelation::Footnote,
                    });
                    result.push(
                        Node::new(
                            node_id,
                            NodeKind::Footnote { href: Some(href) },
                            self.default_style(),
                            vec![Node::new(
                                self.allocate_node(),
                                NodeKind::Text {
                                    value: format!("[{id}]"),
                                },
                                self.default_style(),
                                Vec::new(),
                            )],
                        )
                        .with_semantics(
                            SemanticRole::Footnote,
                            Default::default(),
                            Confidence::Explicit,
                        ),
                    );
                }
                DocxInline::EndnoteReference(id) => {
                    let href = format!("endnotes.xhtml#endnote-{id}");
                    let node_id = self.allocate_node();
                    if !self.endnotes.contains_key(id) {
                        self.diagnostics.push(Diagnostic::warning(
                            "DOCX-N002",
                            format!("unresolved endnote reference {id}"),
                        ));
                    }
                    self.edges.push(AnchorEdge {
                        source: format!("docx.xhtml#node-{}", node_id.get()),
                        target: href.clone(),
                        relation: AnchorRelation::Endnote,
                    });
                    result.push(
                        Node::new(
                            node_id,
                            NodeKind::Footnote { href: Some(href) },
                            self.default_style(),
                            vec![Node::new(
                                self.allocate_node(),
                                NodeKind::Text {
                                    value: format!("[{id}]"),
                                },
                                self.default_style(),
                                Vec::new(),
                            )],
                        )
                        .with_semantics(
                            SemanticRole::Endnote,
                            Default::default(),
                            Confidence::Explicit,
                        ),
                    );
                }
                DocxInline::Image(image) => {
                    result.push(self.decode_image(image, paragraph_style, paragraph_direct));
                }
            }
        }
        Ok(result)
    }

    fn wrap_inline_style(&mut self, node: Node, style: &StyleProperties) -> Node {
        let style_id = node.style;
        let mut result = node;
        if style
            .values
            .get("vertical-align")
            .is_some_and(|value| value == "super" || value == "sub")
        {
            let tag = if style.values.get("vertical-align") == Some(&"super".to_owned()) {
                "sup"
            } else {
                "sub"
            };
            result = Node::new(
                self.allocate_node(),
                NodeKind::GenericInline {
                    tag: tag.to_owned(),
                },
                style_id,
                vec![result],
            );
        }
        if style
            .values
            .get("font-style")
            .is_some_and(|value| value == "italic")
        {
            result = Node::new(
                self.allocate_node(),
                NodeKind::Emphasis,
                style_id,
                vec![result],
            )
            .with_semantics(
                SemanticRole::Emphasis,
                Default::default(),
                Confidence::Explicit,
            );
        }
        if style
            .values
            .get("font-weight")
            .is_some_and(|value| value == "bold")
        {
            result = Node::new(
                self.allocate_node(),
                NodeKind::Strong,
                style_id,
                vec![result],
            )
            .with_semantics(
                SemanticRole::Strong,
                Default::default(),
                Confidence::Explicit,
            );
        }
        result
    }

    fn decode_image(
        &mut self,
        image: &DocxImage,
        paragraph_style: Option<&str>,
        paragraph_direct: &StyleProperties,
    ) -> Node {
        let mut style =
            self.styles
                .resolve_paragraph(paragraph_style, paragraph_direct, &mut self.diagnostics);
        if let Some(width) = image.width_emu.and_then(emu_to_pt) {
            style.values.insert("width".to_owned(), width);
        }
        if let Some(height) = image.height_emu.and_then(emu_to_pt) {
            style.values.insert("height".to_owned(), height);
        }
        let style_id = self.style_id(&style);
        let Some(resource) = self.resource_ids.get(&image.target).copied() else {
            self.diagnostics.push(Diagnostic::warning(
                "DOCX-M002",
                format!(
                    "image relationship target '{}' has no package media part",
                    image.target
                ),
            ));
            return Node::new(
                self.allocate_node(),
                NodeKind::GenericInline {
                    tag: "image".to_owned(),
                },
                style_id,
                Vec::new(),
            );
        };
        let mut presentation = PresentationIntent::default();
        let confidence = if image.floating {
            self.diagnostics.push(Diagnostic::warning(
                "DOCX-M001",
                "floating/anchored drawing was imported as a compatible in-flow image placement",
            ));
            presentation = presentation.with(PresentationFeature::FixedPosition);
            Confidence::StronglyInferred
        } else {
            Confidence::Explicit
        };
        Node::new(
            self.allocate_node(),
            NodeKind::Image {
                resource,
                alt: image.alt.clone(),
            },
            style_id,
            Vec::new(),
        )
        .with_semantics(SemanticRole::Image, presentation, confidence)
    }

    fn decode_table(&mut self, table: &DocxTable, document: DocumentId) -> Result<Node, DocxError> {
        let mut rows = Vec::new();
        for row in &table.rows {
            let mut cells = Vec::new();
            for cell in row {
                let children = self.decode_blocks(cell, document)?;
                cells.push(Node::new(
                    self.allocate_node(),
                    NodeKind::TableCell,
                    self.default_style(),
                    children,
                ));
            }
            rows.push(Node::new(
                self.allocate_node(),
                NodeKind::TableRow,
                self.default_style(),
                cells,
            ));
        }
        Ok(Node::new(
            self.allocate_node(),
            NodeKind::Table,
            self.default_style(),
            rows,
        )
        .with_semantics(
            SemanticRole::Table,
            Default::default(),
            Confidence::Explicit,
        ))
    }

    fn decode_note_blocks(
        &mut self,
        footnote: bool,
        document: DocumentId,
    ) -> Result<Vec<Node>, DocxError> {
        let notes = if footnote {
            self.footnotes.clone()
        } else {
            self.endnotes.clone()
        };
        let mut result = Vec::new();
        for (id, blocks) in notes {
            let node_id = self.allocate_node();
            let anchor_name = if footnote {
                format!("footnote-{id}")
            } else {
                format!("endnote-{id}")
            };
            self.add_anchor(document, node_id, anchor_name);
            let children = self.decode_blocks(&blocks, document)?;
            let role = if footnote {
                SemanticRole::Footnote
            } else {
                SemanticRole::Endnote
            };
            result.push(
                Node::new(
                    node_id,
                    NodeKind::GenericBlock {
                        tag: "aside".to_owned(),
                    },
                    self.default_style(),
                    children,
                )
                .with_semantics(role, Default::default(), Confidence::Explicit),
            );
        }
        Ok(result)
    }

    fn add_anchor(&mut self, document: DocumentId, node: NodeId, name: String) {
        if name.is_empty()
            || self
                .anchors
                .iter()
                .any(|anchor| anchor.document == document && anchor.name == name)
        {
            return;
        }
        self.anchors.push(Anchor {
            id: AnchorId::new(self.next_anchor),
            document,
            node,
            name,
        });
        self.next_anchor = self.next_anchor.saturating_add(1);
    }

    fn add_navigation(&mut self, point: NavPoint, level: u8) {
        add_navpoint(&mut self.navigation, point, level.max(1));
    }

    fn anchor_edges(&self) -> Vec<AnchorEdge> {
        self.edges.clone()
    }
}

fn add_navpoint(points: &mut Vec<NavPoint>, point: NavPoint, level: u8) {
    if level <= 1 || points.is_empty() {
        points.push(point);
        return;
    }
    if let Some(last) = points.last_mut() {
        add_navpoint(&mut last.children, point, level.saturating_sub(1));
    }
}

fn emu_to_pt(value: i64) -> Option<String> {
    if value <= 0 {
        return None;
    }
    if value % 12_700 == 0 {
        Some(format!("{}pt", value / 12_700))
    } else {
        Some(format!("{:.2}pt", value as f64 / 12_700.0))
    }
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-docx/src/lib.rs"]
mod tests;
