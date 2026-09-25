use std::{collections::BTreeMap, fmt};

use blake3::Hasher;

use crate::ComicModelError;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourcePageId([u8; 32]);

impl SourcePageId {
    pub(crate) fn derive(
        book_id: &[u8; 32],
        location: &SourceLocation,
        discriminator: &str,
    ) -> Self {
        let mut hasher = Hasher::new();
        hasher.update(b"FolioForge Comic Source Page v1\0");
        super::hash_part(&mut hasher, book_id);
        super::hash_part(&mut hasher, location.identity_key().as_bytes());
        super::hash_part(&mut hasher, discriminator.as_bytes());
        Self(*hasher.finalize().as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

impl fmt::Display for SourcePageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum SourceLocation {
    /// Safe POSIX-style path inside a directory or archive.
    Member { path: String },
    /// One-based page number from a PDF source.
    PdfPage { page_number: u32 },
    /// Safe package-relative href and zero-based spine position.
    FixedLayoutSpineItem { href: String, spine_index: u32 },
}

impl SourceLocation {
    pub fn member(path: impl AsRef<str>) -> Result<Self, ComicModelError> {
        Ok(Self::Member {
            path: validate_relative_path(path.as_ref())?,
        })
    }

    pub fn pdf_page(page_number: u32) -> Result<Self, ComicModelError> {
        if page_number == 0 {
            return Err(ComicModelError::InvalidSourceLocation(
                "PDF page numbers are one-based".to_owned(),
            ));
        }
        Ok(Self::PdfPage { page_number })
    }

    pub fn fixed_layout_spine_item(
        href: impl AsRef<str>,
        spine_index: u32,
    ) -> Result<Self, ComicModelError> {
        Ok(Self::FixedLayoutSpineItem {
            href: validate_relative_path(href.as_ref())?,
            spine_index,
        })
    }

    pub fn ordering_key(&self) -> String {
        match self {
            Self::Member { path } => path.clone(),
            Self::PdfPage { page_number } => format!("pdf/page/{page_number}"),
            Self::FixedLayoutSpineItem { href, spine_index } => {
                format!("epub/spine/{spine_index}/{href}")
            }
        }
    }

    pub(crate) fn identity_key(&self) -> String {
        match self {
            Self::Member { path } => format!("member\0{path}"),
            Self::PdfPage { page_number } => format!("pdf-page\0{page_number}"),
            Self::FixedLayoutSpineItem { href, spine_index } => {
                format!("fixed-layout-spine\0{spine_index}\0{href}")
            }
        }
    }

    fn default_name(&self) -> String {
        match self {
            Self::Member { path } => path.rsplit('/').next().unwrap_or(path).to_owned(),
            Self::PdfPage { page_number } => format!("Page {page_number}"),
            Self::FixedLayoutSpineItem { href, .. } => {
                href.rsplit('/').next().unwrap_or(href).to_owned()
            }
        }
    }
}

fn validate_relative_path(path: &str) -> Result<String, ComicModelError> {
    let is_windows_drive = path.as_bytes().get(1) == Some(&b':')
        && path
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic());
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || is_windows_drive
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(ComicModelError::InvalidSourceLocation(path.to_owned()));
    }
    Ok(path.to_owned())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComicSourceFormat {
    Jpeg,
    Png,
    Gif,
    Webp,
    PdfPage,
    FixedLayoutResource,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComicPageDimensions {
    width: u32,
    height: u32,
}

impl ComicPageDimensions {
    pub fn new(width: u32, height: u32) -> Result<Self, ComicModelError> {
        if width == 0 || height == 0 {
            return Err(ComicModelError::InvalidDimensions { width, height });
        }
        Ok(Self { width, height })
    }

    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }
}

/// Mutable construction value; only validated immutable pages enter a book.
#[derive(Clone, Debug)]
pub struct ComicSourcePageDraft {
    pub(super) location: SourceLocation,
    pub(super) discriminator: String,
    pub(super) encoded_size: Option<u64>,
    pub(super) original_dimensions: Option<ComicPageDimensions>,
    pub(super) source_format: ComicSourceFormat,
    pub(super) source_name: String,
    pub(super) explicit_metadata: BTreeMap<String, String>,
}

impl ComicSourcePageDraft {
    pub fn new(location: SourceLocation, source_format: ComicSourceFormat) -> Self {
        let source_name = location.default_name();
        Self {
            location,
            discriminator: String::new(),
            encoded_size: None,
            original_dimensions: None,
            source_format,
            source_name,
            explicit_metadata: BTreeMap::new(),
        }
    }

    pub fn with_discriminator(mut self, value: impl Into<String>) -> Result<Self, ComicModelError> {
        let value = value.into();
        if value.chars().any(char::is_control) {
            return Err(ComicModelError::InvalidIdentityDiscriminator);
        }
        self.discriminator = value;
        Ok(self)
    }

    pub fn with_encoded_size(mut self, size: u64) -> Self {
        self.encoded_size = Some(size);
        self
    }

    pub fn with_dimensions(mut self, dimensions: ComicPageDimensions) -> Self {
        self.original_dimensions = Some(dimensions);
        self
    }

    pub fn with_source_name(mut self, name: impl Into<String>) -> Result<Self, ComicModelError> {
        let name = name.into();
        if name.trim().is_empty() || name.chars().any(char::is_control) {
            return Err(ComicModelError::InvalidSourceName);
        }
        self.source_name = name;
        Ok(self)
    }

    pub fn with_explicit_metadata(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, ComicModelError> {
        let key = key.into();
        if key.trim().is_empty() || key.chars().any(char::is_control) {
            return Err(ComicModelError::InvalidMetadataKey);
        }
        self.explicit_metadata.insert(key, value.into());
        Ok(self)
    }

    pub fn location(&self) -> &SourceLocation {
        &self.location
    }
}

/// Immutable source facts for one logical page. Encoded bytes remain in the
/// source adapter and are fetched by `location` only when a later stage needs
/// them; decoded pixels are never retained here.
#[derive(Clone, Debug)]
pub struct ComicSourcePage {
    pub(super) id: SourcePageId,
    pub(super) location: SourceLocation,
    pub(super) identity_discriminator: String,
    pub(super) encoded_size: Option<u64>,
    pub(super) original_dimensions: Option<ComicPageDimensions>,
    pub(super) source_format: ComicSourceFormat,
    pub(super) source_name: String,
    pub(super) explicit_metadata: BTreeMap<String, String>,
}

impl ComicSourcePage {
    pub fn id(&self) -> &SourcePageId {
        &self.id
    }

    pub fn location(&self) -> &SourceLocation {
        &self.location
    }

    pub fn identity_discriminator(&self) -> &str {
        &self.identity_discriminator
    }

    pub const fn encoded_size(&self) -> Option<u64> {
        self.encoded_size
    }

    pub const fn original_dimensions(&self) -> Option<ComicPageDimensions> {
        self.original_dimensions
    }

    pub const fn source_format(&self) -> ComicSourceFormat {
        self.source_format
    }

    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    pub fn explicit_metadata(&self) -> &BTreeMap<String, String> {
        &self.explicit_metadata
    }

    pub fn encoded_resource_location(&self) -> &SourceLocation {
        &self.location
    }
}

pub(crate) fn hash_part(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}
