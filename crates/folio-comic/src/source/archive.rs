use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use sevenz_rust2::{Archive, BlockDecoder, EncoderMethod, Password};
use zip::{read::ZipFile, ZipArchive};

use crate::{
    ComicNativeBook, ComicSourceIdentity, ComicSourceKind, ComicSourcePageDraft, PageOrdering,
    SourceLocation,
};

use super::{
    check_cancelled, make_imported, metadata_from_path, raster, ComicContainerKind,
    ComicImportError, ComicImportLimits, ImportedComic,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveInputFormat {
    Zip,
    SevenZip,
    Rar,
}

pub(super) fn is_pdf(path: &Path) -> Result<bool, ComicImportError> {
    Ok(read_prefix(path, 5)?.starts_with(b"%PDF-"))
}

pub(super) fn is_rar(path: &Path) -> Result<bool, ComicImportError> {
    let prefix = read_prefix(path, 8)?;
    Ok(prefix.starts_with(b"Rar!\x1a\x07\x00") || prefix.starts_with(b"Rar!\x1a\x07\x01\x00"))
}

pub(super) fn is_sevenz(path: &Path) -> Result<bool, ComicImportError> {
    Ok(read_prefix(path, 6)?.starts_with(b"7z\xbc\xaf'\x1c"))
}

pub(super) fn is_zip(path: &Path) -> Result<bool, ComicImportError> {
    let prefix = read_prefix(path, 4)?;
    Ok(prefix.starts_with(b"PK\x03\x04") || prefix.starts_with(b"PK\x05\x06"))
}

fn read_prefix(path: &Path, count: usize) -> Result<Vec<u8>, ComicImportError> {
    let mut file = File::open(path)?;
    let mut prefix = vec![0; count];
    let read = file.read(&mut prefix)?;
    prefix.truncate(read);
    Ok(prefix)
}

pub(super) fn looks_like_epub(
    path: &Path,
    limits: &ComicImportLimits,
) -> Result<bool, ComicImportError> {
    let mut archive = open_safe_zip(path, limits)?;
    let Ok(mut entry) = archive.by_name("mimetype") else {
        return Ok(false);
    };
    if entry.encrypted() || entry.size() > 128 {
        return Err(ComicImportError::InvalidSource(
            "EPUB mimetype entry is encrypted or oversized".to_owned(),
        ));
    }
    let declared_size = entry.size();
    let bytes = raster::read_bounded(&mut entry, 128, &mut || false)?;
    if bytes.len() as u64 != declared_size {
        return Err(ComicImportError::InvalidSource(
            "EPUB mimetype entry length differs from its declared size".to_owned(),
        ));
    }
    Ok(bytes == b"application/epub+zip")
}

pub(super) fn import_zip<F>(
    path: &Path,
    stable_source_key: &str,
    limits: &ComicImportLimits,
    is_cancelled: &mut F,
) -> Result<ImportedComic, ComicImportError>
where
    F: FnMut() -> bool,
{
    preflight_zip(path, limits)?;
    let mut archive = open_safe_zip(path, limits)?;
    if archive.len() > limits.max_entries {
        return Err(ComicImportError::LimitExceeded(format!(
            "ZIP contains {} entries; cap is {}",
            archive.len(),
            limits.max_entries
        )));
    }

    let mut paths = BTreeSet::new();
    let mut total_size = 0u64;
    let mut drafts = Vec::new();
    for index in 0..archive.len() {
        check_cancelled(is_cancelled)?;
        let entry = archive.by_index(index)?;
        let name = zip_member_name(&entry)?;
        if entry.is_symlink() {
            return Err(ComicImportError::UnsafePath(format!(
                "ZIP symlink entry {name:?} is not allowed"
            )));
        }
        let locator = validate_archive_path(&name, entry.is_dir(), limits.max_path_depth)?;
        if !paths.insert(locator.clone()) {
            return Err(ComicImportError::UnsafePath(format!(
                "duplicate ZIP member path {locator:?}"
            )));
        }
        if entry.encrypted() {
            return Err(ComicImportError::Unsupported(format!(
                "encrypted ZIP member {locator:?}"
            )));
        }
        let size = entry.size();
        if size > limits.max_member_bytes {
            return Err(ComicImportError::LimitExceeded(format!(
                "ZIP member {locator:?} is {size} bytes; cap is {}",
                limits.max_member_bytes
            )));
        }
        total_size = total_size
            .checked_add(size)
            .ok_or_else(|| ComicImportError::LimitExceeded("ZIP size total overflow".to_owned()))?;
        if total_size > limits.max_total_uncompressed_bytes {
            return Err(ComicImportError::LimitExceeded(format!(
                "ZIP exceeds the {} byte uncompressed total cap",
                limits.max_total_uncompressed_bytes
            )));
        }
        if entry.is_dir() {
            continue;
        }
        if let Some(format) = raster::format_from_path(Path::new(&locator)) {
            if size > limits.max_page_bytes {
                return Err(ComicImportError::LimitExceeded(format!(
                    "ZIP page {locator:?} is {size} bytes; cap is {}",
                    limits.max_page_bytes
                )));
            }
            let location = SourceLocation::member(&locator)?;
            drafts.push(
                ComicSourcePageDraft::new(location, format)
                    .with_encoded_size(size)
                    .with_source_name(locator)?,
            );
            if drafts.len() > limits.max_pages {
                return Err(ComicImportError::LimitExceeded(format!(
                    "ZIP exceeds the {} page cap",
                    limits.max_pages
                )));
            }
        }
    }

    let identity = ComicSourceIdentity::new(ComicSourceKind::Archive, stable_source_key)?;
    let native = ComicNativeBook::new(
        identity,
        metadata_from_path(path),
        drafts,
        [("container".to_owned(), "zip".to_owned())].into(),
        PageOrdering::Natural,
    )?;
    Ok(make_imported(native, path, ComicContainerKind::Zip, limits))
}

pub(super) fn open_safe_zip(
    path: &Path,
    limits: &ComicImportLimits,
) -> Result<ZipArchive<File>, ComicImportError> {
    preflight_zip(path, limits)?;
    let mut archive = ZipArchive::new(File::open(path)?)?;
    if archive.len() > limits.max_entries {
        return Err(ComicImportError::LimitExceeded(format!(
            "ZIP contains {} entries; cap is {}",
            archive.len(),
            limits.max_entries
        )));
    }
    let mut names = BTreeSet::new();
    let mut total_size = 0u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let raw_name = zip_member_name(&entry)?;
        let name = validate_archive_path(&raw_name, entry.is_dir(), limits.max_path_depth)?;
        if !names.insert(name.clone()) {
            return Err(ComicImportError::UnsafePath(format!(
                "duplicate ZIP member path {name:?}"
            )));
        }
        if entry.is_symlink() || entry.encrypted() {
            return Err(ComicImportError::UnsafePath(format!(
                "ZIP member {name:?} is a symlink or encrypted"
            )));
        }
        if entry.size() > limits.max_member_bytes {
            return Err(ComicImportError::LimitExceeded(format!(
                "ZIP member {name:?} is {} bytes; cap is {}",
                entry.size(),
                limits.max_member_bytes
            )));
        }
        total_size = total_size
            .checked_add(entry.size())
            .ok_or_else(|| ComicImportError::LimitExceeded("ZIP size total overflow".to_owned()))?;
        if total_size > limits.max_total_uncompressed_bytes {
            return Err(ComicImportError::LimitExceeded(format!(
                "ZIP exceeds the {} byte uncompressed total cap",
                limits.max_total_uncompressed_bytes
            )));
        }
    }
    Ok(archive)
}

