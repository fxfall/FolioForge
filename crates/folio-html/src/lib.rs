//! Offline HTML and HTMLZ input adapter.
//!
//! HTMLZ is deliberately treated as a compatibility container rather than a
//! claimed universal standard.  Both single HTML and HTMLZ feed the same
//! XHTML-shaped document inputs into `folio-normalize`; the container layer
//! only selects documents and exposes safe local resources.

use std::{
    collections::{BTreeMap, BTreeSet},
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
    Book, Diagnostic, MemoryResourceLoader, Metadata, NavPoint, Resource, ResourceId, ResourceKind,
};
use folio_normalize::{
    import_xhtml_documents_with_options, XhtmlDocumentInput, XhtmlImportOptions,
};
use folio_style::{StyleResolver, TargetProfile};
use zip::ZipArchive;

const MAX_HTMLZ_ENTRIES: usize = 100_000;
const MAX_HTMLZ_ENTRY: u64 = 256 << 20;
const MAX_HTMLZ_TOTAL: u64 = 1 << 30;

#[derive(Debug, thiserror::Error)]
pub enum HtmlError {
    #[error("HTML source has no document")]
    Empty,
    #[error("HTML source is not valid UTF-8 or a supported legacy text encoding: {0}")]
    Encoding(String),
    #[error("HTML/XML parsing failed for {path}: {message}")]
    Parse { path: String, message: String },
    #[error("HTML source path is unsafe: {0}")]
    UnsafePath(String),
    #[error("HTMLZ container is invalid: {0}")]
    Container(String),
    #[error("HTML safety limit exceeded: {0}")]
    Limit(String),
    #[error("HTML source I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug)]
struct HtmlDocument {
    href: String,
    content: String,
}

#[derive(Clone, Debug, Default)]
struct HtmlNativeDocument {
    documents: Vec<HtmlDocument>,
    metadata: Metadata,
    resources: BTreeMap<String, Vec<u8>>,
    stylesheets: BTreeMap<String, String>,
    diagnostics: Vec<Diagnostic>,
}

pub struct HtmlAdapter;

impl FormatAdapter for HtmlAdapter {
    fn format(&self) -> DetectedFormat {
        DetectedFormat::Html
    }

    fn support(&self) -> FormatSupport {
        html_support()
    }

    fn detect(&self, source: &BookSource) -> Result<DetectionResult, FormatError> {
        match source.kind() {
            SourceKind::Directory => {
                let count = source
                    .files()
                    .iter()
                    .filter(|file| is_html_path(&file.relative_path))
                    .count();
                if count == 0 {
                    return Err(FormatError::Unsupported(
                        "directory contains no HTML documents".to_owned(),
                    ));
                }
                Ok(DetectionResult {
                    format: DetectedFormat::Html.name().to_owned(),
                    confidence: DetectionConfidence::Strong,
                    evidence: vec![format!("directory contains {count} HTML documents")],
                    warnings: vec![
                        "document order is taken from an explicit manifest, index links, or natural filename order".to_owned(),
                    ],
                })
            }
            SourceKind::SingleFile => {
                let file = source.files().first().ok_or_else(|| {
                    FormatError::Unsupported("source contains no files".to_owned())
                })?;
                let detected =
                    folio_input::detect_format(&file.relative_path, &fs::read(&file.path)?)
                        .map_err(|error| FormatError::Unsupported(error.to_string()))?;
                match detected {
                    DetectedFormat::Html => Ok(DetectionResult::exact(
                        DetectedFormat::Html,
                        "HTML extension or HTML document signature",
                    )),
                    DetectedFormat::Htmlz => Ok(DetectionResult::exact(
                        DetectedFormat::Htmlz,
                        "HTMLZ extension and ZIP HTML entry",
                    )),
                    _ => Err(FormatError::Unsupported(format!(
                        "detected {} instead of HTML/HTMLZ",
                        detected.name()
                    ))),
                }
            }
            SourceKind::FileSet => Err(FormatError::Unsupported(
                "HTML adapter expects a directory or one HTML/HTMLZ file".to_owned(),
            )),
        }
    }

