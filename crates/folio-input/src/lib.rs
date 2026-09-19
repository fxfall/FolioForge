//! Input source abstraction and format detection.
//!
//! This crate discovers and classifies inputs only. Format-specific semantic
//! parsing remains in the corresponding importer crate.

use std::{
    collections::BTreeSet,
    fs,
    io::{Cursor, Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
};

use thiserror::Error;
use zip::{CompressionMethod, ZipArchive};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetectedFormat {
    Text,
    Markdown,
    Html,
    Htmlz,
    Fb2,
    Docx,
    Epub,
    Kf7,
    Kf8,
    Kfx,
    Ffkfx,
    Kf7Kf8Combo,
}

impl DetectedFormat {
    pub const fn all() -> [Self; 12] {
        [
            Self::Text,
            Self::Markdown,
            Self::Html,
            Self::Htmlz,
            Self::Fb2,
            Self::Docx,
            Self::Epub,
            Self::Kf7,
            Self::Kf8,
            Self::Kfx,
            Self::Ffkfx,
            Self::Kf7Kf8Combo,
        ]
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "TXT",
            Self::Markdown => "Markdown",
            Self::Html => "HTML",
            Self::Htmlz => "HTMLZ",
            Self::Fb2 => "FB2",
            Self::Docx => "DOCX",
            Self::Epub => "EPUB",
            Self::Kf7 => "KF7",
            Self::Kf8 => "KF8",
            Self::Kfx => "KFX",
            Self::Ffkfx => "FolioForge KFX compatibility container",
            Self::Kf7Kf8Combo => "KF7+KF8Combo",
        }
    }

    pub const fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Text => &["txt", "text"],
            Self::Markdown => &["md", "markdown"],
            Self::Html => &["html", "htm", "xhtml"],
            Self::Htmlz => &["htmlz"],
            Self::Fb2 => &["fb2"],
            Self::Docx => &["docx"],
            Self::Epub => &["epub", "zip"],
            Self::Kf7 => &["mobi", "azw"],
            Self::Kf8 => &["azw3"],
            Self::Kfx => &["kfx"],
            Self::Ffkfx => &["ffkfx"],
            Self::Kf7Kf8Combo => &["mobi"],
        }
    }

    pub const fn mime_types(self) -> &'static [&'static str] {
        match self {
            Self::Text => &["text/plain"],
            Self::Markdown => &["text/markdown", "text/plain"],
            Self::Html => &["text/html", "application/xhtml+xml"],
            Self::Htmlz => &["application/zip"],
            Self::Fb2 => &["application/x-fictionbook+xml"],
            Self::Docx => {
                &["application/vnd.openxmlformats-officedocument.wordprocessingml.document"]
            }
            Self::Epub => &["application/epub+zip", "application/zip"],
            Self::Kf7 | Self::Kf7Kf8Combo => &["application/x-mobipocket-ebook"],
            Self::Kf8 => &["application/vnd.amazon.ebook"],
            Self::Kfx | Self::Ffkfx => &["application/x-kfx"],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    SingleFile,
    FileSet,
    Directory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFile {
    /// Canonical local path to this member.
    pub path: PathBuf,
    /// Safe, normalized path relative to the source root.
    pub relative_path: PathBuf,
}

/// Access capability of a local input. Format adapters may require seeking
/// for ZIP central directories or container indexes, while future streaming
/// callers can use the same description to avoid assuming `read_to_end`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessCapability {
    Sequential,
    Seekable,
    RandomAccess,
}

/// A bounded local byte source. It is deliberately independent from any
/// format parser and can be used by adapters that need lazy or ranged reads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputSource {
    path: PathBuf,
    capability: AccessCapability,
    size: Option<u64>,
}

