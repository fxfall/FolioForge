//! KFX compatibility model, Ion abstraction, and two-pass writer.
//!
//! Amazon's complete KFX binary/container specification is not public.  This
//! crate therefore keeps the semantic model and Ion-shaped layer explicit and
//! deterministic, while the container writer is a FolioForge compatibility
//! format suitable for inspection and future reference-corpus integration.

use std::collections::BTreeSet;

use folio_model::{
    Book, Diagnostic, MemoryResourceLoader, Metadata, Node, NodeKind, Resource, ResourceId,
    ResourceKind,
};
use folio_normalize::{import_xhtml_documents, XhtmlDocumentInput, XhtmlError};
use serde::Serialize;
use thiserror::Error;

/// Real Amazon `CONT` input frontend. The legacy compatibility container is
/// intentionally kept below for internal fixtures and is never used to
/// classify a formal `.kfx` input.
pub mod amazon;

const CONTAINER_MAGIC: &[u8; 8] = b"FFKFX\0\x01\0";

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct IonSymbol(String);

#[derive(Clone, Debug)]
enum IonValue {
    Null,
    Bool(bool),
    Int(i64),
    String(String),
    Symbol(IonSymbol),
    List(Vec<IonValue>),
    Struct(Vec<(IonSymbol, IonValue)>),
    Annotation {
        annotations: Vec<IonSymbol>,
        value: Box<IonValue>,
    },
}

/// Amazon KFX input adapter. The parser remains the sole owner of Native KFX
/// details; the adapter only maps its result to the neutral contract.
pub struct KfxAdapter;

impl folio_format::FormatAdapter for KfxAdapter {
    fn format(&self) -> folio_input::DetectedFormat {
        folio_input::DetectedFormat::Kfx
    }

    fn support(&self) -> folio_format::FormatSupport {
        folio_format::FormatSupport {
            detect: true,
            inspect: true,
            import: true,
            metadata_read: true,
            metadata_write: true,
            edit: true,
            preview: true,
            ..folio_format::FormatSupport::default()
        }
    }

    fn detect(
        &self,
        source: &folio_input::BookSource,
    ) -> Result<folio_format::DetectionResult, folio_format::FormatError> {
        folio_format::supports_detected(source, [folio_input::DetectedFormat::Kfx])
    }

    fn import(
        &self,
        source: &folio_input::BookSource,
        context: &folio_format::ImportContext,
    ) -> Result<folio_format::ImportedBook, folio_format::FormatError> {
        let path = folio_format::primary_file(source)?;
        let bytes = std::fs::read(path)?;
        let parse_mode = if context.strict {
            amazon::ParseMode::Strict
        } else {
            amazon::ParseMode::Compatible
        };
        let read = amazon::import(&bytes, parse_mode).map_err(|error| match error {
            amazon::AmazonKfxError::ProtectedContent => {
                folio_format::FormatError::AmazonKfxProtected
            }
            other => folio_format::FormatError::AmazonKfx(other.to_string()),
        })?;
        Ok(folio_format::ImportedBook {
            format: folio_input::DetectedFormat::Kfx,
            parser: read.input.parser_version.clone(),
            book: read.book,
            diagnostics: Vec::new(),
            input_loss: read.input.input_loss.clone(),
            text: None,
        })
    }
}

/// Internal FolioForge compatibility-container adapter used only for fixtures
/// and round-trip tests. It is not Amazon KFX.
pub struct FfkfxAdapter;

impl folio_format::FormatAdapter for FfkfxAdapter {
    fn format(&self) -> folio_input::DetectedFormat {
        folio_input::DetectedFormat::Ffkfx
    }

    fn support(&self) -> folio_format::FormatSupport {
        folio_format::FormatSupport {
            detect: true,
            inspect: true,
            import: true,
            metadata_read: true,
            metadata_write: true,
            edit: true,
            preview: true,
            ..folio_format::FormatSupport::default()
        }
    }

    fn detect(
        &self,
        source: &folio_input::BookSource,
    ) -> Result<folio_format::DetectionResult, folio_format::FormatError> {
        folio_format::supports_detected(source, [folio_input::DetectedFormat::Ffkfx])
    }

    fn import(
        &self,
        source: &folio_input::BookSource,
        _context: &folio_format::ImportContext,
    ) -> Result<folio_format::ImportedBook, folio_format::FormatError> {
        let path = folio_format::primary_file(source)?;
        let bytes = std::fs::read(path)?;
        let read = import(&bytes)
            .map_err(|error| folio_format::FormatError::Invalid(error.to_string()))?;
        Ok(folio_format::ImportedBook {
            format: folio_input::DetectedFormat::Ffkfx,
            parser: "folio-kfx-ffkfx/1".to_owned(),
            book: read.book,
            diagnostics: read.diagnostics,
            input_loss: Vec::new(),
            text: None,
        })
    }
}