fn zip_member_name(entry: &ZipFile<'_>) -> Result<String, ComicImportError> {
    let raw = std::str::from_utf8(entry.name_raw()).map_err(|_| {
        ComicImportError::UnsafePath("ZIP entry names must be valid UTF-8".to_owned())
    })?;
    if entry.enclosed_name().is_none() {
        return Err(ComicImportError::UnsafePath(format!(
            "ZIP member path is absolute or escapes the container: {raw:?}"
        )));
    }
    Ok(raw.to_owned())
}

fn validate_archive_path(
    raw: &str,
    is_directory: bool,
    max_depth: usize,
) -> Result<String, ComicImportError> {
    let path = if is_directory {
        raw.strip_suffix('/').unwrap_or(raw)
    } else {
        raw
    };
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.as_bytes().get(1) == Some(&b':')
        || path.chars().any(char::is_control)
    {
        return Err(ComicImportError::UnsafePath(format!(
            "unsafe archive member path {raw:?}"
        )));
    }
    let components = path.split('/').collect::<Vec<_>>();
    if components.len() > max_depth
        || components
            .iter()
            .any(|component| component.is_empty() || *component == "." || *component == "..")
    {
        return Err(ComicImportError::UnsafePath(format!(
            "archive member has traversal or exceeds path depth: {raw:?}"
        )));
    }
    // Run the same locator validation used by the immutable model.
    SourceLocation::member(path)?;
    Ok(path.to_owned())
}