    fn import(
        &self,
        source: &BookSource,
        context: &ImportContext,
    ) -> Result<ImportedBook, FormatError> {
        let native = match source.kind() {
            SourceKind::Directory => import_directory(source)?,
            SourceKind::SingleFile => {
                let file = source.files().first().ok_or_else(|| {
                    FormatError::Unsupported("source contains no files".to_owned())
                })?;
                let bytes = fs::read(&file.path)?;
                let detected = folio_input::detect_format(&file.relative_path, &bytes)
                    .map_err(|error| FormatError::Unsupported(error.to_string()))?;
                if detected == DetectedFormat::Htmlz {
                    import_htmlz(&bytes, context.strict)?
                } else {
                    import_single(source, &file.relative_path.to_string_lossy(), &bytes)?
                }
            }
            SourceKind::FileSet => {
                return Err(FormatError::Unsupported(
                    "HTML adapter expects a directory or one HTML/HTMLZ file".to_owned(),
                ))
            }
        };
        let (book, decode_diagnostics) = decode_native(source, native.clone())?;
        let mut diagnostics = native.diagnostics;
        diagnostics.extend(decode_diagnostics);
        let input_loss = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.starts_with("FF-HTML-"))
            .map(|diagnostic| diagnostic.message.clone())
            .collect();
        Ok(ImportedBook {
            format: if source
                .files()
                .first()
                .and_then(|file| file.relative_path.extension())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("htmlz"))
            {
                DetectedFormat::Htmlz
            } else {
                DetectedFormat::Html
            },
            parser: "folio-html/1".to_owned(),
            book,
            diagnostics,
            input_loss,
            text: None,
        })
    }
}

/// HTMLZ has a separate registry identity, but deliberately delegates all
/// source decoding to the same HTML adapter and semantic path.
pub struct HtmlzAdapter;

impl FormatAdapter for HtmlzAdapter {
    fn format(&self) -> DetectedFormat {
        DetectedFormat::Htmlz
    }

    fn support(&self) -> FormatSupport {
        html_support()
    }

    fn detect(&self, source: &BookSource) -> Result<DetectionResult, FormatError> {
        if source.kind() != SourceKind::SingleFile {
            return Err(FormatError::Unsupported(
                "HTMLZ adapter expects one .htmlz file".to_owned(),
            ));
        }
        let file = source
            .files()
            .first()
            .ok_or_else(|| FormatError::Unsupported("source contains no files".to_owned()))?;
        let detected = folio_input::detect_format(&file.relative_path, &fs::read(&file.path)?)
            .map_err(|error| FormatError::Unsupported(error.to_string()))?;
        if detected != DetectedFormat::Htmlz {
            return Err(FormatError::Unsupported(format!(
                "detected {} instead of HTMLZ",
                detected.name()
            )));
        }
        Ok(DetectionResult::exact(
            DetectedFormat::Htmlz,
            "HTMLZ extension and ZIP HTML entry",
        ))
    }

    fn import(
        &self,
        source: &BookSource,
        context: &ImportContext,
    ) -> Result<ImportedBook, FormatError> {
        let mut imported = HtmlAdapter.import(source, context)?;
        imported.format = DetectedFormat::Htmlz;
        Ok(imported)
    }
}

