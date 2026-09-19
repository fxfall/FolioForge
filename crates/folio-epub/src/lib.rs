//! EPUB 2/3 OCF frontend.  Package metadata and document structure are read
//! eagerly into the canonical model; large binary resources stay in the ZIP
//! and are loaded through a path-backed `ResourceLoader` on demand.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use folio_model::{
    AnchorEdge, AnchorGraph, AnchorRelation, Book, Diagnostic, Document, Metadata, NavPoint,
    Navigation, Node, NodeKind, Presentation, Resource, ResourceId, ResourceKind,
    ResourceLoadError, ResourceLoader, SemanticRole, VariantTarget,
};
use folio_style::{StyleResolver, TargetProfile};
use roxmltree::{Document as XmlDocument, Node as XmlNode};
use thiserror::Error;
use zip::{result::ZipError, write::SimpleFileOptions, ZipArchive, ZipWriter};

pub use folio_normalize::{normalize_legacy_xhtml, XhtmlDocumentInput};

#[derive(Clone, Debug)]
pub struct EpubLimits {
    pub max_entries: usize,
    pub max_total_uncompressed: u64,
    pub max_entry_size: u64,
    pub max_xhtml_size: u64,
    pub max_css_size: u64,
    pub max_xml_depth: usize,
    pub max_dom_nodes: usize,
}

/// EPUB's input-side adapter. The adapter lives with the EPUB parser; the
/// shared `folio-format` crate contains only the contract and neutral reports.
pub struct EpubAdapter;

impl folio_format::FormatAdapter for EpubAdapter {
    fn format(&self) -> folio_input::DetectedFormat {
        folio_input::DetectedFormat::Epub
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
        folio_format::supports_detected(source, [folio_input::DetectedFormat::Epub])
    }

    fn import(
        &self,
        source: &folio_input::BookSource,
        _context: &folio_format::ImportContext,
    ) -> Result<folio_format::ImportedBook, folio_format::FormatError> {
        let path = folio_format::primary_file(source)?;
        let read = EpubReader::default()
            .read(path)
            .map_err(|error| folio_format::FormatError::Invalid(error.to_string()))?;
        Ok(folio_format::ImportedBook {
            format: folio_input::DetectedFormat::Epub,
            parser: "folio-epub/1".to_owned(),
            book: read.book,
            diagnostics: read.diagnostics,
            input_loss: Vec::new(),
            text: None,
        })
    }
}

impl Default for EpubLimits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_total_uncompressed: 1 << 30,
            max_entry_size: 256 << 20,
            max_xhtml_size: 64 << 20,
            max_css_size: 16 << 20,
            max_xml_depth: 128,
            max_dom_nodes: 2_000_000,
        }
    }
}

