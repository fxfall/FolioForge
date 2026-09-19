//! Optional Library data foundation for FolioForge.
//!
//! This crate owns SQLite, durable Library records, scanner policy and
//! rebuildable search. It consumes the public Core contract and is deliberately
//! absent from `folio-core`, the standalone converter and the existing UI.

mod db;
mod error;
pub mod metadata;
pub mod model;
pub mod repository;
pub mod scanner;

pub use db::{LibraryDb, CURRENT_SCHEMA_VERSION};
pub use error::LibraryError;
pub use metadata::{metadata_diff, MetadataDiff, INSPECTION_CONTRACT_VERSION};
pub use model::{
    BookDetail, BookId, BookListItem, BookMetadata, BookRecord, ConversionRecord, FileMetadata,
    FileVariant, FileVariantDraft, FileVariantId, FileVariantOrigin, FileVariantState, Identifier,
    LibraryStats, PersonId, SeriesId, StorageRoot, StorageRootId, SyncStatus, TagId,
};
pub use repository::{BookPage, Library, MetadataSyncResult, QueryBudget, SearchResult};
pub use scanner::{scan, ScanOptions, ScanReport};
