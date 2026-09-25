//! KF8 target lowering and writer.

use std::collections::BTreeMap;

use folio_kindle_common::Compression;
use folio_kindle_common::{
    binary::{read_u16_be, read_u32_be},
    compression::{decompress, huffdic_text_budget, CompressionError, HuffDicDecoder},
};
use folio_mobi::{build_mobi, inspect_pdb, MobiError, MobiInput, MobiReadError};
use folio_model::{
    Book, Diagnostic, MemoryResourceLoader, Node, NodeKind, Resource, ResourceId, ResourceKind,
};
use folio_normalize::{
    import_xhtml_documents, normalize_legacy_xhtml, XhtmlDocumentInput, XhtmlError,
};
use folio_style::inline_css;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct Kf8Options {
    pub compression: Compression,
    pub deterministic: bool,
}

/// KF8/AZW3's input-side adapter, kept beside the structural record parser.
pub struct Kf8Adapter;

impl folio_format::FormatAdapter for Kf8Adapter {
    fn format(&self) -> folio_input::DetectedFormat {
        folio_input::DetectedFormat::Kf8
    }

    fn support(&self) -> folio_format::FormatSupport {
        folio_format::FormatSupport {
            detect: true,
            inspect: true,
            import: true,
            metadata_read: true,
            metadata_write: true,
            edit: true,
            preview: true,
            ..folio_format::FormatSupport::default()
        }
    }

    fn detect(
        &self,
        source: &folio_input::BookSource,
    ) -> Result<folio_format::DetectionResult, folio_format::FormatError> {
        folio_format::supports_detected(source, [folio_input::DetectedFormat::Kf8])
    }

    fn import(
        &self,
        source: &folio_input::BookSource,
        _context: &folio_format::ImportContext,
    ) -> Result<folio_format::ImportedBook, folio_format::FormatError> {
        let path = folio_format::primary_file(source)?;
        let bytes = std::fs::read(path)?;
        let read = import(&bytes)
            .map_err(|error| folio_format::FormatError::Invalid(error.to_string()))?;
        Ok(folio_format::ImportedBook {
            format: folio_input::DetectedFormat::Kf8,
            parser: "folio-kf8/1".to_owned(),
            book: read.book,
            diagnostics: read.diagnostics,
            input_loss: Vec::new(),
            text: None,
        })
    }
}

