//! Bounded, read-only comic source ingestion.
//!
//! Source adapters keep encoded page data out of `ComicNativeBook`; the
//! `ImportedComic` handle reopens one page at a time under the same limits.

mod archive;
mod epub;
mod pdf;
mod raster;

use std::{
    fs,
    path::{Path, PathBuf},
};

use folio_epub::EpubReadReport;
use folio_input::{BookSource, SourceLimits};
use folio_model::Metadata;
use thiserror::Error;

use crate::{
    ComicIrProjection, ComicNativeBook, ComicSourceIdentity, ComicSourceKind, ComicSourcePageDraft,
    PageOrdering, SourceLocation, SourcePageId,
};

pub use archive::ArchiveInputFormat;

/// The physical source container behind an imported native manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComicContainerKind {
    Directory,
    Zip,
    SevenZip,
    Pdf,
    FixedLayoutEpub,
}

/// Resource ceilings applied before and during comic source parsing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComicImportLimits {
    pub max_source_bytes: u64,
    pub max_total_uncompressed_bytes: u64,
    pub max_member_bytes: u64,
    pub max_page_bytes: u64,
    pub max_entries: usize,
    pub max_pages: usize,
    pub max_path_depth: usize,
    pub max_archive_header_bytes: usize,
    pub max_sevenz_dictionary_bytes: u64,
    pub max_sevenz_solid_decode_bytes: u64,
    pub max_pdf_bytes: u64,
    pub max_pdf_stream_bytes: usize,
    pub max_pdf_pages: usize,
}

impl Default for ComicImportLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 8 * 1024 * 1024 * 1024,
            max_total_uncompressed_bytes: 16 * 1024 * 1024 * 1024,
            max_member_bytes: 512 * 1024 * 1024,
            max_page_bytes: 256 * 1024 * 1024,
            max_entries: 100_000,
            max_pages: 20_000,
            max_path_depth: 128,
            max_archive_header_bytes: 32 * 1024 * 1024,
            max_sevenz_dictionary_bytes: 512 * 1024 * 1024,
            max_sevenz_solid_decode_bytes: 2 * 1024 * 1024 * 1024,
            max_pdf_bytes: 128 * 1024 * 1024,
            max_pdf_stream_bytes: 32 * 1024 * 1024,
            max_pdf_pages: 20_000,
        }
    }
}