#[derive(Debug, Error)]
pub enum EpubError {
    #[error("failed to open EPUB: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid ZIP container: {0}")]
    Zip(#[from] ZipError),
    #[error("invalid XML in {path}: {source}")]
    Xml {
        path: String,
        source: roxmltree::Error,
    },
    #[error("EPUB is invalid: {0}")]
    Invalid(String),
    #[error("EPUB safety limit exceeded: {0}")]
    Limit(String),
    #[error("shared XHTML import failed: {0}")]
    Xhtml(#[from] folio_normalize::XhtmlError),
}

#[derive(Clone, Debug)]
pub struct EpubReadReport {
    pub book: Book,
    pub diagnostics: Vec<Diagnostic>,
}

/// Compatibility adapter retained for callers of the earlier API. The
/// format-neutral parser itself is owned by `folio-normalize`.
pub fn import_xhtml_documents(
    metadata: Metadata,
    documents: &[XhtmlDocumentInput],
    resources: Vec<Resource>,
    loader: Arc<dyn ResourceLoader>,
) -> Result<EpubReadReport, EpubError> {
    import_xhtml_documents_with_options(
        metadata,
        documents,
        resources,
        loader,
        folio_normalize::XhtmlImportOptions::default(),
    )
}

pub fn import_xhtml_documents_with_options(
    metadata: Metadata,
    documents: &[XhtmlDocumentInput],
    resources: Vec<Resource>,
    loader: Arc<dyn ResourceLoader>,
    options: folio_normalize::XhtmlImportOptions,
) -> Result<EpubReadReport, EpubError> {
    let report = folio_normalize::import_xhtml_documents_with_options(
        metadata, documents, resources, loader, options,
    )?;
    Ok(EpubReadReport {
        book: report.book,
        diagnostics: report.diagnostics,
    })
}

#[derive(Clone, Debug, Default)]
pub struct EpubReader {
    pub limits: EpubLimits,
}

impl EpubReader {
    pub fn new(limits: EpubLimits) -> Self {
        Self { limits }
    }

    pub fn read(&self, path: impl AsRef<Path>) -> Result<EpubReadReport, EpubError> {
        let path = path.as_ref().to_path_buf();
        let mut diagnostics = Vec::new();
        validate_zip_safety(&path, &self.limits)?;

        let mimetype = read_zip_entry(&path, "mimetype", self.limits.max_entry_size)?;
        if mimetype != b"application/epub+zip" {
            return Err(EpubError::Invalid(
                "mimetype entry is not application/epub+zip".to_owned(),
            ));
        }
        let container_bytes =
            read_zip_entry(&path, "META-INF/container.xml", self.limits.max_entry_size)?;
        let container = parse_xml("META-INF/container.xml", &container_bytes)?;
        let rootfile = container
            .descendants()
            .find(|node| node.is_element() && local_name(*node) == "rootfile")
            .and_then(|node| node.attribute("full-path"))
            .ok_or_else(|| {
                EpubError::Invalid("META-INF/container.xml has no rootfile full-path".to_owned())
            })?;
        let opf_path = normalize_epub_path(rootfile)?;
        let opf_bytes = read_zip_entry(&path, &opf_path, self.limits.max_entry_size)?;
        let opf = parse_xml(&opf_path, &opf_bytes)?;

        let package = parse_package(&opf, &opf_path)?;
        let metadata = package.metadata;
        let manifest = package.manifest;
        let spine_items = package.spine_items;
        let ncx_path = package.ncx_path;
        let layout = package.layout;
        let opf_dir = parent_path(&opf_path);
        let mut resources = Vec::with_capacity(manifest.len());
        let mut manifest_by_id = BTreeMap::new();
        for (index, item) in manifest.iter().enumerate() {
            let resource_id = ResourceId::new(index as u32);
            let resolved = resolve_epub_href(&opf_dir, &item.href)?;
            manifest_by_id.insert(item.id.clone(), item.clone());
            resources.push(Resource {
                id: resource_id,
                path: resolved,
                media_type: item.media_type.clone(),
                kind: resource_kind(&item.media_type, &item.href),
                properties: item.properties.clone(),
                size: zip_entry_size(&path, &resolve_epub_href(&opf_dir, &item.href)?)?,
            });
        }

        let mut css_sources = Vec::new();
        for item in &manifest {
            if resource_kind(&item.media_type, &item.href) != ResourceKind::Stylesheet {
                continue;
            }
            let css_path = resolve_epub_href(&opf_dir, &item.href)?;
            let bytes = read_zip_entry(&path, &css_path, self.limits.max_css_size)?;
            css_sources.push((css_path, String::from_utf8_lossy(&bytes).into_owned()));
        }
        let resolver = StyleResolver::from_stylesheets(
            css_sources
                .iter()
                .map(|(source, css)| (source.as_str(), css.as_str())),
            TargetProfile::Generic,
        );
        let variant_resolvers = vec![
            (
                VariantTarget::Legacy,
                StyleResolver::from_stylesheets(
                    css_sources
                        .iter()
                        .map(|(source, css)| (source.as_str(), css.as_str())),
                    TargetProfile::Kf7,
                ),
            ),
            (
                VariantTarget::Modern,
                StyleResolver::from_stylesheets(
                    css_sources
                        .iter()
                        .map(|(source, css)| (source.as_str(), css.as_str())),
                    TargetProfile::Kf8,
                ),
            ),
            (
                VariantTarget::Kfx,
                StyleResolver::from_stylesheets(
                    css_sources
                        .iter()
                        .map(|(source, css)| (source.as_str(), css.as_str())),
                    TargetProfile::Kfx,
                ),
            ),
        ];
        let mut xhtml_documents = Vec::with_capacity(spine_items.len());
        for item_id in &spine_items {
            let Some(item) = manifest_by_id.get(item_id) else {
                return Err(EpubError::Invalid(format!(
                    "spine references missing manifest item {item_id}"
                )));
            };
            let href = resolve_epub_href(&opf_dir, &item.href)?;
            let bytes = read_zip_entry(&path, &href, self.limits.max_xhtml_size)?;
            let content = String::from_utf8(bytes)
                .map_err(|error| EpubError::Invalid(format!("{href} is not UTF-8: {error}")))?;
            xhtml_documents.push(XhtmlDocumentInput {
                href,
                media_type: item.media_type.clone(),
                content: folio_normalize::normalize_xhtml_for_xml(&content),
            });
        }

        let imported = folio_normalize::import_xhtml_documents_with_options(
            metadata,
            &xhtml_documents,
            resources,
            Arc::new(ZipResourceLoader {
                path: path.clone(),
                max_entry_size: self.limits.max_entry_size,
            }),
            folio_normalize::XhtmlImportOptions {
                max_dom_nodes: self.limits.max_dom_nodes,
                max_xml_depth: self.limits.max_xml_depth,
                require_body_element: true,
                resolver,
                variant_resolvers,
            },
        )?;
        let mut book = imported.book;
        diagnostics.extend(imported.diagnostics);
        book.presentation = Presentation {
            layout,
            direction: None,
            writing_mode: None,
        };

        let navigation = parse_navigation(
            &path,
            &opf,
            &manifest_by_id,
            &opf_dir,
            ncx_path.as_deref(),
            &self.limits,
            &mut diagnostics,
        )?;
        let mut anchor_graph = std::mem::take(&mut book.navigation.anchor_graph);
        append_navigation_edges(&navigation, &mut anchor_graph);
        book.navigation = Navigation {
            anchor_graph,
            ..navigation
        };

        Ok(EpubReadReport { book, diagnostics })
    }
}

#[derive(Clone, Debug)]
pub struct EpubOptions {
    pub deterministic: bool,
    pub include_ncx: bool,
}

impl Default for EpubOptions {
    fn default() -> Self {
        Self {
            deterministic: true,
            include_ncx: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EpubArtifact {
    pub bytes: Vec<u8>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Serialize the unified semantic book as a standards-shaped EPUB 3 package.
/// The package is generated from computed semantics; source CSS and DOM
/// implementation details are never required by this exporter.
pub fn export(book: &Book, options: &EpubOptions) -> Result<EpubArtifact, EpubError> {
    let mut diagnostics = Vec::new();
    let mut resource_paths = BTreeMap::new();
    for resource in &book.resources {
        if resource.kind == ResourceKind::Stylesheet {
            continue;
        }
        let basename = resource
            .path
            .rsplit('/')
            .next()
            .filter(|value| !value.is_empty())
            .unwrap_or("resource.bin");
        let name = format!(
            "OEBPS/assets/{:04}-{}",
            resource.id.get(),
            safe_name(basename)
        );
        resource_paths.insert(resource.id, name);
    }
    let document_paths = book
        .documents
        .iter()
        .enumerate()
        .map(|(index, document)| {
            (
                document.href.clone(),
                format!("OEBPS/text/{:04}.xhtml", index + 1),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut document_bytes = Vec::with_capacity(book.documents.len());
    for document in &book.documents {
        let output_path = document_paths
            .get(&document.href)
            .cloned()
            .unwrap_or_else(|| "OEBPS/text/0001.xhtml".to_owned());
        let mut xhtml = String::from(
            r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>"#,
        );
        xhtml.push_str(&escape_xml(
            document
                .title
                .as_deref()
                .unwrap_or(book.metadata.display_title()),
        ));
        let body_style = document_body_style(book, document);
        xhtml.push_str("</title><link rel=\"stylesheet\" type=\"text/css\" href=\"../styles.css\"/></head><body style=\"");
        xhtml.push_str(&escape_xml(&body_style));
        xhtml.push_str("\">");
        for node in &document.nodes {
            render_epub_node(
                node,
                book,
                &resource_paths,
                &document_paths,
                &mut xhtml,
                &mut diagnostics,
            );
        }
        xhtml.push_str("</body></html>");
        document_bytes.push((output_path, xhtml.into_bytes()));
    }

    let nav = render_navigation(book, &document_paths);
    let opf = render_opf(book, &document_paths, &resource_paths, options.include_ncx);
    let container = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;

    let mut output = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut output);
        let mut zip = ZipWriter::new(cursor);
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored)?;
        zip.write_all(b"application/epub+zip")?;
        zip.start_file("META-INF/container.xml", stored)?;
        zip.write_all(container)?;
        zip.start_file("OEBPS/content.opf", stored)?;
        zip.write_all(opf.as_bytes())?;
        zip.start_file("OEBPS/nav.xhtml", stored)?;
        zip.write_all(nav.as_bytes())?;
        zip.start_file("OEBPS/styles.css", stored)?;
        zip.write_all(render_stylesheet(book, &resource_paths).as_bytes())?;
        if options.include_ncx {
            zip.start_file("OEBPS/toc.ncx", stored)?;
            zip.write_all(render_ncx(book, &document_paths).as_bytes())?;
        }
        for (path, bytes) in document_bytes {
            zip.start_file(path, stored)?;
            zip.write_all(&bytes)?;
        }
        for resource in &book.resources {
            let Some(path) = resource_paths.get(&resource.id) else {
                continue;
            };
            match book.load_resource(resource.id, resource.size.or(Some(256 << 20))) {
                Ok(bytes) => {
                    zip.start_file(path, stored)?;
                    zip.write_all(&bytes)?;
                }
                Err(error) => diagnostics.push(Diagnostic::warning(
                    "FF-EPUB-EXPORT-RES-0001",
                    format!("resource {} was not embedded: {error}", resource.path),
                )),
            }
        }
        zip.finish()?;
    }
    let _ = options.deterministic;
    Ok(EpubArtifact {
        bytes: output,
        diagnostics,
    })
}

fn render_stylesheet(book: &Book, resource_paths: &BTreeMap<ResourceId, String>) -> String {
    let mut output = String::from(
        "html,body{margin:0;padding:0;} body{font-family:Stsong,Song S,Song;font-size:1em;} img{max-width:100%;height:auto;} .ff-pagebreak{page-break-before:always;}\n",
    );
    for face in &book.font_faces {
        let Some(resource) = book.resource(face.resource) else {
            continue;
        };
        if resource.kind != ResourceKind::Font {
            continue;
        }
        let Some(path) = resource_paths.get(&face.resource) else {
            continue;
        };
        let Some(path) = path.strip_prefix("OEBPS/") else {
            continue;
        };
        output.push_str("@font-face{font-family:");
        output.push_str(&css_quoted_string(&face.family));
        output.push_str(";src:url(");
        output.push_str(&css_quoted_string(path));
        output.push(')');
        if let Some(format) = css_font_format(&resource.media_type) {
            output.push_str(" format(");
            output.push_str(&css_quoted_string(format));
            output.push(')');
        }
        output.push_str(";font-style:normal;font-weight:normal;}\n");
    }
    output
}

fn document_body_style(book: &Book, document: &Document) -> String {
    let base = "display:block;font-family:Stsong,Song S,Song;font-size:1em;padding-left:0;padding-right:0;margin:0 5pt";
    let has_image = document.nodes.iter().any(node_contains_image);
    let has_text = document.nodes.iter().any(|node| {
        matches!(
            &node.kind,
            NodeKind::Paragraph | NodeKind::Heading { .. } | NodeKind::GenericBlock { .. }
        )
    });
    let has_part_heading = document
        .nodes
        .iter()
        .any(|node| matches!(&node.kind, NodeKind::Heading { level: 3 }));
    let is_toc = document
        .nodes
        .first()
        .and_then(|node| book.styles.get(node.style))
        .and_then(|style| style.get("font-size"))
        == Some("2em");
    let is_qr_card = document
        .nodes
        .iter()
        .any(|node| node_has_style_value(book, node, "width", "8.125em"))
        && has_text;
    let is_centered_standalone_image = !has_text
        && has_image
        && document
            .nodes
            .iter()
            .any(|node| node_has_style_value(book, node, "margin", "1.3025em auto"));

    let mut style = String::from(base);
    if is_qr_card {
        style.push_str(";text-align:center;text-indent:6.25%;padding:2em 0;border:gray solid 2px");
    } else if is_toc {
        style.push_str(";text-align:justify;text-indent:6.25%");
    } else if has_part_heading {
        style.push_str(";font-weight:bold;text-align:center;text-indent:1.709em");
    } else if has_image && has_text {
        style.push_str(";text-align:center;text-indent:2em");
    } else if is_centered_standalone_image {
        style.push_str(";text-align:center;text-indent:0");
    } else if !has_text && has_image {
        // A cover/standalone illustration must not inherit a paragraph
        // indent. Centering is supplied only by the source's observed image
        // wrapper style (e.g. sH), matching Calibre/Bokō behavior.
        style.push_str(";text-indent:0");
    } else if document.nodes.iter().all(|node| {
        matches!(&node.kind, NodeKind::Paragraph)
            && node
                .children
                .iter()
                .any(|child| child.text_content().starts_with('【'))
    }) {
        style.push_str(";text-indent:0");
    } else {
        style.push_str(";text-align:left;text-indent:2em");
    }
    style
}

fn node_contains_image(node: &Node) -> bool {
    matches!(&node.kind, NodeKind::Image { .. }) || node.children.iter().any(node_contains_image)
}

fn node_has_style_value(book: &Book, node: &Node, property: &str, expected: &str) -> bool {
    book.styles
        .get(node.style)
        .and_then(|style| style.get(property))
        == Some(expected)
        || node
            .children
            .iter()
            .any(|child| node_has_style_value(book, child, property, expected))
}

fn css_font_format(media_type: &str) -> Option<&'static str> {
    match media_type.to_ascii_lowercase().as_str() {
        "font/woff2" => Some("woff2"),
        "font/woff" | "application/font-woff" => Some("woff"),
        "font/otf" | "application/vnd.ms-opentype" => Some("opentype"),
        "font/ttf" | "application/x-font-ttf" => Some("truetype"),
        _ => None,
    }
}

fn css_quoted_string(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\5c "),
            '"' => output.push_str("\\22 "),
            character if character.is_control() => {
                output.push_str(&format!("\\{:x} ", character as u32));
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn render_opf(
    book: &Book,
    document_paths: &BTreeMap<String, String>,
    resource_paths: &BTreeMap<ResourceId, String>,
    include_ncx: bool,
) -> String {
    let mut output = String::from(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>"#,
    );
    output.push_str(&escape_xml(book.metadata.display_title()));
    output.push_str("</dc:title>");
    if let Some(language) = &book.metadata.language {
        output.push_str("<dc:language>");
        output.push_str(&escape_xml(language));
        output.push_str("</dc:language>");
    }
    if let Some(identifier) = book.metadata.identifier_values().first() {
        output.push_str("<dc:identifier id=\"bookid\">");
        output.push_str(&escape_xml(identifier));
        output.push_str("</dc:identifier>");
    } else {
        output.push_str("<dc:identifier id=\"bookid\">urn:folioforge:");
        output.push_str(&stable_book_id(book));
        output.push_str("</dc:identifier>");
    }
    for creator in book.metadata.author_names() {
        output.push_str("<dc:creator>");
        output.push_str(&escape_xml(&creator));
        output.push_str("</dc:creator>");
    }
    if let Some(description) = &book.metadata.description {
        output.push_str("<dc:description>");
        output.push_str(&escape_xml(description));
        output.push_str("</dc:description>");
    }
    if let Some(series) = &book.metadata.series {
        output.push_str("<meta property=\"belongs-to-collection\">");
        output.push_str(&escape_xml(series));
        output.push_str("</meta>");
    }
    if let Some(index) = book.metadata.series_index {
        output.push_str(&format!("<meta property=\"group-position\">{index}</meta>"));
    }
    for date in book.metadata.date_values() {
        output.push_str("<dc:date>");
        output.push_str(&escape_xml(&date));
        output.push_str("</dc:date>");
    }
    if let Some(cover) = book.resources.iter().find(|resource| {
        resource
            .properties
            .iter()
            .any(|property| property == "cover-image")
    }) {
        output.push_str(&format!(
            "<meta name=\"cover\" content=\"res{}\"/>",
            cover.id.get()
        ));
    }
    output.push_str("</metadata><manifest>");
    output.push_str("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>");
    output.push_str("<item id=\"css\" href=\"styles.css\" media-type=\"text/css\"/>");
    if include_ncx {
        output.push_str(
            "<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>",
        );
    }
    for (index, document) in book.documents.iter().enumerate() {
        let Some(path) = document_paths.get(&document.href) else {
            continue;
        };
        output.push_str(&format!(
            "<item id=\"doc{}\" href=\"{}\" media-type=\"application/xhtml+xml\"/>",
            index + 1,
            escape_xml(path.strip_prefix("OEBPS/").unwrap_or(path))
        ));
    }
    for resource in &book.resources {
        let Some(path) = resource_paths.get(&resource.id) else {
            continue;
        };
        output.push_str(&format!(
            "<item id=\"res{}\" href=\"{}\" media-type=\"{}\"{} />",
            resource.id.get(),
            escape_xml(path.strip_prefix("OEBPS/").unwrap_or(path)),
            escape_xml(&resource.media_type),
            if resource.properties.is_empty() {
                String::new()
            } else {
                format!(
                    " properties=\"{}\"",
                    escape_xml(&resource.properties.join(" "))
                )
            }
        ));
    }
    output.push_str("</manifest><spine>");
    for (index, document) in book.documents.iter().enumerate() {
        if document_paths.contains_key(&document.href) {
            output.push_str(&format!("<itemref idref=\"doc{}\"/>", index + 1));
        }
    }
    output.push_str("</spine></package>");
    output
}

fn render_navigation(book: &Book, document_paths: &BTreeMap<String, String>) -> String {
    let mut output = String::from(
        r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head><body><nav epub:type="toc" id="toc"><h1>Contents</h1><ol>"#,
    );
    render_nav_points(&book.navigation.toc, document_paths, &mut output);
    output.push_str("</ol></nav>");
    if !book.navigation.landmarks.is_empty() {
        output.push_str("<nav epub:type=\"landmarks\"><ol>");
        render_nav_points(&book.navigation.landmarks, document_paths, &mut output);
        output.push_str("</ol></nav>");
    }
    if !book.navigation.page_list.is_empty() {
        output.push_str("<nav epub:type=\"page-list\"><ol>");
        render_nav_points(&book.navigation.page_list, document_paths, &mut output);
        output.push_str("</ol></nav>");
    }
    output.push_str("</body></html>");
    output
}

fn render_nav_points(
    points: &[NavPoint],
    document_paths: &BTreeMap<String, String>,
    output: &mut String,
) {
    for point in points {
        output.push_str("<li><a href=\"");
        output.push_str(&escape_xml(&rewrite_nav_href(&point.href, document_paths)));
        output.push_str("\">");
        output.push_str(&escape_xml(&point.label));
        output.push_str("</a>");
        if !point.children.is_empty() {
            output.push_str("<ol>");
            render_nav_points(&point.children, document_paths, output);
            output.push_str("</ol>");
        }
        output.push_str("</li>");
    }
}

fn render_ncx(book: &Book, document_paths: &BTreeMap<String, String>) -> String {
    let mut output = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1-1"><head></head><docTitle><text>"#,
    );
    output.push_str(&escape_xml(book.metadata.display_title()));
    output.push_str("</text></docTitle><navMap>");
    let mut order = 1u32;
    render_ncx_points(
        &book.navigation.toc,
        document_paths,
        &mut order,
        &mut output,
    );
    output.push_str("</navMap></ncx>");
    output
}

fn render_ncx_points(
    points: &[NavPoint],
    document_paths: &BTreeMap<String, String>,
    order: &mut u32,
    output: &mut String,
) {
    for point in points {
        output.push_str(&format!(
            "<navPoint id=\"navPoint-{}\" playOrder=\"{}\"><navLabel><text>",
            *order, *order
        ));
        output.push_str(&escape_xml(&point.label));
        output.push_str("</text></navLabel><content src=\"");
        output.push_str(&escape_xml(&rewrite_nav_href(&point.href, document_paths)));
        output.push_str("\"/>");
        *order = order.saturating_add(1);
        render_ncx_points(&point.children, document_paths, order, output);
        output.push_str("</navPoint>");
    }
}

fn render_epub_node(
    node: &Node,
    book: &Book,
    resource_paths: &BTreeMap<ResourceId, String>,
    document_paths: &BTreeMap<String, String>,
    output: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for anchor in book.anchors.iter().filter(|anchor| anchor.node == node.id) {
        output.push_str("<a id=\"");
        output.push_str(&escape_xml(&anchor.name));
        output.push_str("\"></a>");
    }
    let style = book.styles.get(node.style).cloned().unwrap_or_default();
    let style = folio_style::inline_css(&style);
    let style_attr = if style.is_empty() {
        String::new()
    } else {
        format!(" style=\"{}\"", escape_xml(&style))
    };
    match &node.kind {
        NodeKind::Text { value } => output.push_str(&escape_xml(value)),
        NodeKind::Section => {
            output.push_str(&format!("<section{}>", style_attr));
            render_epub_children(
                node,
                book,
                resource_paths,
                document_paths,
                output,
                diagnostics,
            );
            output.push_str("</section>");
        }
        NodeKind::Heading { level } => {
            let level = (*level).clamp(1, 6);
            output.push_str(&format!("<h{}{}>", level, style_attr));
            render_epub_children(
                node,
                book,
                resource_paths,
                document_paths,
                output,
                diagnostics,
            );
            output.push_str(&format!("</h{}>", level));
        }
        NodeKind::Paragraph => {
            output.push_str(&format!("<p{}>", style_attr));
            render_epub_children(
                node,
                book,
                resource_paths,
                document_paths,
                output,
                diagnostics,
            );
            output.push_str("</p>");
        }
        NodeKind::Emphasis => render_epub_wrapped(
            "em",
            node,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
            &style_attr,
        ),
        NodeKind::Strong => render_epub_wrapped(
            "strong",
            node,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
            &style_attr,
        ),
        NodeKind::BlockQuote => render_epub_wrapped(
            "blockquote",
            node,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
            &style_attr,
        ),
        NodeKind::Code => render_epub_wrapped(
            "code",
            node,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
            &style_attr,
        ),
        NodeKind::Preformatted => render_epub_wrapped(
            "pre",
            node,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
            &style_attr,
        ),
        NodeKind::OrderedList => render_epub_wrapped(
            "ol",
            node,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
            &style_attr,
        ),
        NodeKind::UnorderedList => render_epub_wrapped(
            "ul",
            node,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
            &style_attr,
        ),
        NodeKind::ListItem => render_epub_wrapped(
            "li",
            node,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
            &style_attr,
        ),
        NodeKind::Table => render_epub_wrapped(
            "table",
            node,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
            &style_attr,
        ),
        NodeKind::TableRow => render_epub_wrapped(
            "tr",
            node,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
            &style_attr,
        ),
        NodeKind::TableCell => render_epub_wrapped(
            "td",
            node,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
            &style_attr,
        ),
        NodeKind::Link { href } => {
            output.push_str(&format!(
                "<a href=\"{}\"{}>",
                escape_xml(&rewrite_href(href, document_paths)),
                style_attr
            ));
            render_epub_children(
                node,
                book,
                resource_paths,
                document_paths,
                output,
                diagnostics,
            );
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
            render_epub_children(
                node,
                book,
                resource_paths,
                document_paths,
                output,
                diagnostics,
            );
        }
        NodeKind::Image { resource, alt } => {
            let Some(src) = resource_paths.get(resource) else {
                diagnostics.push(Diagnostic::warning(
                    "FF-EPUB-EXPORT-RES-0002",
                    format!("missing image resource {}", resource),
                ));
                return;
            };
            output.push_str(&format!(
                "<img src=\"{}\" alt=\"{}\"{} />",
                escape_xml(&rewrite_resource_href(src)),
                escape_xml(alt),
                style_attr
            ));
        }
        NodeKind::Svg { resource, alt } => {
            if let Some(resource) = resource.as_ref().and_then(|id| resource_paths.get(id)) {
                output.push_str(&format!(
                    "<img src=\"{}\" alt=\"{}\"{} />",
                    escape_xml(&rewrite_resource_href(resource)),
                    escape_xml(alt),
                    style_attr
                ));
            } else if !alt.is_empty() {
                output.push_str(&escape_xml(alt));
            } else {
                diagnostics.push(Diagnostic::warning(
                    "FF-EPUB-EXPORT-SVG-0001",
                    "SVG node has no serializable resource or alt text.",
                ));
            }
        }
        NodeKind::Ruby => render_epub_wrapped(
            "ruby",
            node,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
            &style_attr,
        ),
        NodeKind::Math { mathml, .. } => {
            // MathML has already passed the bounded, namespace-aware
            // sanitizer in the importer. Keep it as semantic markup for EPUB
            // instead of replacing it with a guessed SVG or plain text.
            output.push_str(mathml);
        }
        NodeKind::Footnote { href } => {
            output.push_str(&format!(
                "<sup{}><a epub:type=\"noteref\" href=\"{}\">",
                style_attr,
                escape_xml(&rewrite_href(
                    href.as_deref().unwrap_or("#"),
                    document_paths
                ))
            ));
            render_epub_children(
                node,
                book,
                resource_paths,
                document_paths,
                output,
                diagnostics,
            );
            output.push_str("</a></sup>");
        }
        NodeKind::PageBreak => {
            output.push_str("<div class=\"ff-pagebreak\" epub:type=\"pagebreak\"></div>")
        }
        NodeKind::Inline => render_epub_wrapped(
            "span",
            node,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
            &style_attr,
        ),
        NodeKind::GenericBlock { tag }
            if tag == "aside"
                && matches!(node.role, SemanticRole::Footnote | SemanticRole::Endnote) =>
        {
            let note_type = if node.role == SemanticRole::Endnote {
                "endnote"
            } else {
                "footnote"
            };
            output.push_str(&format!(
                "<aside{} epub:type=\"{}\">",
                style_attr, note_type
            ));
            render_epub_children(
                node,
                book,
                resource_paths,
                document_paths,
                output,
                diagnostics,
            );
            output.push_str("</aside>");
        }
        NodeKind::GenericInline { tag } | NodeKind::GenericBlock { tag } => {
            let safe_tag = safe_tag(tag);
            output.push_str(&format!("<{}{}>", safe_tag, style_attr));
            render_epub_children(
                node,
                book,
                resource_paths,
                document_paths,
                output,
                diagnostics,
            );
            output.push_str(&format!("</{}>", safe_tag));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_epub_wrapped(
    tag: &str,
    node: &Node,
    book: &Book,
    resource_paths: &BTreeMap<ResourceId, String>,
    document_paths: &BTreeMap<String, String>,
    output: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
    style_attr: &str,
) {
    output.push_str(&format!("<{}{}>", tag, style_attr));
    render_epub_children(
        node,
        book,
        resource_paths,
        document_paths,
        output,
        diagnostics,
    );
    output.push_str(&format!("</{}>", tag));
}

fn render_epub_children(
    node: &Node,
    book: &Book,
    resource_paths: &BTreeMap<ResourceId, String>,
    document_paths: &BTreeMap<String, String>,
    output: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for child in &node.children {
        render_epub_node(
            child,
            book,
            resource_paths,
            document_paths,
            output,
            diagnostics,
        );
    }
}

fn rewrite_href(href: &str, document_paths: &BTreeMap<String, String>) -> String {
    let (path, fragment) = href.split_once('#').unwrap_or((href, ""));
    let mapped = document_paths
        .get(path)
        .map(|mapped| {
            let mapped = mapped.strip_prefix("OEBPS/").unwrap_or(mapped);
            format!("../{mapped}")
        })
        .unwrap_or_else(|| path.to_owned());
    if fragment.is_empty() {
        mapped
    } else {
        format!("{}#{}", mapped, fragment)
    }
}

fn rewrite_nav_href(href: &str, document_paths: &BTreeMap<String, String>) -> String {
    let (path, fragment) = href.split_once('#').unwrap_or((href, ""));
    let mapped = document_paths.get(path).map(String::as_str).unwrap_or(path);
    let mapped = mapped.strip_prefix("OEBPS/").unwrap_or(mapped);
    if fragment.is_empty() {
        mapped.to_owned()
    } else {
        format!("{mapped}#{fragment}")
    }
}

fn rewrite_resource_href(path: &str) -> String {
    let path = path.strip_prefix("OEBPS/").unwrap_or(path);
    if path.starts_with("../") {
        path.to_owned()
    } else {
        format!("../{path}")
    }
}

fn safe_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn safe_tag(value: &str) -> &str {
    match value {
        "abbr" | "b" | "cite" | "div" | "dl" | "dt" | "dd" | "figure" | "figcaption" | "i"
        | "kbd" | "q" | "rb" | "rt" | "s" | "small" | "span" | "sub" | "sup" | "time" | "var"
        | "address" | "main" => value,
        _ => "div",
    }
}

fn stable_book_id(book: &Book) -> String {
    let mut hash = 2166136261u32;
    for byte in book.metadata.display_title().bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16777619);
    }
    format!("{hash:08x}")
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[derive(Clone, Debug)]
struct ManifestItem {
    id: String,
    href: String,
    media_type: String,
    properties: Vec<String>,
}

struct PackageData {
    metadata: Metadata,
    manifest: Vec<ManifestItem>,
    spine_items: Vec<String>,
    ncx_path: Option<String>,
    layout: folio_model::LayoutMode,
}

fn parse_package(opf: &XmlDocument<'_>, opf_path: &str) -> Result<PackageData, EpubError> {
    let package = opf.root_element();
    let metadata_node = package
        .children()
        .find(|node| node.is_element() && local_name(*node) == "metadata")
        .ok_or_else(|| EpubError::Invalid(format!("{opf_path} has no metadata element")))?;
    let metadata = parse_metadata(metadata_node);
    let manifest_node = package
        .children()
        .find(|node| node.is_element() && local_name(*node) == "manifest")
        .ok_or_else(|| EpubError::Invalid(format!("{opf_path} has no manifest element")))?;
    let mut manifest = Vec::new();
    for item in manifest_node
        .children()
        .filter(|node| node.is_element() && local_name(*node) == "item")
    {
        let id = item
            .attribute("id")
            .ok_or_else(|| EpubError::Invalid(format!("manifest item without id in {opf_path}")))?;
        let href = item
            .attribute("href")
            .ok_or_else(|| EpubError::Invalid(format!("manifest item {id} without href")))?;
        let media_type = item
            .attribute("media-type")
            .unwrap_or("application/octet-stream");
        let properties = item
            .attribute("properties")
            .map(|value| value.split_whitespace().map(ToOwned::to_owned).collect())
            .unwrap_or_default();
        manifest.push(ManifestItem {
            id: id.to_owned(),
            href: href.to_owned(),
            media_type: media_type.to_owned(),
            properties,
        });
    }
    let spine_node = package
        .children()
        .find(|node| node.is_element() && local_name(*node) == "spine")
        .ok_or_else(|| EpubError::Invalid(format!("{opf_path} has no spine element")))?;
    let mut spine = Vec::new();
    for itemref in spine_node
        .children()
        .filter(|node| node.is_element() && local_name(*node) == "itemref")
    {
        if itemref
            .attribute("linear")
            .is_some_and(|value| value.eq_ignore_ascii_case("no"))
        {
            continue;
        }
        let idref = itemref
            .attribute("idref")
            .ok_or_else(|| EpubError::Invalid("spine itemref without idref".to_owned()))?;
        spine.push(idref.to_owned());
    }
    if spine.is_empty() {
        spine.extend(
            manifest
                .iter()
                .filter(|item| {
                    item.media_type == "application/xhtml+xml" || item.media_type == "text/html"
                })
                .map(|item| item.id.clone()),
        );
    }
    let ncx_id = spine_node.attribute("toc").map(ToOwned::to_owned);
    let ncx_path = ncx_id.and_then(|id| {
        manifest
            .iter()
            .find(|item| item.id == id)
            .map(|item| item.href.clone())
    });
    let layout = if package
        .descendants()
        .any(|node| node.is_element() && is_pre_paginated(node))
    {
        folio_model::LayoutMode::Fixed
    } else {
        folio_model::LayoutMode::Reflowable
    };
    Ok(PackageData {
        metadata,
        manifest,
        spine_items: spine,
        ncx_path: ncx_path
            .map(|href| resolve_epub_href(&parent_path(opf_path), &href))
            .transpose()?,
        layout,
    })
}

fn parse_metadata(metadata: XmlNode<'_, '_>) -> Metadata {
    let mut result = Metadata::default();
    for child in metadata.children().filter(|node| node.is_element()) {
        let value = text_content(child);
        if value.is_empty() {
            continue;
        }
        match local_name(child) {
            "title" => {
                result.title.get_or_insert(value);
            }
            "subtitle" => {
                result.subtitle.get_or_insert(value);
            }
            "language" => {
                result.language.get_or_insert(value);
            }
            "creator" => result.add_author(value),
            "contributor" => result.contributors.push(value),
            "publisher" => {
                result.publisher.get_or_insert(value);
            }
            "identifier" => {
                result.add_identifier(value);
            }
            "description" => {
                result.description.get_or_insert(value);
            }
            "subject" => result.subjects.push(value),
            "date" => {
                if result.date.is_none() {
                    result.date = Some(value.clone());
                }
                result.dates.push(value);
            }
            "rights" => {
                result.rights.get_or_insert(value);
            }
            _ => {}
        };
    }
    result
}

fn parse_navigation(
    path: &Path,
    opf: &XmlDocument<'_>,
    manifest: &BTreeMap<String, ManifestItem>,
    opf_dir: &str,
    ncx_path: Option<&str>,
    limits: &EpubLimits,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Navigation, EpubError> {
    let nav_item = manifest
        .values()
        .find(|item| item.properties.iter().any(|property| property == "nav"));
    if let Some(nav_item) = nav_item {
        let nav_path = resolve_epub_href(opf_dir, &nav_item.href)?;
        let bytes = read_zip_entry(path, &nav_path, limits.max_xhtml_size)?;
        let xml = parse_xml(&nav_path, &bytes)?;
        let mut navigation = Navigation::default();
        for nav in xml
            .descendants()
            .filter(|node| node.is_element() && local_name(*node) == "nav")
        {
            let kind = attribute_value(nav, "epub:type").unwrap_or("");
            let list = nav
                .children()
                .find(|node| node.is_element() && local_name(*node) == "ol");
            let Some(list) = list else { continue };
            let points = parse_nav_list(list, &parent_path(&nav_path));
            if kind.split_whitespace().any(|value| value == "landmarks") {
                navigation.landmarks = points;
            } else if kind.split_whitespace().any(|value| value == "page-list") {
                navigation.page_list = points;
            } else if navigation.toc.is_empty() {
                navigation.toc = points;
            }
        }
        if navigation.start_location.is_none() {
            navigation.start_location =
                navigation.landmarks.first().map(|point| point.href.clone());
        }
        apply_epub2_guide(opf, opf_dir, &mut navigation);
        if navigation.toc.is_empty() {
            diagnostics.push(Diagnostic::warning(
                "FF-EPUB-NAV-0001",
                "EPUB navigation document has no TOC list.",
            ));
        }
        return Ok(navigation);
    }

    if let Some(ncx_path) = ncx_path {
        let bytes = read_zip_entry(path, ncx_path, limits.max_xhtml_size)?;
        let xml = parse_xml(ncx_path, &bytes)?;
        let mut navigation = Navigation::default();
        if let Some(nav_map) = xml
            .descendants()
            .find(|node| node.is_element() && local_name(*node) == "navMap")
        {
            navigation.toc = nav_points_from_ncx(nav_map, &parent_path(ncx_path));
        }
        if let Some(page_list) = xml
            .descendants()
            .find(|node| node.is_element() && local_name(*node) == "pageList")
        {
            navigation.page_list = nav_points_from_ncx(page_list, &parent_path(ncx_path));
        }
        apply_epub2_guide(opf, opf_dir, &mut navigation);
        return Ok(navigation);
    }

    let mut navigation = Navigation::default();
    apply_epub2_guide(opf, opf_dir, &mut navigation);
    Ok(navigation)
}

fn append_navigation_edges(navigation: &Navigation, graph: &mut AnchorGraph) {
    fn append(points: &[NavPoint], graph: &mut AnchorGraph, section: &str, ordinal: &mut usize) {
        for point in points {
            graph.edges.push(AnchorEdge {
                source: format!("navigation:{section}:{}", *ordinal),
                target: point.href.clone(),
                relation: AnchorRelation::Navigation,
            });
            *ordinal = (*ordinal).saturating_add(1);
            append(&point.children, graph, section, ordinal);
        }
    }
    let mut ordinal = 0usize;
    append(&navigation.toc, graph, "toc", &mut ordinal);
    append(&navigation.landmarks, graph, "landmarks", &mut ordinal);
    append(&navigation.page_list, graph, "page-list", &mut ordinal);
}

fn apply_epub2_guide(opf: &XmlDocument<'_>, opf_dir: &str, navigation: &mut Navigation) {
    if navigation.start_location.is_some() {
        return;
    }
    let Some(guide) = opf
        .descendants()
        .find(|node| node.is_element() && local_name(*node) == "guide")
    else {
        return;
    };
    let reference = guide.children().find(|node| {
        node.is_element()
            && local_name(*node) == "reference"
            && node.attribute("href").is_some()
            && node
                .attribute("type")
                .is_some_and(|value| matches!(value, "text" | "start" | "bodymatter"))
    });
    if let Some(reference) = reference {
        if let Some(href) = reference.attribute("href") {
            navigation.start_location = resolve_epub_link_href(opf_dir, href).ok();
        }
    }
}

fn parse_nav_list(list: XmlNode<'_, '_>, base_dir: &str) -> Vec<NavPoint> {
    let mut result = Vec::new();
    for li in list
        .children()
        .filter(|node| node.is_element() && local_name(*node) == "li")
    {
        let link = li
            .children()
            .find(|node| node.is_element() && matches!(local_name(*node), "a" | "span"));
        let Some(link) = link else { continue };
        let href = link.attribute("href").unwrap_or("");
        let href = resolve_epub_link_href(base_dir, href).unwrap_or_else(|_| href.to_owned());
        let child_list = li
            .children()
            .find(|node| node.is_element() && local_name(*node) == "ol");
        result.push(NavPoint {
            label: text_content(link),
            href,
            children: child_list
                .map(|node| parse_nav_list(node, base_dir))
                .unwrap_or_default(),
        });
    }
    result
}

fn nav_points_from_ncx(parent: XmlNode<'_, '_>, base_dir: &str) -> Vec<NavPoint> {
    let mut result = Vec::new();
    for point in parent
        .children()
        .filter(|node| node.is_element() && local_name(*node) == "navPoint")
    {
        let label = point
            .descendants()
            .find(|node| node.is_element() && local_name(*node) == "text")
            .map(text_content)
            .unwrap_or_default();
        let src = point
            .descendants()
            .find(|node| node.is_element() && local_name(*node) == "content")
            .and_then(|node| node.attribute("src"))
            .unwrap_or("");
        let href = resolve_epub_link_href(base_dir, src).unwrap_or_else(|_| src.to_owned());
        result.push(NavPoint {
            label,
            href,
            children: nav_points_from_ncx(point, base_dir),
        });
    }
    result
}

fn local_name<'a, 'input>(node: XmlNode<'a, 'input>) -> &'input str {
    node.tag_name()
        .name()
        .rsplit(':')
        .next()
        .unwrap_or(node.tag_name().name())
}

fn attribute_value<'a, 'input>(node: XmlNode<'a, 'input>, name: &str) -> Option<&'a str> {
    node.attribute(name).or_else(|| {
        let local = name.rsplit(':').next().unwrap_or(name);
        node.attributes()
            .find(|attribute| attribute.name() == local)
            .map(|attribute| attribute.value())
    })
}

fn is_pre_paginated(node: XmlNode<'_, '_>) -> bool {
    attribute_value(node, "rendition:layout").is_some_and(|value| value.contains("pre-paginated"))
        || (attribute_value(node, "property")
            .is_some_and(|value| value.eq_ignore_ascii_case("rendition:layout"))
            && node
                .text()
                .is_some_and(|value| value.contains("pre-paginated")))
        || (attribute_value(node, "property")
            .is_some_and(|value| value.eq_ignore_ascii_case("rendition:layout"))
            && attribute_value(node, "content")
                .is_some_and(|value| value.contains("pre-paginated")))
}

fn text_content(node: XmlNode<'_, '_>) -> String {
    fn collect(node: XmlNode<'_, '_>, output: &mut String) {
        for child in node.children() {
            if child.is_text() {
                output.push_str(child.text().unwrap_or_default());
            } else if child.is_element() {
                collect(child, output);
            }
        }
    }
    let mut output = String::new();
    collect(node, &mut output);
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_xml<'a>(path: &str, bytes: &'a [u8]) -> Result<XmlDocument<'a>, EpubError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| EpubError::Invalid(format!("{path} is not UTF-8: {error}")))?;
    // EPUBs produced by Kindle tooling and by Bōkō commonly carry only the
    // external XHTML 1.1 DTD.  Permit that declaration without enabling
    // arbitrary/internal DTDs; roxmltree does not fetch the external URL.
    let allows_external_xhtml_doctype = folio_normalize::strip_external_xhtml_doctype(text) != text;
    let options = roxmltree::ParsingOptions {
        allow_dtd: allows_external_xhtml_doctype,
        ..roxmltree::ParsingOptions::default()
    };
    XmlDocument::parse_with_options(text, options).map_err(|source| EpubError::Xml {
        path: path.to_owned(),
        source,
    })
}

fn validate_zip_safety(path: &Path, limits: &EpubLimits) -> Result<(), EpubError> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    if archive.len() > limits.max_entries {
        return Err(EpubError::Limit(format!(
            "ZIP entry count exceeds {}",
            limits.max_entries
        )));
    }
    let mut total = 0u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if index == 0 && entry.name() != "mimetype" {
            return Err(EpubError::Invalid(
                "OCF mimetype entry must be the first ZIP entry".to_owned(),
            ));
        }
        if entry.enclosed_name().is_none() || entry.name().starts_with('/') {
            return Err(EpubError::Invalid(format!(
                "unsafe ZIP path: {}",
                entry.name()
            )));
        }
        if entry.size() > limits.max_entry_size {
            return Err(EpubError::Limit(format!(
                "ZIP entry {} exceeds {} bytes",
                entry.name(),
                limits.max_entry_size
            )));
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| EpubError::Limit("ZIP total size overflow".to_owned()))?;
        if total > limits.max_total_uncompressed {
            return Err(EpubError::Limit(format!(
                "ZIP total uncompressed size exceeds {}",
                limits.max_total_uncompressed
            )));
        }
    }
    Ok(())
}

fn read_zip_entry(path: &Path, name: &str, max_size: u64) -> Result<Vec<u8>, EpubError> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    let mut entry = archive.by_name(name)?;
    if entry.size() > max_size {
        return Err(EpubError::Limit(format!(
            "ZIP entry {name} exceeds {max_size} bytes"
        )));
    }
    let capacity = usize::try_from(entry.size()).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity.min(16 << 20));
    let mut limited = entry.by_ref().take(max_size.saturating_add(1));
    limited.read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_size {
        return Err(EpubError::Limit(format!(
            "ZIP entry {name} exceeds {max_size} bytes"
        )));
    }
    Ok(bytes)
}

fn zip_entry_size(path: &Path, name: &str) -> Result<Option<u64>, EpubError> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    let result = match archive.by_name(name) {
        Ok(entry) => Ok(Some(entry.size())),
        Err(ZipError::FileNotFound) => Ok(None),
        Err(error) => Err(EpubError::Zip(error)),
    };
    result
}

fn parent_path(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_owned())
        .unwrap_or_default()
}

