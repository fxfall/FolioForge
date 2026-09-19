//! KF7 target lowering and writer.

use std::collections::BTreeMap;

use folio_kindle_common::Compression;
use folio_kindle_common::{
    binary::{read_u16_be, read_u32_be},
    compression::{decompress, CompressionError},
};
use folio_mobi::{build_mobi, MobiError, MobiInput};
use folio_model::{
    Book, Diagnostic, MemoryResourceLoader, Node, NodeKind, Resource, ResourceId, ResourceKind,
};
use folio_normalize::{
    import_xhtml_documents, normalize_legacy_xhtml, XhtmlDocumentInput, XhtmlError,
};
use folio_style::inline_css;
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct Kf7Options {
    pub compression: Compression,
    pub deterministic: bool,
}

impl Default for Kf7Options {
    fn default() -> Self {
        Self {
            compression: Compression::None,
            deterministic: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Kf7Artifact {
    pub bytes: Vec<u8>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Error)]
pub enum Kf7Error {
    #[error("failed to build MOBI container: {0}")]
    Mobi(#[from] MobiError),
    #[error("KF7 decompression failed: {0}")]
    Compression(#[from] CompressionError),
    #[error("KF7 input is invalid: {0}")]
    Invalid(String),
    #[error("KF7 XHTML import failed: {0}")]
    Xhtml(#[from] XhtmlError),
    #[error("KF7 input requires semantic projection before writing: {0}")]
    RequiresProjection(String),
}

#[derive(Clone, Debug)]
pub struct Kf7ImportReport {
    pub book: Book,
    pub diagnostics: Vec<Diagnostic>,
}

/// Import a KF7/PalmDOC MOBI into the same semantic parser used by EPUB.
/// Unknown legacy metadata is kept as diagnostics; no binary implementation
/// details are placed in the shared IR.
pub fn import(bytes: &[u8]) -> Result<Kf7ImportReport, Kf7Error> {
    let inspection =
        folio_mobi::inspect_pdb(bytes).map_err(|error| Kf7Error::Invalid(error.to_string()))?;
    let record_zero = inspection
        .records
        .first()
        .ok_or_else(|| Kf7Error::Invalid("PDB has no record zero".to_owned()))?;
    if record_zero.len() < 16 + 232 || &record_zero[16..20] != b"MOBI" {
        return Err(Kf7Error::Invalid(
            "record zero is not a MOBI header".to_owned(),
        ));
    }
    let compression_code =
        read_u16_be(record_zero, 0).map_err(|error| Kf7Error::Invalid(error.to_string()))?;
    let compression = match compression_code {
        1 => Compression::None,
        2 => Compression::PalmDoc,
        17_480 => Compression::HuffDic,
        value => {
            return Err(Kf7Error::Invalid(format!(
                "unsupported compression {value}"
            )))
        }
    };
    let text_length =
        read_u32_be(record_zero, 4).map_err(|error| Kf7Error::Invalid(error.to_string()))? as usize;
    let text_record_count =
        read_u16_be(record_zero, 8).map_err(|error| Kf7Error::Invalid(error.to_string()))? as usize;
    let text_end = 1usize
        .checked_add(text_record_count)
        .ok_or_else(|| Kf7Error::Invalid("text record count overflowed".to_owned()))?;
    if text_end > inspection.records.len() {
        return Err(Kf7Error::Invalid(
            "text records exceed PDB record table".to_owned(),
        ));
    }
    let mut html_bytes = Vec::new();
    for record in &inspection.records[1..text_end] {
        let decoded = decompress(record, compression)?;
        html_bytes.extend_from_slice(&decoded);
        if html_bytes.len() > 256 << 20 {
            return Err(Kf7Error::Invalid(
                "decompressed text exceeds 256 MiB".to_owned(),
            ));
        }
    }
    html_bytes.truncate(text_length.min(html_bytes.len()));
    let metadata_report = folio_mobi::read_metadata(record_zero, folio_mobi::MobiVariant::Kf7);
    let mut metadata = metadata_report.metadata;
    let mut diagnostics = metadata_report.diagnostics;

    let first_image_index =
        read_u32_be(record_zero, 16 + 52).map_err(|error| Kf7Error::Invalid(error.to_string()))?;
    let mut resources = Vec::new();
    let mut loader = MemoryResourceLoader::default();
    let mut image_paths = BTreeMap::new();
    if first_image_index != u32::MAX {
        let start = usize::try_from(first_image_index)
            .map_err(|_| Kf7Error::Invalid("image index overflows usize".to_owned()))?;
        for (offset, record) in inspection.records.iter().enumerate().skip(start) {
            let kind = folio_mobi::image_kind(record);
            let extension = match kind {
                ResourceKind::Jpeg => "jpg",
                ResourceKind::Png => "png",
                ResourceKind::Gif => "gif",
                _ => "bin",
            };
            let path = format!("images/{:04}.{}", offset - start, extension);
            let id = ResourceId::new(resources.len() as u32);
            resources.push(Resource {
                id,
                path: path.clone(),
                media_type: folio_mobi::media_type_for_kind(&kind).to_owned(),
                kind,
                properties: Vec::new(),
                size: Some(record.len() as u64),
            });
            loader.insert(path, record.clone());
            image_paths.insert(
                offset,
                format!("images/{:04}.{}", offset - start, extension),
            );
        }
    }
    let html = normalize_legacy_xhtml(&String::from_utf8_lossy(&html_bytes), &image_paths);
    if metadata.title.is_none() {
        metadata.title = Some("Imported KF7 book".to_owned());
    }
    let report = import_xhtml_documents(
        metadata,
        &[XhtmlDocumentInput {
            href: "book.xhtml".to_owned(),
            media_type: "application/xhtml+xml".to_owned(),
            content: html,
        }],
        resources,
        std::sync::Arc::new(loader),
    )?;
    diagnostics.extend(report.diagnostics);
    Ok(Kf7ImportReport {
        book: report.book,
        diagnostics,
    })
}

pub fn convert(book: &Book, options: &Kf7Options) -> Result<Kf7Artifact, Kf7Error> {
    ensure_projected(book)?;
    let mut diagnostics = Vec::new();
    let image_paths = image_paths(book);
    let mut html = String::new();
    html.push_str("<html><head><meta charset=\"utf-8\"><title>");
    html.push_str(&escape_html(book.metadata.display_title()));
    html.push_str("</title></head><body>");
    for (index, document) in book.documents.iter().enumerate() {
        if index > 0 {
            html.push_str("<mbp:pagebreak/>");
        }
        for node in &document.nodes {
            render_node(node, book, &image_paths, &mut html, &mut diagnostics);
        }
    }
    html.push_str("</body></html>");

    let mut images = Vec::new();
    for resource in &book.resources {
        if !matches!(
            resource.kind,
            ResourceKind::Jpeg | ResourceKind::Png | ResourceKind::Gif
        ) {
            continue;
        }
        match book.load_resource(resource.id, resource.size.or(Some(256 << 20))) {
            Ok(bytes) => images.push(bytes),
            Err(error) => diagnostics.push(Diagnostic::warning(
                "FF-KF7-RES-0003",
                format!("could not load image {}: {error}", resource.path),
            )),
        }
    }
    let mobi = build_mobi(&MobiInput {
        title: book.metadata.display_title().to_owned(),
        author: book.metadata.author_names().first().cloned(),
        publisher: book.metadata.publisher.clone(),
        description: book.metadata.description.clone(),
        language: book.metadata.language.clone(),
        html: html.into_bytes(),
        images,
        compression: options.compression,
        kf8: false,
        deterministic: options.deterministic,
        extra_exth: Vec::new(),
        extra_records: Vec::new(),
    })?;
    Ok(Kf7Artifact {
        bytes: mobi.bytes,
        diagnostics,
    })
}

fn image_paths(book: &Book) -> BTreeMap<ResourceId, String> {
    book.resources
        .iter()
        .filter(|resource| {
            matches!(
                resource.kind,
                ResourceKind::Jpeg | ResourceKind::Png | ResourceKind::Gif
            )
        })
        .enumerate()
        .map(|(index, resource)| {
            let extension = match resource.kind {
                ResourceKind::Jpeg => "jpg",
                ResourceKind::Png => "png",
                ResourceKind::Gif => "gif",
                _ => "bin",
            };
            (resource.id, format!("images/{index:04}.{extension}"))
        })
        .collect()
}

fn ensure_projected(book: &Book) -> Result<(), Kf7Error> {
    if book.presentation.layout == folio_model::LayoutMode::Fixed {
        return Err(Kf7Error::RequiresProjection(
            "fixed layout must be lowered to reflowable order".to_owned(),
        ));
    }
    fn visit(book: &Book, nodes: &[Node]) -> Result<(), Kf7Error> {
        for node in nodes {
            match &node.kind {
                NodeKind::Svg { .. }
                | NodeKind::Ruby
                | NodeKind::Math { .. }
                | NodeKind::Footnote { .. }
                | NodeKind::Table
                | NodeKind::TableRow
                | NodeKind::TableCell => {
                    return Err(Kf7Error::RequiresProjection(format!(
                        "node {} still contains {}",
                        node.id,
                        node.kind.name()
                    )));
                }
                NodeKind::Image { resource, .. }
                    if book.resource(*resource).is_some_and(|resource| {
                        !matches!(
                            resource.kind,
                            ResourceKind::Jpeg | ResourceKind::Png | ResourceKind::Gif
                        )
                    }) =>
                {
                    return Err(Kf7Error::RequiresProjection(format!(
                        "node {} still references an SVG resource",
                        node.id
                    )));
                }
                _ => {}
            }
            if let Some(style) = book.styles.get(node.style) {
                if style
                    .get("writing-mode")
                    .is_some_and(|value| value != "horizontal-tb")
                {
                    return Err(Kf7Error::RequiresProjection(format!(
                        "node {} still uses vertical writing",
                        node.id
                    )));
                }
                if style
                    .get("position")
                    .is_some_and(|value| value == "fixed" || value == "absolute")
                    || style.get("float").is_some_and(|value| value != "none")
                {
                    return Err(Kf7Error::RequiresProjection(format!(
                        "node {} still uses non-flow positioning",
                        node.id
                    )));
                }
            }
            visit(book, &node.children)?;
        }
        Ok(())
    }
    for document in &book.documents {
        visit(book, &document.nodes)?;
    }
    Ok(())
}

fn render_node(
    node: &Node,
    book: &Book,
    image_paths: &BTreeMap<ResourceId, String>,
    output: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for anchor in book.anchors.iter().filter(|anchor| anchor.node == node.id) {
        output.push_str(&format!(
            "<a name=\"{}\"></a>",
            escape_attribute(&anchor.name)
        ));
    }
    let style = book.styles.get(node.style).cloned().unwrap_or_default();
    let style_attr = inline_css(&style);
    let style_attr = if style_attr.is_empty() {
        String::new()
    } else {
        format!(" style=\"{}\"", escape_attribute(&style_attr))
    };
    match &node.kind {
        NodeKind::Text { value } => {
            output.push_str(&escape_html(value));
        }
        NodeKind::Section => {
            output.push_str("<div");
            output.push_str(&style_attr);
            output.push('>');
            render_children(node, book, image_paths, output, diagnostics);
            output.push_str("</div>");
        }
        NodeKind::Heading { level } => {
            let level = (*level).clamp(1, 6);
            output.push_str(&format!("<h{level}{style_attr}>"));
            render_children(node, book, image_paths, output, diagnostics);
            output.push_str(&format!("</h{level}>"));
        }
        NodeKind::Paragraph => {
            output.push_str(&format!("<p{style_attr}>"));
            render_children(node, book, image_paths, output, diagnostics);
            output.push_str("</p>");
        }
        NodeKind::Emphasis => wrap(
            "em",
            &style_attr,
            node,
            book,
            image_paths,
            output,
            diagnostics,
        ),
        NodeKind::Strong => wrap(
            "strong",
            &style_attr,
            node,
            book,
            image_paths,
            output,
            diagnostics,
        ),
        NodeKind::BlockQuote => wrap(
            "blockquote",
            &style_attr,
            node,
            book,
            image_paths,
            output,
            diagnostics,
        ),
        NodeKind::Code => wrap(
            "code",
            &style_attr,
            node,
            book,
            image_paths,
            output,
            diagnostics,
        ),
        NodeKind::Preformatted => wrap(
            "pre",
            &style_attr,
            node,
            book,
            image_paths,
            output,
            diagnostics,
        ),
        NodeKind::OrderedList => wrap(
            "ol",
            &style_attr,
            node,
            book,
            image_paths,
            output,
            diagnostics,
        ),
        NodeKind::UnorderedList => wrap(
            "ul",
            &style_attr,
            node,
            book,
            image_paths,
            output,
            diagnostics,
        ),
        NodeKind::ListItem => wrap(
            "li",
            &style_attr,
            node,
            book,
            image_paths,
            output,
            diagnostics,
        ),
        NodeKind::Link { href } => {
            output.push_str(&format!(
                "<a href=\"{}\"{style_attr}>",
                escape_attribute(href)
            ));
            render_children(node, book, image_paths, output, diagnostics);
            output.push_str("</a>");
        }
        NodeKind::Anchor { name } => {
            if !book
                .anchors
                .iter()
                .any(|anchor| anchor.node == node.id && anchor.name == *name)
            {
                output.push_str(&format!("<a name=\"{}\"></a>", escape_attribute(name)));
            }
            render_children(node, book, image_paths, output, diagnostics);
        }
        NodeKind::Image { resource, alt } => {
            let src = image_paths.get(resource).map(String::as_str).unwrap_or("");
            output.push_str(&format!(
                "<img src=\"{}\" alt=\"{}\"{style_attr}/>",
                escape_attribute(src),
                escape_attribute(alt)
            ));
        }
        NodeKind::Svg { .. }
        | NodeKind::Ruby
        | NodeKind::Math { .. }
        | NodeKind::Footnote { .. }
        | NodeKind::Table
        | NodeKind::TableRow
        | NodeKind::TableCell => {
            unreachable!("KF7 writer received an unprojected semantic node")
        }
        NodeKind::PageBreak => output.push_str("<mbp:pagebreak/>"),
        NodeKind::Inline | NodeKind::GenericInline { .. } | NodeKind::GenericBlock { .. } => {
            let tag = match &node.kind {
                NodeKind::GenericInline { tag } | NodeKind::GenericBlock { tag } => tag.as_str(),
                _ => "span",
            };
            let tag = if tag == "img" { "span" } else { tag };
            output.push_str(&format!("<{tag}{style_attr}>"));
            render_children(node, book, image_paths, output, diagnostics);
            output.push_str(&format!("</{tag}>"));
        }
    }
}

fn render_children(
    node: &Node,
    book: &Book,
    image_paths: &BTreeMap<ResourceId, String>,
    output: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for child in &node.children {
        render_node(child, book, image_paths, output, diagnostics);
    }
}

fn wrap(
    tag: &str,
    style: &str,
    node: &Node,
    book: &Book,
    image_paths: &BTreeMap<ResourceId, String>,
    output: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
) {
    output.push_str(&format!("<{tag}{style}>"));
    render_children(node, book, image_paths, output, diagnostics);
    output.push_str(&format!("</{tag}>"));
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attribute(value: &str) -> String {
    escape_html(value).replace('"', "&quot;")
}

/// KF7's input-side adapter. KF7 parsing and compatibility-container reading
/// stay in this crate instead of in the shared adapter contract.
pub struct Kf7Adapter;

impl folio_format::FormatAdapter for Kf7Adapter {
    fn format(&self) -> folio_input::DetectedFormat {
        folio_input::DetectedFormat::Kf7
    }

    fn input_formats(&self) -> Vec<folio_format::FormatId> {
        vec![
            folio_input::DetectedFormat::Kf7,
            folio_input::DetectedFormat::Kf7Kf8Combo,
        ]
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
        folio_format::supports_detected(
            source,
            [
                folio_input::DetectedFormat::Kf7,
                folio_input::DetectedFormat::Kf7Kf8Combo,
            ],
        )
    }

    fn import(
        &self,
        source: &folio_input::BookSource,
        _context: &folio_format::ImportContext,
    ) -> Result<folio_format::ImportedBook, folio_format::FormatError> {
        let path = folio_format::primary_file(source)?;
        let bytes = std::fs::read(path)?;
        let detected = folio_format::detect_single(source)?;
        let read = import(&bytes)
            .map_err(|error| folio_format::FormatError::Invalid(error.to_string()))?;
        let mut diagnostics = read.diagnostics;
        if detected == folio_input::DetectedFormat::Kf7Kf8Combo {
            diagnostics.push(folio_model::Diagnostic::warning(
                "FF-COMBO-IMPORT-0001",
                "imported the KF7 semantic view of FolioForge's explicit composite container",
            ));
        }
        Ok(folio_format::ImportedBook {
            format: detected,
            parser: "folio-kf7/1".to_owned(),
            book: read.book,
            diagnostics,
            input_loss: Vec::new(),
            text: None,
        })
    }
}
