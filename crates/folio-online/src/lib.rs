//! Optional online-resource boundary.
//!
//! Core conversion does not depend on this crate and never performs network
//! I/O. Providers return candidates or validated assets; only a user-confirmed
//! merge plan may be translated into a `BookEditPlan`.

mod providers;

pub use providers::{
    GoogleFontsProvider, HttpResponse, HttpTransport, OpenLibraryCoverProvider,
    OpenLibraryProvider, UreqTransport,
};

use std::{collections::BTreeMap, sync::RwLock};

use folio_edit::{BookEditPlan, MetadataEdit};
use folio_model::Metadata;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_QUERY_BYTES: usize = 512;
pub const MAX_RESULTS: usize = 20;
pub const MAX_FONT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_COVER_BYTES: usize = 24 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct OnlineSettings {
    /// Disabled by default. Enabling it is an explicit frontend choice.
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ProviderRequest {
    /// Free-form fallback query.
    pub query: String,
    pub locale: Option<String>,
    pub isbn: Option<String>,
    pub identifier: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    /// Providers clamp this to `1..=MAX_RESULTS`.
    pub max_results: Option<usize>,
}

impl ProviderRequest {
    fn has_query(&self) -> bool {
        [
            self.query.as_str(),
            self.isbn.as_deref().unwrap_or_default(),
            self.identifier.as_deref().unwrap_or_default(),
            self.title.as_deref().unwrap_or_default(),
            self.author.as_deref().unwrap_or_default(),
        ]
        .iter()
        .any(|value| !value.trim().is_empty())
    }

    fn result_limit(&self) -> usize {
        self.max_results.unwrap_or(10).clamp(1, MAX_RESULTS)
    }

    pub(crate) fn validate(&self) -> Result<(), OnlineError> {
        if !self.has_query() {
            return Err(OnlineError::InvalidRequest(
                "search query is empty".to_owned(),
            ));
        }
        let values = [
            self.query.as_str(),
            self.locale.as_deref().unwrap_or_default(),
            self.isbn.as_deref().unwrap_or_default(),
            self.identifier.as_deref().unwrap_or_default(),
            self.title.as_deref().unwrap_or_default(),
            self.author.as_deref().unwrap_or_default(),
        ];
        if values.iter().any(|value| value.len() > MAX_QUERY_BYTES) {
            return Err(OnlineError::InvalidRequest(format!(
                "search fields must be at most {MAX_QUERY_BYTES} bytes"
            )));
        }
        if values
            .iter()
            .any(|value| value.chars().any(char::is_control))
        {
            return Err(OnlineError::InvalidRequest(
                "search fields must not contain control characters".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MetadataCandidate {
    pub candidate_id: String,
    pub provider: String,
    pub confidence: f32,
    pub metadata: Metadata,
    pub source_url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CoverCandidate {
    pub candidate_id: String,
    pub provider: String,
    pub title: String,
    pub authors: Vec<String>,
    pub preview_url: String,
    pub download_url: String,
    pub cache_key: String,
    pub source_url: Option<String>,
    /// Open Library does not provide a reliable per-image license in its
    /// search response; the user must review the source before importing it.
    pub license: Option<String>,
    pub license_url: Option<String>,
    pub requires_license_confirmation: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct FontCandidate {
    pub candidate_id: String,
    pub provider: String,
    pub family: String,
    pub variant: String,
    pub category: Option<String>,
    pub subsets: Vec<String>,
    pub version: Option<String>,
    pub download_url: String,
    pub cache_key: String,
    /// The Google Fonts API does not return the exact license text for a
    /// family, so this remains explicit and must be reviewed by the user.
    pub license: Option<String>,
    pub license_url: Option<String>,
    pub requires_license_confirmation: bool,
    pub source_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataMergeAction {
    #[default]
    Keep,
    Replace,
    Append,
    Ignore,
}

/// User decisions for turning an online candidate into a local edit plan.
/// Search results are never applied to a book automatically.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MetadataMergePlan {
    pub candidate: MetadataCandidate,
    pub fields: BTreeMap<String, MetadataMergeAction>,
    #[serde(default = "confirmation_required")]
    pub requires_confirmation: bool,
}

impl MetadataMergePlan {
    pub fn build_edit_plan(
        &self,
        current: &Metadata,
        confirmed: bool,
    ) -> Result<BookEditPlan, OnlineError> {
        if self.requires_confirmation && !confirmed {
            return Err(OnlineError::ConfirmationRequired);
        }
        let mut edit = MetadataEdit::default();
        for (field, action) in &self.fields {
            if matches!(
                action,
                MetadataMergeAction::Keep | MetadataMergeAction::Ignore
            ) {
                continue;
            }
            apply_metadata_field(&mut edit, current, &self.candidate.metadata, field, *action)?;
        }
        Ok(BookEditPlan {
            metadata: edit,
            ..BookEditPlan::default()
        })
    }
}

fn confirmation_required() -> bool {
    true
}

fn apply_metadata_field(
    edit: &mut MetadataEdit,
    current: &Metadata,
    candidate: &Metadata,
    field: &str,
    action: MetadataMergeAction,
) -> Result<(), OnlineError> {
    macro_rules! text_field {
        ($target:ident, $field:literal, $current:expr, $candidate:expr) => {
            match action {
                MetadataMergeAction::Replace => match $candidate
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    Some(value) => edit.$target = Some(value.to_owned()),
                    None => {
                        edit.clear_fields.insert($field.to_owned());
                    }
                },
                MetadataMergeAction::Append => {
                    edit.$target = merge_text($current.as_deref(), $candidate.as_deref());
                }
                MetadataMergeAction::Keep | MetadataMergeAction::Ignore => unreachable!(),
            }
        };
    }
    macro_rules! list_field {
        ($target:ident, $current:expr, $candidate:expr) => {{
            let values = match action {
                MetadataMergeAction::Replace => $candidate.clone(),
                MetadataMergeAction::Append => append_unique($current, &$candidate),
                MetadataMergeAction::Keep | MetadataMergeAction::Ignore => unreachable!(),
            };
            edit.$target = Some(values);
        }};
    }
    match field {
        "title" => text_field!(title, "title", &current.title, &candidate.title),
        "subtitle" => text_field!(subtitle, "subtitle", &current.subtitle, &candidate.subtitle),
        "language" => text_field!(language, "language", &current.language, &candidate.language),
        "publisher" => text_field!(
            publisher,
            "publisher",
            &current.publisher,
            &candidate.publisher
        ),
        "date" => text_field!(date, "date", &current.date, &candidate.date),
        "series" => text_field!(series, "series", &current.series, &candidate.series),
        "description" => {
            text_field!(
                description,
                "description",
                &current.description,
                &candidate.description
            )
        }
        "rights" => text_field!(rights, "rights", &current.rights, &candidate.rights),
        "authors" => list_field!(authors, &current.author_names(), &candidate.author_names()),
        "contributors" => list_field!(contributors, &current.contributors, &candidate.contributors),
        "subjects" => list_field!(subjects, &current.subjects, &candidate.subjects),
        "identifiers" => list_field!(
            identifiers,
            &current.identifier_values(),
            &candidate.identifier_values()
        ),
        "series_index" => match action {
            MetadataMergeAction::Replace => match candidate.series_index {
                Some(value) => edit.series_index = Some(value),
                None => {
                    edit.clear_fields.insert("series_index".to_owned());
                }
            },
            MetadataMergeAction::Append => {
                return Err(OnlineError::InvalidRequest(
                    "series_index does not support append".to_owned(),
                ));
            }
            MetadataMergeAction::Keep | MetadataMergeAction::Ignore => unreachable!(),
        },
        _ => {
            return Err(OnlineError::InvalidRequest(format!(
                "unknown metadata field {field}"
            )));
        }
    }
    Ok(())
}

fn merge_text(current: Option<&str>, candidate: Option<&str>) -> Option<String> {
    let candidate = candidate?.trim();
    if candidate.is_empty() {
        return None;
    }
    let current = current.unwrap_or_default().trim();
    if current.is_empty() {
        Some(candidate.to_owned())
    } else if current.eq_ignore_ascii_case(candidate) {
        None
    } else {
        Some(format!("{current}; {candidate}"))
    }
}

fn append_unique(current: &[String], candidate: &[String]) -> Vec<String> {
    let mut values = current.to_vec();
    for value in candidate
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        if !values
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(value))
        {
            values.push(value.to_owned());
        }
    }
    values
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DownloadRequest {
    pub provider: String,
    pub url: String,
    pub cache_key: String,
    pub license_acknowledged: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DownloadedAsset {
    pub cache_key: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
    pub source_url: Option<String>,
    pub license: Option<String>,
}

pub trait MetadataProvider: Send + Sync {
    fn name(&self) -> &str;
    fn search(&self, request: &ProviderRequest) -> Result<Vec<MetadataCandidate>, OnlineError>;
}

pub trait CoverProvider: Send + Sync {
    fn name(&self) -> &str;
    fn search(&self, request: &ProviderRequest) -> Result<Vec<CoverCandidate>, OnlineError>;
}

pub trait FontProvider: Send + Sync {
    fn name(&self) -> &str;
    fn search(&self, request: &ProviderRequest) -> Result<Vec<FontCandidate>, OnlineError>;
}

pub trait AssetProvider: Send + Sync {
    fn name(&self) -> &str;
    fn requires_license_confirmation(&self) -> bool {
        false
    }
    fn download(&self, request: &DownloadRequest) -> Result<DownloadedAsset, OnlineError>;
}

pub trait CacheStore: Send + Sync {
    fn get(&self, key: &str) -> Option<DownloadedAsset>;
    fn put(&self, asset: DownloadedAsset);
}

pub struct MemoryCache {
    values: RwLock<BTreeMap<String, DownloadedAsset>>,
    max_bytes: usize,
}

impl Default for MemoryCache {
    fn default() -> Self {
        Self::with_max_bytes(64 * 1024 * 1024)
    }
}

impl MemoryCache {
    pub fn with_max_bytes(max_bytes: usize) -> Self {
        Self {
            values: RwLock::new(BTreeMap::new()),
            max_bytes,
        }
    }
}

impl CacheStore for MemoryCache {
    fn get(&self, key: &str) -> Option<DownloadedAsset> {
        self.values.read().ok()?.get(key).cloned()
    }

    fn put(&self, asset: DownloadedAsset) {
        if asset.bytes.len() > self.max_bytes || self.max_bytes == 0 {
            return;
        }
        let Ok(mut values) = self.values.write() else {
            return;
        };
        let mut total = values
            .values()
            .map(|value| value.bytes.len())
            .sum::<usize>();
        if let Some(previous) = values.remove(&asset.cache_key) {
            total = total.saturating_sub(previous.bytes.len());
        }
        while total.saturating_add(asset.bytes.len()) > self.max_bytes {
            let Some(oldest_key) = values.keys().next().cloned() else {
                break;
            };
            if let Some(oldest) = values.remove(&oldest_key) {
                total = total.saturating_sub(oldest.bytes.len());
            }
        }
        values.insert(asset.cache_key.clone(), asset);
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum OnlineError {
    #[error("online providers are disabled in offline mode")]
    Disabled,
    #[error("provider request is invalid: {0}")]
    InvalidRequest(String),
    #[error("provider is unavailable: {0}")]
    Unavailable(String),
    #[error("provider asset failed signature or size validation")]
    InvalidAsset,
    #[error("user confirmation is required before applying this online result")]
    ConfirmationRequired,
}

/// Policy-enforcing boundary for frontends. Core conversion does not depend
/// on this type and remains offline even when a frontend enables providers.
pub struct OnlineBoundary<C> {
    pub settings: OnlineSettings,
    pub cache: C,
}

impl<C: CacheStore> OnlineBoundary<C> {
    pub fn new(settings: OnlineSettings, cache: C) -> Self {
        Self { settings, cache }
    }

    pub fn search<P: MetadataProvider>(
        &self,
        provider: &P,
        request: &ProviderRequest,
    ) -> Result<Vec<MetadataCandidate>, OnlineError> {
        self.check_search(request)?;
        provider.search(request)
    }

    pub fn search_covers<P: CoverProvider>(
        &self,
        provider: &P,
        request: &ProviderRequest,
    ) -> Result<Vec<CoverCandidate>, OnlineError> {
        self.check_search(request)?;
        provider.search(request)
    }

    pub fn search_fonts<P: FontProvider>(
        &self,
        provider: &P,
        request: &ProviderRequest,
    ) -> Result<Vec<FontCandidate>, OnlineError> {
        self.check_search(request)?;
        provider.search(request)
    }

    pub fn download<P: AssetProvider>(
        &self,
        provider: &P,
        request: &DownloadRequest,
    ) -> Result<DownloadedAsset, OnlineError> {
        if !self.settings.enabled {
            return Err(OnlineError::Disabled);
        }
        if request.provider != provider.name() {
            return Err(OnlineError::InvalidRequest(
                "provider identifier does not match the selected provider".to_owned(),
            ));
        }
        if provider.requires_license_confirmation() && !request.license_acknowledged {
            return Err(OnlineError::ConfirmationRequired);
        }
        if request.url.trim().is_empty()
            || request.cache_key.trim().is_empty()
            || request.cache_key.len() > 256
            || request.cache_key.chars().any(char::is_control)
        {
            return Err(OnlineError::InvalidRequest(
                "download URL or cache key is invalid".to_owned(),
            ));
        }
        let cache_key = format!("{}:{}", provider.name(), request.cache_key);
        if let Some(asset) = self.cache.get(&cache_key) {
            validate_asset(&asset)?;
            if asset.source_url.as_deref() == Some(request.url.as_str()) {
                return Ok(asset);
            }
        }
        let mut asset = provider.download(request)?;
        validate_asset(&asset)?;
        asset.cache_key = cache_key;
        self.cache.put(asset.clone());
        Ok(asset)
    }

    fn check_search(&self, request: &ProviderRequest) -> Result<(), OnlineError> {
        if !self.settings.enabled {
            return Err(OnlineError::Disabled);
        }
        request.validate()
    }
}

pub(crate) fn validate_asset(asset: &DownloadedAsset) -> Result<(), OnlineError> {
    let detected = detect_media_type(&asset.bytes).ok_or(OnlineError::InvalidAsset)?;
    let limit = if detected.starts_with("font/") {
        MAX_FONT_BYTES
    } else {
        MAX_COVER_BYTES
    };
    if asset.bytes.is_empty()
        || asset.bytes.len() > limit
        || !media_types_compatible(&asset.media_type, detected)
    {
        return Err(OnlineError::InvalidAsset);
    }
    Ok(())
}

pub(crate) fn detect_media_type(bytes: &[u8]) -> Option<&'static str> {
    if valid_png(bytes) {
        Some("image/png")
    } else if valid_jpeg_signature(bytes) {
        Some("image/jpeg")
    } else if valid_gif(bytes) {
        Some("image/gif")
    } else if valid_woff2(bytes) {
        Some("font/woff2")
    } else if valid_woff(bytes) {
        Some("font/woff")
    } else if valid_sfnt(bytes, 0, true) && bytes.starts_with(b"OTTO") {
        Some("font/otf")
    } else if valid_ttc(bytes) {
        Some("font/collection")
    } else if (bytes.starts_with(&[0, 1, 0, 0]) || bytes.starts_with(b"true"))
        && valid_sfnt(bytes, 0, true)
    {
        Some("font/ttf")
    } else {
        None
    }
}

fn valid_png(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.len() < 33
        || read_u32(bytes, 8) != Some(13)
        || bytes.get(12..16) != Some(b"IHDR")
    {
        return false;
    }
    valid_image_dimensions(read_u32(bytes, 16), read_u32(bytes, 20))
}

fn valid_jpeg_signature(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes.starts_with(&[0xff, 0xd8, 0xff]) && bytes.ends_with(&[0xff, 0xd9])
}

fn valid_gif(bytes: &[u8]) -> bool {
    if bytes.len() < 14
        || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"))
        || bytes.last() != Some(&0x3b)
    {
        return false;
    }
    let width = bytes
        .get(6..8)
        .map(|value| u16::from_le_bytes([value[0], value[1]]) as u32);
    let height = bytes
        .get(8..10)
        .map(|value| u16::from_le_bytes([value[0], value[1]]) as u32);
    valid_image_dimensions(width, height)
}

fn valid_image_dimensions(width: Option<u32>, height: Option<u32>) -> bool {
    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            u64::from(width) * u64::from(height) <= 100_000_000
        }
        _ => false,
    }
}

fn valid_woff2(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"wOF2") || bytes.len() < 49 {
        return false;
    }
    let declared_length = read_u32(bytes, 8).map(|value| value as usize);
    let table_count = read_u16(bytes, 12);
    let reserved = read_u16(bytes, 14);
    let total_sfnt_size = read_u32(bytes, 16).map(|value| value as usize);
    let compressed_size = read_u32(bytes, 20).map(|value| value as usize);
    let flavor = bytes.get(4..8);
    let valid_flavor = matches!(flavor, Some([0, 1, 0, 0]) | Some(b"true") | Some(b"OTTO"));
    let directory_floor = table_count.map(|count| 48usize + usize::from(count) * 2);
    declared_length == Some(bytes.len())
        && valid_flavor
        && table_count.is_some_and(|count| count > 0 && count <= 4096)
        && reserved == Some(0)
        && total_sfnt_size.is_some_and(|size| size > 0 && size <= MAX_FONT_BYTES)
        && compressed_size.is_some_and(|size| {
            size > 0
                && directory_floor
                    .and_then(|directory| directory.checked_add(size))
                    .is_some_and(|minimum| minimum <= bytes.len())
        })
}

fn valid_woff(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"wOFF") || bytes.len() < 44 {
        return false;
    }
    let Some(declared_length) = read_u32(bytes, 8).map(|value| value as usize) else {
        return false;
    };
    let Some(table_count) = read_u16(bytes, 12).map(usize::from) else {
        return false;
    };
    let flavor = bytes.get(4..8);
    let valid_flavor = matches!(flavor, Some([0, 1, 0, 0]) | Some(b"true") | Some(b"OTTO"));
    if declared_length != bytes.len()
        || !valid_flavor
        || table_count == 0
        || table_count > 4096
        || read_u16(bytes, 14) != Some(0)
        || !read_u32(bytes, 16).is_some_and(|size| size > 0 && size as usize <= MAX_FONT_BYTES)
    {
        return false;
    }
    let directory_end = match 44usize.checked_add(table_count.saturating_mul(20)) {
        Some(end) if end <= bytes.len() => end,
        _ => return false,
    };
    let mut ranges = Vec::with_capacity(table_count);
    for index in 0..table_count {
        let offset = 44 + index * 20;
        let Some(start) = read_u32(bytes, offset + 4).map(|value| value as usize) else {
            return false;
        };
        let Some(compressed) = read_u32(bytes, offset + 8).map(|value| value as usize) else {
            return false;
        };
        let Some(original) = read_u32(bytes, offset + 12).map(|value| value as usize) else {
            return false;
        };
        let Some(end) = start.checked_add(compressed) else {
            return false;
        };
        if start < directory_end
            || compressed == 0
            || original == 0
            || compressed > original
            || end > bytes.len()
        {
            return false;
        }
        ranges.push((start, end));
    }
    ranges.sort_unstable();
    !ranges.windows(2).any(|pair| pair[0].1 > pair[1].0)
}

fn valid_ttc(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"ttcf") || bytes.len() < 16 {
        return false;
    }
    if !matches!(read_u32(bytes, 4), Some(0x0001_0000 | 0x0002_0000)) {
        return false;
    }
    let Some(font_count) = read_u32(bytes, 8).map(|value| value as usize) else {
        return false;
    };
    if font_count == 0 || font_count > 256 || 12 + font_count * 4 > bytes.len() {
        return false;
    }
    (0..font_count).all(|index| {
        read_u32(bytes, 12 + index * 4)
            .map(|offset| offset as usize)
            .is_some_and(|offset| valid_sfnt(bytes, offset, false))
    })
}

fn valid_sfnt(bytes: &[u8], offset: usize, require_tables_after_directory: bool) -> bool {
    let Some(header_end) = offset.checked_add(12) else {
        return false;
    };
    if header_end > bytes.len()
        || !matches!(
            bytes.get(offset..offset + 4),
            Some([0, 1, 0, 0]) | Some(b"true") | Some(b"OTTO")
        )
    {
        return false;
    }
    let Some(table_count) = read_u16(bytes, offset + 4).map(usize::from) else {
        return false;
    };
    if table_count == 0 || table_count > 4096 {
        return false;
    }
    let Some(directory_end) = offset
        .checked_add(12)
        .and_then(|value| value.checked_add(table_count * 16))
    else {
        return false;
    };
    if directory_end > bytes.len() {
        return false;
    }
    (0..table_count).all(|index| {
        let record = offset + 12 + index * 16;
        let Some(start) = read_u32(bytes, record + 8).map(|value| value as usize) else {
            return false;
        };
        let Some(length) = read_u32(bytes, record + 12).map(|value| value as usize) else {
            return false;
        };
        start
            .checked_add(length)
            .is_some_and(|end| length > 0 && end <= bytes.len())
            && (!require_tables_after_directory || start >= directory_end)
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let value = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_be_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let value = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_be_bytes([value[0], value[1], value[2], value[3]]))
}

fn media_types_compatible(declared: &str, detected: &str) -> bool {
    matches!(
        (declared.to_ascii_lowercase().as_str(), detected),
        ("image/png", "image/png")
            | ("image/jpeg", "image/jpeg")
            | ("image/gif", "image/gif")
            | ("font/woff2", "font/woff2")
            | ("font/woff", "font/woff")
            | ("font/otf", "font/otf")
            | ("font/ttf", "font/ttf")
            | ("font/collection", "font/collection")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NeverProvider;

    impl MetadataProvider for NeverProvider {
        fn name(&self) -> &str {
            "never"
        }

        fn search(&self, _: &ProviderRequest) -> Result<Vec<MetadataCandidate>, OnlineError> {
            panic!("offline boundary must stop before provider")
        }
    }

    #[test]
    fn offline_is_the_default_and_does_not_call_a_provider() {
        let boundary = OnlineBoundary::new(OnlineSettings::default(), MemoryCache::default());
        let error = boundary
            .search(
                &NeverProvider,
                &ProviderRequest {
                    query: "book".to_owned(),
                    ..ProviderRequest::default()
                },
            )
            .unwrap_err();
        assert_eq!(error, OnlineError::Disabled);
    }

    #[test]
    fn candidate_merge_requires_explicit_confirmation_and_keeps_unselected_fields() {
        let current = Metadata {
            title: Some("Source title".to_owned()),
            subtitle: Some("Source subtitle".to_owned()),
            authors: vec!["Existing Author".to_owned()],
            ..Metadata::default()
        };
        let candidate_metadata = Metadata {
            title: Some("Catalog title".to_owned()),
            authors: vec!["Existing Author".to_owned(), "New Author".to_owned()],
            publisher: Some("Publisher".to_owned()),
            ..Metadata::default()
        };
        let plan = MetadataMergePlan {
            candidate: MetadataCandidate {
                metadata: candidate_metadata,
                ..MetadataCandidate::default()
            },
            fields: BTreeMap::from([
                ("title".to_owned(), MetadataMergeAction::Replace),
                ("subtitle".to_owned(), MetadataMergeAction::Replace),
                ("authors".to_owned(), MetadataMergeAction::Append),
            ]),
            requires_confirmation: true,
        };
        assert_eq!(
            plan.build_edit_plan(&current, false).unwrap_err(),
            OnlineError::ConfirmationRequired
        );
        let edit = plan.build_edit_plan(&current, true).unwrap();
        assert_eq!(edit.metadata.title.as_deref(), Some("Catalog title"));
        assert!(edit.metadata.clear_fields.contains("subtitle"));
        assert_eq!(
            edit.metadata.authors.clone().unwrap(),
            vec!["Existing Author".to_owned(), "New Author".to_owned()]
        );
        assert_eq!(edit.metadata.publisher, None);
    }

    #[test]
    fn downloaded_asset_requires_signature_and_matching_media_type() {
        let invalid = DownloadedAsset {
            media_type: "image/png".to_owned(),
            bytes: b"not an image".to_vec(),
            ..DownloadedAsset::default()
        };
        assert_eq!(validate_asset(&invalid), Err(OnlineError::InvalidAsset));
    }

    #[test]
    fn a_font_signature_without_a_complete_container_header_is_rejected() {
        let invalid = DownloadedAsset {
            media_type: "font/woff2".to_owned(),
            bytes: b"wOF2mock".to_vec(),
            ..DownloadedAsset::default()
        };
        assert_eq!(validate_asset(&invalid), Err(OnlineError::InvalidAsset));
    }

    #[test]
    fn memory_cache_stays_within_its_byte_budget() {
        let cache = MemoryCache::with_max_bytes(4);
        cache.put(DownloadedAsset {
            cache_key: "z".to_owned(),
            media_type: "image/png".to_owned(),
            bytes: vec![1, 2, 3],
            ..DownloadedAsset::default()
        });
        cache.put(DownloadedAsset {
            cache_key: "a".to_owned(),
            media_type: "image/png".to_owned(),
            bytes: vec![4, 5, 6],
            ..DownloadedAsset::default()
        });
        assert!(cache.get("z").is_none());
        assert_eq!(cache.get("a").unwrap().bytes.len(), 3);
    }

    #[test]
    fn sfnt_fonts_require_a_complete_table_directory_and_bounded_table_ranges() {
        let mut bytes = vec![0; 29];
        bytes[..4].copy_from_slice(&[0, 1, 0, 0]);
        bytes[4..6].copy_from_slice(&1u16.to_be_bytes());
        bytes[12..16].copy_from_slice(b"name");
        bytes[20..24].copy_from_slice(&28u32.to_be_bytes());
        bytes[24..28].copy_from_slice(&1u32.to_be_bytes());
        bytes[28] = 1;
        assert_eq!(detect_media_type(&bytes), Some("font/ttf"));

        bytes[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(detect_media_type(&bytes), None);
    }
}