pub fn normalize_epub_path(path: &str) -> Result<String, EpubError> {
    let decoded = percent_decode(path.split('#').next().unwrap_or(path))?;
    let decoded = decoded.replace('\\', "/");
    let mut parts = Vec::new();
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(EpubError::Invalid(format!(
                        "path escapes EPUB root: {path}"
                    )));
                }
            }
            value => parts.push(value),
        }
    }
    if parts.is_empty() {
        return Err(EpubError::Invalid(format!("empty EPUB path: {path}")));
    }
    Ok(parts.join("/"))
}

pub fn resolve_epub_href(base_dir: &str, href: &str) -> Result<String, EpubError> {
    let href = href.split('#').next().unwrap_or(href);
    if href.is_empty() {
        return normalize_epub_path(base_dir);
    }
    let combined = if base_dir.is_empty() {
        href.to_owned()
    } else {
        format!("{base_dir}/{href}")
    };
    normalize_epub_path(&combined)
}

fn resolve_epub_link_href(base_dir: &str, href: &str) -> Result<String, EpubError> {
    let (path, fragment) = href.split_once('#').unwrap_or((href, ""));
    let path = resolve_epub_href(base_dir, path)?;
    if fragment.is_empty() {
        Ok(path)
    } else {
        Ok(format!("{path}#{fragment}"))
    }
}