impl InputSource {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, SourceError> {
        let path = path.as_ref();
        let metadata = fs::metadata(path)?;
        if !metadata.is_file() {
            return Err(SourceError::NotFile(path.to_owned()));
        }
        Ok(Self {
            path: fs::canonicalize(path)?,
            capability: AccessCapability::RandomAccess,
            size: Some(metadata.len()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn capability(&self) -> AccessCapability {
        self.capability
    }

    pub const fn size(&self) -> Option<u64> {
        self.size
    }

    pub fn open(&self) -> Result<fs::File, SourceError> {
        Ok(fs::File::open(&self.path)?)
    }

    /// Read a bounded range without requiring callers to materialize the
    /// whole source. This is the common denominator for indexed containers;
    /// sequential-only sources can be represented later without changing the
    /// adapter contract.
    pub fn read_range(&self, offset: u64, length: usize) -> Result<Vec<u8>, SourceError> {
        let mut file = self.open()?;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; length];
        let count = file.read(&mut bytes)?;
        bytes.truncate(count);
        Ok(bytes)
    }
}

impl SourceFile {
    pub fn input_source(&self) -> Result<InputSource, SourceError> {
        InputSource::from_path(&self.path)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceLimits {
    pub max_files: usize,
    pub max_depth: usize,
}

impl Default for SourceLimits {
    fn default() -> Self {
        Self {
            max_files: 100_000,
            max_depth: 128,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BookSource {
    kind: SourceKind,
    root: PathBuf,
    files: Vec<SourceFile>,
}

impl BookSource {
    pub fn single_file(path: impl AsRef<Path>) -> Result<Self, SourceError> {
        let path = path.as_ref();
        let file_type = fs::symlink_metadata(path)?.file_type();
        if file_type.is_symlink() {
            return Err(SourceError::Symlink(path.to_owned()));
        }
        if !file_type.is_file() {
            return Err(SourceError::NotFile(path.to_owned()));
        }
        let canonical = fs::canonicalize(path)?;
        let root = canonical
            .parent()
            .ok_or_else(|| SourceError::InvalidRoot(canonical.clone()))?
            .to_owned();
        let relative_path = canonical
            .file_name()
            .map(PathBuf::from)
            .ok_or_else(|| SourceError::InvalidRelativePath(canonical.clone()))?;
        Ok(Self {
            kind: SourceKind::SingleFile,
            root,
            files: vec![SourceFile {
                path: canonical,
                relative_path,
            }],
        })
    }

    /// Construct an explicit logical file set. Every member path is relative
    /// to `root`; absolute paths, parent traversal, duplicate names and
    /// symlinks are rejected before a parser receives the files.
    pub fn file_set<I, P>(root: impl AsRef<Path>, members: I) -> Result<Self, SourceError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let root = canonical_directory(root.as_ref())?;
        let mut files = Vec::new();
        let mut seen = BTreeSet::new();
        for member in members {
            let relative_path = normalize_relative_path(member.as_ref())?;
            if !seen.insert(relative_path.clone()) {
                return Err(SourceError::DuplicateMember(relative_path));
            }
            let path = checked_member_path(&root, &relative_path)?;
            files.push(SourceFile {
                path,
                relative_path,
            });
        }
        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        if files.is_empty() {
            return Err(SourceError::EmptyFileSet);
        }
        Ok(Self {
            kind: SourceKind::FileSet,
            root,
            files,
        })
    }

    pub fn directory(root: impl AsRef<Path>, limits: SourceLimits) -> Result<Self, SourceError> {
        let root = canonical_directory(root.as_ref())?;
        let mut files = Vec::new();
        collect_directory_files(&root, &root, 0, limits, &mut files)?;
        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(Self {
            kind: SourceKind::Directory,
            root,
            files,
        })
    }

    pub fn kind(&self) -> SourceKind {
        self.kind
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }

    pub fn supported_book_files(&self) -> impl Iterator<Item = &SourceFile> {
        self.files
            .iter()
            .filter(|file| is_supported_book_path(&file.relative_path))
    }
}

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("source directory does not exist or is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("source path is not a regular file: {0}")]
    NotFile(PathBuf),
    #[error("source path contains a symbolic link: {0}")]
    Symlink(PathBuf),
    #[error("source root is invalid: {0}")]
    InvalidRoot(PathBuf),
    #[error("source member path must be a normalized relative path: {0}")]
    InvalidRelativePath(PathBuf),
    #[error("source member escapes its root: {0}")]
    OutsideRoot(PathBuf),
    #[error("duplicate source member path: {0}")]
    DuplicateMember(PathBuf),
    #[error("a logical file set must contain at least one member")]
    EmptyFileSet,
    #[error("directory source exceeds the maximum traversal depth of {0}")]
    DepthLimit(usize),
    #[error("directory source exceeds the maximum file count of {0}")]
    FileLimit(usize),
    #[error("source I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum DetectionError {
    #[error("unsupported or malformed input: {0}")]
    Unsupported(String),
}

/// Detect by container signature and structure, using the filename extension
/// only as a hint for distinguishing KF7 from KF8.
pub fn detect_format(path_hint: &Path, bytes: &[u8]) -> Result<DetectedFormat, DetectionError> {
    let extension = path_hint
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);

    if bytes.starts_with(b"CONT") {
        return Ok(DetectedFormat::Kfx);
    }
    if bytes.starts_with(b"FFKFX\0\x01\0") {
        return Ok(DetectedFormat::Ffkfx);
    }
    if bytes.starts_with(b"PK\x03\x04") {
        if is_epub_ocf(bytes) {
            return Ok(DetectedFormat::Epub);
        }
        if is_docx_package(bytes) {
            return Ok(DetectedFormat::Docx);
        }
        if extension.as_deref() == Some("htmlz") || is_htmlz_container(bytes) {
            return Ok(DetectedFormat::Htmlz);
        }
        return Err(DetectionError::Unsupported(
            "ZIP input is not a valid EPUB OCF, DOCX OOXML, or HTMLZ package".to_owned(),
        ));
    }
    if matches!(extension.as_deref(), Some("epub" | "zip")) {
        if is_htmlz_container(bytes) {
            return Ok(DetectedFormat::Htmlz);
        }
        return Err(DetectionError::Unsupported(
            "EPUB/ZIP extension hint does not match a ZIP container".to_owned(),
        ));
    }

    if matches!(extension.as_deref(), Some("md" | "markdown")) {
        folio_text::detect(bytes).map_err(|error| {
            DetectionError::Unsupported(format!(
                "Markdown extension does not contain plausible text: {error}"
            ))
        })?;
        return Ok(DetectedFormat::Markdown);
    }

    if (matches!(extension.as_deref(), Some("html" | "htm" | "xhtml")) || looks_like_html(bytes))
        && !looks_like_fictionbook(bytes)
    {
        if std::str::from_utf8(bytes).is_err() {
            let detection = folio_text::detect(bytes).map_err(|error| {
                DetectionError::Unsupported(format!(
                    "HTML input is neither valid UTF-8 nor a detectable legacy text encoding: {error}"
                ))
            })?;
            if detection.selected.is_none() {
                return Err(DetectionError::Unsupported(
                    "HTML input has ambiguous legacy encoding".to_owned(),
                ));
            }
        }
        return Ok(DetectedFormat::Html);
    }

    if extension.as_deref() == Some("fb2") || looks_like_fictionbook(bytes) {
        if std::str::from_utf8(bytes).is_err() {
            return Err(DetectionError::Unsupported(
                "FB2 input is not valid UTF-8".to_owned(),
            ));
        }
        return Ok(DetectedFormat::Fb2);
    }

    if matches!(extension.as_deref(), Some("txt" | "text")) {
        folio_text::detect(bytes).map_err(|error| {
            DetectionError::Unsupported(format!(
                "text extension does not contain plausible text: {error}"
            ))
        })?;
        return Ok(DetectedFormat::Text);
    }

    let inspection = folio_mobi::inspect_pdb(bytes)
        .map_err(|error| DetectionError::Unsupported(error.to_string()))?;
    let record_zero = inspection
        .records
        .first()
        .ok_or_else(|| DetectionError::Unsupported("MOBI has no record zero".to_owned()))?;
    if record_zero.len() < 16 + 20 || &record_zero[16..20] != b"MOBI" {
        return Err(DetectionError::Unsupported(
            "input is not a supported EPUB, MOBI-family, or KFX file".to_owned(),
        ));
    }
    if folio_mobi::inspect_combo(bytes).is_ok() {
        return Ok(DetectedFormat::Kf7Kf8Combo);
    }
    let is_kf8 = matches!(extension.as_deref(), Some("azw3"))
        || record_zero
            .get(16 + 16..16 + 20)
            .is_some_and(|value| value == 8u32.to_be_bytes());
    Ok(if is_kf8 {
        DetectedFormat::Kf8
    } else {
        DetectedFormat::Kf7
    })
}

pub fn is_supported_book_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "epub"
                    | "zip"
                    | "mobi"
                    | "azw"
                    | "azw3"
                    | "kfx"
                    | "txt"
                    | "text"
                    | "md"
                    | "markdown"
                    | "html"
                    | "htm"
                    | "xhtml"
                    | "htmlz"
                    | "fb2"
                    | "docx"
            )
        })
}

fn is_docx_package(bytes: &[u8]) -> bool {
    let Ok(mut archive) = ZipArchive::new(Cursor::new(bytes)) else {
        return false;
    };
    if archive.by_name("word/document.xml").is_err() {
        return false;
    }
    let Ok(mut content_types) = archive.by_name("[Content_Types].xml") else {
        return false;
    };
    if content_types.size() > 8 << 20 {
        return false;
    }
    let mut bytes = Vec::new();
    if content_types.read_to_end(&mut bytes).is_err() {
        return false;
    }
    let value = String::from_utf8_lossy(&bytes);
    value.contains(
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
    ) && !value.contains("macroEnabled.main+xml")
}

fn looks_like_html(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let prefix = text.trim_start().get(..512).unwrap_or(text.trim_start());
    let lower = prefix.to_ascii_lowercase();
    lower.contains("<html")
        || lower.starts_with("<!doctype html")
        || (lower.contains("<body") && lower.contains("<p"))
}

fn looks_like_fictionbook(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    text.trim_start()
        .to_ascii_lowercase()
        .contains("<fictionbook")
}

fn is_htmlz_container(bytes: &[u8]) -> bool {
    let Ok(archive) = ZipArchive::new(Cursor::new(bytes)) else {
        return false;
    };
    let result = archive.file_names().any(|name| {
        let name = name.to_ascii_lowercase();
        !name.starts_with("/")
            && !name.split('/').any(|part| part == "..")
            && matches!(name.rsplit('.').next(), Some("html" | "htm" | "xhtml"))
    });
    result
}

fn is_epub_ocf(bytes: &[u8]) -> bool {
    let Ok(mut archive) = ZipArchive::new(Cursor::new(bytes)) else {
        return false;
    };
    let Ok(mut first) = archive.by_index(0) else {
        return false;
    };
    if first.name() != "mimetype"
        || first.compression() != CompressionMethod::Stored
        || first.size() > 64
    {
        return false;
    }
    let mut mimetype = Vec::with_capacity(first.size() as usize);
    if first.read_to_end(&mut mimetype).is_err() || mimetype != b"application/epub+zip" {
        return false;
    }
    drop(first);
    let has_container = archive.by_name("META-INF/container.xml").is_ok();
    has_container
}

fn canonical_directory(path: &Path) -> Result<PathBuf, SourceError> {
    let file_type = fs::symlink_metadata(path)?.file_type();
    if file_type.is_symlink() {
        return Err(SourceError::Symlink(path.to_owned()));
    }
    if !file_type.is_dir() {
        return Err(SourceError::NotDirectory(path.to_owned()));
    }
    Ok(fs::canonicalize(path)?)
}

pub fn normalize_relative_path(path: &Path) -> Result<PathBuf, SourceError> {
    let portable = path.to_string_lossy().replace('\\', "/");
    if portable.is_empty() {
        return Err(SourceError::InvalidRelativePath(path.to_owned()));
    }
    let mut normalized = PathBuf::new();
    for component in Path::new(&portable).components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            _ => return Err(SourceError::InvalidRelativePath(path.to_owned())),
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(SourceError::InvalidRelativePath(path.to_owned()));
    }
    Ok(normalized)
}

fn checked_member_path(root: &Path, relative: &Path) -> Result<PathBuf, SourceError> {
    let relative = normalize_relative_path(relative)?;
    let mut current = root.to_owned();
    let components = relative.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(value) = component else {
            return Err(SourceError::InvalidRelativePath(relative));
        };
        current.push(value);
        let file_type = fs::symlink_metadata(&current)?.file_type();
        if file_type.is_symlink() {
            return Err(SourceError::Symlink(current));
        }
        if index + 1 == components.len() {
            if !file_type.is_file() {
                return Err(SourceError::NotFile(current));
            }
        } else if !file_type.is_dir() {
            return Err(SourceError::InvalidRelativePath(relative));
        }
    }
    let canonical = fs::canonicalize(&current)?;
    if !canonical.starts_with(root) {
        return Err(SourceError::OutsideRoot(canonical));
    }
    Ok(canonical)
}