impl IonValue {
    fn to_text(&self) -> String {
        match self {
            Self::Null => "null.null".to_owned(),
            Self::Bool(value) => value.to_string(),
            Self::Int(value) => value.to_string(),
            Self::String(value) => quote(value),
            Self::Symbol(symbol) => ion_symbol_text(&symbol.0),
            Self::List(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(Self::to_text)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Self::Struct(fields) => {
                let fields = fields
                    .iter()
                    .map(|(key, value)| format!("{}:{}", ion_symbol_text(&key.0), value.to_text()))
                    .collect::<Vec<_>>()
                    .join(",");
                format!("{{{fields}}}")
            }
            Self::Annotation { annotations, value } => {
                let prefix = annotations
                    .iter()
                    .map(|symbol| ion_symbol_text(&symbol.0))
                    .collect::<Vec<_>>()
                    .join("::");
                format!("{prefix}::{}", value.to_text())
            }
        }
    }
}

#[derive(Clone, Debug)]
struct SurveyPass {
    symbols: Vec<String>,
    navigation: Vec<NavigationRecord>,
    positions: Vec<Position>,
}

#[derive(Clone, Debug)]
struct NavigationRecord {
    label: String,
    href: String,
    children: Vec<NavigationRecord>,
}

#[derive(Clone, Debug)]
struct Position {
    fragment_id: u32,
    ordinal: u32,
}

#[derive(Clone, Debug)]
struct KfxFragment {
    id: u32,
    document: u32,
    text: String,
    html: String,
    style_ids: Vec<u32>,
    resource_ids: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct KfxOptions {
    pub deterministic: bool,
}

impl Default for KfxOptions {
    fn default() -> Self {
        Self {
            deterministic: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct KfxArtifact {
    pub bytes: Vec<u8>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompatibilitySection {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Error)]
pub enum KfxError {
    #[error("KFX container is truncated")]
    Truncated,
    #[error("KFX container has invalid magic")]
    InvalidMagic,
    #[error("KFX section is not valid UTF-8")]
    InvalidUtf8,
    #[error("KFX container length calculation overflowed")]
    Overflow,
    #[error("KFX container has too many sections")]
    TooManySections,
    #[error("KFX section name must be at most four ASCII bytes")]
    InvalidSectionName,
    #[error("KFX container is missing required section: {0}")]
    MissingSection(String),
    #[error("KFX container has duplicate section: {0}")]
    DuplicateSection(String),
    #[error("KFX container has trailing bytes")]
    TrailingData,
    #[error("KFX reference is invalid: {0}")]
    InvalidReference(String),
    #[error("invalid Ion value: {0}")]
    InvalidIon(String),
    #[error("KFX semantic serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("KFX XHTML import failed: {0}")]
    Xhtml(#[from] XhtmlError),
    #[error("KFX input is not a FolioForge compatibility container")]
    UnsupportedReferenceContainer,
    #[error("KFX resource data is invalid hex")]
    InvalidResourceData,
}

#[derive(Clone, Debug)]
pub struct KfxImportReport {
    pub book: Book,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn convert(book: &Book, options: &KfxOptions) -> Result<KfxArtifact, KfxError> {
    // The first pass gathers stable symbol, navigation, and position references.
    let survey = survey(book);
    // The second pass serializes in source order for deterministic output.
    let sections = synthesis(book, survey)?;
    let bytes = write_container(&sections, options.deterministic)?;
    Ok(KfxArtifact {
        bytes,
        diagnostics: Vec::new(),
    })
}

fn survey(book: &Book) -> SurveyPass {
    let mut symbols = BTreeSet::new();
    let mut positions = Vec::new();
    for (document_index, document) in book.documents.iter().enumerate() {
        symbols.insert(document.href.clone());
        collect_symbols(&document.nodes, &mut symbols);
        positions.push(Position {
            fragment_id: document_index as u32,
            ordinal: document_index as u32,
        });
    }
    for anchor in &book.anchors {
        symbols.insert(anchor.name.clone());
    }
    let mut navigation = Vec::new();
    collect_navigation(&book.navigation.toc, &mut navigation);
    collect_navigation(&book.navigation.landmarks, &mut navigation);
    collect_navigation(&book.navigation.page_list, &mut navigation);
    collect_navigation_symbols(&navigation, &mut symbols);
    SurveyPass {
        symbols: symbols.into_iter().collect(),
        navigation,
        positions,
    }
}

fn synthesis(book: &Book, survey: SurveyPass) -> Result<Vec<(String, IonValue)>, KfxError> {
    let mut fragments = Vec::new();
    for (index, document) in book.documents.iter().enumerate() {
        let mut text = String::new();
        let mut html = String::new();
        let mut styles = BTreeSet::new();
        let mut resources = BTreeSet::new();
        for node in &document.nodes {
            text.push_str(&node.text_content());
            render_fragment_node(node, book, &mut html);
            collect_node_ids(node, &mut styles, &mut resources);
        }
        fragments.push(KfxFragment {
            id: index as u32,
            document: document.id.get(),
            text: text.trim().to_owned(),
            html,
            style_ids: styles.into_iter().collect(),
            resource_ids: resources.into_iter().collect(),
        });
    }
    let metadata = IonValue::Struct(vec![
        (
            IonSymbol("title".to_owned()),
            IonValue::String(book.metadata.display_title().to_owned()),
        ),
        (
            IonSymbol("language".to_owned()),
            book.metadata
                .language
                .clone()
                .map(IonValue::String)
                .unwrap_or(IonValue::Null),
        ),
        (
            IonSymbol("creator".to_owned()),
            IonValue::List(
                book.metadata
                    .creators
                    .iter()
                    .cloned()
                    .map(IonValue::String)
                    .collect(),
            ),
        ),
    ]);
    let content = IonValue::List(
        fragments
            .iter()
            .map(|fragment| {
                IonValue::Struct(vec![
                    (
                        IonSymbol("id".to_owned()),
                        IonValue::Int(i64::from(fragment.id)),
                    ),
                    (
                        IonSymbol("document".to_owned()),
                        IonValue::Int(i64::from(fragment.document)),
                    ),
                    (
                        IonSymbol("text".to_owned()),
                        IonValue::String(fragment.text.clone()),
                    ),
                    (
                        IonSymbol("html".to_owned()),
                        IonValue::String(fragment.html.clone()),
                    ),
                    (
                        IonSymbol("styles".to_owned()),
                        IonValue::List(
                            fragment
                                .style_ids
                                .iter()
                                .map(|id| IonValue::Int(i64::from(*id)))
                                .collect(),
                        ),
                    ),
                    (
                        IonSymbol("resources".to_owned()),
                        IonValue::List(
                            fragment
                                .resource_ids
                                .iter()
                                .map(|id| IonValue::Int(i64::from(*id)))
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect(),
    );
    let resources = IonValue::List(
        book.resources
            .iter()
            .map(|resource| {
                let data = book
                    .load_resource(resource.id, resource.size.or(Some(256 << 20)))
                    .ok()
                    .map(|bytes| IonValue::String(hex_encode(&bytes)))
                    .unwrap_or(IonValue::Null);
                IonValue::Struct(vec![
                    (
                        IonSymbol("id".to_owned()),
                        IonValue::Int(i64::from(resource.id.get())),
                    ),
                    (
                        IonSymbol("path".to_owned()),
                        IonValue::String(resource.path.clone()),
                    ),
                    (
                        IonSymbol("media_type".to_owned()),
                        IonValue::String(resource.media_type.clone()),
                    ),
                    (
                        IonSymbol("kind".to_owned()),
                        IonValue::String(format!("{:?}", resource.kind)),
                    ),
                    (
                        IonSymbol("properties".to_owned()),
                        IonValue::List(
                            resource
                                .properties
                                .iter()
                                .cloned()
                                .map(IonValue::String)
                                .collect(),
                        ),
                    ),
                    (IonSymbol("data_hex".to_owned()), data),
                ])
            })
            .collect(),
    );
    let styles = IonValue::List(
        book.styles
            .iter()
            .map(|(id, style)| {
                let properties = IonValue::Struct(
                    style
                        .properties
                        .iter()
                        .map(|(key, value)| {
                            (IonSymbol(key.clone()), IonValue::String(value.clone()))
                        })
                        .collect(),
                );
                IonValue::Struct(vec![
                    (
                        IonSymbol("id".to_owned()),
                        IonValue::Int(i64::from(id.get())),
                    ),
                    (IonSymbol("properties".to_owned()), properties),
                ])
            })
            .collect(),
    );
    let navigation = IonValue::List(
        survey
            .navigation
            .iter()
            .map(navigation_record_to_ion)
            .collect(),
    );
    let positions = IonValue::List(
        survey
            .positions
            .iter()
            .map(|position| {
                IonValue::Struct(vec![
                    (
                        IonSymbol("fragment".to_owned()),
                        IonValue::Int(i64::from(position.fragment_id)),
                    ),
                    (
                        IonSymbol("ordinal".to_owned()),
                        IonValue::Int(i64::from(position.ordinal)),
                    ),
                ])
            })
            .collect(),
    );
    let symbols = IonValue::List(
        survey
            .symbols
            .iter()
            .cloned()
            .map(|symbol| IonValue::Symbol(IonSymbol(symbol)))
            .collect(),
    );
    let sections = vec![
        ("META".to_owned(), metadata),
        ("CONT".to_owned(), content),
        ("SYMB".to_owned(), symbols),
        ("STYL".to_owned(), styles),
        ("RES ".to_owned(), resources),
        ("NAV ".to_owned(), navigation),
        ("POS ".to_owned(), positions),
    ];
    Ok(sections)
}

/// Import FolioForge's KFX compatibility container through the shared
/// semantic XHTML normalizer. Amazon CONT files are handled by the separate
/// `amazon` module; this function never guesses their private content model.
pub fn import(bytes: &[u8]) -> Result<KfxImportReport, KfxError> {
    if bytes.starts_with(b"CONT") {
        return Err(KfxError::UnsupportedReferenceContainer);
    }
    let sections = inspect_compatibility_container(bytes)?;
    let by_name = sections
        .iter()
        .map(|section| (section.name.as_str(), section.value.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let metadata_value = parse_ion(
        by_name
            .get("META")
            .ok_or_else(|| KfxError::MissingSection("META".to_owned()))?,
    )?;
    let metadata_fields = require_struct(&metadata_value, "metadata")?;
    let mut metadata = Metadata {
        title: ion_string_field(metadata_fields, "title"),
        language: ion_string_field(metadata_fields, "language"),
        ..Metadata::default()
    };
    for creator in ion_string_list_field(metadata_fields, "creator") {
        metadata.add_author(creator);
    }
    let content = parse_ion(
        by_name
            .get("CONT")
            .ok_or_else(|| KfxError::MissingSection("CONT".to_owned()))?,
    )?;
    let mut documents = Vec::new();
    if let IonValue::List(items) = content {
        for item in &items {
            let fields = require_struct(item, "content fragment")?;
            let document = ion_u32_field(fields, "document").unwrap_or(documents.len() as u32);
            let text = ion_string_field(fields, "text").unwrap_or_default();
            let html = ion_string_field(fields, "html")
                .unwrap_or_else(|| format!("<p>{}</p>", escape_xml(&text)));
            documents.push(XhtmlDocumentInput {
                href: format!("document-{document}.xhtml"),
                media_type: "application/xhtml+xml".to_owned(),
                content: format!(
                    "<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><body>{html}</body></html>"
                ),
            });
        }
    } else {
        return Err(KfxError::InvalidReference(
            "KFX content is not a list".to_owned(),
        ));
    }
    if documents.is_empty() {
        documents.push(XhtmlDocumentInput {
            href: "book.xhtml".to_owned(),
            media_type: "application/xhtml+xml".to_owned(),
            content: "<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><body></body></html>".to_owned(),
        });
    }

    let resources_value = parse_ion(
        by_name
            .get("RES")
            .ok_or_else(|| KfxError::MissingSection("RES".to_owned()))?,
    )?;
    let mut resources = Vec::new();
    let mut loader = MemoryResourceLoader::default();
    if let IonValue::List(items) = resources_value {
        for item in &items {
            let fields = require_struct(item, "resource")?;
            let id = ion_u32_field(fields, "id")
                .ok_or_else(|| KfxError::InvalidReference("resource has no id".to_owned()))?;
            let path = ion_string_field(fields, "path")
                .ok_or_else(|| KfxError::InvalidReference("resource has no path".to_owned()))?;
            let media_type = ion_string_field(fields, "media_type").ok_or_else(|| {
                KfxError::InvalidReference("resource has no media_type".to_owned())
            })?;
            let kind = ion_string_field(fields, "kind")
                .as_deref()
                .map(parse_resource_kind)
                .unwrap_or_else(|| resource_kind_from_media_type(&media_type));
            if let Some(hex) = ion_string_field(fields, "data_hex") {
                let data = hex_decode(&hex)?;
                loader.insert(path.clone(), data);
            }
            resources.push(Resource {
                id: ResourceId::new(id),
                path,
                media_type,
                kind,
                properties: ion_string_list_field(fields, "properties"),
                size: None,
            });
        }
    } else {
        return Err(KfxError::InvalidReference(
            "KFX resources are not a list".to_owned(),
        ));
    }

    let mut report =
        import_xhtml_documents(metadata, &documents, resources, std::sync::Arc::new(loader))?;
    if let Some(navigation_text) = by_name.get("NAV") {
        if let Ok(IonValue::List(items)) = parse_ion(navigation_text) {
            report.book.navigation.toc = parse_navigation_records(&items);
        }
    }
    Ok(KfxImportReport {
        book: report.book,
        diagnostics: report.diagnostics,
    })
}

fn ion_string_field(fields: &[(IonSymbol, IonValue)], name: &str) -> Option<String> {
    match struct_value(fields, name) {
        Some(IonValue::String(value)) => Some(value.clone()),
        _ => None,
    }
}

fn ion_string_list_field(fields: &[(IonSymbol, IonValue)], name: &str) -> Vec<String> {
    match struct_value(fields, name) {
        Some(IonValue::List(values)) => values
            .iter()
            .filter_map(|value| match value {
                IonValue::String(value) => Some(value.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn ion_u32_field(fields: &[(IonSymbol, IonValue)], name: &str) -> Option<u32> {
    match struct_value(fields, name) {
        Some(IonValue::Int(value)) => u32::try_from(*value).ok(),
        _ => None,
    }
}

fn parse_resource_kind(value: &str) -> ResourceKind {
    match value {
        "Jpeg" => ResourceKind::Jpeg,
        "Png" => ResourceKind::Png,
        "Gif" => ResourceKind::Gif,
        "Svg" => ResourceKind::Svg,
        "Font" => ResourceKind::Font,
        "Stylesheet" => ResourceKind::Stylesheet,
        "Audio" => ResourceKind::Audio,
        _ => ResourceKind::Unknown,
    }
}

fn resource_kind_from_media_type(media_type: &str) -> ResourceKind {
    if media_type.eq_ignore_ascii_case("image/jpeg") {
        ResourceKind::Jpeg
    } else if media_type.eq_ignore_ascii_case("image/png") {
        ResourceKind::Png
    } else if media_type.eq_ignore_ascii_case("image/gif") {
        ResourceKind::Gif
    } else if media_type.eq_ignore_ascii_case("image/svg+xml") {
        ResourceKind::Svg
    } else if media_type.starts_with("font/") || media_type.contains("font") {
        ResourceKind::Font
    } else if media_type == "text/css" {
        ResourceKind::Stylesheet
    } else if media_type.starts_with("audio/") {
        ResourceKind::Audio
    } else {
        ResourceKind::Unknown
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_decode(value: &str) -> Result<Vec<u8>, KfxError> {
    if !value.len().is_multiple_of(2) {
        return Err(KfxError::InvalidResourceData);
    }
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let high = hex_digit(pair[0]).ok_or(KfxError::InvalidResourceData)?;
        let low = hex_digit(pair[1]).ok_or(KfxError::InvalidResourceData)?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn render_fragment_node(node: &Node, book: &Book, output: &mut String) {
    let style = book
        .styles
        .get(node.style)
        .map(folio_style::inline_css)
        .unwrap_or_default();
    let style = if style.is_empty() {
        String::new()
    } else {
        format!(" style=\"{}\"", escape_xml(&style))
    };
    match &node.kind {
        NodeKind::Text { value } => output.push_str(&escape_xml(value)),
        NodeKind::Section => render_fragment_wrapped("section", node, book, output, &style),
        NodeKind::Heading { level } => {
            let level = (*level).clamp(1, 6);
            output.push_str(&format!("<h{level}{style}>"));
            render_fragment_children(node, book, output);
            output.push_str(&format!("</h{level}>"));
        }
        NodeKind::Paragraph => render_fragment_wrapped("p", node, book, output, &style),
        NodeKind::Emphasis => render_fragment_wrapped("em", node, book, output, &style),
        NodeKind::Strong => render_fragment_wrapped("strong", node, book, output, &style),
        NodeKind::BlockQuote => render_fragment_wrapped("blockquote", node, book, output, &style),
        NodeKind::Code => render_fragment_wrapped("code", node, book, output, &style),
        NodeKind::Preformatted => render_fragment_wrapped("pre", node, book, output, &style),
        NodeKind::OrderedList => render_fragment_wrapped("ol", node, book, output, &style),
        NodeKind::UnorderedList => render_fragment_wrapped("ul", node, book, output, &style),
        NodeKind::ListItem => render_fragment_wrapped("li", node, book, output, &style),
        NodeKind::Table => render_fragment_wrapped("table", node, book, output, &style),
        NodeKind::TableRow => render_fragment_wrapped("tr", node, book, output, &style),
        NodeKind::TableCell => render_fragment_wrapped("td", node, book, output, &style),
        NodeKind::Link { href } => {
            output.push_str(&format!("<a href=\"{}\"{}>", escape_xml(href), style));
            render_fragment_children(node, book, output);
            output.push_str("</a>");
        }
        NodeKind::Anchor { name } => {
            if !book
                .anchors
                .iter()
                .any(|anchor| anchor.node == node.id && anchor.name == *name)
            {
                output.push_str(&format!("<a id=\"{}\"></a>", escape_xml(name)));
            }
            render_fragment_children(node, book, output);
        }
        NodeKind::Image { resource, alt } => {
            if let Some(resource) = book.resource(*resource) {
                output.push_str(&format!(
                    "<img src=\"{}\" alt=\"{}\"{} />",
                    escape_xml(&resource.path),
                    escape_xml(alt),
                    style
                ));
            } else {
                output.push_str(&escape_xml(alt));
            }
        }
        NodeKind::Svg { resource, alt } => {
            if let Some(resource) = resource.as_ref().and_then(|id| book.resource(*id)) {
                output.push_str(&format!(
                    "<img src=\"{}\" alt=\"{}\"{} />",
                    escape_xml(&resource.path),
                    escape_xml(alt),
                    style
                ));
            } else {
                output.push_str(&escape_xml(alt));
            }
        }
        NodeKind::Ruby => render_fragment_wrapped("ruby", node, book, output, &style),
        NodeKind::Math { mathml, .. } => output.push_str(mathml),
        NodeKind::Footnote { href } => {
            output.push_str(&format!(
                "<sup{}><a epub:type=\"noteref\" href=\"{}\">",
                style,
                escape_xml(href.as_deref().unwrap_or("#"))
            ));
            render_fragment_children(node, book, output);
            output.push_str("</a></sup>");
        }
        NodeKind::PageBreak => output.push_str("<div class=\"ff-pagebreak\"></div>"),
        NodeKind::Inline => render_fragment_wrapped("span", node, book, output, &style),
        NodeKind::GenericInline { tag } => {
            render_fragment_wrapped(safe_inline_tag(tag), node, book, output, &style)
        }
        NodeKind::GenericBlock { .. } => render_fragment_wrapped("div", node, book, output, &style),
    }
}

fn safe_inline_tag(tag: &str) -> &str {
    match tag.to_ascii_lowercase().as_str() {
        "abbr" | "b" | "cite" | "em" | "i" | "kbd" | "q" | "rb" | "rp" | "rt" | "s" | "small"
        | "span" | "strong" | "sub" | "sup" | "time" | "var" => tag,
        _ => "span",
    }
}

fn render_fragment_wrapped(tag: &str, node: &Node, book: &Book, output: &mut String, style: &str) {
    output.push_str(&format!("<{tag}{style}>"));
    render_fragment_children(node, book, output);
    output.push_str(&format!("</{tag}>"));
}

fn render_fragment_children(node: &Node, book: &Book, output: &mut String) {
    for child in &node.children {
        render_fragment_node(child, book, output);
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
}

fn write_container(
    sections: &[(String, IonValue)],
    deterministic: bool,
) -> Result<Vec<u8>, KfxError> {
    let section_count = u32::try_from(sections.len()).map_err(|_| KfxError::TooManySections)?;
    let mut output = Vec::new();
    output.extend_from_slice(CONTAINER_MAGIC);
    output.extend_from_slice(&section_count.to_be_bytes());
    for (name, value) in sections {
        if name.len() > 4 || !name.is_ascii() {
            return Err(KfxError::InvalidSectionName);
        }
        let mut section_name = [b' '; 4];
        let name_bytes = name.as_bytes();
        section_name[..name_bytes.len().min(4)]
            .copy_from_slice(&name_bytes[..name_bytes.len().min(4)]);
        let text = value.to_text();
        let bytes = text.as_bytes();
        let byte_length = u64::try_from(bytes.len()).map_err(|_| KfxError::Overflow)?;
        output.extend_from_slice(&section_name);
        output.extend_from_slice(&byte_length.to_be_bytes());
        output.extend_from_slice(bytes);
    }
    if !deterministic {
        // Timestamps are intentionally not introduced yet: keeping this path
        // byte-identical makes non-deterministic mode safe until a reference
        // corpus establishes its semantics.
    }
    Ok(output)
}

/// Inspects the explicitly non-Amazon FolioForge compatibility container.
/// Amazon KFX Ion input is handled by [`amazon::inspect`].
pub fn inspect_compatibility_container(
    bytes: &[u8],
) -> Result<Vec<CompatibilitySection>, KfxError> {
    if bytes.len() < CONTAINER_MAGIC.len() + 4 {
        return Err(KfxError::Truncated);
    }
    if &bytes[..CONTAINER_MAGIC.len()] != CONTAINER_MAGIC {
        return Err(KfxError::InvalidMagic);
    }
    let count_offset = CONTAINER_MAGIC.len();
    let count = u32::from_be_bytes(
        bytes[count_offset..count_offset + 4]
            .try_into()
            .map_err(|_| KfxError::Truncated)?,
    ) as usize;
    if count > 1_000_000 {
        return Err(KfxError::TooManySections);
    }
    let mut cursor = count_offset + 4;
    let mut sections = Vec::with_capacity(count);
    for _ in 0..count {
        let header_end = cursor.checked_add(12).ok_or(KfxError::Overflow)?;
        let header = bytes.get(cursor..header_end).ok_or(KfxError::Truncated)?;
        if !header[..4].is_ascii() {
            return Err(KfxError::InvalidSectionName);
        }
        let name = String::from_utf8_lossy(&header[..4]).trim().to_owned();
        let length = usize::try_from(u64::from_be_bytes(
            header[4..12].try_into().map_err(|_| KfxError::Truncated)?,
        ))
        .map_err(|_| KfxError::Overflow)?;
        cursor = header_end;
        let end = cursor.checked_add(length).ok_or(KfxError::Overflow)?;
        let value = std::str::from_utf8(bytes.get(cursor..end).ok_or(KfxError::Truncated)?)
            .map_err(|_| KfxError::InvalidUtf8)?
            .to_owned();
        parse_ion(&value)?;
        sections.push(CompatibilitySection { name, value });
        cursor = end;
    }
    if cursor != bytes.len() {
        return Err(KfxError::TrailingData);
    }
    Ok(sections)
}

pub fn validate_container(bytes: &[u8]) -> Result<(), KfxError> {
    let sections = inspect_compatibility_container(bytes)?;
    let mut by_name = std::collections::BTreeMap::new();
    for section in &sections {
        if by_name.insert(section.name.clone(), section).is_some() {
            return Err(KfxError::DuplicateSection(section.name.clone()));
        }
    }
    for required in ["META", "CONT", "SYMB", "STYL", "RES", "NAV", "POS"] {
        if !by_name.contains_key(required) {
            return Err(KfxError::MissingSection(required.to_owned()));
        }
    }

    let metadata = parse_ion(&by_name["META"].value)?;
    let metadata_fields = require_struct(&metadata, "metadata")?;
    if !matches!(
        struct_value(metadata_fields, "title"),
        Some(IonValue::String(value)) if !value.is_empty()
    ) {
        return Err(KfxError::InvalidReference(
            "metadata title is missing or not a string".to_owned(),
        ));
    }
    if !matches!(
        struct_value(metadata_fields, "language"),
        Some(IonValue::Null | IonValue::String(_))
    ) {
        return Err(KfxError::InvalidReference(
            "metadata language is not a string or null".to_owned(),
        ));
    }
    if !matches!(
        struct_value(metadata_fields, "creator"),
        Some(IonValue::List(values)) if values.iter().all(|value| matches!(value, IonValue::String(_)))
    ) {
        return Err(KfxError::InvalidReference(
            "metadata creator is not a string list".to_owned(),
        ));
    }

    let content = parse_ion(&by_name["CONT"].value)?;
    let styles = parse_ion(&by_name["STYL"].value)?;
    let resources = parse_ion(&by_name["RES"].value)?;
    let positions = parse_ion(&by_name["POS"].value)?;
    let navigation = parse_ion(&by_name["NAV"].value)?;
    let symbols = parse_ion(&by_name["SYMB"].value)?;

    let content_items = require_list(&content, "content fragments")?;
    let style_items = require_list(&styles, "styles")?;
    let resource_items = require_list(&resources, "resources")?;
    let position_items = require_list(&positions, "positions")?;
    let navigation_items = require_list(&navigation, "navigation")?;
    let symbol_items = require_list(&symbols, "symbol table")?;
    let fragment_ids = list_struct_ids(&content, "id", "content fragments")?;
    let style_ids = list_struct_ids(&styles, "id", "styles")?;
    let resource_ids = list_struct_ids(&resources, "id", "resources")?;
    let position_ids = list_struct_ids(&positions, "fragment", "positions")?;
    if symbol_items
        .iter()
        .any(|value| !matches!(value, IonValue::Symbol(_)))
    {
        return Err(KfxError::InvalidReference(
            "symbol table contains a non-symbol".to_owned(),
        ));
    }
    if navigation_items
        .iter()
        .any(|value| !valid_navigation_record(value))
    {
        return Err(KfxError::InvalidReference(
            "navigation contains an invalid record".to_owned(),
        ));
    }
    for item in style_items {
        let fields = require_struct(item, "styles")?;
        let properties = struct_value(fields, "properties")
            .ok_or_else(|| KfxError::InvalidReference("style has no properties".to_owned()))?;
        require_struct(properties, "style properties")?;
    }
    for item in resource_items {
        let fields = require_struct(item, "resources")?;
        require_string_field(fields, "path", "resource")?;
        require_string_field(fields, "media_type", "resource")?;
    }
    for item in position_items {
        let fields = require_struct(item, "positions")?;
        require_integer_field(fields, "ordinal", "position")?;
    }
    for item in content_items {
        let fields = require_struct(item, "content fragment")?;
        require_integer_field(fields, "document", "content fragment")?;
        require_string_field(fields, "text", "content fragment")?;
        let style_refs = struct_value(fields, "styles")
            .ok_or_else(|| KfxError::InvalidReference("fragment has no styles".to_owned()))?;
        let resource_refs = struct_value(fields, "resources")
            .ok_or_else(|| KfxError::InvalidReference("fragment has no resources".to_owned()))?;
        validate_integer_refs(style_refs, &style_ids, "styles")?;
        validate_integer_refs(resource_refs, &resource_ids, "resources")?;
    }
    if position_ids.iter().any(|id| !fragment_ids.contains(id)) {
        return Err(KfxError::InvalidReference(
            "position refers to a missing fragment".to_owned(),
        ));
    }
    Ok(())
}

fn require_list<'a>(value: &'a IonValue, description: &str) -> Result<&'a [IonValue], KfxError> {
    match value {
        IonValue::List(values) => Ok(values),
        _ => Err(KfxError::InvalidReference(format!(
            "{description} are not a list"
        ))),
    }
}

fn require_struct<'a>(
    value: &'a IonValue,
    description: &str,
) -> Result<&'a [(IonSymbol, IonValue)], KfxError> {
    match value {
        IonValue::Struct(fields) => Ok(fields),
        _ => Err(KfxError::InvalidReference(format!(
            "{description} are not a struct"
        ))),
    }
}

fn require_integer_field(
    fields: &[(IonSymbol, IonValue)],
    field: &str,
    description: &str,
) -> Result<i64, KfxError> {
    struct_value(fields, field)
        .and_then(ion_int)
        .ok_or_else(|| KfxError::InvalidReference(format!("{description} has no integer {field}")))
}

fn require_string_field<'a>(
    fields: &'a [(IonSymbol, IonValue)],
    field: &str,
    description: &str,
) -> Result<&'a str, KfxError> {
    match struct_value(fields, field) {
        Some(IonValue::String(value)) => Ok(value),
        _ => Err(KfxError::InvalidReference(format!(
            "{description} has no string {field}"
        ))),
    }
}

fn validate_integer_refs(
    value: &IonValue,
    known_ids: &std::collections::BTreeSet<i64>,
    description: &str,
) -> Result<(), KfxError> {
    let values = require_list(value, description)?;
    for value in values {
        let id = ion_int(value).ok_or_else(|| {
            KfxError::InvalidReference(format!("{description} contains a non-integer reference"))
        })?;
        if !known_ids.contains(&id) {
            return Err(KfxError::InvalidReference(format!(
                "{description} reference {id} is missing"
            )));
        }
    }
    Ok(())
}

fn list_struct_ids(
    value: &IonValue,
    field: &str,
    description: &str,
) -> Result<std::collections::BTreeSet<i64>, KfxError> {
    let IonValue::List(items) = value else {
        return Err(KfxError::InvalidReference(format!(
            "{description} are not a list"
        )));
    };
    let mut ids = std::collections::BTreeSet::new();
    for item in items {
        let IonValue::Struct(fields) = item else {
            return Err(KfxError::InvalidReference(format!(
                "{description} contain a non-struct"
            )));
        };
        let Some(id) = struct_value(fields, field).and_then(ion_int) else {
            return Err(KfxError::InvalidReference(format!(
                "{description} contain an item without {field}"
            )));
        };
        if !ids.insert(id) {
            return Err(KfxError::InvalidReference(format!(
                "{description} contain duplicate {field} {id}"
            )));
        }
    }
    Ok(ids)
}

fn struct_value<'a>(fields: &'a [(IonSymbol, IonValue)], name: &str) -> Option<&'a IonValue> {
    fields
        .iter()
        .find(|(key, _)| key.0 == name)
        .map(|(_, value)| value)
}

fn ion_int(value: &IonValue) -> Option<i64> {
    match value {
        IonValue::Int(value) => Some(*value),
        _ => None,
    }
}

fn parse_ion(value: &str) -> Result<IonValue, KfxError> {
    let mut parser = IonParser {
        chars: value.chars().collect(),
        position: 0,
    };
    let parsed = parser.parse_value()?;
    parser.skip_whitespace();
    if parser.position != parser.chars.len() {
        return Err(KfxError::InvalidIon("trailing data".to_owned()));
    }
    Ok(parsed)
}

struct IonParser {
    chars: Vec<char>,
    position: usize,
}

impl IonParser {
    fn parse_value(&mut self) -> Result<IonValue, KfxError> {
        self.skip_whitespace();
        let mut annotations = Vec::new();
        loop {
            let start = self.position;
            let token = match self.peek() {
                Some('"') => self.parse_string()?,
                Some('\'') => self.parse_symbol()?,
                Some('[') | Some('{') => break,
                Some(_) => self.parse_token()?,
                None => break,
            };
            if self.consume_annotation_separator() {
                annotations.push(IonSymbol(token));
            } else {
                self.position = start;
                break;
            }
        }
        let value = match self.peek() {
            Some('"') => IonValue::String(self.parse_string()?),
            Some('\'') => IonValue::Symbol(IonSymbol(self.parse_symbol()?)),
            Some('[') => self.parse_list()?,
            Some('{') => self.parse_struct()?,
            Some(_) => self.parse_atom()?,
            None => return Err(KfxError::InvalidIon("value is missing".to_owned())),
        };
        if annotations.is_empty() {
            Ok(value)
        } else {
            Ok(IonValue::Annotation {
                annotations,
                value: Box::new(value),
            })
        }
    }

    fn parse_atom(&mut self) -> Result<IonValue, KfxError> {
        let token = self.parse_token()?;
        self.atom_from_token(token)
    }

    fn atom_from_token(&self, token: String) -> Result<IonValue, KfxError> {
        match token.as_str() {
            "null.null" => Ok(IonValue::Null),
            "true" => Ok(IonValue::Bool(true)),
            "false" => Ok(IonValue::Bool(false)),
            _ => match token.parse::<i64>() {
                Ok(value) => Ok(IonValue::Int(value)),
                Err(_) => Ok(IonValue::Symbol(IonSymbol(token))),
            },
        }
    }

    fn parse_list(&mut self) -> Result<IonValue, KfxError> {
        self.expect('[')?;
        let mut values = Vec::new();
        loop {
            self.skip_whitespace();
            if self.consume(']') {
                return Ok(IonValue::List(values));
            }
            values.push(self.parse_value()?);
            self.skip_whitespace();
            if self.consume(']') {
                return Ok(IonValue::List(values));
            }
            self.expect(',')?;
        }
    }

    fn parse_struct(&mut self) -> Result<IonValue, KfxError> {
        self.expect('{')?;
        let mut fields = Vec::new();
        loop {
            self.skip_whitespace();
            if self.consume('}') {
                return Ok(IonValue::Struct(fields));
            }
            let key = match self.peek() {
                Some('"') => IonSymbol(self.parse_string()?),
                Some('\'') => IonSymbol(self.parse_symbol()?),
                Some(_) => IonSymbol(self.parse_token()?),
                None => return Err(KfxError::InvalidIon("struct key is missing".to_owned())),
            };
            self.skip_whitespace();
            self.expect(':')?;
            let value = self.parse_value()?;
            fields.push((key, value));
            self.skip_whitespace();
            if self.consume('}') {
                return Ok(IonValue::Struct(fields));
            }
            self.expect(',')?;
        }
    }

    fn parse_string(&mut self) -> Result<String, KfxError> {
        self.parse_quoted('"')
    }

    fn parse_symbol(&mut self) -> Result<String, KfxError> {
        self.parse_quoted('\'')
    }

    fn parse_quoted(&mut self, delimiter: char) -> Result<String, KfxError> {
        self.expect(delimiter)?;
        let mut output = String::new();
        while let Some(character) = self.next() {
            match character {
                value if value == delimiter => return Ok(output),
                '\\' => {
                    let escaped = self
                        .next()
                        .ok_or_else(|| KfxError::InvalidIon("unterminated escape".to_owned()))?;
                    match escaped {
                        '"' => output.push('"'),
                        '\'' => output.push('\''),
                        '\\' => output.push('\\'),
                        'n' => output.push('\n'),
                        'r' => output.push('\r'),
                        't' => output.push('\t'),
                        'u' => output.push(self.parse_unicode_escape()?),
                        _ => {
                            return Err(KfxError::InvalidIon(format!(
                                "unsupported escape \\{escaped}"
                            )))
                        }
                    }
                }
                value if value.is_control() => {
                    return Err(KfxError::InvalidIon(
                        "control character in string".to_owned(),
                    ))
                }
                value => output.push(value),
            }
        }
        Err(KfxError::InvalidIon("unterminated string".to_owned()))
    }

    fn parse_unicode_escape(&mut self) -> Result<char, KfxError> {
        let mut value = 0u32;
        for _ in 0..4 {
            let character = self
                .next()
                .ok_or_else(|| KfxError::InvalidIon("short unicode escape".to_owned()))?;
            let digit = character
                .to_digit(16)
                .ok_or_else(|| KfxError::InvalidIon("invalid unicode escape".to_owned()))?;
            value = value
                .checked_mul(16)
                .and_then(|value| value.checked_add(digit))
                .ok_or_else(|| KfxError::InvalidIon("unicode escape overflow".to_owned()))?;
        }
        char::from_u32(value)
            .ok_or_else(|| KfxError::InvalidIon("invalid unicode scalar".to_owned()))
    }

    fn parse_token(&mut self) -> Result<String, KfxError> {
        self.skip_whitespace();
        let start = self.position;
        while let Some(character) = self.peek() {
            if character.is_whitespace() || matches!(character, '[' | ']' | '{' | '}' | ',' | ':') {
                break;
            }
            self.position += 1;
        }
        if self.position == start {
            return Err(KfxError::InvalidIon("atom is missing".to_owned()));
        }
        Ok(self.chars[start..self.position].iter().collect())
    }

    fn consume_annotation_separator(&mut self) -> bool {
        self.skip_whitespace();
        if self.peek() == Some(':') && self.chars.get(self.position + 1) == Some(&':') {
            self.position += 2;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), KfxError> {
        self.skip_whitespace();
        if self.consume(expected) {
            Ok(())
        } else {
            Err(KfxError::InvalidIon(format!("expected '{expected}'")))
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.position += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.position).copied()
    }

    fn next(&mut self) -> Option<char> {
        let value = self.peek()?;
        self.position += 1;
        Some(value)
    }
}

fn collect_symbols(nodes: &[Node], symbols: &mut BTreeSet<String>) {
    for node in nodes {
        if let NodeKind::Link { href } = &node.kind {
            symbols.insert(href.clone());
        }
        collect_symbols(&node.children, symbols);
    }
}

fn collect_node_ids(node: &Node, styles: &mut BTreeSet<u32>, resources: &mut BTreeSet<u32>) {
    styles.insert(node.style.get());
    match &node.kind {
        NodeKind::Image { resource, .. } => {
            resources.insert(resource.get());
        }
        NodeKind::Svg {
            resource: Some(resource),
            ..
        } => {
            resources.insert(resource.get());
        }
        _ => {}
    }
    for child in &node.children {
        collect_node_ids(child, styles, resources);
    }
}

fn collect_navigation(points: &[folio_model::NavPoint], output: &mut Vec<NavigationRecord>) {
    for point in points {
        let mut children = Vec::new();
        collect_navigation(&point.children, &mut children);
        output.push(NavigationRecord {
            label: point.label.clone(),
            href: point.href.clone(),
            children,
        });
    }
}

fn collect_navigation_symbols(points: &[NavigationRecord], output: &mut BTreeSet<String>) {
    for point in points {
        output.insert(point.href.clone());
        collect_navigation_symbols(&point.children, output);
    }
}

fn navigation_record_to_ion(point: &NavigationRecord) -> IonValue {
    IonValue::Struct(vec![
        (
            IonSymbol("label".to_owned()),
            IonValue::String(point.label.clone()),
        ),
        (
            IonSymbol("href".to_owned()),
            IonValue::String(point.href.clone()),
        ),
        (
            IonSymbol("children".to_owned()),
            IonValue::List(
                point
                    .children
                    .iter()
                    .map(navigation_record_to_ion)
                    .collect(),
            ),
        ),
    ])
}

fn parse_navigation_records(items: &[IonValue]) -> Vec<folio_model::NavPoint> {
    items.iter().filter_map(parse_navigation_record).collect()
}

fn parse_navigation_record(value: &IonValue) -> Option<folio_model::NavPoint> {
    match value {
        // Accept the original FFKFX representation for backward
        // compatibility. New artifacts use the struct form below so labels
        // and hierarchy survive the IR -> FFKFX -> IR round trip.
        IonValue::String(href) => Some(folio_model::NavPoint {
            label: href.clone(),
            href: href.clone(),
            children: Vec::new(),
        }),
        IonValue::Struct(fields) => {
            let label = match struct_value(fields, "label") {
                Some(IonValue::String(value)) => value.clone(),
                _ => return None,
            };
            let href = match struct_value(fields, "href") {
                Some(IonValue::String(value)) => value.clone(),
                _ => return None,
            };
            let children = match struct_value(fields, "children") {
                Some(IonValue::List(values)) => parse_navigation_records(values),
                None => Vec::new(),
                _ => return None,
            };
            Some(folio_model::NavPoint {
                label,
                href,
                children,
            })
        }
        _ => None,
    }
}

fn valid_navigation_record(value: &IonValue) -> bool {
    match value {
        IonValue::String(_) => true,
        IonValue::Struct(fields) => {
            let Some(IonValue::String(_)) = struct_value(fields, "label") else {
                return false;
            };
            let Some(IonValue::String(_)) = struct_value(fields, "href") else {
                return false;
            };
            match struct_value(fields, "children") {
                Some(IonValue::List(children)) => children.iter().all(valid_navigation_record),
                None => true,
                _ => false,
            }
        }
        _ => false,
    }
}

fn quote(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            value if value.is_control() => output.push_str(&format!("\\u{:04x}", value as u32)),
            value => output.push(value),
        }
    }
    output.push('"');
    output
}

fn ion_symbol_text(value: &str) -> String {
    let mut characters = value.chars();
    let valid_first = characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || matches!(character, '_' | '$'));
    if valid_first
        && characters
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '$'))
    {
        value.to_owned()
    } else {
        quote_symbol(value)
    }
}

fn quote_symbol(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('\'');
    for character in value.chars() {
        match character {
            '\'' => output.push_str("\\'"),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            value if value.is_control() => output.push_str(&format!("\\u{:04x}", value as u32)),
            value => output.push(value),
        }
    }
    output.push('\'');
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ion_sections_round_trip() {
        let bytes = write_container(
            &[(
                "META".to_owned(),
                IonValue::Struct(vec![(IonSymbol("x".to_owned()), IonValue::Int(1))]),
            )],
            true,
        )
        .unwrap();
        let sections = inspect_compatibility_container(&bytes).unwrap();
        assert_eq!(sections[0].name, "META");
        assert_eq!(sections[0].value, "{x:1}");
    }

    #[test]
    fn compatibility_inspector_rejects_amazon_cont_signature() {
        assert!(inspect_compatibility_container(b"CONT\0\0\0\0\0\0\0\0").is_err());
    }

    #[test]
    fn ion_parser_supports_annotations_on_structs_and_rejects_trailing_data() {
        let value = parse_ion(r#"document::{"text":"line\n","enabled":true}"#).unwrap();
        match value {
            IonValue::Annotation { annotations, value } => {
                assert_eq!(annotations, vec![IonSymbol("document".to_owned())]);
                assert!(matches!(*value, IonValue::Struct(_)));
            }
            other => panic!("unexpected Ion value: {other:?}"),
        }
        assert!(parse_ion("1 2").is_err());
        assert!(parse_ion(r#"{"bad":"\q"}"#).is_err());
    }

    #[test]
    fn compatibility_container_validates_cross_references() {
        let sections = vec![
            (
                "META".to_owned(),
                IonValue::Struct(vec![
                    (
                        IonSymbol("title".to_owned()),
                        IonValue::String("Fixture".to_owned()),
                    ),
                    (IonSymbol("language".to_owned()), IonValue::Null),
                    (IonSymbol("creator".to_owned()), IonValue::List(Vec::new())),
                ]),
            ),
            (
                "CONT".to_owned(),
                IonValue::List(vec![IonValue::Struct(vec![
                    (IonSymbol("id".to_owned()), IonValue::Int(0)),
                    (IonSymbol("document".to_owned()), IonValue::Int(0)),
                    (
                        IonSymbol("text".to_owned()),
                        IonValue::String("Fixture".to_owned()),
                    ),
                    (
                        IonSymbol("styles".to_owned()),
                        IonValue::List(vec![IonValue::Int(0)]),
                    ),
                    (
                        IonSymbol("resources".to_owned()),
                        IonValue::List(vec![IonValue::Int(0)]),
                    ),
                ])]),
            ),
            ("SYMB".to_owned(), IonValue::List(Vec::new())),
            (
                "STYL".to_owned(),
                IonValue::List(vec![IonValue::Struct(vec![
                    (IonSymbol("id".to_owned()), IonValue::Int(0)),
                    (
                        IonSymbol("properties".to_owned()),
                        IonValue::Struct(Vec::new()),
                    ),
                ])]),
            ),
            (
                "RES ".to_owned(),
                IonValue::List(vec![IonValue::Struct(vec![
                    (IonSymbol("id".to_owned()), IonValue::Int(0)),
                    (
                        IonSymbol("path".to_owned()),
                        IonValue::String("image.png".to_owned()),
                    ),
                    (
                        IonSymbol("media_type".to_owned()),
                        IonValue::String("image/png".to_owned()),
                    ),
                ])]),
            ),
            ("NAV ".to_owned(), IonValue::List(Vec::new())),
            (
                "POS ".to_owned(),
                IonValue::List(vec![IonValue::Struct(vec![
                    (IonSymbol("fragment".to_owned()), IonValue::Int(0)),
                    (IonSymbol("ordinal".to_owned()), IonValue::Int(0)),
                ])]),
            ),
        ];
        let bytes = write_container(&sections, true).unwrap();
        assert!(validate_container(&bytes).is_ok());
    }

    #[test]
    fn ruby_round_trip_preserves_base_annotation_and_visible_text() {
        let input = XhtmlDocumentInput {
            href: "chapter.xhtml".to_owned(),
            media_type: "application/xhtml+xml".to_owned(),
            content: "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p><ruby>漢<rt>かん</rt></ruby>字。</p></body></html>".to_owned(),
        };
        let imported = import_xhtml_documents(
            Metadata {
                title: Some("Ruby".to_owned()),
                ..Metadata::default()
            },
            &[input],
            Vec::new(),
            std::sync::Arc::new(MemoryResourceLoader::default()),
        )
        .unwrap();
        let artifact = convert(&imported.book, &KfxOptions::default()).unwrap();
        let restored = import(&artifact.bytes).unwrap().book;
        assert_eq!(
            imported.book.ruby_projections(),
            restored.ruby_projections()
        );
        assert_eq!(
            imported.book.documents[0].nodes[0].visible_text_content(),
            restored.documents[0].nodes[0].visible_text_content()
        );
    }

    #[test]
    fn ruby_regression_fixtures_cover_required_shapes() {
        let fixtures = [
            (
                "single",
                "<p><ruby>漢<rt>かん</rt></ruby></p>",
                vec![("漢", "かん")],
            ),
            (
                "multiple",
                "<p><ruby>漢<rt>かん</rt></ruby><ruby>字<rt>じ</rt></ruby></p>",
                vec![("漢", "かん"), ("字", "じ")],
            ),
            (
                "punctuation",
                "<p><ruby>東京<rt>とうきょう</rt></ruby>。</p>",
                vec![("東京", "とうきょう")],
            ),
            (
                "mixed",
                "<p>前<ruby>漢<rt>かん</rt></ruby>後</p>",
                vec![("漢", "かん")],
            ),
        ];
        for (name, body, expected) in fixtures {
            let imported = import_xhtml_documents(
                Metadata {
                    title: Some(name.to_owned()),
                    ..Metadata::default()
                },
                &[XhtmlDocumentInput {
                    href: format!("{name}.xhtml"),
                    media_type: "application/xhtml+xml".to_owned(),
                    content: format!(
                        "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body>{body}</body></html>"
                    ),
                }],
                Vec::new(),
                std::sync::Arc::new(MemoryResourceLoader::default()),
            )
            .unwrap();
            let artifact = convert(&imported.book, &KfxOptions::default()).unwrap();
            let restored = import(&artifact.bytes).unwrap().book;
            let expected = expected
                .into_iter()
                .map(|(base, annotation)| folio_model::RubyProjection {
                    base: base.to_owned(),
                    annotation: annotation.to_owned(),
                })
                .collect::<Vec<_>>();
            assert_eq!(imported.book.ruby_projections(), expected, "fixture {name}");
            assert_eq!(
                restored.ruby_projections(),
                expected,
                "round-trip fixture {name}"
            );
            assert_eq!(
                imported.book.documents[0].nodes[0].visible_text_content(),
                restored.documents[0].nodes[0].visible_text_content(),
                "visible projection fixture {name}"
            );
        }
    }
}
