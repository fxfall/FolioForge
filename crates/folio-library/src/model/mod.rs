use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type BookId = i64;
pub type FileVariantId = i64;
pub type StorageRootId = i64;
pub type PersonId = i64;
pub type SeriesId = i64;
pub type TagId = i64;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Identifier {
    pub scheme: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct BookMetadata {
    pub title: Option<String>,
    pub sort_title: Option<String>,
    pub subtitle: Option<String>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub published_date: Option<String>,
    pub description: Option<String>,
    pub authors: Vec<String>,
    pub series: Option<String>,
    pub series_index: Option<f64>,
    pub tags: Vec<String>,
    pub identifiers: Vec<Identifier>,
}

impl BookMetadata {
    pub fn sort_key(&self) -> String {
        self.sort_title
            .clone()
            .or_else(|| self.title.clone())
            .unwrap_or_default()
            .trim()
            .to_lowercase()
    }

    pub fn from_core(metadata: &folio_model::Metadata) -> Self {
        Self {
            title: metadata.title.clone(),
            sort_title: metadata.title.clone(),
            subtitle: metadata.subtitle.clone(),
            language: metadata.language.clone(),
            publisher: metadata.publisher.clone(),
            published_date: metadata
                .date
                .clone()
                .or_else(|| metadata.dates.first().cloned()),
            description: metadata.description.clone(),
            authors: metadata.author_names(),
            series: metadata.series.clone(),
            series_index: metadata.series_index,
            tags: metadata.subjects.clone(),
            identifiers: metadata
                .identifiers
                .iter()
                .map(|value| Identifier {
                    scheme: "identifier".to_owned(),
                    value: value.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BookRecord {
    pub id: BookId,
    pub uuid: Uuid,
    pub metadata: BookMetadata,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BookListItem {
    pub id: BookId,
    pub uuid: Uuid,
    pub title: Option<String>,
    pub sort_title: String,
    pub language: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StorageRoot {
    pub id: StorageRootId,
    pub uuid: Uuid,
    pub path: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FileVariant {
    pub id: FileVariantId,
    pub uuid: Uuid,
    pub book_id: BookId,
    pub storage_root_id: StorageRootId,
    pub relative_path: String,
    pub format_id: String,
    pub size_bytes: u64,
    pub mtime_ns: i128,
    pub content_hash: Option<String>,
    pub origin: FileVariantOrigin,
    pub source_file_id: Option<FileVariantId>,
    pub state: FileVariantState,
    pub added_at: i64,
    pub last_seen_at: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FileVariantDraft {
    pub book_id: BookId,
    pub storage_root_id: StorageRootId,
    pub relative_path: String,
    pub format_id: String,
    pub size_bytes: u64,
    pub mtime_ns: i128,
    pub content_hash: Option<String>,
    pub origin: FileVariantOrigin,
    pub source_file_id: Option<FileVariantId>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileVariantOrigin {
    #[default]
    Scan,
    Conversion,
    Import,
}

impl FileVariantOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Conversion => "conversion",
            Self::Import => "import",
        }
    }
}

impl fmt::Display for FileVariantOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for FileVariantOrigin {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, String> {
        match value {
            "scan" => Ok(Self::Scan),
            "conversion" => Ok(Self::Conversion),
            "import" => Ok(Self::Import),
            other => Err(format!("unknown file variant origin {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileVariantState {
    #[default]
    Present,
    Missing,
    Duplicate,
    Error,
}

impl FileVariantState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Missing => "missing",
            Self::Duplicate => "duplicate",
            Self::Error => "error",
        }
    }
}

impl TryFrom<&str> for FileVariantState {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, String> {
        match value {
            "present" => Ok(Self::Present),
            "missing" => Ok(Self::Missing),
            "duplicate" => Ok(Self::Duplicate),
            "error" => Ok(Self::Error),
            other => Err(format!("unknown file variant state {other}")),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct FileMetadata {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub published_date: Option<String>,
    pub metadata_fingerprint: String,
    pub inspected_at: i64,
    pub inspection_contract_version: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BookDetail {
    pub book: BookRecord,
    pub variants: Vec<FileVariant>,
    pub file_metadata: Vec<(FileVariantId, FileMetadata)>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ConversionRecord {
    pub id: i64,
    pub uuid: Uuid,
    pub source_file_id: Option<FileVariantId>,
    pub output_file_id: Option<FileVariantId>,
    pub target_format_id: String,
    pub status: String,
    pub created_at: i64,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    Different,
    Missing,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct LibraryStats {
    pub books: u64,
    pub file_variants: u64,
    pub present_file_variants: u64,
    pub missing_file_variants: u64,
    pub storage_roots: u64,
}