impl ComicImportLimits {
    fn validate(&self) -> Result<(), ComicImportError> {
        if self.max_source_bytes == 0
            || self.max_total_uncompressed_bytes == 0
            || self.max_member_bytes == 0
            || self.max_page_bytes == 0
            || self.max_entries == 0
            || self.max_pages == 0
            || self.max_path_depth == 0
            || self.max_archive_header_bytes == 0
            || self.max_sevenz_dictionary_bytes == 0
            || self.max_sevenz_solid_decode_bytes == 0
            || self.max_pdf_bytes == 0
            || self.max_pdf_stream_bytes == 0
            || self.max_pdf_pages == 0
        {
            return Err(ComicImportError::InvalidLimits);
        }
        if self.max_page_bytes > self.max_member_bytes
            || self.max_pdf_bytes > self.max_source_bytes
            || self.max_total_uncompressed_bytes < self.max_page_bytes
        {
            return Err(ComicImportError::InvalidLimits);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ComicImportError {
    #[error("comic source I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("comic source traversal failed: {0}")]
    Input(#[from] folio_input::SourceError),
    #[error("comic source path is unsafe or unsupported: {0}")]
    UnsafePath(String),
    #[error("comic import limit exceeded: {0}")]
    LimitExceeded(String),
    #[error("comic source is malformed: {0}")]
    InvalidSource(String),
    #[error("comic input is unsupported: {0}")]
    Unsupported(String),
    #[error("RAR/CBR is deferred: the reviewed Rust readers do not meet FolioForge's license and safety gates")]
    RarDeferred,
    #[error("comic source import was cancelled")]
    Cancelled,
    #[error("comic page identity was not found in this imported source")]
    PageNotFound,
    #[error("PDF pages are vector pages and require the later bounded raster stage")]
    PdfPageRequiresRasterization,
    #[error("comic source page is not a supported raster image")]
    InvalidRaster,
    #[error("EPUB source is not an eligible single-image Fixed Layout book: {0}")]
    IneligibleFixedLayoutEpub(String),
    #[error("EPUB import failed: {0}")]
    Epub(#[from] folio_epub::EpubError),
    #[error("comic source model validation failed: {0}")]
    Model(#[from] crate::ComicModelError),
    #[error("ZIP source failed: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("7z source failed: {0}")]
    SevenZip(String),
    #[error("PDF source failed: {0}")]
    Pdf(#[from] lopdf::Error),
    #[error("comic importer limits are inconsistent or zero")]
    InvalidLimits,
}

/// Stateless importer with caller-selected resource ceilings.
#[derive(Clone, Debug, Default)]
pub struct ComicSourceImporter {
    limits: ComicImportLimits,
}

impl ComicSourceImporter {
    pub fn new(limits: ComicImportLimits) -> Result<Self, ComicImportError> {
        limits.validate()?;
        Ok(Self { limits })
    }

    pub fn limits(&self) -> &ComicImportLimits {
        &self.limits
    }

    pub fn import_path(
        &self,
        path: impl AsRef<Path>,
        stable_source_key: &str,
    ) -> Result<ImportedComic, ComicImportError> {
        self.import_path_with_cancel(path, stable_source_key, || false)
    }

    /// Import one source. `stable_source_key` must be stable across reopening;
    /// a scratch path or generated temporary name must never be used as it.
    pub fn import_path_with_cancel<F>(
        &self,
        path: impl AsRef<Path>,
        stable_source_key: &str,
        mut is_cancelled: F,
    ) -> Result<ImportedComic, ComicImportError>
    where
        F: FnMut() -> bool,
    {
        check_cancelled(&mut is_cancelled)?;
        self.limits.validate()?;
        let path = path.as_ref();
        let path_metadata = fs::symlink_metadata(path)?;
        if path_metadata.file_type().is_symlink() {
            return Err(ComicImportError::UnsafePath(
                "the source root is a symbolic link".to_owned(),
            ));
        }
        if path_metadata.is_dir() {
            return self.import_directory(path, stable_source_key, &mut is_cancelled);
        }
        if !path_metadata.is_file() {
            return Err(ComicImportError::UnsafePath(
                "the source is not a regular file or directory".to_owned(),
            ));
        }

        let source = BookSource::single_file(path)?;
        let canonical_path = source
            .files()
            .first()
            .ok_or_else(|| ComicImportError::InvalidSource("empty file source".to_owned()))?
            .path
            .clone();
        let size = fs::metadata(&canonical_path)?.len();
        if size > self.limits.max_source_bytes {
            return Err(ComicImportError::LimitExceeded(format!(
                "source file is {size} bytes; cap is {}",
                self.limits.max_source_bytes
            )));
        }
        check_cancelled(&mut is_cancelled)?;

        if archive::is_pdf(&canonical_path)? {
            return pdf::import_pdf(
                &canonical_path,
                stable_source_key,
                &self.limits,
                &mut is_cancelled,
            );
        }
        if archive::is_rar(&canonical_path)? {
            return Err(ComicImportError::RarDeferred);
        }
        if archive::is_sevenz(&canonical_path)? {
            return archive::import_sevenz(
                &canonical_path,
                stable_source_key,
                &self.limits,
                &mut is_cancelled,
            );
        }
        if archive::is_zip(&canonical_path)? {
            let epub_by_signature = archive::looks_like_epub(&canonical_path, &self.limits)?;
            let epub_by_extension = canonical_path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("epub"));
            if epub_by_signature || epub_by_extension {
                return epub::import_fixed_layout_epub(
                    &canonical_path,
                    stable_source_key,
                    &self.limits,
                    &mut is_cancelled,
                );
            }
            return archive::import_zip(
                &canonical_path,
                stable_source_key,
                &self.limits,
                &mut is_cancelled,
            );
        }

        Err(ComicImportError::Unsupported(
            "expected an image directory, ZIP/CBZ, 7z/CB7, PDF, or eligible Fixed Layout EPUB; RAR/CBR is deferred".to_owned(),
        ))
    }

    fn import_directory<F>(
        &self,
        path: &Path,
        stable_source_key: &str,
        is_cancelled: &mut F,
    ) -> Result<ImportedComic, ComicImportError>
    where
        F: FnMut() -> bool,
    {
        let source = BookSource::directory(
            path,
            SourceLimits {
                max_files: self.limits.max_entries,
                max_depth: self.limits.max_path_depth,
            },
        )?;
        let mut total_bytes = 0u64;
        let mut drafts = Vec::new();
        for file in source.files() {
            check_cancelled(is_cancelled)?;
            let metadata = fs::metadata(&file.path)?;
            total_bytes = total_bytes.checked_add(metadata.len()).ok_or_else(|| {
                ComicImportError::LimitExceeded("directory byte total overflow".to_owned())
            })?;
            if total_bytes > self.limits.max_total_uncompressed_bytes {
                return Err(ComicImportError::LimitExceeded(format!(
                    "directory exceeds the {} byte aggregate limit",
                    self.limits.max_total_uncompressed_bytes
                )));
            }
            let Some(format) = raster::format_from_path(&file.relative_path) else {
                continue;
            };
            if metadata.len() > self.limits.max_page_bytes {
                return Err(ComicImportError::LimitExceeded(format!(
                    "page {} is {} bytes; cap is {}",
                    file.relative_path.display(),
                    metadata.len(),
                    self.limits.max_page_bytes
                )));
            }
            let locator = raster::relative_path_to_posix(&file.relative_path)?;
            let location = SourceLocation::member(&locator)?;
            drafts.push(
                ComicSourcePageDraft::new(location, format)
                    .with_encoded_size(metadata.len())
                    .with_source_name(locator)?,
            );
            if drafts.len() > self.limits.max_pages {
                return Err(ComicImportError::LimitExceeded(format!(
                    "directory exceeds the {} page limit",
                    self.limits.max_pages
                )));
            }
        }

        let canonical_root = source.root().to_path_buf();
        let identity = ComicSourceIdentity::new(ComicSourceKind::Directory, stable_source_key)?;
        let native = ComicNativeBook::new(
            identity,
            metadata_from_path(path),
            drafts,
            [("container".to_owned(), "directory".to_owned())].into(),
            PageOrdering::Natural,
        )?;
        Ok(ImportedComic {
            native,
            source_path: canonical_root,
            kind: ComicContainerKind::Directory,
            limits: self.limits.clone(),
            source_ir: None,
        })
    }
}

/// Runtime source handle paired with its immutable native manifest.
///
/// The canonical source path is held only in memory and is never incorporated
/// into source IDs or serialized native facts.
#[derive(Clone, Debug)]
pub struct ImportedComic {
    native: ComicNativeBook,
    source_path: PathBuf,
    kind: ComicContainerKind,
    limits: ComicImportLimits,
    source_ir: Option<EpubReadReport>,
}

impl ImportedComic {
    pub fn native(&self) -> &ComicNativeBook {
        &self.native
    }

    pub const fn container_kind(&self) -> ComicContainerKind {
        self.kind
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    /// Project this imported source into the format-neutral Semantic IR.
    /// Fixed Layout EPUB reuses the semantic import already produced by the
    /// EPUB adapter; raster-page sources use an identity Comic composition.
    pub fn to_semantic_ir(&self) -> Result<ComicIrProjection, ComicImportError> {
        crate::composition::project_imported(self, self.source_ir.as_ref())
    }

    pub fn read_page_bytes(
        &self,
        page_id: &SourcePageId,
        max_bytes: u64,
    ) -> Result<Vec<u8>, ComicImportError> {
        self.read_page_bytes_with_cancel(page_id, max_bytes, || false)
    }

    /// Read one encoded raster page, enforcing both the importer ceiling and
    /// the caller's smaller request. PDF pages remain vector-backed until the
    /// dedicated rasterization milestone.
    pub fn read_page_bytes_with_cancel<F>(
        &self,
        page_id: &SourcePageId,
        max_bytes: u64,
        mut is_cancelled: F,
    ) -> Result<Vec<u8>, ComicImportError>
    where
        F: FnMut() -> bool,
    {
        check_cancelled(&mut is_cancelled)?;
        let page = self
            .native
            .source_page(page_id)
            .ok_or(ComicImportError::PageNotFound)?;
        let limit = max_bytes.min(self.limits.max_page_bytes);
        if limit == 0 {
            return Err(ComicImportError::InvalidLimits);
        }
        if page.encoded_size().is_some_and(|size| size > limit) {
            return Err(ComicImportError::LimitExceeded(format!(
                "page declares {} bytes; requested cap is {limit}",
                page.encoded_size().unwrap_or_default()
            )));
        }

        let bytes = match (&self.kind, page.location()) {
            (ComicContainerKind::Directory, SourceLocation::Member { path }) => {
                let source = BookSource::file_set(&self.source_path, [Path::new(path)])?;
                let file = source
                    .files()
                    .first()
                    .ok_or(ComicImportError::PageNotFound)?;
                raster::read_regular_file(&file.path, limit, &mut is_cancelled)?
            }
            (ComicContainerKind::Zip, SourceLocation::Member { path }) => archive::read_zip_member(
                &self.source_path,
                path,
                &self.limits,
                limit,
                &mut is_cancelled,
            )?,
            (ComicContainerKind::SevenZip, SourceLocation::Member { path }) => {
                archive::read_sevenz_member(
                    &self.source_path,
                    path,
                    &self.limits,
                    limit,
                    &mut is_cancelled,
                )?
            }
            (
                ComicContainerKind::FixedLayoutEpub,
                SourceLocation::FixedLayoutSpineItem { href, .. },
            ) => archive::read_zip_member(
                &self.source_path,
                href,
                &self.limits,
                limit,
                &mut is_cancelled,
            )?,
            (ComicContainerKind::Pdf, SourceLocation::PdfPage { .. }) => {
                return Err(ComicImportError::PdfPageRequiresRasterization)
            }
            _ => return Err(ComicImportError::PageNotFound),
        };

        if bytes.len() as u64 > limit {
            return Err(ComicImportError::LimitExceeded(format!(
                "page expands beyond {limit} bytes"
            )));
        }
        if raster::sniff_image_format(&bytes) != Some(page.source_format()) {
            return Err(ComicImportError::InvalidRaster);
        }
        Ok(bytes)
    }
}

pub(super) fn check_cancelled<F>(is_cancelled: &mut F) -> Result<(), ComicImportError>
where
    F: FnMut() -> bool,
{
    if is_cancelled() {
        Err(ComicImportError::Cancelled)
    } else {
        Ok(())
    }
}

pub(super) fn metadata_from_path(path: &Path) -> Metadata {
    let title = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned);
    Metadata {
        title,
        ..Metadata::default()
    }
}

pub(super) fn make_imported(
    native: ComicNativeBook,
    source_path: &Path,
    kind: ComicContainerKind,
    limits: &ComicImportLimits,
) -> ImportedComic {
    ImportedComic {
        native,
        source_path: source_path.to_path_buf(),
        kind,
        limits: limits.clone(),
        source_ir: None,
    }
}
