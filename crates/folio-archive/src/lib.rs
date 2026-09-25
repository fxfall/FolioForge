//! Deterministic output archives used by batch and service adapters.

use std::{
    fs::File,
    io::{Seek, Write},
    path::{Path, PathBuf},
};

use thiserror::Error;
use zip::{write::SimpleFileOptions, ZipWriter};

#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("archive I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("archive ZIP construction failed: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("archive report serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn write_archive(
    output: &Path,
    entries: &[(PathBuf, Vec<u8>)],
    report: &serde_json::Value,
) -> Result<(), ArchiveError> {
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(output)?;
    write_archive_to(file, entries, report)
}

pub fn write_archive_to<W: Write + Seek>(
    writer: W,
    entries: &[(PathBuf, Vec<u8>)],
    report: &serde_json::Value,
) -> Result<(), ArchiveError> {
    let mut zip = ZipWriter::new(writer);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (path, bytes) in entries {
        let name = safe_archive_path(path);
        zip.start_file(name, options)?;
        zip.write_all(bytes)?;
    }
    zip.start_file("FolioForge-report.json", options)?;
    zip.write_all(serde_json::to_string_pretty(report)?.as_bytes())?;
    zip.finish()?;
    Ok(())
}

fn safe_archive_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    value
        .split('/')
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .map(|part| {
            part.chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric()
                        || matches!(character, '.' | '-' | '_' | '/')
                    {
                        character
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-archive/src/lib.rs"]
mod tests;