fn preflight_zip(path: &Path, limits: &ComicImportLimits) -> Result<(), ComicImportError> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    if length > limits.max_source_bytes {
        return Err(ComicImportError::LimitExceeded(format!(
            "ZIP is {length} bytes; cap is {}",
            limits.max_source_bytes
        )));
    }
    const MAX_EOCD_SEARCH: u64 = 22 + u16::MAX as u64;
    let tail_len = length.min(MAX_EOCD_SEARCH) as usize;
    let mut tail = vec![0; tail_len];
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    file.read_exact(&mut tail)?;
    let eocd = tail
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .ok_or_else(|| ComicImportError::InvalidSource("ZIP end record not found".to_owned()))?;
    if eocd + 22 > tail.len() {
        return Err(ComicImportError::InvalidSource(
            "truncated ZIP end record".to_owned(),
        ));
    }
    let comment_len = u16_at(&tail, eocd + 20)? as usize;
    if eocd + 22 + comment_len != tail.len() {
        return Err(ComicImportError::InvalidSource(
            "ZIP has a truncated comment or trailing data".to_owned(),
        ));
    }

    let eocd_absolute = length - tail_len as u64 + eocd as u64;
    let disk_number = u16_at(&tail, eocd + 4)?;
    let central_disk = u16_at(&tail, eocd + 6)?;
    let entries_on_disk = u16_at(&tail, eocd + 8)?;
    let mut entries = u64::from(u16_at(&tail, eocd + 10)?);
    let mut central_size = u64::from(u32_at(&tail, eocd + 12)?);
    let mut central_offset = u64::from(u32_at(&tail, eocd + 16)?);
    if disk_number != 0 || central_disk != 0 {
        return Err(ComicImportError::Unsupported(
            "multi-disk ZIP archives are not supported".to_owned(),
        ));
    }
    let mut directory_end_limit = eocd_absolute;
    if entries == u16::MAX as u64
        || central_size == u32::MAX as u64
        || central_offset == u32::MAX as u64
    {
        let locator_offset = eocd_absolute.checked_sub(20).ok_or_else(|| {
            ComicImportError::InvalidSource("ZIP64 locator is missing".to_owned())
        })?;
        file.seek(SeekFrom::Start(locator_offset))?;
        let mut locator = [0u8; 20];
        file.read_exact(&mut locator)?;
        if &locator[..4] != b"PK\x06\x07" {
            return Err(ComicImportError::InvalidSource(
                "ZIP64 locator is invalid".to_owned(),
            ));
        }
        if u32_at(&locator, 4)? != 0 || u32_at(&locator, 16)? != 1 {
            return Err(ComicImportError::Unsupported(
                "multi-disk ZIP64 archives are not supported".to_owned(),
            ));
        }
        let zip64_offset = u64::from_le_bytes(
            locator[8..16]
                .try_into()
                .map_err(|_| ComicImportError::InvalidSource("invalid ZIP64 offset".to_owned()))?,
        );
        file.seek(SeekFrom::Start(zip64_offset))?;
        let mut record = [0u8; 56];
        file.read_exact(&mut record)?;
        if &record[..4] != b"PK\x06\x06" {
            return Err(ComicImportError::InvalidSource(
                "ZIP64 end record is invalid".to_owned(),
            ));
        }
        let record_size = u64::from_le_bytes(record[4..12].try_into().map_err(|_| {
            ComicImportError::InvalidSource("invalid ZIP64 record size".to_owned())
        })?);
        if record_size < 44 || record_size > limits.max_archive_header_bytes as u64 {
            return Err(ComicImportError::LimitExceeded(format!(
                "ZIP64 end record declares {record_size} bytes"
            )));
        }
        let zip64_end = zip64_offset
            .checked_add(12)
            .and_then(|start| start.checked_add(record_size))
            .ok_or_else(|| {
                ComicImportError::InvalidSource("ZIP64 record end overflow".to_owned())
            })?;
        if zip64_end > locator_offset {
            return Err(ComicImportError::InvalidSource(
                "ZIP64 end record overlaps its locator".to_owned(),
            ));
        }
        if u32_at(&record, 16)? != 0 || u32_at(&record, 20)? != 0 {
            return Err(ComicImportError::Unsupported(
                "multi-disk ZIP64 archives are not supported".to_owned(),
            ));
        }
        let entries_on_disk_64 = u64::from_le_bytes(record[24..32].try_into().map_err(|_| {
            ComicImportError::InvalidSource("invalid ZIP64 per-disk entry count".to_owned())
        })?);
        entries = u64::from_le_bytes(record[32..40].try_into().map_err(|_| {
            ComicImportError::InvalidSource("invalid ZIP64 entry count".to_owned())
        })?);
        if entries_on_disk_64 != entries {
            return Err(ComicImportError::Unsupported(
                "split ZIP64 central directories are not supported".to_owned(),
            ));
        }
        central_size = u64::from_le_bytes(record[40..48].try_into().map_err(|_| {
            ComicImportError::InvalidSource("invalid ZIP64 directory size".to_owned())
        })?);
        central_offset = u64::from_le_bytes(record[48..56].try_into().map_err(|_| {
            ComicImportError::InvalidSource("invalid ZIP64 directory offset".to_owned())
        })?);
        directory_end_limit = zip64_offset;
    } else if u64::from(entries_on_disk) != entries {
        return Err(ComicImportError::Unsupported(
            "split ZIP central directories are not supported".to_owned(),
        ));
    }
    if entries > limits.max_entries as u64 {
        return Err(ComicImportError::LimitExceeded(format!(
            "ZIP declares {entries} entries; cap is {}",
            limits.max_entries
        )));
    }
    if central_size > limits.max_archive_header_bytes as u64 {
        return Err(ComicImportError::LimitExceeded(format!(
            "ZIP central directory is {central_size} bytes; cap is {}",
            limits.max_archive_header_bytes
        )));
    }
    let central_end = central_offset.checked_add(central_size).ok_or_else(|| {
        ComicImportError::InvalidSource("ZIP central directory end overflow".to_owned())
    })?;
    if central_end > directory_end_limit {
        return Err(ComicImportError::InvalidSource(
            "ZIP central directory extends beyond its end record".to_owned(),
        ));
    }
    Ok(())
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, ComicImportError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| ComicImportError::InvalidSource("truncated ZIP metadata".to_owned()))?;
    Ok(u16::from_le_bytes(
        value.try_into().expect("fixed two-byte slice"),
    ))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, ComicImportError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| ComicImportError::InvalidSource("truncated ZIP metadata".to_owned()))?;
    Ok(u32::from_le_bytes(
        value.try_into().expect("fixed four-byte slice"),
    ))
}