fn html_support() -> FormatSupport {
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

fn import_single(
    source: &BookSource,
    href: &str,
    bytes: &[u8],
) -> Result<HtmlNativeDocument, FormatError> {
    let decoded = decode_html(bytes).map_err(|error| FormatError::Invalid(error.to_string()))?;
    let content = prepare_markup(&decoded);
    let mut native = HtmlNativeDocument::default();
    record_unsafe_script_diagnostic(&content, &mut native.diagnostics);
    native.documents.push(HtmlDocument {
        href: href.replace('\\', "/"),
        content,
    });
    collect_metadata(&native.documents[0], &mut native.metadata);
    collect_source_resources(
        source,
        &native.documents,
        &mut native.resources,
        &mut native.diagnostics,
    )?;
    load_external_stylesheets_from_directory(
        source.root(),
        &native.documents,
        &mut native.stylesheets,
        &mut native.diagnostics,
    )?;
    collect_stylesheet_resources(
        source,
        &native.stylesheets,
        &mut native.resources,
        &mut native.diagnostics,
    )?;
    inject_stylesheets(&mut native.documents, &native.stylesheets);
    Ok(native)
}

fn import_directory(source: &BookSource) -> Result<HtmlNativeDocument, FormatError> {
    let (files, order_diagnostics) = ordered_html_files(source)?;
    if files.is_empty() {
        return Err(FormatError::Invalid(HtmlError::Empty.to_string()));
    }
    let mut native = HtmlNativeDocument::default();
    native.diagnostics.extend(order_diagnostics);
    for file in files {
        let bytes = fs::read(&file.path)?;
        let content = prepare_markup(
            &decode_html(&bytes).map_err(|error| FormatError::Invalid(error.to_string()))?,
        );
        record_unsafe_script_diagnostic(&content, &mut native.diagnostics);
        native.documents.push(HtmlDocument {
            href: file.relative_path.to_string_lossy().replace('\\', "/"),
            content,
        });
    }
    if let Some(first) = native.documents.first() {
        collect_metadata(first, &mut native.metadata);
    }
    collect_source_resources(
        source,
        &native.documents,
        &mut native.resources,
        &mut native.diagnostics,
    )?;
    load_external_stylesheets_from_directory(
        source.root(),
        &native.documents,
        &mut native.stylesheets,
        &mut native.diagnostics,
    )?;
    collect_stylesheet_resources(
        source,
        &native.stylesheets,
        &mut native.resources,
        &mut native.diagnostics,
    )?;
    inject_stylesheets(&mut native.documents, &native.stylesheets);
    Ok(native)
}

fn import_htmlz(bytes: &[u8], strict: bool) -> Result<HtmlNativeDocument, FormatError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|error| {
        FormatError::Invalid(HtmlError::Container(error.to_string()).to_string())
    })?;
    if archive.len() > MAX_HTMLZ_ENTRIES {
        return Err(FormatError::Invalid(
            HtmlError::Limit(format!("HTMLZ has more than {MAX_HTMLZ_ENTRIES} entries"))
                .to_string(),
        ));
    }
    let mut entries = BTreeMap::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            FormatError::Invalid(HtmlError::Container(error.to_string()).to_string())
        })?;
        let name = normalize_archive_path(entry.name()).ok_or_else(|| {
            FormatError::Invalid(HtmlError::UnsafePath(entry.name().to_owned()).to_string())
        })?;
        if entry.is_dir() {
            continue;
        }
        if entry.size() > MAX_HTMLZ_ENTRY {
            return Err(FormatError::Invalid(
                HtmlError::Limit(format!("entry {name} exceeds {MAX_HTMLZ_ENTRY} bytes"))
                    .to_string(),
            ));
        }
        total = total.saturating_add(entry.size());
        if total > MAX_HTMLZ_TOTAL {
            return Err(FormatError::Invalid(
                HtmlError::Limit(format!(
                    "uncompressed entries exceed {MAX_HTMLZ_TOTAL} bytes"
                ))
                .to_string(),
            ));
        }
        if entries.contains_key(&name) {
            return Err(FormatError::Invalid(
                HtmlError::Container(format!("duplicate entry {name}")).to_string(),
            ));
        }
        let mut value = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut value)?;
        entries.insert(name, value);
    }
    let html_paths = entries
        .keys()
        .filter(|name| is_html_path(Path::new(name)))
        .cloned()
        .collect::<Vec<_>>();
    if html_paths.is_empty() {
        return Err(FormatError::Invalid(HtmlError::Empty.to_string()));
    }
    let html_names = html_paths.iter().cloned().collect::<BTreeSet<_>>();
    let manifest = ["folioforge-manifest.txt", "manifest.txt"]
        .into_iter()
        .filter_map(|name| entries.get(name).map(|value| (name, value)))
        .collect::<Vec<_>>();
    if manifest.len() > 1 {
        return Err(FormatError::Invalid(
            "HTMLZ contains more than one recognized manifest".to_owned(),
        ));
    }
    let (ordered, used_manifest, manifest_appended) = if let Some((_, bytes)) = manifest.first() {
        let (names, appended) = parse_manifest_order(bytes, &html_names)?;
        (names, true, appended)
    } else {
        let entrypoint = if entries.contains_key("index.html") {
            "index.html".to_owned()
        } else if entries.contains_key("index.htm") {
            "index.htm".to_owned()
        } else if html_paths.len() == 1 {
            html_paths[0].clone()
        } else if strict {
            return Err(FormatError::Invalid(
                "HTMLZ has multiple HTML files and no index.html/index.htm entrypoint".to_owned(),
            ));
        } else {
            html_paths[0].clone()
        };
        let mut names = vec![entrypoint.clone()];
        names.extend(html_paths.into_iter().filter(|path| path != &entrypoint));
        (names, false, false)
    };
    let mut native = HtmlNativeDocument::default();
    if used_manifest {
        native.diagnostics.push(Diagnostic::info(
            "FF-HTMLZ-ORDER-0002",
            "HTMLZ document order came from folioforge-manifest.txt or manifest.txt",
        ));
        if manifest_appended {
            native.diagnostics.push(Diagnostic::warning(
                "FF-HTMLZ-ORDER-0003",
                "HTMLZ manifest omitted one or more HTML documents; omitted documents were appended in natural path order",
            ));
        }
    } else if ordered.len() > 1 {
        native.diagnostics.push(Diagnostic::warning(
            "FF-HTMLZ-ORDER-0001",
            "HTMLZ has no explicit package manifest; entrypoint and natural archive path order were used",
        ));
    }
    for path in ordered {
        let content =
            decode_html(entries.get(&path).ok_or_else(|| {
                FormatError::Invalid(format!("HTMLZ entrypoint is missing: {path}"))
            })?)
            .map_err(|error| FormatError::Invalid(error.to_string()))?;
        let content = prepare_markup(&content);
        record_unsafe_script_diagnostic(&content, &mut native.diagnostics);
        record_remote_document_resource_diagnostics(&content, &mut native.diagnostics);
        native.documents.push(HtmlDocument {
            href: path,
            content,
        });
    }
    if let Some(first) = native.documents.first() {
        collect_metadata(first, &mut native.metadata);
    }
    for (name, value) in entries {
        if is_resource_path(Path::new(&name)) {
            if let Some(text) = decode_optional_text(&value) {
                if is_stylesheet_path(Path::new(&name)) {
                    native.stylesheets.insert(name.clone(), text);
                    continue;
                }
            }
            native.resources.insert(name, value);
        }
    }
    for stylesheet in native.stylesheets.values() {
        record_remote_css_diagnostics(stylesheet, &mut native.diagnostics);
    }
    inject_stylesheets(&mut native.documents, &native.stylesheets);
    Ok(native)
}