/// Validate the OCF envelope before an output is atomically committed.  The
/// path-backed reader remains the authoritative full semantic validator; this
/// byte-oriented check is used by Core for pre-commit validation.
pub fn validate_bytes(bytes: &[u8]) -> Result<(), EpubError> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor)?;
    let first = archive.by_index(0)?;
    if first.name() != "mimetype" {
        return Err(EpubError::Invalid(
            "OCF mimetype entry must be the first ZIP entry".to_owned(),
        ));
    }
    drop(first);
    let mimetype = read_archive_entry(&mut archive, "mimetype")?;
    if mimetype != b"application/epub+zip" {
        return Err(EpubError::Invalid(
            "mimetype entry is not application/epub+zip".to_owned(),
        ));
    }
    for required in [
        "META-INF/container.xml",
        "OEBPS/content.opf",
        "OEBPS/nav.xhtml",
    ] {
        if archive.by_name(required).is_err() {
            return Err(EpubError::Invalid(format!(
                "missing required EPUB entry {required}"
            )));
        }
    }
    Ok(())
}

fn read_archive_entry<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>, EpubError> {
    let mut entry = archive.by_name(name)?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn percent_decode(value: &str) -> Result<String, EpubError> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let end = index
                .checked_add(3)
                .ok_or_else(|| EpubError::Invalid("percent escape overflow".to_owned()))?;
            let digits = bytes
                .get(index + 1..end)
                .ok_or_else(|| EpubError::Invalid(format!("invalid percent escape in {value}")))?;
            let high = hex_digit(digits[0])
                .ok_or_else(|| EpubError::Invalid(format!("invalid percent escape in {value}")))?;
            let low = hex_digit(digits[1])
                .ok_or_else(|| EpubError::Invalid(format!("invalid percent escape in {value}")))?;
            output.push((high << 4) | low);
            index = end;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output)
        .map_err(|error| EpubError::Invalid(format!("path is not UTF-8: {error}")))
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn resource_kind(media_type: &str, href: &str) -> ResourceKind {
    let media_type = media_type.to_ascii_lowercase();
    let extension = href
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match media_type.as_str() {
        "image/jpeg" | "image/jpg" => ResourceKind::Jpeg,
        "image/png" => ResourceKind::Png,
        "image/gif" => ResourceKind::Gif,
        "image/svg+xml" => ResourceKind::Svg,
        "text/css" => ResourceKind::Stylesheet,
        "font/woff"
        | "font/woff2"
        | "application/font-woff"
        | "application/vnd.ms-opentype"
        | "font/ttf"
        | "font/otf" => ResourceKind::Font,
        "audio/mpeg" | "audio/mp4" | "audio/ogg" => ResourceKind::Audio,
        _ => match extension.as_str() {
            "jpg" | "jpeg" => ResourceKind::Jpeg,
            "png" => ResourceKind::Png,
            "gif" => ResourceKind::Gif,
            "svg" => ResourceKind::Svg,
            "css" => ResourceKind::Stylesheet,
            "ttf" | "otf" | "woff" | "woff2" => ResourceKind::Font,
            _ => ResourceKind::Unknown,
        },
    }
}