fn collect_directory_files(
    root: &Path,
    current: &Path,
    depth: usize,
    limits: SourceLimits,
    files: &mut Vec<SourceFile>,
) -> Result<(), SourceError> {
    if depth > limits.max_depth {
        return Err(SourceError::DepthLimit(limits.max_depth));
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_directory_files(root, &path, depth + 1, limits, files)?;
        } else if file_type.is_file() {
            let relative_path = path
                .strip_prefix(root)
                .map_err(|_| SourceError::OutsideRoot(path.clone()))?;
            let relative_path = normalize_relative_path(relative_path)?;
            let path = fs::canonicalize(&path)?;
            if !path.starts_with(root) {
                return Err(SourceError::OutsideRoot(path));
            }
            files.push(SourceFile {
                path,
                relative_path,
            });
            if files.len() > limits.max_files {
                return Err(SourceError::FileLimit(limits.max_files));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    use zip::{write::SimpleFileOptions, ZipWriter};

    fn temp_root() -> PathBuf {
        static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(0);
        loop {
            let sequence = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
            let root =
                std::env::temp_dir().join(format!("folio-input-{}-{sequence}", std::process::id()));
            match fs::create_dir(&root) {
                Ok(()) => return root,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("could not create isolated test directory: {error}"),
            }
        }
    }

    #[test]
    fn format_detection_requires_epub_container_structure() {
        let error = detect_format(Path::new("book.epub"), b"PK\x03\x04not an epub").unwrap_err();
        assert!(error.to_string().contains("valid EPUB OCF"));
        assert!(detect_format(Path::new("book.ffkfx"), b"not ffkfx").is_err());

        let cursor = Cursor::new(Vec::new());
        let mut archive = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        archive.start_file("mimetype", options).unwrap();
        archive.write_all(b"application/epub+zip").unwrap();
        archive
            .start_file("META-INF/container.xml", options)
            .unwrap();
        archive.write_all(b"<container/>").unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        assert_eq!(
            detect_format(Path::new("book.epub"), &bytes).unwrap(),
            DetectedFormat::Epub
        );
        assert_eq!(
            detect_format(Path::new("book.txt"), "一段合成文本".as_bytes()).unwrap(),
            DetectedFormat::Text
        );
        assert!(detect_format(Path::new("book.txt"), b"PK\x00\x01").is_err());
    }

    #[test]
    fn directory_sources_preserve_relative_paths_and_skip_symlinks() {
        let root = temp_root();
        fs::create_dir_all(root.join("Sub")).unwrap();
        fs::write(root.join("A.epub"), b"a").unwrap();
        fs::write(root.join("Sub/B.kfx"), b"b").unwrap();
        fs::write(root.join("notes.txt"), b"ignore").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("A.epub"), root.join("linked.epub")).unwrap();

        let source = BookSource::directory(&root, SourceLimits::default()).unwrap();
        assert_eq!(source.kind(), SourceKind::Directory);
        assert_eq!(source.files().len(), 3);
        let books = source
            .supported_book_files()
            .map(|file| file.relative_path.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            books,
            vec![
                PathBuf::from("A.epub"),
                PathBuf::from("Sub/B.kfx"),
                PathBuf::from("notes.txt")
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_sets_reject_parent_traversal_and_symlink_members() {
        let root = temp_root();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("book.kfx"), b"book").unwrap();
        assert!(BookSource::file_set(&root, [PathBuf::from("../outside.kfx")]).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("book.kfx"), root.join("alias.kfx")).unwrap();
            assert!(BookSource::file_set(&root, [PathBuf::from("alias.kfx")]).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn single_file_source_is_canonicalized() {
        let root = temp_root();
        fs::create_dir_all(&root).unwrap();
        let path = root.join("single.txt");
        fs::File::create(&path).unwrap().write_all(b"text").unwrap();
        let source = BookSource::single_file(&path).unwrap();
        assert_eq!(source.kind(), SourceKind::SingleFile);
        assert_eq!(source.files()[0].relative_path, PathBuf::from("single.txt"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn input_source_exposes_random_access_without_reading_the_whole_file() {
        let root = temp_root();
        fs::create_dir_all(&root).unwrap();
        let path = root.join("source.bin");
        fs::write(&path, b"0123456789").unwrap();
        let source = InputSource::from_path(&path).unwrap();
        assert_eq!(source.capability(), AccessCapability::RandomAccess);
        assert_eq!(source.size(), Some(10));
        assert_eq!(source.read_range(3, 4).unwrap(), b"3456");
        fs::remove_dir_all(root).unwrap();
    }
}
