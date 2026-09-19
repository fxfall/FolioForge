//! Shared PalmDOC/MOBI/PDB writer primitives for KF7 and KF8.

use folio_kindle_common::{
    binary::{put_u32_be, read_u16_be, read_u32_be},
    compression::compress,
    write_pdb, Compression, PdbDocument, PdbRecord,
};
use folio_model::{Diagnostic, Metadata, ResourceKind};
use thiserror::Error;

const PALMDOC_HEADER_LEN: usize = 16;
const MOBI_HEADER_LEN: usize = 232;
const TEXT_RECORD_SIZE: usize = 4096;

#[derive(Clone, Debug)]
pub struct MobiInput {
    pub title: String,
    pub author: Option<String>,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub html: Vec<u8>,
    pub images: Vec<Vec<u8>>,
    pub compression: Compression,
    pub kf8: bool,
    pub deterministic: bool,
    pub extra_exth: Vec<(u32, Vec<u8>)>,
    /// Additional records owned by a higher Kindle target (for example KF8's
    /// FDST/SKEL/FRAG/INDX records).  The shared PDB/MOBI layer only packages
    /// them and does not interpret their target-specific payloads.
    pub extra_records: Vec<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct MobiArtifact {
    pub bytes: Vec<u8>,
    pub text_record_count: u16,
    pub image_record_start: Option<u16>,
}

#[derive(Clone, Debug)]
pub struct MobiInspection {
    pub record_offsets: Vec<u32>,
    pub records: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MobiVariant {
    Kf7,
    Kf8,
}

impl MobiVariant {
    fn diagnostic_code(self, suffix: &str) -> String {
        let format = match self {
            Self::Kf7 => "KF7",
            Self::Kf8 => "KF8",
        };
        format!("FF-{format}-IMPORT-{suffix}")
    }
}

#[derive(Clone, Debug)]
pub struct MobiMetadataReport {
    pub metadata: Metadata,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug)]
pub struct ComboArtifact {
    pub bytes: Vec<u8>,
    pub kf7_record_count: usize,
    pub kf8_record_count: usize,
}

/// Return the EXTH 201 cover-image index when a MOBI/KF8 input declares one.
/// The value is relative to the first image record, exactly as used by the
/// MOBI header's image record range. A missing or malformed declaration is
/// intentionally represented as `None`; callers must not infer a cover from
/// image order.
pub fn read_cover_image_index(record_zero: &[u8]) -> Option<u32> {
    let standard_flags = read_u32_be(record_zero, 0x80).ok()?;
    let standard_header_length = read_u32_be(record_zero, 20).ok()? as usize;
    let (flags, exth_start) = if standard_flags & 0x40 != 0 && standard_header_length >= 232 {
        (standard_flags, 16usize.checked_add(standard_header_length)?)
    } else {
        (read_u32_be(record_zero, 16 + 72).ok()?, 16usize + 232)
    };
    if flags & 0x40 == 0 {
        return None;
    }
    let header_end = exth_start.checked_add(12)?;
    let header = record_zero.get(exth_start..header_end)?;
    if &header[..4] != b"EXTH" {
        return None;
    }
    let total_length = read_u32_be(header, 4).ok()? as usize;
    let count = read_u32_be(header, 8).ok()? as usize;
    let exth_end = exth_start.checked_add(total_length)?;
    if total_length < 12 || exth_end > record_zero.len() {
        return None;
    }
    let mut cursor = header_end;
    for _ in 0..count {
        let entry_header_end = cursor.checked_add(8)?;
        let entry = record_zero.get(cursor..entry_header_end)?;
        let kind = read_u32_be(entry, 0).ok()?;
        let length = read_u32_be(entry, 4).ok()? as usize;
        if length < 8 {
            return None;
        }
        let end = cursor.checked_add(length)?;
        if end > exth_end {
            return None;
        }
        if kind == 201 {
            return read_u32_be(record_zero.get(entry_header_end..end)?, 0).ok();
        }
        cursor = end;
    }
    None
}

#[derive(Debug, Error)]
pub enum MobiReadError {
    #[error("MOBI/PDB header is truncated")]
    Truncated,
    #[error("MOBI/PDB record table is invalid")]
    InvalidRecordTable,
    #[error("MOBI/PDB record offset is out of bounds")]
    OffsetOutOfBounds,
}

#[derive(Debug, Error)]
pub enum ComboError {
    #[error("KF7 combo input is invalid: {0}")]
    Kf7(#[from] MobiReadError),
    #[error("KF8 combo input is invalid: {0}")]
    Kf8(MobiReadError),
    #[error("KF7/KF8 combo payload is too large or malformed")]
    TooLarge,
    #[error("KF7/KF8 combo PDB construction failed: {0}")]
    Pdb(#[from] folio_kindle_common::pdb::PdbError),
}

#[derive(Debug, Error)]
pub enum MobiError {
    #[error("unsupported HUFF/CDIC compression")]
    Compression(#[from] folio_kindle_common::compression::CompressionError),
    #[error("MOBI document is too large for its record counters")]
    TooLarge,
    #[error("MOBI binary construction failed: {0}")]
    Pdb(#[from] folio_kindle_common::pdb::PdbError),
    #[error("MOBI header construction failed: {0}")]
    Binary(#[from] folio_kindle_common::binary::BinaryError),
}

/// Read PDB records without interpreting target-specific payloads. This is
/// shared by semantic inspectors and validators so KF7/KF8 do not duplicate
/// record-table parsing.
pub fn inspect_pdb(bytes: &[u8]) -> Result<MobiInspection, MobiReadError> {
    if bytes.len() < 78 {
        return Err(MobiReadError::Truncated);
    }
    let record_count = read_u16_be(bytes, 76).map_err(|_| MobiReadError::Truncated)? as usize;
    if record_count == 0 {
        return Err(MobiReadError::InvalidRecordTable);
    }
    let table_size = record_count
        .checked_mul(8)
        .ok_or(MobiReadError::InvalidRecordTable)?;
    let data_start = 78usize
        .checked_add(table_size)
        .ok_or(MobiReadError::InvalidRecordTable)?;
    if data_start > bytes.len() {
        return Err(MobiReadError::InvalidRecordTable);
    }
    let mut offsets = Vec::with_capacity(record_count);
    for index in 0..record_count {
        let table_offset = 78usize
            .checked_add(
                index
                    .checked_mul(8)
                    .ok_or(MobiReadError::InvalidRecordTable)?,
            )
            .ok_or(MobiReadError::InvalidRecordTable)?;
        let offset =
            read_u32_be(bytes, table_offset).map_err(|_| MobiReadError::Truncated)? as usize;
        if offset < data_start || offset > bytes.len() {
            return Err(MobiReadError::OffsetOutOfBounds);
        }
        if let Some(previous) = offsets.last().copied() {
            if offset < previous {
                return Err(MobiReadError::InvalidRecordTable);
            }
        }
        offsets.push(offset);
    }
    let records = offsets
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = offsets.get(index + 1).copied().unwrap_or(bytes.len());
            bytes
                .get(*start..end)
                .map(ToOwned::to_owned)
                .ok_or(MobiReadError::OffsetOutOfBounds)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(MobiInspection {
        record_offsets: offsets.into_iter().map(|value| value as u32).collect(),
        records,
    })
}

/// Decode the shared MOBI name and supported EXTH metadata fields for both
/// KF7 and KF8 importers. Target-specific records remain the responsibility of
/// their respective format adapters.
pub fn read_metadata(record_zero: &[u8], variant: MobiVariant) -> MobiMetadataReport {
    let mut metadata = Metadata {
        title: read_full_name(record_zero),
        ..Metadata::default()
    };
    let mut diagnostics = Vec::new();
    let standard_flags = read_u32_be(record_zero, 0x80).unwrap_or(0);
    let standard_header_length = read_u32_be(record_zero, 20).unwrap_or(0) as usize;
    let (flags, exth_start) = if standard_flags & 0x40 != 0 && standard_header_length >= 232 {
        let exth_start = 16usize.saturating_add(standard_header_length);
        (standard_flags, exth_start)
    } else {
        let flags = match read_u32_be(record_zero, 16 + 72) {
            Ok(value) => value,
            Err(_) => {
                return MobiMetadataReport {
                    metadata,
                    diagnostics,
                }
            }
        };
        (flags, 16usize + 232)
    };
    if flags & 0x40 == 0 {
        return MobiMetadataReport {
            metadata,
            diagnostics,
        };
    }

    let Some(header_end) = exth_start.checked_add(12) else {
        return MobiMetadataReport {
            metadata,
            diagnostics,
        };
    };
    let Some(header) = record_zero.get(exth_start..header_end) else {
        diagnostics.push(Diagnostic::warning(
            variant.diagnostic_code("0001"),
            "EXTH flag is set but EXTH is truncated.",
        ));
        return MobiMetadataReport {
            metadata,
            diagnostics,
        };
    };
    if &header[..4] != b"EXTH" {
        diagnostics.push(Diagnostic::warning(
            variant.diagnostic_code("0002"),
            "EXTH flag is set but the EXTH marker is absent.",
        ));
        return MobiMetadataReport {
            metadata,
            diagnostics,
        };
    }

    let total_length = match read_u32_be(header, 4) {
        Ok(value) => value as usize,
        Err(_) => 0,
    };
    let count = match read_u32_be(header, 8) {
        Ok(value) => value as usize,
        Err(_) => 0,
    };
    let Some(exth_end) = exth_start.checked_add(total_length) else {
        diagnostics.push(Diagnostic::warning(
            variant.diagnostic_code("0003"),
            "EXTH length overflows the record-zero address space.",
        ));
        return MobiMetadataReport {
            metadata,
            diagnostics,
        };
    };
    if total_length < 12 || exth_end > record_zero.len() {
        diagnostics.push(Diagnostic::warning(
            variant.diagnostic_code("0003"),
            "EXTH length is invalid or extends beyond record zero.",
        ));
        return MobiMetadataReport {
            metadata,
            diagnostics,
        };
    }

    let mut cursor = header_end;
    for _ in 0..count {
        let Some(entry_header_end) = cursor.checked_add(8) else {
            break;
        };
        if entry_header_end > exth_end {
            diagnostics.push(Diagnostic::warning(
                variant.diagnostic_code("0004"),
                "EXTH entry header extends beyond the EXTH block.",
            ));
            break;
        }
        let Some(entry) = record_zero.get(cursor..entry_header_end) else {
            break;
        };
        let kind = read_u32_be(entry, 0).unwrap_or(0);
        let length = read_u32_be(entry, 4).unwrap_or(0) as usize;
        if length < 8 {
            diagnostics.push(Diagnostic::warning(
                variant.diagnostic_code("0005"),
                "EXTH entry length is smaller than its header.",
            ));
            break;
        }
        let Some(end) = cursor.checked_add(length) else {
            diagnostics.push(Diagnostic::warning(
                variant.diagnostic_code("0005"),
                "EXTH entry length overflows the record-zero address space.",
            ));
            break;
        };
        if end > exth_end {
            diagnostics.push(Diagnostic::warning(
                variant.diagnostic_code("0005"),
                "EXTH entry extends beyond the EXTH block.",
            ));
            break;
        }
        let Some(value) = record_zero.get(entry_header_end..end) else {
            break;
        };
        let text = String::from_utf8_lossy(value).trim_matches('\0').to_owned();
        match kind {
            100 if !text.is_empty() => metadata.add_author(text),
            101 if !text.is_empty() => metadata.publisher = Some(text),
            103 if !text.is_empty() => metadata.description = Some(text),
            503 if !text.is_empty() && metadata.title.is_none() => metadata.title = Some(text),
            _ => {}
        }
        cursor = end;
    }

    MobiMetadataReport {
        metadata,
        diagnostics,
    }
}

fn read_full_name(record_zero: &[u8]) -> Option<String> {
    if let (Ok(offset), Ok(length)) = (
        read_u32_be(record_zero, 0x54),
        read_u32_be(record_zero, 0x58),
    ) {
        let start = offset as usize;
        let end = start.checked_add(length as usize)?;
        if let Some(value) = record_zero.get(start..end) {
            let value = value.split(|byte| *byte == 0).next().unwrap_or(value);
            let value = String::from_utf8_lossy(value).trim().to_owned();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    let offset = read_u32_be(record_zero, 16 + 28).ok()? as usize;
    let length = read_u32_be(record_zero, 16 + 32).ok()? as usize;
    let start = 16usize.checked_add(offset)?;
    let end = start.checked_add(length)?;
    let value = record_zero.get(start..end)?;
    let value = value.split(|byte| *byte == 0).next().unwrap_or(value);
    let value = String::from_utf8_lossy(value).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

/// Return the semantic image kind shared by KF7 and KF8 PDB resource records.
pub fn image_kind(bytes: &[u8]) -> ResourceKind {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        ResourceKind::Jpeg
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ResourceKind::Png
    } else if bytes.starts_with(b"GIF8") {
        ResourceKind::Gif
    } else {
        ResourceKind::Unknown
    }
}

/// MIME type corresponding to the shared MOBI image sniffing result.
pub fn media_type_for_kind(kind: &ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Jpeg => "image/jpeg",
        ResourceKind::Png => "image/png",
        ResourceKind::Gif => "image/gif",
        _ => "application/octet-stream",
    }
}

/// Package two independently lowered artifacts into a stable, inspectable
/// composite MOBI container. The embedded KF8 artifact is deliberately
/// prefixed with `FFKF8COMBO` so this compatibility form cannot be mistaken
/// for an undocumented Amazon dual-format boundary.
pub fn pack_combo(
    kf7: &MobiArtifact,
    kf8: &MobiArtifact,
    title: &str,
    deterministic: bool,
) -> Result<ComboArtifact, ComboError> {
    let kf7_records = inspect_pdb(&kf7.bytes)?;
    let kf8_records = inspect_pdb(&kf8.bytes).map_err(ComboError::Kf8)?;
    let embedded_size = u64::try_from(kf8.bytes.len()).map_err(|_| ComboError::TooLarge)?;
    let mut embedded = b"FFKF8COMBO\0".to_vec();
    embedded.extend_from_slice(&embedded_size.to_be_bytes());
    embedded.extend_from_slice(&kf8.bytes);
    let mut records = kf7_records
        .records
        .into_iter()
        .map(PdbRecord::new)
        .collect::<Vec<_>>();
    records.push(PdbRecord::new(embedded));
    let record_count = records.len();
    let pdb = PdbDocument {
        name: title.to_owned(),
        type_code: *b"BOOK",
        creator: *b"MOBI",
        records,
        deterministic,
    };
    Ok(ComboArtifact {
        bytes: write_pdb(&pdb)?,
        kf7_record_count: record_count.saturating_sub(1),
        kf8_record_count: kf8_records.records.len(),
    })
}

pub fn inspect_combo(bytes: &[u8]) -> Result<serde_json::Value, ComboError> {
    let inspection = inspect_pdb(bytes).map_err(ComboError::Kf7)?;
    let marker = inspection
        .records
        .iter()
        .find(|record| record.starts_with(b"FFKF8COMBO\0"))
        .ok_or(ComboError::TooLarge)?;
    let size_start = b"FFKF8COMBO\0".len();
    let size_end = size_start.checked_add(8).ok_or(ComboError::TooLarge)?;
    let size_bytes = marker
        .get(size_start..size_end)
        .ok_or(ComboError::TooLarge)?;
    let embedded_size = usize::try_from(u64::from_be_bytes(
        size_bytes.try_into().map_err(|_| ComboError::TooLarge)?,
    ))
    .map_err(|_| ComboError::TooLarge)?;
    let embedded_start = size_end;
    let embedded_end = embedded_start
        .checked_add(embedded_size)
        .ok_or(ComboError::TooLarge)?;
    let embedded = marker
        .get(embedded_start..embedded_end)
        .ok_or(ComboError::TooLarge)?;
    let kf8 = inspect_pdb(embedded).map_err(ComboError::Kf8)?;
    Ok(serde_json::json!({
        "format": "KF7+KF8-composite",
        "compatibility_container": "FolioForge-KF7-KF8-Combo",
        "kf7_record_count": inspection.records.len().saturating_sub(1),
        "kf8_record_count": kf8.records.len(),
        "embedded_kf8_bytes": embedded.len(),
    }))
}

pub fn build_mobi(input: &MobiInput) -> Result<MobiArtifact, MobiError> {
    let compressed = compress(&input.html, input.compression)?;
    let chunks: Vec<Vec<u8>> = if compressed.is_empty() {
        vec![Vec::new()]
    } else {
        compressed
            .chunks(TEXT_RECORD_SIZE)
            .map(ToOwned::to_owned)
            .collect()
    };
    let text_record_count = u16::try_from(chunks.len()).map_err(|_| MobiError::TooLarge)?;
    if chunks.len() > usize::from(u16::MAX) || input.images.len() > usize::from(u16::MAX) {
        return Err(MobiError::TooLarge);
    }

    let exth = build_exth(input);
    let mut record_zero = vec![0u8; PALMDOC_HEADER_LEN + MOBI_HEADER_LEN];
    let text_length = u32::try_from(input.html.len()).map_err(|_| MobiError::TooLarge)?;
    put_u16(&mut record_zero, 0, input.compression.code())?;
    put_u32_be(&mut record_zero, 4, text_length)?;
    put_u16(&mut record_zero, 8, chunks.len() as u16)?;
    put_u16(&mut record_zero, 10, TEXT_RECORD_SIZE as u16)?;

    let exth_offset = record_zero.len();
    record_zero.extend_from_slice(&exth);
    let full_name_offset = record_zero.len();
    record_zero.extend_from_slice(input.title.as_bytes());
    record_zero.push(0);
    if full_name_offset > u32::MAX as usize {
        return Err(MobiError::TooLarge);
    }
    let full_name_length = input.title.len().saturating_add(1);
    let mut mobi = vec![0u8; MOBI_HEADER_LEN];
    mobi[..4].copy_from_slice(b"MOBI");
    put_u32_be(&mut mobi, 4, MOBI_HEADER_LEN as u32)?;
    put_u32_be(&mut mobi, 8, 2)?; // Mobipocket book
    put_u32_be(&mut mobi, 12, 65001)?; // UTF-8
    put_u32_be(&mut mobi, 16, stable_id(&input.title))?;
    put_u32_be(&mut mobi, 20, if input.kf8 { 8 } else { 6 })?;
    // MOBI offsets are relative to the beginning of the MOBI header, not the
    // PalmDOC prefix in record zero.
    put_u32_be(
        &mut mobi,
        28,
        full_name_offset.saturating_sub(PALMDOC_HEADER_LEN) as u32,
    )?;
    put_u32_be(&mut mobi, 32, full_name_length as u32)?;
    put_u32_be(&mut mobi, 36, language_code(input.language.as_deref()))?;
    put_u32_be(&mut mobi, 48, if input.kf8 { 8 } else { 6 })?;
    let first_image_index = if input.images.is_empty() {
        u32::MAX
    } else {
        1u32 + chunks.len() as u32
    };
    put_u32_be(&mut mobi, 52, first_image_index)?;
    put_u32_be(&mut mobi, 72, 0x40)?; // EXTH present
    put_u32_be(&mut mobi, 92, 0)?; // no DRM
    record_zero[PALMDOC_HEADER_LEN..PALMDOC_HEADER_LEN + MOBI_HEADER_LEN].copy_from_slice(&mobi);
    debug_assert_eq!(exth_offset, PALMDOC_HEADER_LEN + MOBI_HEADER_LEN);

    let mut records =
        Vec::with_capacity(1 + chunks.len() + input.images.len() + input.extra_records.len());
    records.push(PdbRecord::new(record_zero));
    records.extend(chunks.into_iter().map(PdbRecord::new));
    let image_record_start = if input.images.is_empty() {
        None
    } else {
        Some((1 + records.len() - 1) as u16)
    };
    records.extend(input.images.iter().cloned().map(PdbRecord::new));
    records.extend(input.extra_records.iter().cloned().map(PdbRecord::new));

    let pdb = PdbDocument {
        name: input.title.clone(),
        type_code: *b"BOOK",
        creator: *b"MOBI",
        records,
        deterministic: input.deterministic,
    };
    Ok(MobiArtifact {
        bytes: folio_kindle_common::write_pdb(&pdb)?,
        text_record_count,
        image_record_start,
    })
}

fn put_u16(output: &mut [u8], offset: usize, value: u16) -> Result<(), MobiError> {
    folio_kindle_common::binary::put_u16_be(output, offset, value).map_err(MobiError::from)
}

fn stable_id(title: &str) -> u32 {
    let mut hash = 2166136261u32;
    for byte in title.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16777619);
    }
    hash
}

fn language_code(language: Option<&str>) -> u32 {
    match language.unwrap_or_default().to_ascii_lowercase().as_str() {
        "en" | "en-us" | "en-gb" => 0x0000_0009,
        "zh" | "zh-cn" | "zh-hans" => 0x0000_0004,
        "ja" => 0x0000_0011,
        "de" => 0x0000_0007,
        "fr" => 0x0000_000c,
        _ => 0,
    }
}

fn build_exth(input: &MobiInput) -> Vec<u8> {
    let mut entries: Vec<(u32, Vec<u8>)> = Vec::new();
    if let Some(author) = &input.author {
        entries.push((100, author.as_bytes().to_vec()));
    }
    if let Some(publisher) = &input.publisher {
        entries.push((101, publisher.as_bytes().to_vec()));
    }
    if let Some(description) = &input.description {
        entries.push((103, description.as_bytes().to_vec()));
    }
    entries.push((503, input.title.as_bytes().to_vec()));
    entries.extend(input.extra_exth.iter().cloned());
    let total_len = 12usize
        + entries
            .iter()
            .map(|(_, value)| 8usize.saturating_add(value.len()))
            .sum::<usize>();
    let mut output = Vec::with_capacity(total_len);
    output.extend_from_slice(b"EXTH");
    output.extend_from_slice(&(total_len as u32).to_be_bytes());
    output.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (kind, value) in entries {
        output.extend_from_slice(&kind.to_be_bytes());
        output.extend_from_slice(&(8u32.saturating_add(value.len() as u32)).to_be_bytes());
        output.extend_from_slice(&value);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_mobi_has_a_single_shared_pdb_header() {
        let artifact = build_mobi(&MobiInput {
            title: "Fixture".to_owned(),
            author: Some("Author".to_owned()),
            publisher: None,
            description: None,
            language: Some("en".to_owned()),
            html: b"<p>hello</p>".to_vec(),
            images: Vec::new(),
            compression: Compression::None,
            kf8: false,
            deterministic: true,
            extra_exth: Vec::new(),
            extra_records: Vec::new(),
        })
        .unwrap();
        assert_eq!(&artifact.bytes[..4], b"Fixt");
        assert_eq!(&artifact.bytes[56..60], b"BOOK");
        assert_eq!(&artifact.bytes[60..64], b"MOBI");
    }

    #[test]
    fn kf7_and_kf8_share_mobi_name_and_exth_metadata_decoding() {
        let artifact = build_mobi(&MobiInput {
            title: "Shared title".to_owned(),
            author: Some("Shared author".to_owned()),
            publisher: Some("Shared publisher".to_owned()),
            description: Some("Shared description".to_owned()),
            language: Some("en".to_owned()),
            html: b"<p>metadata fixture</p>".to_vec(),
            images: Vec::new(),
            compression: Compression::None,
            kf8: false,
            deterministic: true,
            extra_exth: Vec::new(),
            extra_records: Vec::new(),
        })
        .unwrap();
        let record_zero = inspect_pdb(&artifact.bytes).unwrap().records.remove(0);

        let kf7 = read_metadata(&record_zero, MobiVariant::Kf7);
        let kf8 = read_metadata(&record_zero, MobiVariant::Kf8);
        assert!(kf7.diagnostics.is_empty());
        assert!(kf8.diagnostics.is_empty());
        assert_eq!(kf7.metadata.title.as_deref(), Some("Shared title"));
        assert_eq!(kf7.metadata.title, kf8.metadata.title);
        assert_eq!(kf7.metadata.authors, kf8.metadata.authors);
        assert_eq!(kf7.metadata.publisher, kf8.metadata.publisher);
        assert_eq!(kf7.metadata.description, kf8.metadata.description);
        assert_eq!(kf7.metadata.authors, vec!["Shared author"]);
        assert_eq!(kf7.metadata.publisher.as_deref(), Some("Shared publisher"));
        assert_eq!(
            kf7.metadata.description.as_deref(),
            Some("Shared description")
        );
    }

    #[test]
    fn malformed_exth_lengths_are_reported_without_panicking() {
        let exth_start = 16 + 232;
        let mut record_zero = vec![0; exth_start + 12];
        put_u32_be(&mut record_zero, 16 + 72, 0x40).unwrap();
        record_zero[exth_start..exth_start + 4].copy_from_slice(b"EXTH");
        put_u32_be(&mut record_zero, exth_start + 4, 12).unwrap();
        put_u32_be(&mut record_zero, exth_start + 8, 1).unwrap();

        for variant in [MobiVariant::Kf7, MobiVariant::Kf8] {
            let report = read_metadata(&record_zero, variant);
            assert_eq!(report.diagnostics.len(), 1);
            assert!(report.diagnostics[0].code.ends_with("0004"));
        }
    }

    #[test]
    fn shared_mobi_image_sniffer_maps_supported_mime_types() {
        assert_eq!(image_kind(&[0xff, 0xd8, 0xff]), ResourceKind::Jpeg);
        assert_eq!(media_type_for_kind(&ResourceKind::Jpeg), "image/jpeg");
        assert_eq!(image_kind(b"\x89PNG\r\n\x1a\n"), ResourceKind::Png);
        assert_eq!(media_type_for_kind(&ResourceKind::Png), "image/png");
        assert_eq!(image_kind(b"GIF89a"), ResourceKind::Gif);
        assert_eq!(image_kind(b"not an image"), ResourceKind::Unknown);
    }

    #[test]
    fn combo_packager_keeps_kf7_and_kf8_artifacts_independent() {
        let make = |kf8| {
            build_mobi(&MobiInput {
                title: "Combo fixture".to_owned(),
                author: None,
                publisher: None,
                description: None,
                language: Some("en".to_owned()),
                html: b"<p>fixture</p>".to_vec(),
                images: Vec::new(),
                compression: Compression::None,
                kf8,
                deterministic: true,
                extra_exth: Vec::new(),
                extra_records: Vec::new(),
            })
            .unwrap()
        };
        let kf7 = make(false);
        let kf8 = make(true);
        let combo = pack_combo(&kf7, &kf8, "Combo fixture", true).unwrap();
        let value = inspect_combo(&combo.bytes).unwrap();
        assert_eq!(value["format"], "KF7+KF8-composite");
        assert_eq!(value["kf7_record_count"], 2);
        assert_eq!(value["kf8_record_count"], 2);
    }

    #[test]
    fn malformed_record_offsets_return_errors() {
        let mut bytes = vec![0u8; 78];
        bytes[76..78].copy_from_slice(&1u16.to_be_bytes());
        assert!(matches!(
            inspect_pdb(&bytes),
            Err(MobiReadError::InvalidRecordTable | MobiReadError::OffsetOutOfBounds)
        ));
    }
}