fn ordered_html_files(
    source: &BookSource,
) -> Result<(Vec<folio_input::SourceFile>, Vec<Diagnostic>), FormatError> {
    let mut files = source
        .files()
        .iter()
        .filter(|file| is_html_path(&file.relative_path))
        .cloned()
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let mut diagnostics = Vec::new();
    let manifests = ["folioforge-manifest.txt", "manifest.txt"]
        .into_iter()
        .filter_map(|name| {
            source
                .files()
                .iter()
                .find(|file| file.relative_path == Path::new(name))
                .map(|file| (name, file))
        })
        .collect::<Vec<_>>();
    if manifests.len() > 1 {
        return Err(FormatError::Invalid(
            "HTML directory contains more than one recognized manifest".to_owned(),
        ));
    }
    if let Some((_, manifest_file)) = manifests.first() {
        let bytes = fs::read(&manifest_file.path)?;
        let names = files
            .iter()
            .map(|file| file.relative_path.to_string_lossy().replace('\\', "/"))
            .collect::<BTreeSet<_>>();
        let (ordered_names, appended) = parse_manifest_order(&bytes, &names)?;
        let by_name = files
            .into_iter()
            .map(|file| {
                (
                    file.relative_path.to_string_lossy().replace('\\', "/"),
                    file,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let ordered = ordered_names
            .into_iter()
            .filter_map(|name| by_name.get(&name).cloned())
            .collect::<Vec<_>>();
        diagnostics.push(Diagnostic::info(
            "FF-HTML-ORDER-0002",
            "HTML document order came from folioforge-manifest.txt or manifest.txt",
        ));
        if appended {
            diagnostics.push(Diagnostic::warning(
                "FF-HTML-ORDER-0003",
                "HTML manifest omitted one or more documents; omitted documents were appended in natural path order",
            ));
        }
        return Ok((ordered, diagnostics));
    }
    let index_position = files.iter().position(|file| {
        file.relative_path.file_name().is_some_and(|name| {
            name.eq_ignore_ascii_case("index.html") || name.eq_ignore_ascii_case("index.htm")
        })
    });
    let Some(index_position) = index_position else {
        if files.len() > 1 {
            diagnostics.push(Diagnostic::warning(
                "FF-HTML-ORDER-0001",
                "HTML directory has no explicit manifest or index links; natural filename order was used",
            ));
        }
        return Ok((files, diagnostics));
    };
    let index_file = files[index_position].clone();
    let index_href = index_file
        .relative_path
        .to_string_lossy()
        .replace('\\', "/");
    let index_content = decode_html(&fs::read(&index_file.path)?)
        .map_err(|error| FormatError::Invalid(error.to_string()))?;
    let by_name = files
        .iter()
        .map(|file| {
            (
                file.relative_path.to_string_lossy().replace('\\', "/"),
                file,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut ordered_names = vec![index_href.clone()];
    for href in attribute_values(&index_content, "href") {
        let Some(relative) = resolve_local_path(&index_href, &href) else {
            continue;
        };
        if is_html_path(Path::new(&relative))
            && by_name.contains_key(&relative)
            && !ordered_names.iter().any(|name| name == &relative)
        {
            ordered_names.push(relative);
        }
    }
    for file in &files {
        let name = file.relative_path.to_string_lossy().replace('\\', "/");
        if !ordered_names.iter().any(|item| item == &name) {
            ordered_names.push(name);
        }
    }
    let linked_count = ordered_names.len().saturating_sub(1);
    if files.len() > 1 {
        if linked_count > 0 {
            diagnostics.push(Diagnostic::info(
                "FF-HTML-ORDER-0004",
                "HTML document order used index document links, with unlinked documents appended naturally",
            ));
        } else {
            diagnostics.push(Diagnostic::warning(
                "FF-HTML-ORDER-0001",
                "HTML index did not link to other documents; natural filename order was used after index.html",
            ));
        }
    }
    let ordered = ordered_names
        .into_iter()
        .filter_map(|name| by_name.get(&name).copied().cloned())
        .collect::<Vec<_>>();
    Ok((ordered, diagnostics))
}

fn parse_manifest_order(
    bytes: &[u8],
    available: &BTreeSet<String>,
) -> Result<(Vec<String>, bool), FormatError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| FormatError::Invalid(format!("HTML manifest is not UTF-8: {error}")))?;
    let mut names = Vec::new();
    let mut seen = BTreeSet::new();
    for (line_number, line) in text.lines().enumerate() {
        let value = line.trim();
        if value.is_empty() || value.starts_with('#') {
            continue;
        }
        let name = normalize_archive_path(value).ok_or_else(|| {
            FormatError::Invalid(format!(
                "HTML manifest line {} is not a safe relative path: {value}",
                line_number + 1
            ))
        })?;
        if !is_html_path(Path::new(&name)) {
            return Err(FormatError::Invalid(format!(
                "HTML manifest line {} is not an HTML document: {value}",
                line_number + 1
            )));
        }
        if !available.contains(&name) {
            return Err(FormatError::Invalid(format!(
                "HTML manifest line {} names a missing document: {name}",
                line_number + 1
            )));
        }
        if !seen.insert(name.clone()) {
            return Err(FormatError::Invalid(format!(
                "HTML manifest contains duplicate document: {name}"
            )));
        }
        names.push(name);
    }
    if names.is_empty() {
        return Err(FormatError::Invalid(
            "HTML manifest does not list any HTML documents".to_owned(),
        ));
    }
    let mut appended = false;
    for name in available {
        if seen.insert(name.clone()) {
            names.push(name.clone());
            appended = true;
        }
    }
    Ok((names, appended))
}

fn collect_source_resources(
    source: &BookSource,
    documents: &[HtmlDocument],
    resources: &mut BTreeMap<String, Vec<u8>>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), FormatError> {
    let mut referenced = BTreeSet::new();
    for document in documents {
        for value in attribute_values(&document.content, "src")
            .into_iter()
            .chain(css_url_values(&document.content))
        {
            if value.starts_with('#') {
                continue;
            }
            if is_remote_reference(&value) {
                diagnostics.push(Diagnostic::warning(
                    "FF-HTML-REMOTE-0002",
                    format!("remote or data HTML resource was not fetched: {value}"),
                ));
                continue;
            }
            if let Some(path) = resolve_local_path(&document.href, &value) {
                if is_resource_path(Path::new(&path)) {
                    referenced.insert(path);
                }
            }
        }
    }
    if source.kind() == SourceKind::Directory {
        for file in source.files() {
            if is_resource_path(&file.relative_path) {
                referenced.insert(file.relative_path.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    for relative in referenced {
        let path = source.root().join(&relative);
        if !path.is_file() {
            diagnostics.push(Diagnostic::warning(
                "FF-HTML-RES-0001",
                format!("local HTML resource was not found: {relative}"),
            ));
            continue;
        }
        let canonical = fs::canonicalize(&path)?;
        if !canonical.starts_with(source.root()) {
            return Err(FormatError::Invalid(
                HtmlError::UnsafePath(relative).to_string(),
            ));
        }
        resources.insert(relative, fs::read(canonical)?);
    }
    Ok(())
}

fn collect_stylesheet_resources(
    source: &BookSource,
    stylesheets: &BTreeMap<String, String>,
    resources: &mut BTreeMap<String, Vec<u8>>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), FormatError> {
    for (stylesheet, content) in stylesheets {
        for value in css_url_values(content) {
            if is_remote_reference(&value) {
                diagnostics.push(Diagnostic::warning(
                    "FF-HTML-REMOTE-0003",
                    format!("remote or data CSS resource was not fetched: {value}"),
                ));
                continue;
            }
            let Some(relative) = resolve_local_path(stylesheet, &value) else {
                return Err(FormatError::Invalid(
                    HtmlError::UnsafePath(value).to_string(),
                ));
            };
            if !is_resource_path(Path::new(&relative)) {
                continue;
            }
            let file = source.root().join(&relative);
            if !file.is_file() {
                diagnostics.push(Diagnostic::warning(
                    "FF-HTML-RES-0002",
                    format!("local CSS resource was not found: {relative}"),
                ));
                continue;
            }
            let canonical = fs::canonicalize(&file)?;
            if !canonical.starts_with(source.root()) {
                return Err(FormatError::Invalid(
                    HtmlError::UnsafePath(relative).to_string(),
                ));
            }
            resources.insert(relative, fs::read(canonical)?);
        }
    }
    Ok(())
}

fn load_external_stylesheets_from_directory(
    root: &Path,
    documents: &[HtmlDocument],
    stylesheets: &mut BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), FormatError> {
    for document in documents {
        for href in stylesheet_links(&document.content) {
            if is_remote_reference(&href) {
                diagnostics.push(Diagnostic::warning(
                    "FF-HTML-REMOTE-0001",
                    format!("remote stylesheet was not fetched: {href}"),
                ));
                continue;
            }
            let Some(relative) = resolve_local_path(&document.href, &href) else {
                return Err(FormatError::Invalid(
                    HtmlError::UnsafePath(href).to_string(),
                ));
            };
            let path = root.join(&relative);
            if !path.is_file() {
                diagnostics.push(Diagnostic::warning(
                    "FF-HTML-CSS-0001",
                    format!("local stylesheet was not found: {relative}"),
                ));
                continue;
            }
            let canonical = fs::canonicalize(&path)?;
            if !canonical.starts_with(root) {
                return Err(FormatError::Invalid(
                    HtmlError::UnsafePath(relative).to_string(),
                ));
            }
            stylesheets.insert(
                relative,
                decode_html(&fs::read(canonical)?)
                    .map_err(|error| FormatError::Invalid(error.to_string()))?,
            );
        }
    }
    Ok(())
}

fn decode_html(bytes: &[u8]) -> Result<String, HtmlError> {
    if let Ok(value) = std::str::from_utf8(bytes) {
        return Ok(value.trim_start_matches('\u{feff}').to_owned());
    }
    let detection =
        folio_text::detect(bytes).map_err(|error| HtmlError::Encoding(error.to_string()))?;
    let encoding = detection
        .selected
        .ok_or_else(|| HtmlError::Encoding("encoding is ambiguous".to_owned()))?;
    folio_text::decode_as(bytes, encoding).map_err(|error| HtmlError::Encoding(error.to_string()))
}

fn record_unsafe_script_diagnostic(value: &str, diagnostics: &mut Vec<Diagnostic>) {
    if value.to_ascii_lowercase().contains("<script") {
        diagnostics.push(Diagnostic::warning(
            "FF-HTML-SCRIPT-0001",
            "script elements are ignored and never executed during local import",
        ));
    }
}

fn prepare_markup(value: &str) -> String {
    let mut value = strip_doctype(value)
        .replace("&nbsp;", "&#160;")
        .replace("&copy;", "&#169;")
        .replace("&mdash;", "&#8212;");
    if !value.to_ascii_lowercase().contains("<html") {
        value = format!(
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head></head><body>{value}</body></html>"
        );
    }
    value = close_void_elements(&value);
    if !value.to_ascii_lowercase().contains("<body") {
        value = value.replace("</html>", "<body></body></html>");
    }
    value
}

fn strip_doctype(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(start) = lower.find("<!doctype") else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        rest = &rest[start + end + 1..];
    }
    output
}

fn close_void_elements(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 32);
    let mut rest = value;
    while let Some(start) = rest.find('<') {
        output.push_str(&rest[..start]);
        let Some(end_relative) = rest[start..].find('>') else {
            output.push_str(&rest[start..]);
            break;
        };
        let end = start + end_relative;
        let tag = &rest[start..=end];
        let trimmed = tag.trim_end();
        let name = trimmed
            .trim_start_matches('<')
            .trim_start_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_end_matches('/');
        if !tag.starts_with("</")
            && !trimmed.ends_with('/')
            && matches!(
                name.to_ascii_lowercase().as_str(),
                "meta" | "link" | "img" | "br" | "hr" | "input" | "source" | "base"
            )
        {
            output.push_str(trimmed.strip_suffix('>').unwrap_or(trimmed).trim_end());
            output.push_str("/>");
        } else {
            output.push_str(tag);
        }
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    output
}

fn collect_metadata(document: &HtmlDocument, metadata: &mut Metadata) {
    let Ok(xml) = roxmltree::Document::parse(&document.content) else {
        return;
    };
    let root = xml.root_element();
    metadata.language = root
        .attribute("lang")
        .or_else(|| root.attribute("xml:lang"))
        .map(ToOwned::to_owned);
    for element in xml.descendants().filter(|node| node.is_element()) {
        let tag = element
            .tag_name()
            .name()
            .rsplit(':')
            .next()
            .unwrap_or_default();
        if tag.eq_ignore_ascii_case("title") && metadata.title.is_none() {
            metadata.title = element
                .text()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(ToOwned::to_owned);
        }
        if tag.eq_ignore_ascii_case("meta") {
            let name = element
                .attribute("name")
                .unwrap_or_default()
                .to_ascii_lowercase();
            let content = element
                .attribute("content")
                .map(str::trim)
                .filter(|text| !text.is_empty());
            match (name.as_str(), content) {
                ("author", Some(value)) => metadata.add_author(value.to_owned()),
                ("description", Some(value)) => metadata.description = Some(value.to_owned()),
                ("keywords", Some(value)) => metadata.subjects.extend(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(ToOwned::to_owned),
                ),
                ("publisher", Some(value)) => metadata.publisher = Some(value.to_owned()),
                ("date", Some(value)) => metadata.date = Some(value.to_owned()),
                ("identifier" | "isbn", Some(value)) => metadata.add_identifier(value.to_owned()),
                _ => {}
            }
        }
    }
}

fn inject_stylesheets(documents: &mut [HtmlDocument], stylesheets: &BTreeMap<String, String>) {
    if stylesheets.is_empty() {
        return;
    }
    let css = stylesheets
        .values()
        .map(|value| format!("<style>{value}</style>"))
        .collect::<String>();
    for document in documents {
        if let Some(position) = document.content.to_ascii_lowercase().find("</head>") {
            document.content.insert_str(position, &css);
        } else if let Some(position) = document.content.to_ascii_lowercase().find("<body") {
            document
                .content
                .insert_str(position, &format!("<head>{css}</head>"));
        }
    }
}

fn decode_native(
    source: &BookSource,
    native: HtmlNativeDocument,
) -> Result<(Book, Vec<Diagnostic>), FormatError> {
    let mut loader = MemoryResourceLoader::default();
    let mut resources = Vec::new();
    for (index, (path, bytes)) in native.resources.iter().enumerate() {
        loader.insert(path.clone(), bytes.clone());
        resources.push(Resource {
            id: ResourceId::new(index as u32),
            path: path.clone(),
            media_type: mime_for_path(path).to_owned(),
            kind: kind_for_path(path),
            properties: Vec::new(),
            size: Some(bytes.len() as u64),
        });
    }
    let imported = import_xhtml_documents_with_options(
        native.metadata.clone(),
        &native
            .documents
            .iter()
            .map(|document| XhtmlDocumentInput {
                href: document.href.clone(),
                media_type: "application/xhtml+xml".to_owned(),
                content: document.content.clone(),
            })
            .collect::<Vec<_>>(),
        resources,
        Arc::new(loader),
        XhtmlImportOptions {
            require_body_element: true,
            resolver: StyleResolver::new(TargetProfile::Generic),
            ..XhtmlImportOptions::default()
        },
    )
    .map_err(|error| FormatError::Invalid(error.to_string()))?;
    let mut book = imported.book;
    let mut diagnostics = native.diagnostics;
    diagnostics.extend(imported.diagnostics);
    if book.metadata.title.is_none() {
        book.metadata.title = source
            .files()
            .first()
            .and_then(|file| file.relative_path.file_stem())
            .and_then(|value| value.to_str())
            .map(ToOwned::to_owned);
    }
    book.navigation.toc = book
        .documents
        .iter()
        .filter_map(|document| {
            document.title.as_ref().map(|title| NavPoint {
                label: title.clone(),
                href: document.href.clone(),
                children: Vec::new(),
            })
        })
        .collect();
    Ok((book, diagnostics))
}

fn stylesheet_links(value: &str) -> Vec<String> {
    let lower = value.to_ascii_lowercase();
    let mut result = Vec::new();
    let mut cursor = 0usize;
    while let Some(start) = lower[cursor..].find("<link") {
        let start = cursor + start;
        let Some(end) = lower[start..].find('>') else {
            break;
        };
        let tag = &value[start..start + end];
        if tag.to_ascii_lowercase().contains("stylesheet") {
            if let Some(href) = attribute_value(tag, "href") {
                result.push(href);
            }
        }
        cursor = start + end + 1;
    }
    result
}

fn css_url_values(value: &str) -> Vec<String> {
    let lower = value.to_ascii_lowercase();
    let mut result = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative_start) = lower[cursor..].find("url(") {
        let start = cursor + relative_start + 4;
        let Some(end) = value[start..].find(')') else {
            break;
        };
        let candidate = value[start..start + end].trim().trim_matches(['"', '\'']);
        if !candidate.is_empty() {
            result.push(candidate.to_owned());
        }
        cursor = start + end + 1;
    }
    result
}

fn is_remote_reference(value: &str) -> bool {
    value.contains("://") || value.starts_with("//") || value.starts_with("data:")
}

fn record_remote_document_resource_diagnostics(value: &str, diagnostics: &mut Vec<Diagnostic>) {
    for source in attribute_values(value, "src")
        .into_iter()
        .chain(css_url_values(value))
    {
        if is_remote_reference(&source) {
            diagnostics.push(Diagnostic::warning(
                "FF-HTML-REMOTE-0002",
                format!("remote or data HTML resource was not fetched: {source}"),
            ));
        }
    }
}

fn record_remote_css_diagnostics(value: &str, diagnostics: &mut Vec<Diagnostic>) {
    for source in css_url_values(value) {
        if is_remote_reference(&source) {
            diagnostics.push(Diagnostic::warning(
                "FF-HTML-REMOTE-0003",
                format!("remote or data CSS resource was not fetched: {source}"),
            ));
        }
    }
}

fn attribute_values(value: &str, name: &str) -> Vec<String> {
    let lower = value.to_ascii_lowercase();
    let mut result = Vec::new();
    let mut cursor = 0usize;
    let needle = format!("{name}=");
    while let Some(start) = lower[cursor..].find(&needle) {
        let start = cursor + start + needle.len();
        let rest = &value[start..];
        let rest_trimmed = rest.trim_start();
        let skipped = rest.len() - rest_trimmed.len();
        let rest = &rest_trimmed[skipped..];
        let (value, consumed) =
            if let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') {
                let Some(end) = rest[quote.len_utf8()..].find(quote) else {
                    break;
                };
                (
                    rest[quote.len_utf8()..quote.len_utf8() + end].to_owned(),
                    quote.len_utf8() + end + quote.len_utf8(),
                )
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                (rest[..end].trim_end_matches('>').to_owned(), end)
            };
        result.push(value);
        cursor = start + consumed;
    }
    result
}

fn attribute_value(value: &str, name: &str) -> Option<String> {
    attribute_values(value, name).into_iter().next()
}

fn resolve_local_path(document: &str, value: &str) -> Option<String> {
    let value = value
        .split('#')
        .next()
        .unwrap_or(value)
        .split('?')
        .next()
        .unwrap_or(value);
    let mut parts = document.split('/').collect::<Vec<_>>();
    parts.pop();
    let normalized = value.replace('\\', "/");
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            value => parts.push(value),
        }
    }
    Some(parts.join("/"))
}

fn normalize_archive_path(value: &str) -> Option<String> {
    if value.starts_with('/') || value.contains('\\') {
        return None;
    }
    let mut parts = Vec::new();
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            value => parts.push(value),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn is_html_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "html" | "htm" | "xhtml"
            )
        })
}

