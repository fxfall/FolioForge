use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use lopdf::{Document, LoadOptions};

use crate::{
    ComicNativeBook, ComicSourceFormat, ComicSourceIdentity, ComicSourceKind, ComicSourcePageDraft,
    PageOrdering, SourceLocation,
};

use super::{
    check_cancelled, make_imported, metadata_from_path, ComicContainerKind, ComicImportError,
    ComicImportLimits, ImportedComic,
};

pub(super) fn import_pdf<F>(
    path: &Path,
    stable_source_key: &str,
    limits: &ComicImportLimits,
    is_cancelled: &mut F,
) -> Result<ImportedComic, ComicImportError>
where
    F: FnMut() -> bool,
{
    check_cancelled(is_cancelled)?;
    let source_bytes = fs::metadata(path)?.len();
    if source_bytes > limits.max_pdf_bytes {
        return Err(ComicImportError::LimitExceeded(format!(
            "PDF is {source_bytes} bytes; cap is {}",
            limits.max_pdf_bytes
        )));
    }
    preflight_pdf(path, source_bytes, is_cancelled)?;
    let document = Document::load_with_options(
        path,
        LoadOptions {
            strict: true,
            max_decompressed_size: Some(limits.max_pdf_stream_bytes),
            ..LoadOptions::default()
        },
    )?;
    check_cancelled(is_cancelled)?;
    if document.is_encrypted() {
        return Err(ComicImportError::Unsupported(
            "encrypted PDFs are not supported".to_owned(),
        ));
    }
    let pages = document.get_pages();
    if pages.is_empty() {
        return Err(ComicImportError::InvalidSource(
            "PDF contains no pages".to_owned(),
        ));
    }
    if pages.len() > limits.max_pdf_pages || pages.len() > limits.max_pages {
        return Err(ComicImportError::LimitExceeded(format!(
            "PDF has {} pages; caps are {} and {}",
            pages.len(),
            limits.max_pdf_pages,
            limits.max_pages
        )));
    }

    let identity = ComicSourceIdentity::new(ComicSourceKind::Pdf, stable_source_key)?;
    let mut drafts = Vec::with_capacity(pages.len());
    let mut explicit_order = Vec::with_capacity(pages.len());
    for page_number in pages.keys().copied() {
        check_cancelled(is_cancelled)?;
        let location = SourceLocation::pdf_page(page_number)?;
        explicit_order.push(identity.page_id(&location, "")?);
        drafts.push(
            ComicSourcePageDraft::new(location, ComicSourceFormat::PdfPage)
                .with_source_name(format!("Page {page_number}"))?,
        );
    }

    let native = ComicNativeBook::new(
        identity,
        metadata_from_path(path),
        drafts,
        [("container".to_owned(), "pdf".to_owned())].into(),
        PageOrdering::Explicit(explicit_order),
    )?;
    Ok(make_imported(native, path, ComicContainerKind::Pdf, limits))
}

/// Restrict lopdf to classic cross-reference tables. Xref streams, hybrid
/// xrefs and incremental-update chains may require decompression outside the
/// single-stream limit exposed by lopdf, so they stay deferred until aggregate
/// accounting is available.
fn preflight_pdf<F>(
    path: &Path,
    file_len: u64,
    is_cancelled: &mut F,
) -> Result<(), ComicImportError>
where
    F: FnMut() -> bool,
{
    const TAIL_LIMIT: u64 = 64 * 1024;
    let tail_len = file_len.min(TAIL_LIMIT) as usize;
    let mut file = File::open(path)?;
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    let mut tail = vec![0; tail_len];
    file.read_exact(&mut tail)?;
    let marker = b"startxref";
    let marker_offset = tail
        .windows(marker.len())
        .rposition(|window| window == marker)
        .ok_or_else(|| {
            ComicImportError::InvalidSource(
                "PDF startxref marker was not found near EOF".to_owned(),
            )
        })?;
    let mut cursor = marker_offset + marker.len();
    while tail.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    let number_start = cursor;
    while tail.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if number_start == cursor {
        return Err(ComicImportError::InvalidSource(
            "PDF startxref offset is missing".to_owned(),
        ));
    }
    let offset_text = std::str::from_utf8(&tail[number_start..cursor]).map_err(|_| {
        ComicImportError::InvalidSource("PDF startxref offset is not ASCII".to_owned())
    })?;
    let xref_offset = offset_text.parse::<u64>().map_err(|_| {
        ComicImportError::InvalidSource("PDF startxref offset is invalid".to_owned())
    })?;
    if xref_offset >= file_len {
        return Err(ComicImportError::InvalidSource(
            "PDF startxref points outside the file".to_owned(),
        ));
    }

    file.seek(SeekFrom::Start(xref_offset))?;
    let mut xref_prefix = [0u8; 32];
    let prefix_len = file.read(&mut xref_prefix)?;
    let prefix = &xref_prefix[..prefix_len];
    let mut leading = 0;
    while prefix.get(leading).is_some_and(u8::is_ascii_whitespace) {
        leading += 1;
    }
    if !prefix[leading..].starts_with(b"xref")
        || prefix
            .get(leading + 4)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        return Err(ComicImportError::Unsupported(
            "PDF xref streams are deferred; only classic xref tables are currently accepted"
                .to_owned(),
        ));
    }

    // In the latest trailer, /Prev indicates an incremental chain and
    // /XRefStm indicates a hybrid xref. Search only the xref/trailer tail and
    // use a rolling overlap so the check remains bounded in memory.
    file.seek(SeekFrom::Start(xref_offset))?;
    const CHUNK_SIZE: usize = 32 * 1024;
    const OVERLAP: usize = 16;
    let mut buffer = [0u8; CHUNK_SIZE + OVERLAP];
    let mut carry = 0usize;
    loop {
        check_cancelled(is_cancelled)?;
        let read = file.read(&mut buffer[carry..CHUNK_SIZE + OVERLAP])?;
        if read == 0 {
            break;
        }
        let available = carry + read;
        let chunk = &buffer[..available];
        if chunk.windows(5).any(|window| window == b"/Prev")
            || chunk.windows(8).any(|window| window == b"/XRefStm")
        {
            return Err(ComicImportError::Unsupported(
                "PDF incremental and hybrid xref chains are deferred until aggregate stream accounting is available"
                    .to_owned(),
            ));
        }
        carry = available.min(OVERLAP);
        buffer.copy_within(available - carry..available, 0);
    }
    Ok(())
}