#[derive(Clone, Debug)]
struct ZipResourceLoader {
    path: PathBuf,
    max_entry_size: u64,
}

impl ResourceLoader for ZipResourceLoader {
    fn load(&self, locator: &str, max_bytes: Option<u64>) -> Result<Vec<u8>, ResourceLoadError> {
        let file = File::open(&self.path).map_err(ResourceLoadError::Io)?;
        let mut archive =
            ZipArchive::new(file).map_err(|error| ResourceLoadError::Other(error.to_string()))?;
        let mut entry = archive.by_name(locator).map_err(|error| match error {
            ZipError::FileNotFound => ResourceLoadError::NotFound(locator.to_owned()),
            other => ResourceLoadError::Other(other.to_string()),
        })?;
        let limit = max_bytes
            .unwrap_or(self.max_entry_size)
            .min(self.max_entry_size);
        if entry.size() > limit {
            return Err(ResourceLoadError::TooLarge(locator.to_owned()));
        }
        let mut bytes =
            Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0).min(16 << 20));
        entry
            .read_to_end(&mut bytes)
            .map_err(ResourceLoadError::Io)?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use folio_model::{
        Book, ComputedStyle, Document, DocumentId, FontFace, MemoryResourceLoader, Node, NodeId,
        NodeKind, Resource, ResourceId,
    };
    use std::{
        io::{Seek, Write},
        path::Path,
        sync::Arc,
    };
    use zip::{write::SimpleFileOptions, ZipWriter};

    #[test]
    fn paths_are_normalized_and_fragments_removed() {
        assert_eq!(
            normalize_epub_path("OEBPS/../Text/ch%31.xhtml#part").unwrap(),
            "Text/ch1.xhtml"
        );
        assert_eq!(
            resolve_epub_link_href("OEBPS/text", "../chapter.xhtml#part").unwrap(),
            "OEBPS/chapter.xhtml#part"
        );
        assert!(normalize_epub_path("../../escape").is_err());
    }

    #[test]
    fn resource_kinds_use_media_type_first() {
        assert_eq!(resource_kind("image/jpeg", "cover.bin"), ResourceKind::Jpeg);
        assert_eq!(
            resource_kind("text/css", "style.dat"),
            ResourceKind::Stylesheet
        );
    }

    #[test]
    fn exporter_embeds_replacement_font_and_emits_linked_font_face_css() {
        let font_bytes = vec![0, 1, 0, 0, 1, 2, 3, 4];
        let mut loader = MemoryResourceLoader::default();
        loader.insert("fonts/BookFont.ttf".to_owned(), font_bytes.clone());
        let mut book = Book::new().with_resource_loader(Arc::new(loader));
        book.resources.push(Resource {
            id: ResourceId::new(0),
            path: "fonts/BookFont.ttf".to_owned(),
            media_type: "font/ttf".to_owned(),
            kind: ResourceKind::Font,
            properties: Vec::new(),
            size: Some(font_bytes.len() as u64),
        });
        book.font_faces.push(FontFace {
            resource: ResourceId::new(0),
            family: "Book Serif".to_owned(),
        });
        book.documents.push(Document {
            id: DocumentId::new(0),
            href: "chapter.xhtml".to_owned(),
            media_type: "application/xhtml+xml".to_owned(),
            title: Some("Chapter".to_owned()),
            nodes: Vec::new(),
        });

        let artifact = export(&book, &EpubOptions::default()).unwrap();
        let mut archive = ZipArchive::new(std::io::Cursor::new(artifact.bytes)).unwrap();
        let mut css = String::new();
        archive
            .by_name("OEBPS/styles.css")
            .unwrap()
            .read_to_string(&mut css)
            .unwrap();
        let mut embedded = Vec::new();
        archive
            .by_name("OEBPS/assets/0000-BookFont.ttf")
            .unwrap()
            .read_to_end(&mut embedded)
            .unwrap();
        let mut xhtml = String::new();
        archive
            .by_name("OEBPS/text/0001.xhtml")
            .unwrap()
            .read_to_string(&mut xhtml)
            .unwrap();

        assert!(css.contains("font-family:\"Book Serif\""));
        assert!(css.contains("url(\"assets/0000-BookFont.ttf\") format(\"truetype\")"));
        assert!(xhtml.contains("href=\"../styles.css\""));
        assert_eq!(embedded, font_bytes);
    }

    #[test]
    fn export_preserves_document_order_when_hrefs_sort_lexically_differently() {
        let mut book = Book::new();
        let style = book.styles.intern(ComputedStyle::default());
        let hrefs = [
            "document-0.xhtml",
            "document-1.xhtml",
            "document-2.xhtml",
            "document-10.xhtml",
        ];
        let expected = hrefs
            .iter()
            .enumerate()
            .map(|(index, href)| {
                let text = format!("document-{index}");
                book.documents.push(Document {
                    id: DocumentId::new(index as u32),
                    href: (*href).to_owned(),
                    media_type: "application/xhtml+xml".to_owned(),
                    title: None,
                    nodes: vec![Node::new(
                        NodeId::new(index as u32),
                        NodeKind::Text {
                            value: text.clone(),
                        },
                        style,
                        Vec::new(),
                    )],
                });
                text
            })
            .collect::<Vec<_>>();

        let artifact = export(&book, &EpubOptions::default()).unwrap();
        let path = std::env::temp_dir().join(format!(
            "folioforge-epub-spine-order-{}.epub",
            std::process::id()
        ));
        std::fs::write(&path, artifact.bytes).unwrap();
        let imported = EpubReader::default().read(&path);
        std::fs::remove_file(path).unwrap();
        let imported = imported.unwrap();
        let actual = imported
            .book
            .documents
            .iter()
            .map(|document| {
                document
                    .nodes
                    .iter()
                    .map(Node::text_content)
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
    }

    #[test]
    fn reader_builds_canonical_ir_from_a_small_epub() {
        let path =
            std::env::temp_dir().join(format!("folioforge-epub-{}.epub", std::process::id()));
        let file = File::create(&path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        add_test_entry(&mut zip, "mimetype", "application/epub+zip", options);
        add_test_entry(
            &mut zip,
            "META-INF/container.xml",
            r#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#,
            options,
        );
        add_test_entry(
            &mut zip,
            "OEBPS/content.opf",
            r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Smoke</dc:title><dc:language>zh</dc:language></metadata><manifest><item id="c" href="chapter.xhtml" media-type="application/xhtml+xml"/><item id="n" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest><spine><itemref idref="c"/></spine></package>"#,
            options,
        );
        add_test_entry(
            &mut zip,
            "OEBPS/chapter.xhtml",
            r#"<?xml version="1.0" encoding="utf-8"?><!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.1//EN" "http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd"><html xmlns="http://www.w3.org/1999/xhtml"><body><h1 id="one">第一章</h1><p>Hello，世界。</p></body></html>"#,
            options,
        );
        add_test_entry(
            &mut zip,
            "OEBPS/nav.xhtml",
            r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="chapter.xhtml#one">第一章</a></li></ol></nav></body></html>"#,
            options,
        );
        zip.finish().unwrap();
        let report = EpubReader::default().read(&path).unwrap();
        assert_eq!(report.book.metadata.title.as_deref(), Some("Smoke"));
        assert_eq!(report.book.documents.len(), 1);
        assert_eq!(report.book.navigation.toc.len(), 1);
        assert!(report.book.feature_summary().contains_key("heading"));
        assert!(Path::new(&path).exists());
        std::fs::remove_file(path).unwrap();
    }

    fn add_test_entry<W: Write + Seek>(
        zip: &mut ZipWriter<W>,
        name: &str,
        value: &str,
        options: SimpleFileOptions,
    ) {
        zip.start_file(name, options).unwrap();
        zip.write_all(value.as_bytes()).unwrap();
    }
}
