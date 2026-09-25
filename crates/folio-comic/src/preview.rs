//! Bounded source-page dimensions and source-only preview rendering.

use std::io::Cursor;

use image::{ImageFormat, ImageReader, Limits};
use thiserror::Error;

use crate::{ComicImportError, ComicSourceFormat, ImportedComic, SourcePageId};

const MAX_SOURCE_EDGE: u32 = 32_768;
const MAX_DECODED_PIXELS: u64 = 32_000_000;
const MAX_DECODE_ALLOCATION: u64 = 256 * 1024 * 1024;
const MAX_PREVIEW_EDGE: u32 = 2_048;
const MAX_THUMBNAIL_EDGE: u32 = 512;
const MAX_OUTPUT_PIXELS: u64 = 8_000_000;
const MAX_ENCODED_PREVIEW_BYTES: usize = 16 * 1024 * 1024;
const MAX_SOURCE_PAGE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ComicImageKind {
    Thumbnail,
    SourcePreview,
}

#[derive(Clone, Copy, Debug)]
pub struct ComicImageBounds {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

#[derive(Clone, Debug)]
pub struct RenderedComicImage {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
}

#[derive(Debug, Error)]
pub enum ComicRenderError {
    #[error("comic image page could not be read: {0}")]
    Import(#[from] ComicImportError),
    #[error("comic image decoder failed: {0}")]
    Image(#[from] image::ImageError),
    #[error("comic image preview request has invalid bounds or scale")]
    InvalidBounds,
    #[error("comic source image dimensions exceed the safe preview limit")]
    DimensionsExceeded,
    #[error("comic source image format is not supported for preview")]
    UnsupportedFormat,
    #[error("comic preview image exceeds the encoded output limit")]
    EncodedOutputExceeded,
    #[error("comic preview was cancelled")]
    Cancelled,
}

/// Read a single encoded page's image header without decoding its pixel data.
pub fn source_dimensions(
    source: &ImportedComic,
    page_id: &SourcePageId,
) -> Result<(u32, u32), ComicRenderError> {
    let page = source
        .native()
        .source_page(page_id)
        .ok_or(ComicImportError::PageNotFound)?;
    let format = image_format(page.source_format())?;
    let bytes = source.read_page_bytes(page_id, MAX_SOURCE_PAGE_BYTES)?;
    checked_dimensions(&bytes, format)
}

/// Render one requested source page to an encoded PNG. Original source bytes
/// are not changed. Only one full decoded page is held at a time.
pub fn render_source_page<F>(
    source: &ImportedComic,
    page_id: &SourcePageId,
    kind: ComicImageKind,
    bounds: ComicImageBounds,
    mut is_cancelled: F,
) -> Result<RenderedComicImage, ComicRenderError>
where
    F: FnMut() -> bool,
{
    if is_cancelled() {
        return Err(ComicRenderError::Cancelled);
    }
    let max_base_edge = match kind {
        ComicImageKind::Thumbnail => MAX_THUMBNAIL_EDGE,
        ComicImageKind::SourcePreview => MAX_PREVIEW_EDGE,
    };
    if bounds.width == 0
        || bounds.height == 0
        || bounds.width > max_base_edge
        || bounds.height > max_base_edge
        || !bounds.scale.is_finite()
        || !(1.0..=2.0).contains(&bounds.scale)
    {
        return Err(ComicRenderError::InvalidBounds);
    }
    let output_width = scaled_dimension(bounds.width, bounds.scale)?;
    let output_height = scaled_dimension(bounds.height, bounds.scale)?;
    if output_width > MAX_PREVIEW_EDGE
        || output_height > MAX_PREVIEW_EDGE
        || u64::from(output_width) * u64::from(output_height) > MAX_OUTPUT_PIXELS
    {
        return Err(ComicRenderError::InvalidBounds);
    }

    let page = source
        .native()
        .source_page(page_id)
        .ok_or(ComicImportError::PageNotFound)?;
    let format = image_format(page.source_format())?;
    let bytes =
        source.read_page_bytes_with_cancel(page_id, MAX_SOURCE_PAGE_BYTES, &mut is_cancelled)?;
    let (source_width, source_height) = checked_dimensions(&bytes, format)?;
    if is_cancelled() {
        return Err(ComicRenderError::Cancelled);
    }

    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_EDGE);
    limits.max_image_height = Some(MAX_SOURCE_EDGE);
    limits.max_alloc = Some(MAX_DECODE_ALLOCATION);
    let mut reader = ImageReader::with_format(Cursor::new(bytes.as_slice()), format);
    reader.limits(limits);
    let decoded = reader.decode()?;
    if is_cancelled() {
        return Err(ComicRenderError::Cancelled);
    }

    let thumbnail = decoded.thumbnail(output_width, output_height);
    let mut encoded = Cursor::new(Vec::new());
    thumbnail.write_to(&mut encoded, ImageFormat::Png)?;
    let encoded = encoded.into_inner();
    if encoded.len() > MAX_ENCODED_PREVIEW_BYTES {
        return Err(ComicRenderError::EncodedOutputExceeded);
    }
    if is_cancelled() {
        return Err(ComicRenderError::Cancelled);
    }

    Ok(RenderedComicImage {
        width: thumbnail.width(),
        height: thumbnail.height(),
        source_width,
        source_height,
        bytes: encoded,
    })
}

fn scaled_dimension(value: u32, scale: f32) -> Result<u32, ComicRenderError> {
    let scaled = f64::from(value) * f64::from(scale);
    if !scaled.is_finite() || scaled < 1.0 || scaled > f64::from(u32::MAX) {
        return Err(ComicRenderError::InvalidBounds);
    }
    Ok(scaled.ceil() as u32)
}

fn checked_dimensions(bytes: &[u8], format: ImageFormat) -> Result<(u32, u32), ComicRenderError> {
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), format).into_dimensions()?;
    let pixels = u64::from(width) * u64::from(height);
    if width == 0
        || height == 0
        || width > MAX_SOURCE_EDGE
        || height > MAX_SOURCE_EDGE
        || pixels > MAX_DECODED_PIXELS
    {
        return Err(ComicRenderError::DimensionsExceeded);
    }
    Ok((width, height))
}

fn image_format(format: ComicSourceFormat) -> Result<ImageFormat, ComicRenderError> {
    match format {
        ComicSourceFormat::Jpeg => Ok(ImageFormat::Jpeg),
        ComicSourceFormat::Png => Ok(ImageFormat::Png),
        ComicSourceFormat::Gif => Ok(ImageFormat::Gif),
        ComicSourceFormat::Webp => Ok(ImageFormat::WebP),
        ComicSourceFormat::PdfPage
        | ComicSourceFormat::FixedLayoutResource
        | ComicSourceFormat::Unknown => Err(ComicRenderError::UnsupportedFormat),
    }
}
