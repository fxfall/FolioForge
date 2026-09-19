use std::{io, path::PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Core error: {0}")]
    Core(#[from] folio_core::CoreError),
    #[error("invalid Library input: {0}")]
    InvalidInput(String),
    #[error("unsafe relative path: {0}")]
    UnsafeRelativePath(PathBuf),
    #[error("Library record not found: {0}")]
    NotFound(String),
    #[error("migration failed at version {version} ({name}): {source}")]
    Migration {
        version: i64,
        name: String,
        source: rusqlite::Error,
    },
    #[error("query budget exceeded: {used} statements (budget {budget})")]
    QueryBudgetExceeded { used: usize, budget: usize },
    #[error("query budget was not satisfied: {used} statements (expected at most {budget})")]
    QueryBudgetNotSatisfied { used: usize, budget: usize },
}