pub(super) fn read_zip_member<F>(
    path: &Path,
    member: &str,
    limits: &ComicImportLimits,
    limit: u64,
    is_cancelled: &mut F,
) -> Result<Vec<u8>, ComicImportError>
where
    F: FnMut() -> bool,
{
    let mut archive = open_safe_zip(path, limits)?;
    read_zip_archive_member(&mut archive, member, limit, is_cancelled)
}

pub(super) fn read_zip_archive_member<F>(
    archive: &mut ZipArchive<File>,
    member: &str,
    limit: u64,
    is_cancelled: &mut F,
) -> Result<Vec<u8>, ComicImportError>
where
    F: FnMut() -> bool,
{
    let mut entry = archive.by_name(member).map_err(|error| match error {
        zip::result::ZipError::FileNotFound => ComicImportError::PageNotFound,
        other => ComicImportError::Zip(other),
    })?;
    if entry.is_dir() || entry.is_symlink() || entry.encrypted() {
        return Err(ComicImportError::UnsafePath(format!(
            "ZIP page {member:?} is not a regular unencrypted member"
        )));
    }
    if entry.size() > limit {
        return Err(ComicImportError::LimitExceeded(format!(
            "ZIP page expands to {} bytes; cap is {limit}",
            entry.size()
        )));
    }
    let bytes = raster::read_bounded(&mut entry, limit, is_cancelled)?;
    if bytes.len() as u64 != entry.size() {
        return Err(ComicImportError::InvalidSource(format!(
            "ZIP member {member:?} produced {} bytes; header declared {}",
            bytes.len(),
            entry.size()
        )));
    }
    Ok(bytes)
}

