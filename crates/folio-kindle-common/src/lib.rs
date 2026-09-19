//! Shared low-level building blocks for Kindle-family containers.

pub mod binary;
pub mod compression;
pub mod pdb;

pub use compression::Compression;
pub use pdb::{write_pdb, PdbDocument, PdbRecord};
