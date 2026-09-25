use thiserror::Error;

use crate::SourcePageId;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ComicModelError {
    #[error("source identity must not be empty or contain control characters")]
    InvalidSourceIdentity,
    #[error("source path is empty, absolute, or contains an unsafe component: {0}")]
    InvalidSourceLocation(String),
    #[error("a comic source book must contain at least one page")]
    EmptyBook,
    #[error("page dimensions must both be non-zero (got {width}x{height})")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("source page name must not be empty or contain control characters")]
    InvalidSourceName,
    #[error("source metadata keys must not be empty or contain control characters")]
    InvalidMetadataKey,
    #[error("identity discriminators must not contain control characters")]
    InvalidIdentityDiscriminator,
    #[error("duplicate source page location and discriminator: {0}")]
    DuplicateSourcePage(String),
    #[error("two source locations produced the same stable page identity")]
    DuplicatePageIdentity,
    #[error("explicit page order contains {actual} entries; expected {expected}")]
    InvalidOrderLength { expected: usize, actual: usize },
    #[error("explicit page order contains an unknown page identity: {0}")]
    UnknownPageInOrder(SourcePageId),
    #[error("explicit page order contains a page identity more than once: {0}")]
    DuplicatePageInOrder(SourcePageId),
}