pub(super) fn import_sevenz<F>(
    path: &Path,
    stable_source_key: &str,
    limits: &ComicImportLimits,
    is_cancelled: &mut F,
) -> Result<ImportedComic, ComicImportError>
where
    F: FnMut() -> bool,
{
    let (_source, archive) = open_sevenz(path, limits)?;
    validate_sevenz_archive(&archive, limits)?;
    check_cancelled(is_cancelled)?;
    let mut total_size = 0u64;
    let mut drafts = Vec::new();
    let mut member_paths = BTreeSet::new();
    for entry in &archive.files {
        check_cancelled(is_cancelled)?;
        let locator =
            validate_archive_path(&entry.name, entry.is_directory, limits.max_path_depth)?;
        if !member_paths.insert(locator.clone()) {
            return Err(ComicImportError::UnsafePath(format!(
                "duplicate 7z member path {locator:?}"
            )));
        }
        if entry.is_anti_item {
            return Err(ComicImportError::Unsupported(
                "7z anti-items are not supported".to_owned(),
            ));
        }
        if entry.is_directory {
            continue;
        }
        if entry.size > limits.max_member_bytes {
            return Err(ComicImportError::LimitExceeded(format!(
                "7z member {locator:?} is {} bytes; cap is {}",
                entry.size, limits.max_member_bytes
            )));
        }
        total_size = total_size
            .checked_add(entry.size)
            .ok_or_else(|| ComicImportError::LimitExceeded("7z size total overflow".to_owned()))?;
        if total_size > limits.max_total_uncompressed_bytes {
            return Err(ComicImportError::LimitExceeded(format!(
                "7z exceeds the {} byte uncompressed total cap",
                limits.max_total_uncompressed_bytes
            )));
        }
        if let Some(format) = raster::format_from_path(Path::new(&locator)) {
            if entry.size > limits.max_page_bytes {
                return Err(ComicImportError::LimitExceeded(format!(
                    "7z page {locator:?} is {} bytes; cap is {}",
                    entry.size, limits.max_page_bytes
                )));
            }
            drafts.push(
                ComicSourcePageDraft::new(SourceLocation::member(&locator)?, format)
                    .with_encoded_size(entry.size)
                    .with_source_name(locator)?,
            );
            if drafts.len() > limits.max_pages {
                return Err(ComicImportError::LimitExceeded(format!(
                    "7z exceeds the {} page cap",
                    limits.max_pages
                )));
            }
        }
    }

    let identity = ComicSourceIdentity::new(ComicSourceKind::Archive, stable_source_key)?;
    let native = ComicNativeBook::new(
        identity,
        metadata_from_path(path),
        drafts,
        [("container".to_owned(), "7z".to_owned())].into(),
        PageOrdering::Natural,
    )?;
    Ok(make_imported(
        native,
        path,
        ComicContainerKind::SevenZip,
        limits,
    ))
}

