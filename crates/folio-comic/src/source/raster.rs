use std::{
    fs,
    io::Read,
    path::{Component, Path},
};

use crate::ComicSourceFormat;

use super::{check_cancelled, ComicImportError};

pub(super) fn format_from_path(path: &Path) -> Option<ComicSourceFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" => Some(ComicSourceFormat::Jpeg),
        "png" => Some(ComicSourceFormat::Png),
        "gif" => Some(ComicSourceFormat::Gif),
        "webp" => Some(ComicSourceFormat::Webp),
        _ => None,
    }
}

pub(super) fn relative_path_to_posix(path: &Path) -> Result<String, ComicImportError> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    ComicImportError::UnsafePath("source names must be valid UTF-8".to_owned())
                })?;
                if value.is_empty()
                    || value == "."
                    || value == ".."
                    || value.contains('/')
                    || value.contains('\\')
                    || value.chars().any(char::is_control)
                {
                    return Err(ComicImportError::UnsafePath(format!(
                        "unsafe source path component {value:?}"
                    )));
                }
                components.push(value);
            }
            _ => {
                return Err(ComicImportError::UnsafePath(format!(
                    "source path is not a normalized relative path: {}",
                    path.display()
                )))
            }
        }
    }
    if components.is_empty() {
        return Err(ComicImportError::UnsafePath(
            "source path is empty".to_owned(),
        ));
    }
    Ok(components.join("/"))
}

pub(super) fn sniff_image_format(bytes: &[u8]) -> Option<ComicSourceFormat> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ComicSourceFormat::Jpeg)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ComicSourceFormat::Png)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ComicSourceFormat::Gif)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ComicSourceFormat::Webp)
    } else {
        None
    }
}

pub(super) fn read_bounded<R, F>(
    reader: &mut R,
    limit: u64,
    is_cancelled: &mut F,
) -> Result<Vec<u8>, ComicImportError>
where
    R: Read + ?Sized,
    F: FnMut() -> bool,
{
    let capacity = usize::try_from(limit.min(16 * 1024 * 1024)).unwrap_or(0);
    let mut output = Vec::with_capacity(capacity);
    let mut buffer = [0u8; 32 * 1024];
    loop {
        check_cancelled(is_cancelled)?;
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let next_size = (output.len() as u64)
            .checked_add(count as u64)
            .ok_or_else(|| ComicImportError::LimitExceeded("page size overflow".to_owned()))?;
        if next_size > limit {
            return Err(ComicImportError::LimitExceeded(format!(
                "page expands beyond {limit} bytes"
            )));
        }
        output.extend_from_slice(&buffer[..count]);
    }
    Ok(output)
}

pub(super) fn read_regular_file<F>(
    path: &Path,
    limit: u64,
    is_cancelled: &mut F,
) -> Result<Vec<u8>, ComicImportError>
where
    F: FnMut() -> bool,
{
    check_cancelled(is_cancelled)?;
    let file_type = fs::symlink_metadata(path)?.file_type();
    if file_type.is_symlink() || !file_type.is_file() {
        return Err(ComicImportError::UnsafePath(format!(
            "page is not a regular file: {}",
            path.display()
        )));
    }
    let mut file = fs::File::open(path)?;
    let size = file.metadata()?.len();
    if size > limit {
        return Err(ComicImportError::LimitExceeded(format!(
            "page is {size} bytes; cap is {limit}"
        )));
    }
    read_bounded(&mut file, limit, is_cancelled)
}