impl Default for Kf8Options {
    fn default() -> Self {
        Self {
            compression: Compression::None,
            deterministic: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Flow {
    pub id: u32,
    pub name: String,
    pub fragment_ids: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Fragment {
    pub id: u32,
    pub document: u32,
    pub html: String,
    pub style_ids: Vec<u32>,
    pub resource_ids: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IndexEntry {
    pub label: String,
    pub href: String,
    pub fragment_id: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Kf8Artifact {
    pub bytes: Vec<u8>,
    pub flows: Vec<Flow>,
    pub fragments: Vec<Fragment>,
    pub indexes: Vec<IndexEntry>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StructuralRecord {
    pub tag: String,
    pub payload: serde_json::Value,
    pub payload_length: usize,
}

#[derive(Debug, Error)]
pub enum Kf8Error {
    #[error("failed to build KF8 MOBI container: {0}")]
    Mobi(#[from] MobiError),
    #[error("KF8 structural record encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("KF8/PDB inspection failed: {0}")]
    Read(#[from] MobiReadError),
    #[error("KF8 structural record payload is too large")]
    StructuralRecordTooLarge,
    #[error("KF8 structural record is missing: {0}")]
    MissingStructuralRecord(String),
    #[error("KF8 decompression failed: {0}")]
    Compression(#[from] CompressionError),
    #[error("KF8 input is invalid: {0}")]
    Invalid(String),
    #[error("KF8 XHTML import failed: {0}")]
    Xhtml(#[from] XhtmlError),
}

#[derive(Clone, Debug)]
pub struct Kf8ImportReport {
    pub book: Book,
    pub diagnostics: Vec<Diagnostic>,
}

/// Import a KF8/AZW3 artifact through the shared semantic XHTML normalizer.
/// The structural records are used when they are FolioForge records; unknown
/// records remain opaque and never become public IR fields.
pub fn import(bytes: &[u8]) -> Result<Kf8ImportReport, Kf8Error> {
    let inspection = inspect_pdb(bytes)?;
    let record_zero = inspection
        .records
        .first()
        .ok_or_else(|| Kf8Error::Invalid("PDB has no record zero".to_owned()))?;
    if record_zero.len() < 16 + 232 || &record_zero[16..20] != b"MOBI" {
        return Err(Kf8Error::Invalid(
            "record zero is not a MOBI header".to_owned(),
        ));
    }
    let compression =
        match read_u16_be(record_zero, 0).map_err(|error| Kf8Error::Invalid(error.to_string()))? {
            1 => Compression::None,
            2 => Compression::PalmDoc,
            17_480 => Compression::HuffDic,
            value => {
                return Err(Kf8Error::Invalid(format!(
                    "unsupported compression {value}"
                )))
            }
        };
    let text_length =
        read_u32_be(record_zero, 4).map_err(|error| Kf8Error::Invalid(error.to_string()))? as usize;
    let text_record_count =
        read_u16_be(record_zero, 8).map_err(|error| Kf8Error::Invalid(error.to_string()))? as usize;
    let standard_kf8 = is_standard_kf8(record_zero, &inspection.records);
    let text_end = 1usize
        .checked_add(text_record_count)
        .ok_or_else(|| Kf8Error::Invalid("text record count overflowed".to_owned()))?;
    if text_end > inspection.records.len() {
        return Err(Kf8Error::Invalid(
            "text records exceed PDB record table".to_owned(),
        ));
    }
    let mut huff_decoder = if compression == Compression::HuffDic {
        // HUFF/CDIC indices are record-zero offsets in the standard AZW3
        // layout.  They are independent of the text-record range.
        let huff_index = read_u32_be(record_zero, 0x70)
            .map_err(|error| Kf8Error::Invalid(error.to_string()))?
            as usize;
        let huff_count = read_u32_be(record_zero, 0x74)
            .map_err(|error| Kf8Error::Invalid(error.to_string()))?
            as usize;
        if huff_index == u32::MAX as usize || huff_count < 2 {
            return Err(Kf8Error::Compression(CompressionError::HuffDicUnavailable));
        }
        let huff_end = huff_index
            .checked_add(huff_count)
            .ok_or_else(|| Kf8Error::Invalid("HUFF/CDIC record range overflowed".to_owned()))?;
        if huff_end > inspection.records.len() {
            return Err(Kf8Error::Invalid(
                "HUFF/CDIC records exceed the PDB record table".to_owned(),
            ));
        }
        let huff = &inspection.records[huff_index];
        let cdics = inspection.records[huff_index + 1..huff_end]
            .iter()
            .map(Vec::as_slice)
            .collect::<Vec<_>>();
        Some(HuffDicDecoder::new(huff, &cdics)?)
    } else {
        None
    };
    let mut huff_budget = huffdic_text_budget(bytes.len());
    let extra_data_flags = read_u16_be(record_zero, 0xf2).unwrap_or(0);
    let mut html_bytes = Vec::new();
    for record in &inspection.records[1..text_end] {
        let record = strip_kindle_extra_data(record, extra_data_flags);
        let decompressed = if let Some(decoder) = huff_decoder.as_mut() {
            decoder.decompress(record, &mut huff_budget)?
        } else {
            decompress(record, compression)?
        };
        html_bytes.extend_from_slice(&decompressed);
        if html_bytes.len() > 256 << 20 {
            return Err(Kf8Error::Invalid(
                "decompressed text exceeds 256 MiB".to_owned(),
            ));
        }
    }
    if !standard_kf8 {
        html_bytes.truncate(text_length.min(html_bytes.len()));
    }
    let metadata_report = folio_mobi::read_metadata(record_zero, folio_mobi::MobiVariant::Kf8);
    let mut metadata = metadata_report.metadata;
    let mut diagnostics = metadata_report.diagnostics;
    let cover_image_index = folio_mobi::read_cover_image_index(record_zero);
    let structural = structural_records(&inspection.records[text_end..])?;

    let mut resources = structural
        .iter()
        .find(|record| record.tag == "RESC")
        .and_then(|record| serde_json::from_slice::<Vec<Resource>>(&record.payload).ok())
        .unwrap_or_default();
    let mut loader = MemoryResourceLoader::default();
    let mut image_paths = BTreeMap::new();
    // Amazon AZW3 stores this index at record-zero offset 0x6c.  Keep the
    // legacy FolioForge writer offset as a fallback for artifacts produced by
    // older local builds.
    let standard_first_image_index =
        read_u32_be(record_zero, 0x6c).map_err(|error| Kf8Error::Invalid(error.to_string()))?;
    let legacy_first_image_index =
        read_u32_be(record_zero, 16 + 52).map_err(|error| Kf8Error::Invalid(error.to_string()))?;
    let first_image_index = if standard_first_image_index != u32::MAX
        && (standard_first_image_index as usize) < inspection.records.len()
    {
        standard_first_image_index
    } else {
        legacy_first_image_index
    };
    if first_image_index != u32::MAX {
        let start = usize::try_from(first_image_index)
            .map_err(|_| Kf8Error::Invalid("image index overflows usize".to_owned()))?;
        let mut image_index = 0usize;
        for (record_index, record) in inspection.records.iter().enumerate().skip(start) {
            if is_structural_tag(record.get(..4)) {
                break;
            }
            let kind = folio_mobi::image_kind(record);
            if !matches!(
                kind,
                ResourceKind::Jpeg | ResourceKind::Png | ResourceKind::Gif
            ) {
                continue;
            }
            let resource = resources
                .iter_mut()
                .filter(|resource| {
                    matches!(
                        resource.kind,
                        ResourceKind::Jpeg | ResourceKind::Png | ResourceKind::Gif
                    )
                })
                .nth(image_index);
            let (id, path) = if let Some(resource) = resource {
                if cover_image_index == Some(image_index as u32)
                    && !resource
                        .properties
                        .iter()
                        .any(|property| property == "cover-image")
                {
                    resource.properties.push("cover-image".to_owned());
                }
                (resource.id, resource.path.clone())
            } else {
                let id = ResourceId::new(resources.len() as u32);
                let extension = match kind {
                    ResourceKind::Jpeg => "jpg",
                    ResourceKind::Png => "png",
                    ResourceKind::Gif => "gif",
                    _ => "bin",
                };
                let path = if standard_kf8 {
                    format!("images/image_{:04}.{}", image_index, extension)
                } else {
                    format!("images/{:04}.{}", image_index, extension)
                };
                let properties = if cover_image_index == Some(image_index as u32) {
                    vec!["cover-image".to_owned()]
                } else {
                    Vec::new()
                };
                resources.push(Resource {
                    id,
                    path: path.clone(),
                    media_type: folio_mobi::media_type_for_kind(&kind).to_owned(),
                    kind,
                    properties,
                    size: Some(record.len() as u64),
                });
                (id, path)
            };
            let _ = id;
            image_paths.insert(record_index, path.clone());
            loader.insert(path, record.clone());
            image_index += 1;
        }
    }
    let standard_documents = if standard_kf8 {
        Some(parse_standard_kf8(
            &inspection.records,
            record_zero,
            &html_bytes,
            &mut resources,
            &mut loader,
        )?)
    } else {
        None
    };
    let html = normalize_legacy_xhtml(&String::from_utf8_lossy(&html_bytes), &image_paths);

    let documents = standard_documents
        .as_ref()
        .map(|parsed| parsed.documents.clone())
        .or_else(|| {
            structural
                .iter()
                .find(|record| record.tag == "FRAG")
                .and_then(|record| serde_json::from_slice::<Vec<Fragment>>(&record.payload).ok())
                .map(|fragments| {
                    fragments
                        .into_iter()
                        .map(|fragment| XhtmlDocumentInput {
                            href: format!("document-{}.xhtml", fragment.document),
                            media_type: "application/xhtml+xml".to_owned(),
                            content: normalize_legacy_xhtml(&fragment.html, &image_paths),
                        })
                        .collect::<Vec<_>>()
                })
                .filter(|documents| !documents.is_empty())
                .or_else(|| {
                    Some(vec![XhtmlDocumentInput {
                        href: "book.xhtml".to_owned(),
                        media_type: "application/xhtml+xml".to_owned(),
                        content: html.clone(),
                    }])
                })
        })
        .unwrap_or_default();
    if metadata.title.is_none() {
        metadata.title = Some("Imported KF8 book".to_owned());
    }
    let mut report =
        import_xhtml_documents(metadata, &documents, resources, std::sync::Arc::new(loader))?;
    if let Some(indexes) = structural
        .iter()
        .find(|record| record.tag == "INDX")
        .and_then(|record| serde_json::from_slice::<Vec<IndexEntry>>(&record.payload).ok())
    {
        report.book.navigation.toc = indexes
            .into_iter()
            .map(|index| folio_model::NavPoint {
                label: index.label,
                href: index.href,
                children: Vec::new(),
            })
            .collect();
    }
    if let Some(parsed) = standard_documents {
        report.book.navigation.toc = parsed.toc;
    }
    diagnostics.extend(report.diagnostics);
    Ok(Kf8ImportReport {
        book: report.book,
        diagnostics,
    })
}

#[derive(Clone, Debug)]
struct RawStructuralRecord {
    tag: String,
    payload: Vec<u8>,
}

#[derive(Clone, Debug)]
struct StandardKf8Import {
    documents: Vec<XhtmlDocumentInput>,
    toc: Vec<folio_model::NavPoint>,
}

#[derive(Clone, Debug)]
struct Kf8SkeletonEntry {
    file_number: usize,
    div_count: usize,
    start: usize,
    length: usize,
}

#[derive(Clone, Debug)]
struct Kf8DivEntry {
    insert_pos: usize,
    file_number: usize,
    length: usize,
}

#[derive(Clone, Debug)]
struct Kf8TocEntry {
    label: String,
    href: String,
    parent: i32,
}

#[derive(Clone, Copy, Debug)]
struct Kf8IndexHeader {
    data_record_count: usize,
    cncx_count: usize,
    idxt_start: usize,
    tagx_start: usize,
}

#[derive(Clone, Copy, Debug)]
struct Kf8TagDefinition {
    tag: u8,
    value_count: u8,
    bitmask: u8,
    control_boundary: bool,
}

#[derive(Clone, Debug)]
struct Kf8IndexEntry {
    name: String,
    values: BTreeMap<u8, Vec<u32>>,
}

impl Kf8IndexEntry {
    fn values(&self, tag: u8) -> &[u32] {
        self.values.get(&tag).map(Vec::as_slice).unwrap_or(&[])
    }
}

fn is_standard_kf8(record_zero: &[u8], records: &[Vec<u8>]) -> bool {
    let mobi_version = read_u32_be(record_zero, 0x68).ok();
    let Some(mobi_version) = mobi_version else {
        return false;
    };
    if mobi_version != 8 {
        return false;
    }
    let Some(fdst_index) = read_u32_be(record_zero, 0xc0).ok() else {
        return false;
    };
    let Some(div_index) = read_u32_be(record_zero, 0xf8).ok() else {
        return false;
    };
    let Some(skel_index) = read_u32_be(record_zero, 0xfc).ok() else {
        return false;
    };
    records
        .get(fdst_index as usize)
        .is_some_and(|record| record.starts_with(b"FDST"))
        && records
            .get(div_index as usize)
            .is_some_and(|record| record.starts_with(b"INDX"))
        && records
            .get(skel_index as usize)
            .is_some_and(|record| record.starts_with(b"INDX"))
}

fn strip_kindle_extra_data(record: &[u8], flags: u16) -> &[u8] {
    if flags == 0 || record.is_empty() {
        return record;
    }
    let mut end = record.len();
    let mut remaining_flags = flags >> 1;
    while remaining_flags != 0 {
        if remaining_flags & 1 != 0 && end != 0 {
            let mut size = 0usize;
            let mut shift = 0usize;
            let mut cursor = end;
            while cursor != 0 {
                cursor -= 1;
                let byte = record[cursor];
                size |= usize::from(byte & 0x7f) << shift;
                shift += 7;
                if byte & 0x80 != 0 || shift >= usize::BITS as usize {
                    break;
                }
            }
            if size != 0 && size <= end {
                end -= size;
            }
        }
        remaining_flags >>= 1;
    }
    if flags & 1 != 0 && end != 0 {
        let overlap = usize::from(record[end - 1] & 3) + 1;
        if overlap <= end {
            end -= overlap;
        }
    }
    &record[..end]
}

fn parse_standard_kf8(
    records: &[Vec<u8>],
    record_zero: &[u8],
    text: &[u8],
    resources: &mut Vec<Resource>,
    loader: &mut MemoryResourceLoader,
) -> Result<StandardKf8Import, Kf8Error> {
    let fdst_index = read_u32_be(record_zero, 0xc0)
        .map_err(|error| Kf8Error::Invalid(error.to_string()))? as usize;
    let div_index = read_u32_be(record_zero, 0xf8)
        .map_err(|error| Kf8Error::Invalid(error.to_string()))? as usize;
    let skel_index = read_u32_be(record_zero, 0xfc)
        .map_err(|error| Kf8Error::Invalid(error.to_string()))? as usize;
    let ncx_index = read_u32_be(record_zero, 0xf4)
        .map_err(|error| Kf8Error::Invalid(error.to_string()))? as usize;

    let flows = parse_fdst_record(
        records
            .get(fdst_index)
            .ok_or_else(|| Kf8Error::Invalid("KF8 FDST record is outside the PDB".to_owned()))?,
    )?;
    if flows.is_empty() {
        return Err(Kf8Error::Invalid(
            "KF8 FDST record contains no flows".to_owned(),
        ));
    }
    let html_flow = bounded_range(text, flows[0].0, flows[0].1);
    let skel_entries = read_standard_index(records, skel_index)?;
    let div_entries = read_standard_index(records, div_index)?;
    let skeletons = skel_entries
        .iter()
        .enumerate()
        .map(|(file_number, entry)| Kf8SkeletonEntry {
            file_number,
            div_count: entry.values(1).first().copied().unwrap_or(0) as usize,
            start: entry.values(6).first().copied().unwrap_or(0) as usize,
            length: entry.values(6).get(1).copied().unwrap_or(0) as usize,
        })
        .collect::<Vec<_>>();
    let divisions = div_entries
        .iter()
        .map(|entry| Kf8DivEntry {
            insert_pos: entry.name.parse::<usize>().unwrap_or(0),
            file_number: entry.values(3).first().copied().unwrap_or(0) as usize,
            length: entry.values(6).get(1).copied().unwrap_or(0) as usize,
        })
        .collect::<Vec<_>>();

    let mut documents = Vec::with_capacity(skeletons.len());
    let mut division_index = 0usize;
    for skeleton in &skeletons {
        let skeleton_end = skeleton.start.saturating_add(skeleton.length);
        let Some(skeleton_bytes) = html_flow.get(skeleton.start..skeleton_end) else {
            continue;
        };
        let mut document = skeleton_bytes.to_vec();
        let mut content_cursor = skeleton_end;
        for _ in 0..skeleton.div_count {
            let Some(division) = divisions.get(division_index) else {
                break;
            };
            division_index += 1;
            let content_end = content_cursor.saturating_add(division.length);
            let Some(fragment) = html_flow.get(content_cursor..content_end) else {
                content_cursor = content_end;
                continue;
            };
            let insert_pos = division.insert_pos.saturating_sub(skeleton.start);
            if insert_pos <= document.len() {
                let mut rebuilt = Vec::with_capacity(document.len() + fragment.len());
                rebuilt.extend_from_slice(&document[..insert_pos]);
                rebuilt.extend_from_slice(fragment);
                rebuilt.extend_from_slice(&document[insert_pos..]);
                document = rebuilt;
            }
            content_cursor = content_end;
        }
        let content = normalize_legacy_xhtml(
            &normalize_standard_kf8_markup(&String::from_utf8_lossy(&document), &divisions),
            &BTreeMap::new(),
        );
        documents.push(XhtmlDocumentInput {
            href: format!("part{:04}.xhtml", skeleton.file_number),
            media_type: "application/xhtml+xml".to_owned(),
            content,
        });
    }
    if documents.is_empty() {
        return Err(Kf8Error::Invalid(
            "KF8 skeleton index produced no documents".to_owned(),
        ));
    }

    // CSS is stored in later FDST flows, not as ordinary PDB resources.  Add
    // it to the shared resource table and inline it into each XHTML head so
    // the common style resolver sees the same declarations as a Kindle/KFX
    // renderer without inventing a new IR-specific stylesheet channel.
    let mut styles = Vec::new();
    for (flow_index, &(start, end)) in flows.iter().enumerate().skip(1) {
        let css = normalize_standard_kf8_markup(
            &String::from_utf8_lossy(bounded_range(text, start, end)),
            &divisions,
        )
        .into_bytes();
        let path = format!("styles/style{:04}.css", flow_index - 1);
        let id = ResourceId::new(resources.len() as u32);
        resources.push(Resource {
            id,
            path: path.clone(),
            media_type: "text/css".to_owned(),
            kind: ResourceKind::Stylesheet,
            properties: Vec::new(),
            size: Some(css.len() as u64),
        });
        loader.insert(path, css.clone());
        styles.push(String::from_utf8_lossy(&css).into_owned());
    }
    if !styles.is_empty() {
        for document in &mut documents {
            document.content = inject_kf8_styles(&document.content, &styles);
        }
    }

    let toc = if ncx_index != u32::MAX as usize && ncx_index < records.len() {
        let entries = read_standard_index(records, ncx_index)?;
        let header = parse_index_header(&records[ncx_index])?;
        let strings = parse_cncx_strings(
            records,
            ncx_index,
            header.data_record_count,
            header.cncx_count,
        );
        build_standard_toc(&entries, &divisions, &strings)
    } else {
        Vec::new()
    };
    Ok(StandardKf8Import { documents, toc })
}

fn parse_fdst_record(record: &[u8]) -> Result<Vec<(usize, usize)>, Kf8Error> {
    if record.len() < 12 || &record[..4] != b"FDST" {
        return Err(Kf8Error::Invalid("KF8 FDST record is invalid".to_owned()));
    }
    let table_start =
        read_u32_be(record, 4).map_err(|error| Kf8Error::Invalid(error.to_string()))? as usize;
    let count =
        read_u32_be(record, 8).map_err(|error| Kf8Error::Invalid(error.to_string()))? as usize;
    let mut flows = Vec::new();
    for index in 0..count {
        let start = table_start
            .checked_add(index.saturating_mul(8))
            .ok_or_else(|| Kf8Error::Invalid("KF8 FDST offset overflowed".to_owned()))?;
        let end = start
            .checked_add(8)
            .ok_or_else(|| Kf8Error::Invalid("KF8 FDST entry overflowed".to_owned()))?;
        if end > record.len() {
            break;
        }
        let flow_start = read_u32_be(record, start)
            .map_err(|error| Kf8Error::Invalid(error.to_string()))?
            as usize;
        let flow_end = read_u32_be(record, start + 4)
            .map_err(|error| Kf8Error::Invalid(error.to_string()))? as usize;
        flows.push((flow_start, flow_end));
    }
    Ok(flows)
}

fn read_standard_index(
    records: &[Vec<u8>],
    index_record: usize,
) -> Result<Vec<Kf8IndexEntry>, Kf8Error> {
    let header_record = records.get(index_record).ok_or_else(|| {
        Kf8Error::Invalid(format!(
            "KF8 index record {index_record} is outside the PDB"
        ))
    })?;
    let header = parse_index_header(header_record)?;
    let tagx_start = header.tagx_start;
    let tagx_end = header_record.len();
    if tagx_start >= tagx_end
        || !header_record
            .get(tagx_start..)
            .is_some_and(|value| value.starts_with(b"TAGX"))
    {
        return Err(Kf8Error::Invalid(format!(
            "KF8 index record {index_record} has no TAGX section"
        )));
    }
    let first_entry_offset = read_u32_be(header_record, tagx_start + 4)
        .map_err(|error| Kf8Error::Invalid(error.to_string()))?
        as usize;
    let control_byte_count = read_u32_be(header_record, tagx_start + 8)
        .map_err(|error| Kf8Error::Invalid(error.to_string()))?
        as usize;
    let tag_definition_end = tagx_start
        .checked_add(first_entry_offset)
        .ok_or_else(|| Kf8Error::Invalid("KF8 TAGX offset overflowed".to_owned()))?;
    if tag_definition_end > tagx_end || first_entry_offset < 12 {
        return Err(Kf8Error::Invalid(
            "KF8 TAGX definitions are truncated".to_owned(),
        ));
    }
    let mut definitions = Vec::new();
    let mut cursor = tagx_start + 12;
    while cursor + 4 <= tag_definition_end {
        definitions.push(Kf8TagDefinition {
            tag: header_record[cursor],
            value_count: header_record[cursor + 1],
            bitmask: header_record[cursor + 2],
            control_boundary: header_record[cursor + 3] == 1,
        });
        cursor += 4;
    }

    let mut entries = Vec::new();
    for data_index in 0..header.data_record_count {
        let record_index = index_record
            .checked_add(1 + data_index)
            .ok_or_else(|| Kf8Error::Invalid("KF8 index record range overflowed".to_owned()))?;
        let record = records.get(record_index).ok_or_else(|| {
            Kf8Error::Invalid(format!("KF8 index data record {record_index} is missing"))
        })?;
        if record.len() < 28 || &record[..4] != b"INDX" {
            continue;
        }
        let idxt_start =
            read_u32_be(record, 20).map_err(|error| Kf8Error::Invalid(error.to_string()))? as usize;
        let entry_count =
            read_u32_be(record, 24).map_err(|error| Kf8Error::Invalid(error.to_string()))? as usize;
        let idxt_end = idxt_start
            .checked_add(4 + entry_count.saturating_mul(2))
            .ok_or_else(|| Kf8Error::Invalid("KF8 IDXT offset overflowed".to_owned()))?;
        if idxt_end > record.len() || &record[idxt_start..idxt_start + 4] != b"IDXT" {
            continue;
        }
        let mut positions = Vec::with_capacity(entry_count + 1);
        for entry_index in 0..entry_count {
            let offset = idxt_start + 4 + entry_index * 2;
            positions
                .push(u16::from_be_bytes(record[offset..offset + 2].try_into().unwrap()) as usize);
        }
        positions.push(idxt_start);
        for entry_index in 0..entry_count {
            let start = positions[entry_index];
            let end = positions[entry_index + 1];
            if start >= end || end > record.len() {
                continue;
            }
            let entry = &record[start..end];
            let (name, name_bytes) = decode_index_name(entry)?;
            let tag_values =
                decode_index_tags(&entry[name_bytes..], control_byte_count, &definitions)?;
            entries.push(Kf8IndexEntry {
                name,
                values: tag_values,
            });
        }
    }
    // The parsed header carries the CNCX count so the caller can request the
    // text labels separately.  Keeping this parser focused on entry fields
    // makes malformed optional string tables non-fatal for skeleton/div data.
    let _ = header.cncx_count;
    let _ = header.idxt_start;
    Ok(entries)
}

fn parse_index_header(record: &[u8]) -> Result<Kf8IndexHeader, Kf8Error> {
    if record.len() < 192 || &record[..4] != b"INDX" {
        return Err(Kf8Error::Invalid("KF8 INDX header is invalid".to_owned()));
    }
    Ok(Kf8IndexHeader {
        data_record_count: read_u32_be(record, 24)
            .map_err(|error| Kf8Error::Invalid(error.to_string()))?
            as usize,
        cncx_count: read_u32_be(record, 52).map_err(|error| Kf8Error::Invalid(error.to_string()))?
            as usize,
        idxt_start: read_u32_be(record, 20).map_err(|error| Kf8Error::Invalid(error.to_string()))?
            as usize,
        tagx_start: read_u32_be(record, 180)
            .map_err(|error| Kf8Error::Invalid(error.to_string()))? as usize,
    })
}

fn decode_index_name(entry: &[u8]) -> Result<(String, usize), Kf8Error> {
    let length = *entry
        .first()
        .ok_or_else(|| Kf8Error::Invalid("KF8 index entry has no name".to_owned()))?
        as usize;
    let end = 1usize
        .checked_add(length)
        .ok_or_else(|| Kf8Error::Invalid("KF8 index name length overflowed".to_owned()))?;
    let name_bytes = entry
        .get(1..end)
        .ok_or_else(|| Kf8Error::Invalid("KF8 index name is truncated".to_owned()))?;
    Ok((String::from_utf8_lossy(name_bytes).into_owned(), end))
}

fn decode_index_tags(
    data: &[u8],
    control_byte_count: usize,
    definitions: &[Kf8TagDefinition],
) -> Result<BTreeMap<u8, Vec<u32>>, Kf8Error> {
    let controls = data.get(..control_byte_count).ok_or_else(|| {
        Kf8Error::Invalid("KF8 index entry control bytes are truncated".to_owned())
    })?;
    let mut cursor = control_byte_count;
    let mut control_index = 0usize;
    let mut pending = Vec::new();
    for definition in definitions {
        if definition.control_boundary {
            control_index += 1;
            continue;
        }
        let control = controls.get(control_index).copied().unwrap_or(0);
        let value = control & definition.bitmask;
        if value == 0 {
            continue;
        }
        let (count, byte_length) =
            if value == definition.bitmask && definition.bitmask.count_ones() > 1 {
                let (length, consumed) = decode_index_varint(data.get(cursor..).unwrap_or(&[]))?;
                cursor = cursor.saturating_add(consumed);
                (None, Some(length as usize))
            } else if value == definition.bitmask {
                (Some(1u32), None)
            } else {
                let shift = definition.bitmask.trailing_zeros();
                (Some(u32::from(value >> shift)), None)
            };
        pending.push((definition.tag, definition.value_count, count, byte_length));
    }
    let mut output = BTreeMap::new();
    for (tag, values_per_entry, count, byte_length) in pending {
        let mut values = Vec::new();
        if let Some(count) = count {
            let total = count.saturating_mul(u32::from(values_per_entry));
            for _ in 0..total {
                let (value, consumed) = decode_index_varint(data.get(cursor..).unwrap_or(&[]))?;
                cursor = cursor.saturating_add(consumed);
                values.push(value);
            }
        } else if let Some(byte_length) = byte_length {
            let start = cursor;
            let end = start.saturating_add(byte_length).min(data.len());
            while cursor < end {
                let (value, consumed) = decode_index_varint(&data[cursor..end])?;
                cursor = cursor.saturating_add(consumed);
                values.push(value);
            }
            if end != start.saturating_add(byte_length) {
                return Err(Kf8Error::Invalid(
                    "KF8 variable tag data is truncated".to_owned(),
                ));
            }
        }
        output.insert(tag, values);
    }
    Ok(output)
}

fn decode_index_varint(data: &[u8]) -> Result<(u32, usize), Kf8Error> {
    let mut value = 0u32;
    for (index, byte) in data.iter().copied().enumerate() {
        value = value
            .checked_shl(7)
            .and_then(|value| value.checked_add(u32::from(byte & 0x7f)))
            .ok_or_else(|| Kf8Error::Invalid("KF8 index varint overflowed".to_owned()))?;
        if byte & 0x80 != 0 {
            return Ok((value, index + 1));
        }
        if index >= 4 {
            return Err(Kf8Error::Invalid("KF8 index varint is too long".to_owned()));
        }
    }
    Err(Kf8Error::Invalid(
        "KF8 index varint is truncated".to_owned(),
    ))
}

fn parse_cncx_strings(
    records: &[Vec<u8>],
    index_record: usize,
    data_record_count: usize,
    cncx_count: usize,
) -> BTreeMap<u32, String> {
    let start = index_record
        .checked_add(data_record_count + 1)
        .unwrap_or(records.len());
    let mut strings = BTreeMap::new();
    for record_number in 0..cncx_count {
        let Some(record) = records.get(start + record_number) else {
            break;
        };
        let base = (record_number as u32).saturating_mul(0x1_0000);
        let mut cursor = 0usize;
        while cursor < record.len() {
            let Ok((length, consumed)) = decode_index_varint(&record[cursor..]) else {
                break;
            };
            let start_text = cursor.saturating_add(consumed);
            let end_text = start_text.saturating_add(length as usize);
            if end_text > record.len() {
                break;
            }
            strings.insert(
                base.saturating_add(cursor as u32),
                String::from_utf8_lossy(&record[start_text..end_text]).into_owned(),
            );
            cursor = end_text;
        }
    }
    strings
}

fn bounded_range(data: &[u8], start: usize, end: usize) -> &[u8] {
    let start = start.min(data.len());
    let end = end.min(data.len());
    if start <= end {
        &data[start..end]
    } else {
        &[]
    }
}

fn normalize_standard_kf8_markup(value: &str, divisions: &[Kf8DivEntry]) -> String {
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0usize;
    while let Some(relative) = value[cursor..].find("kindle:") {
        let start = cursor + relative;
        output.push_str(&value[cursor..start]);
        let tail = &value[start + "kindle:".len()..];
        let end = tail
            .find(|character: char| ['"', '\'', ')', ' ', '>'].contains(&character))
            .unwrap_or(tail.len());
        let reference = &tail[..end];
        if let Some(replacement) = rewrite_standard_kindle_reference(reference, divisions) {
            output.push_str(&replacement);
        } else {
            output.push_str("kindle:");
            output.push_str(reference);
        }
        cursor = start + "kindle:".len() + end;
    }
    output.push_str(&value[cursor..]);
    output
}

fn rewrite_standard_kindle_reference(reference: &str, divisions: &[Kf8DivEntry]) -> Option<String> {
    if let Some(flow) = reference.strip_prefix("flow:") {
        let flow = flow.split('?').next().unwrap_or(flow);
        let flow_number = parse_kindle_base32(flow);
        return Some(format!(
            "styles/style{:04}.css",
            flow_number.saturating_sub(1)
        ));
    }
    if let Some(embed) = reference.strip_prefix("embed:") {
        let (number, query) = embed.split_once('?').unwrap_or((embed, ""));
        let image_number = parse_kindle_base32(number).saturating_sub(1);
        let extension = if query.contains("image/png") {
            "png"
        } else if query.contains("image/gif") {
            "gif"
        } else {
            "jpg"
        };
        return Some(format!("images/image_{:04}.{}", image_number, extension));
    }
    let position = reference.strip_prefix("pos:fid:")?;
    let (file_id, offset) = position.split_once(":off:").unwrap_or((position, "0"));
    let division_index = parse_kindle_base32(file_id);
    let offset = parse_kindle_base32(offset);
    let division = divisions.get(division_index)?;
    let _target_position = division.insert_pos.saturating_add(offset);
    Some(format!("part{:04}.xhtml", division.file_number))
}

fn parse_kindle_base32(value: &str) -> usize {
    value.chars().fold(0usize, |accumulator, character| {
        let digit = match character {
            '0'..='9' => character as usize - '0' as usize,
            'A'..='V' => character as usize - 'A' as usize + 10,
            'a'..='v' => character as usize - 'a' as usize + 10,
            _ => 0,
        };
        accumulator.saturating_mul(32).saturating_add(digit)
    })
}

fn inject_kf8_styles(content: &str, styles: &[String]) -> String {
    let style_block = styles
        .iter()
        .map(|style| {
            format!(
                "<style type=\"text/css\">{}</style>",
                escape_xml_text(style)
            )
        })
        .collect::<String>();
    let lower = content.to_ascii_lowercase();
    if let Some(head_end) = lower.find("</head>") {
        let mut output = String::with_capacity(content.len() + style_block.len());
        output.push_str(&content[..head_end]);
        output.push_str(&style_block);
        output.push_str(&content[head_end..]);
        output
    } else {
        content.to_owned()
    }
}

fn escape_xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn build_standard_toc(
    entries: &[Kf8IndexEntry],
    divisions: &[Kf8DivEntry],
    strings: &BTreeMap<u32, String>,
) -> Vec<folio_model::NavPoint> {
    let items = entries
        .iter()
        .map(|entry| {
            let label = entry
                .values(3)
                .first()
                .and_then(|offset| strings.get(offset))
                .filter(|value| !value.is_empty())
                .cloned()
                .unwrap_or_else(|| entry.name.clone());
            let file_number = entry
                .values(6)
                .first()
                .and_then(|index| divisions.get(*index as usize))
                .map(|division| division.file_number)
                .unwrap_or(0);
            Kf8TocEntry {
                label,
                href: format!("part{:04}.xhtml", file_number),
                parent: entry
                    .values(21)
                    .first()
                    .copied()
                    .map(|value| value as i32)
                    .unwrap_or(-1),
            }
        })
        .collect::<Vec<_>>();
    let mut active = BTreeMap::new();
    build_toc_children(&items, -1, &mut active, 0)
}

fn build_toc_children(
    items: &[Kf8TocEntry],
    parent: i32,
    active: &mut BTreeMap<usize, bool>,
    depth: usize,
) -> Vec<folio_model::NavPoint> {
    if depth > items.len() {
        return Vec::new();
    }
    let mut output = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if active.get(&index).copied().unwrap_or(false) || item.parent != parent {
            continue;
        }
        active.insert(index, true);
        output.push(folio_model::NavPoint {
            label: item.label.clone(),
            href: item.href.clone(),
            children: build_toc_children(items, index as i32, active, depth + 1),
        });
    }
    if parent == -1 {
        for (index, item) in items.iter().enumerate() {
            if active.get(&index).copied().unwrap_or(false) {
                continue;
            }
            active.insert(index, true);
            output.push(folio_model::NavPoint {
                label: item.label.clone(),
                href: item.href.clone(),
                children: build_toc_children(items, index as i32, active, depth + 1),
            });
        }
    }
    output
}

fn structural_records(records: &[Vec<u8>]) -> Result<Vec<RawStructuralRecord>, Kf8Error> {
    let mut result = Vec::new();
    for record in records {
        if !is_structural_tag(record.get(..4)) {
            continue;
        }
        let tag = String::from_utf8_lossy(&record[..4]).to_string();
        let length = record
            .get(4..8)
            .map(|value| u32::from_be_bytes([value[0], value[1], value[2], value[3]]) as usize)
            .ok_or_else(|| Kf8Error::Invalid("truncated structural record".to_owned()))?;
        let end = 8usize
            .checked_add(length)
            .ok_or(Kf8Error::StructuralRecordTooLarge)?;
        let payload = record
            .get(8..end)
            .ok_or_else(|| Kf8Error::Invalid("structural record exceeds record size".to_owned()))?
            .to_vec();
        result.push(RawStructuralRecord { tag, payload });
    }
    Ok(result)
}

fn is_structural_tag(tag: Option<&[u8]>) -> bool {
    matches!(tag, Some(b"FDST" | b"SKEL" | b"FRAG" | b"INDX" | b"RESC"))
}

pub fn convert(book: &Book, options: &Kf8Options) -> Result<Kf8Artifact, Kf8Error> {
    let mut diagnostics = Vec::new();
    let mut fragments = Vec::new();
    let mut flows = Vec::new();
    let mut html = String::from(
        "<html xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><meta charset=\"utf-8\"><title>",
    );
    html.push_str(&escape_html(book.metadata.display_title()));
    html.push_str("</title></head><body>");

    for (document_index, document) in book.documents.iter().enumerate() {
        let fragment_id = document_index as u32;
        let mut fragment_html = String::new();
        let mut style_ids = Vec::new();
        let mut resource_ids = Vec::new();
        for node in &document.nodes {
            render_node(
                node,
                book,
                &mut fragment_html,
                &mut style_ids,
                &mut resource_ids,
                &mut diagnostics,
            );
        }
        style_ids.sort_unstable();
        style_ids.dedup();
        resource_ids.sort_unstable();
        resource_ids.dedup();
        html.push_str("<div id=\"ff-fragment-");
        html.push_str(&fragment_id.to_string());
        html.push_str("\">");
        html.push_str(&fragment_html);
        html.push_str("</div>");
        fragments.push(Fragment {
            id: fragment_id,
            document: document.id.get(),
            html: fragment_html,
            style_ids,
            resource_ids,
        });
        flows.push(Flow {
            id: document_index as u32,
            name: document.href.clone(),
            fragment_ids: vec![fragment_id],
        });
        if document_index + 1 < book.documents.len() {
            html.push_str("<mbp:pagebreak/>");
        }
    }
    html.push_str("</body></html>");

    let indexes = flatten_navigation(&book.navigation.toc, &fragments);
    let extra_records = vec![
        structural_record("FDST", &serde_json::to_vec(&flows)?)?,
        structural_record(
            "SKEL",
            &serde_json::to_vec(
                &fragments
                    .iter()
                    .map(|fragment| (fragment.id, fragment.document))
                    .collect::<Vec<_>>(),
            )?,
        )?,
        structural_record("FRAG", &serde_json::to_vec(&fragments)?)?,
        structural_record("INDX", &serde_json::to_vec(&indexes)?)?,
        structural_record("RESC", &serde_json::to_vec(&book.resources)?)?,
    ];
    let images = load_images(book, &mut diagnostics);
    let mobi = build_mobi(&MobiInput {
        title: book.metadata.display_title().to_owned(),
        author: book.metadata.author_names().first().cloned(),
        publisher: book.metadata.publisher.clone(),
        description: book.metadata.description.clone(),
        language: book.metadata.language.clone(),
        html: html.into_bytes(),
        images,
        compression: options.compression,
        kf8: true,
        deterministic: options.deterministic,
        extra_exth: vec![(121, b"KF8".to_vec())],
        extra_records,
    })?;
    Ok(Kf8Artifact {
        bytes: mobi.bytes,
        flows,
        fragments,
        indexes,
        diagnostics,
    })
}

impl Kf8Artifact {
    pub fn semantic_value(&self) -> serde_json::Value {
        serde_json::json!({
            "format": "KF8",
            "flows": self.flows,
            "fragments": self.fragments,
            "indexes": self.indexes,
        })
    }
}

/// Inspect target-specific records without requiring a Kindle reader.
/// Unknown records are retained as length-only entries; their payload is not
/// assigned a guessed business meaning.
pub fn semantic_value(bytes: &[u8]) -> Result<serde_json::Value, Kf8Error> {
    let inspection = inspect_pdb(bytes)?;
    let mut records = Vec::new();
    for record in inspection.records.iter().skip(1) {
        let Some(tag_bytes) = record.get(..4) else {
            continue;
        };
        let tag = String::from_utf8_lossy(tag_bytes).to_string();
        if record.len() < 8 {
            records.push(StructuralRecord {
                tag,
                payload: serde_json::Value::Null,
                payload_length: 0,
            });
            continue;
        }
        let payload_length =
            u32::from_be_bytes([record[4], record[5], record[6], record[7]]) as usize;
        let end = match 8usize.checked_add(payload_length) {
            Some(value) => value,
            None => return Err(Kf8Error::StructuralRecordTooLarge),
        };
        let Some(payload_bytes) = record.get(8..end) else {
            continue;
        };
        let payload = match serde_json::from_slice(payload_bytes) {
            Ok(value) => value,
            Err(_) => serde_json::Value::Null,
        };
        records.push(StructuralRecord {
            tag,
            payload,
            payload_length,
        });
    }
    Ok(serde_json::json!({
        "format": "KF8",
        "record_count": inspection.records.len(),
        "structural_records": records,
    }))
}

pub fn validate_bytes(bytes: &[u8]) -> Result<(), Kf8Error> {
    let inspection = inspect_pdb(bytes)?;
    let mut found = std::collections::BTreeSet::new();
    for record in inspection.records.iter().skip(1) {
        if record.len() >= 4 {
            found.insert(String::from_utf8_lossy(&record[..4]).to_string());
        }
    }
    for required in ["FDST", "SKEL", "FRAG", "INDX", "RESC"] {
        if !found.contains(required) {
            return Err(Kf8Error::MissingStructuralRecord(required.to_owned()));
        }
    }
    Ok(())
}

fn structural_record(tag: &str, payload: &[u8]) -> Result<Vec<u8>, Kf8Error> {
    let payload_length =
        u32::try_from(payload.len()).map_err(|_| Kf8Error::StructuralRecordTooLarge)?;
    let mut output = Vec::with_capacity(8 + payload.len());
    let mut tag_bytes = [0u8; 4];
    let bytes = tag.as_bytes();
    tag_bytes[..bytes.len().min(4)].copy_from_slice(&bytes[..bytes.len().min(4)]);
    output.extend_from_slice(&tag_bytes);
    output.extend_from_slice(&payload_length.to_be_bytes());
    output.extend_from_slice(payload);
    Ok(output)
}

fn flatten_navigation(points: &[folio_model::NavPoint], fragments: &[Fragment]) -> Vec<IndexEntry> {
    let mut output = Vec::new();
    for point in points {
        let fragment_id = point.href.split('#').next().and_then(|href| {
            fragments
                .iter()
                .find(|fragment| fragment.html.contains(href))
                .map(|fragment| fragment.id)
        });
        output.push(IndexEntry {
            label: point.label.clone(),
            href: point.href.clone(),
            fragment_id,
        });
        output.extend(flatten_navigation(&point.children, fragments));
    }
    output
}

fn load_images(book: &Book, diagnostics: &mut Vec<Diagnostic>) -> Vec<Vec<u8>> {
    book.resources
        .iter()
        .filter(|resource| {
            matches!(
                resource.kind,
                ResourceKind::Jpeg | ResourceKind::Png | ResourceKind::Gif
            )
        })
        .filter_map(|resource| {
            match book.load_resource(resource.id, resource.size.or(Some(256 << 20))) {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    diagnostics.push(Diagnostic::warning(
                        "FF-KF8-RES-0003",
                        format!("could not load image {}: {error}", resource.path),
                    ));
                    None
                }
            }
        })
        .collect()
}

fn render_node(
    node: &Node,
    book: &Book,
    output: &mut String,
    style_ids: &mut Vec<u32>,
    resource_ids: &mut Vec<u32>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for anchor in book.anchors.iter().filter(|anchor| anchor.node == node.id) {
        output.push_str(&format!(
            "<a id=\"{}\"></a>",
            escape_attribute(&anchor.name)
        ));
    }
    style_ids.push(node.style.get());
    let style = book.styles.get(node.style).cloned().unwrap_or_default();
    let style_attr = inline_css(&style);
    let style_attr = if style_attr.is_empty() {
        String::new()
    } else {
        format!(" style=\"{}\"", escape_attribute(&style_attr))
    };
    match &node.kind {
        NodeKind::Text { value } => output.push_str(&escape_html(value)),
        NodeKind::Section => wrap(
            "section",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::Heading { level } => {
            let level = (*level).clamp(1, 6);
            output.push_str(&format!("<h{level}{style_attr}>"));
            render_children(node, book, output, style_ids, resource_ids, diagnostics);
            output.push_str(&format!("</h{level}>"));
        }
        NodeKind::Paragraph => wrap(
            "p",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::Emphasis => wrap(
            "em",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::Strong => wrap(
            "strong",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::BlockQuote => wrap(
            "blockquote",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::Code => wrap(
            "code",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::Preformatted => wrap(
            "pre",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::OrderedList => wrap(
            "ol",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::UnorderedList => wrap(
            "ul",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::ListItem => wrap(
            "li",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::Table => wrap(
            "table",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::TableRow => wrap(
            "tr",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::TableCell => wrap(
            "td",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::Link { href } => {
            output.push_str(&format!(
                "<a href=\"{}\"{style_attr}>",
                escape_attribute(href)
            ));
            render_children(node, book, output, style_ids, resource_ids, diagnostics);
            output.push_str("</a>");
        }
        NodeKind::Anchor { name } => {
            if !book
                .anchors
                .iter()
                .any(|anchor| anchor.node == node.id && anchor.name == *name)
            {
                output.push_str(&format!("<a id=\"{}\"></a>", escape_attribute(name)));
            }
            render_children(node, book, output, style_ids, resource_ids, diagnostics);
        }
        NodeKind::Image { resource, alt } => {
            resource_ids.push(resource.get());
            let src = book
                .resource(*resource)
                .map(|item| item.path.as_str())
                .unwrap_or("");
            output.push_str(&format!(
                "<img src=\"{}\" alt=\"{}\"{style_attr}/>",
                escape_attribute(src),
                escape_attribute(alt)
            ));
        }
        NodeKind::Svg { alt, .. } => {
            if !alt.is_empty() {
                output.push_str(&escape_html(alt));
            }
        }
        NodeKind::Ruby => wrap(
            "ruby",
            &style_attr,
            node,
            book,
            output,
            style_ids,
            resource_ids,
            diagnostics,
        ),
        NodeKind::Math { alt, .. } => {
            let fallback = alt.as_deref().unwrap_or("[math]");
            output.push_str(&format!(
                "<span class=\"ff-math-fallback\" role=\"math\" aria-label=\"{}\"{style_attr}>",
                escape_attribute(fallback)
            ));
            output.push_str(&escape_html(fallback));
            output.push_str("</span>");
            diagnostics.push(Diagnostic::warning(
                "FF-KF8-MATH-0001",
                "MathML lowered to its accessible text fallback for KF8.",
            ));
        }
        NodeKind::Footnote { href } => {
            output.push_str(&format!(
                "<sup{style_attr}><a epub:type=\"noteref\" href=\"{}\">",
                escape_attribute(href.as_deref().unwrap_or("#"))
            ));
            render_children(node, book, output, style_ids, resource_ids, diagnostics);
            output.push_str("</a></sup>");
        }
        NodeKind::PageBreak => output.push_str("<mbp:pagebreak/>"),
        NodeKind::Inline | NodeKind::GenericInline { .. } | NodeKind::GenericBlock { .. } => {
            let tag = match &node.kind {
                NodeKind::GenericInline { tag } | NodeKind::GenericBlock { tag } => tag.as_str(),
                _ => "span",
            };
            output.push_str(&format!("<{tag}{style_attr}>"));
            render_children(node, book, output, style_ids, resource_ids, diagnostics);
            output.push_str(&format!("</{tag}>"));
        }
    }
}

fn render_children(
    node: &Node,
    book: &Book,
    output: &mut String,
    style_ids: &mut Vec<u32>,
    resource_ids: &mut Vec<u32>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for child in &node.children {
        render_node(child, book, output, style_ids, resource_ids, diagnostics);
    }
}

#[allow(clippy::too_many_arguments)]
fn wrap(
    tag: &str,
    style: &str,
    node: &Node,
    book: &Book,
    output: &mut String,
    style_ids: &mut Vec<u32>,
    resource_ids: &mut Vec<u32>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    output.push_str(&format!("<{tag}{style}>"));
    render_children(node, book, output, style_ids, resource_ids, diagnostics);
    output.push_str(&format!("</{tag}>"));
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attribute(value: &str) -> String {
    escape_html(value).replace('"', "&quot;")
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-kf8/src/lib.rs"]
mod tests;