fn open_sevenz(
    path: &Path,
    limits: &ComicImportLimits,
) -> Result<(File, Archive), ComicImportError> {
    let length = fs::metadata(path)?.len();
    if length > limits.max_source_bytes {
        return Err(ComicImportError::LimitExceeded(format!(
            "7z source is {length} bytes; cap is {}",
            limits.max_source_bytes
        )));
    }
    if length < 32 {
        return Err(ComicImportError::InvalidSource(
            "7z start header is truncated".to_owned(),
        ));
    }
    let mut file = File::open(path)?;
    let mut header = [0u8; 32];
    file.read_exact(&mut header)?;
    if &header[..6] != b"7z\xbc\xaf'\x1c" {
        return Err(ComicImportError::InvalidSource(
            "7z signature is invalid".to_owned(),
        ));
    }
    let next_header_offset = u64::from_le_bytes(header[12..20].try_into().expect("8 bytes"));
    let next_header_size = u64::from_le_bytes(header[20..28].try_into().expect("8 bytes"));
    if next_header_size > limits.max_archive_header_bytes as u64 {
        return Err(ComicImportError::LimitExceeded(format!(
            "7z next header is {next_header_size} bytes; cap is {}",
            limits.max_archive_header_bytes
        )));
    }
    let header_position = 32u64
        .checked_add(next_header_offset)
        .ok_or_else(|| ComicImportError::InvalidSource("7z header offset overflow".to_owned()))?;
    if header_position
        .checked_add(next_header_size)
        .is_none_or(|end| end > length)
    {
        return Err(ComicImportError::InvalidSource(
            "7z next header extends beyond the source".to_owned(),
        ));
    }
    file.seek(SeekFrom::Start(header_position))?;
    let mut header_kind = [0u8; 1];
    file.read_exact(&mut header_kind)?;
    if header_kind[0] != 0x01 {
        return Err(ComicImportError::Unsupported(
            "7z encoded or encrypted headers are not supported".to_owned(),
        ));
    }
    file.seek(SeekFrom::Start(0))?;
    let archive = Archive::read(&mut file, &Password::empty())
        .map_err(|error| ComicImportError::SevenZip(error.to_string()))?;
    Ok((file, archive))
}

fn validate_sevenz_archive(
    archive: &Archive,
    limits: &ComicImportLimits,
) -> Result<(), ComicImportError> {
    if archive.files.len() > limits.max_entries {
        return Err(ComicImportError::LimitExceeded(format!(
            "7z contains {} entries; cap is {}",
            archive.files.len(),
            limits.max_entries
        )));
    }
    for block in &archive.blocks {
        for coder in &block.coders {
            let id = coder.encoder_method_id();
            let memory_bytes = if id == EncoderMethod::ID_LZMA2 {
                let property = *coder.properties().first().ok_or_else(|| {
                    ComicImportError::InvalidSource("7z LZMA2 properties are missing".to_owned())
                })?;
                if property > 40 {
                    return Err(ComicImportError::InvalidSource(
                        "7z LZMA2 dictionary property is invalid".to_owned(),
                    ));
                }
                if property == 40 {
                    u32::MAX as u64
                } else {
                    u64::from(2 | (property & 1))
                        .checked_shl(u32::from(property / 2) + 11)
                        .ok_or_else(|| {
                            ComicImportError::InvalidSource(
                                "7z dictionary size overflow".to_owned(),
                            )
                        })?
                }
            } else if id == EncoderMethod::ID_LZMA {
                let properties = coder.properties();
                if properties.len() < 5 {
                    return Err(ComicImportError::InvalidSource(
                        "7z LZMA properties are truncated".to_owned(),
                    ));
                }
                u64::from(u32::from_le_bytes(
                    properties[1..5]
                        .try_into()
                        .expect("five-byte LZMA properties"),
                ))
            } else if id == EncoderMethod::ID_PPMD {
                let properties = coder.properties();
                if properties.len() < 5 {
                    return Err(ComicImportError::InvalidSource(
                        "7z PPMd properties are truncated".to_owned(),
                    ));
                }
                u64::from(u32::from_le_bytes(
                    properties[1..5]
                        .try_into()
                        .expect("five-byte PPMd properties"),
                ))
            } else {
                0
            };
            if memory_bytes > limits.max_sevenz_dictionary_bytes {
                return Err(ComicImportError::LimitExceeded(format!(
                    "7z coder dictionary is {memory_bytes} bytes; cap is {}",
                    limits.max_sevenz_dictionary_bytes
                )));
            }
            if !is_supported_sevenz_coder(id) {
                return Err(ComicImportError::Unsupported(format!(
                    "7z compression or filter method {:02x?}",
                    id
                )));
            }
        }
    }
    Ok(())
}

