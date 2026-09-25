//! Comic and manga source-domain model for FolioForge 0.2.
//!
//! Comic and manga processing lives here, behind stable Core-facing models.
//! Source import currently builds immutable page manifests; image transforms,
//! target profiles and output-format behavior remain later gated milestones.

mod composition;
mod error;
mod native;
pub mod ordering;
pub mod output;
pub mod preview;
pub mod source;

pub use composition::{ComicComposition, ComicIrProjection};
pub use error::ComicModelError;
pub use native::{
    ComicNativeBook, ComicPageDimensions, ComicSourceFormat, ComicSourceIdentity, ComicSourceKind,
    ComicSourcePage, ComicSourcePageDraft, PageOrdering, SourceBookId, SourceLocation,
    SourcePageId,
};
pub use source::{
    ComicContainerKind, ComicImportError, ComicImportLimits, ComicSourceImporter, ImportedComic,
};