fn is_stylesheet_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("css"))
}

fn is_resource_path(path: &Path) -> bool {
    is_stylesheet_path(path)
        || matches!(
            path.extension()
                .and_then(|value| value.to_str())
                .map(|value| value.to_ascii_lowercase())
                .as_deref(),
            Some(
                "jpg"
                    | "jpeg"
                    | "png"
                    | "gif"
                    | "webp"
                    | "svg"
                    | "woff"
                    | "woff2"
                    | "ttf"
                    | "otf"
                    | "mp3"
                    | "m4a"
            )
        )
}

fn decode_optional_text(value: &[u8]) -> Option<String> {
    std::str::from_utf8(value).ok().map(ToOwned::to_owned)
}

fn mime_for_path(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "css" => "text/css",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        _ => "application/octet-stream",
    }
}

fn kind_for_path(path: &str) -> ResourceKind {
    match mime_for_path(path) {
        "image/jpeg" => ResourceKind::Jpeg,
        "image/png" => ResourceKind::Png,
        "image/gif" => ResourceKind::Gif,
        "image/svg+xml" => ResourceKind::Svg,
        value if value.starts_with("font/") => ResourceKind::Font,
        "text/css" => ResourceKind::Stylesheet,
        "audio/mpeg" | "audio/mp4" => ResourceKind::Audio,
        _ => ResourceKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::{write::SimpleFileOptions, ZipWriter};

    #[test]
    fn prepares_common_html_void_tags_for_shared_xhtml_normalizer() {
        let value = prepare_markup(
            "<!doctype html><html><body><p>Hello<img src=\"cover.png\"></body></html>",
        );
        assert!(value.contains("<img src=\"cover.png\"/>"));
        assert!(value.contains("<body>"));
    }

    #[test]
    fn rejects_htmlz_traversal_and_accepts_index_entrypoint() {
        let mut cursor = Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(&mut cursor);
        zip.start_file("index.html", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"<html><body><h1>Title</h1></body></html>")
            .unwrap();
        zip.start_file("../escape.png", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"bad").unwrap();
        zip.finish().unwrap();
        assert!(import_htmlz(&cursor.into_inner(), false).is_err());
    }
}