fn is_supported_sevenz_coder(id: &[u8]) -> bool {
    [
        EncoderMethod::ID_COPY,
        EncoderMethod::ID_DELTA,
        EncoderMethod::ID_LZMA,
        EncoderMethod::ID_LZMA2,
        EncoderMethod::ID_BZIP2,
        EncoderMethod::ID_PPMD,
        EncoderMethod::ID_BCJ_X86,
        EncoderMethod::ID_BCJ2,
        EncoderMethod::ID_BCJ_PPC,
        EncoderMethod::ID_BCJ_IA64,
        EncoderMethod::ID_BCJ_ARM,
        EncoderMethod::ID_BCJ_ARM64,
        EncoderMethod::ID_BCJ_ARM_THUMB,
        EncoderMethod::ID_BCJ_SPARC,
        EncoderMethod::ID_BCJ_RISCV,
    ]
    .contains(&id)
}

pub(super) fn read_sevenz_member<F>(
    path: &Path,
    member: &str,
    limits: &ComicImportLimits,
    page_limit: u64,
    is_cancelled: &mut F,
) -> Result<Vec<u8>, ComicImportError>
where
    F: FnMut() -> bool,
{
    let (mut source, archive) = open_sevenz(path, limits)?;
    validate_sevenz_archive(&archive, limits)?;
    let mut target_index = None;
    for (index, entry) in archive.files.iter().enumerate() {
        if entry.name == member && target_index.replace(index).is_some() {
            return Err(ComicImportError::UnsafePath(format!(
                "duplicate 7z member path {member:?}"
            )));
        }
    }
    let target_index = target_index.ok_or(ComicImportError::PageNotFound)?;
    let target = &archive.files[target_index];
    if target.is_directory || target.is_anti_item || !target.has_stream {
        return Err(ComicImportError::InvalidSource(format!(
            "7z page {member:?} has no regular data stream"
        )));
    }
    if target.size > page_limit {
        return Err(ComicImportError::LimitExceeded(format!(
            "7z page expands to {} bytes; cap is {page_limit}",
            target.size
        )));
    }
    let block_index = archive
        .stream_map
        .file_block_index
        .get(target_index)
        .copied()
        .flatten()
        .ok_or_else(|| ComicImportError::InvalidSource("7z page has no data block".to_owned()))?;
    let block_first = *archive
        .stream_map
        .block_first_file_index
        .get(block_index)
        .ok_or_else(|| ComicImportError::InvalidSource("7z block map is invalid".to_owned()))?;
    let mut skipped_bytes = 0u64;
    for entry in archive.files.iter().take(target_index).skip(block_first) {
        if entry.has_stream {
            skipped_bytes = skipped_bytes.checked_add(entry.size).ok_or_else(|| {
                ComicImportError::LimitExceeded("7z solid block size overflow".to_owned())
            })?;
        }
    }
    if skipped_bytes > limits.max_sevenz_solid_decode_bytes {
        return Err(ComicImportError::LimitExceeded(format!(
            "solid 7z block requires decoding {skipped_bytes} prior bytes; cap is {}",
            limits.max_sevenz_solid_decode_bytes
        )));
    }

    let password = Password::empty();
    let mut target_bytes = None;
    let mut prior_bytes = 0u64;
    let decoder = BlockDecoder::new(1, block_index, &archive, &password, &mut source);
    decoder
        .for_each_entries(&mut |entry, reader| {
            check_cancelled(is_cancelled).map_err(std::io::Error::other)?;
            if entry.name == member {
                let bytes = raster::read_bounded(reader, page_limit, is_cancelled)
                    .map_err(std::io::Error::other)?;
                if bytes.len() as u64 != entry.size {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "7z page length differs from its declared size",
                    )
                    .into());
                }
                target_bytes = Some(bytes);
                return Ok(false);
            }
            let next_total = prior_bytes
                .checked_add(entry.size)
                .ok_or_else(|| std::io::Error::other("7z prior output size overflow"))?;
            if next_total > limits.max_sevenz_solid_decode_bytes {
                return Err(std::io::Error::other(format!(
                    "solid 7z block exceeds {} bytes",
                    limits.max_sevenz_solid_decode_bytes
                ))
                .into());
            }
            prior_bytes = next_total;
            let mut buffer = [0u8; 32 * 1024];
            loop {
                check_cancelled(is_cancelled).map_err(std::io::Error::other)?;
                if reader.read(&mut buffer)? == 0 {
                    break;
                }
            }
            Ok(true)
        })
        .map_err(|error| ComicImportError::SevenZip(error.to_string()))?;
    target_bytes.ok_or(ComicImportError::PageNotFound)
}
