//! Comic output writers that consume the generic Semantic IR projection.

use std::io::{Seek, Write};

use folio_model::{Book, Node, NodeKind, Resource, ResourceId};
use thiserror::Error;
use zip::{write::SimpleFileOptions, CompressionMethod, DateTime, ZipWriter};

const MAX_PAGE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ComicOutputError {
    #[error("comic output was cancelled")]
    Cancelled,
    #[error("CBZ output requires at least one page")]
    EmptyBook,
    #[error("CBZ output cannot preserve this Semantic IR structure: {0}")]
    UnsupportedStructure(String),
    #[error("CBZ page resource is missing from the Semantic IR")]
    MissingResource,
    #[error("CBZ page resource exceeds the {MAX_PAGE_BYTES}-byte output limit")]
    PageTooLarge,
    #[error("CBZ page bytes do not match their Semantic IR media type")]
    InvalidImage,
    #[error("CBZ archive construction failed: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("CBZ output I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Semantic IR resource loading failed: {0}")]
    Resource(#[from] folio_model::ResourceLoadError),
}

/// Write an unedited comic IR to a deterministic, report-free CBZ archive.
/// Each source page is resolved lazily and released before the next page.
/// Only one image node per fixed-layout document is accepted; unsupported
/// semantic content fails closed instead of being silently discarded.
pub fn write_cbz<W, C, P>(
    book: &Book,
    writer: W,
    mut is_cancelled: C,
    mut page_progress: P,
) -> Result<W, ComicOutputError>
where
    W: Write + Seek,
    C: FnMut() -> bool,
    P: FnMut(usize, usize),
{
    if book.documents.is_empty() {
        return Err(ComicOutputError::EmptyBook);
    }

    let resources = book
        .resources
        .iter()
        .map(|resource| (resource.id, resource))
        .collect::<std::collections::BTreeMap<ResourceId, &Resource>>();
    let page_resources = book
        .documents
        .iter()
        .map(|document| {
            let mut images = Vec::new();
            collect_page_images(&document.nodes, &mut images)?;
            if images.len() != 1 {
                return Err(ComicOutputError::UnsupportedStructure(format!(
                    "document {} has {} image nodes; exactly one is required",
                    document.href,
                    images.len()
                )));
            }
            resources
                .get(&images[0])
                .copied()
                .ok_or(ComicOutputError::MissingResource)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let total = page_resources.len();
    let mut archive = ZipWriter::new(writer);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644)
        .large_file(true);

    for (index, resource) in page_resources.into_iter().enumerate() {
        if is_cancelled() {
            return Err(ComicOutputError::Cancelled);
        }
        let extension = image_extension(&resource.media_type).ok_or_else(|| {
            ComicOutputError::UnsupportedStructure(format!(
                "unsupported comic image media type {}",
                resource.media_type
            ))
        })?;
        let bytes = book.load_resource(resource.id, Some(MAX_PAGE_BYTES))?;
        if bytes.len() as u64 > MAX_PAGE_BYTES {
            return Err(ComicOutputError::PageTooLarge);
        }
        if !image_bytes_match(&bytes, extension) {
            return Err(ComicOutputError::InvalidImage);
        }
        archive.start_file(format!("images/page-{:06}.{extension}", index + 1), options)?;
        archive.write_all(&bytes)?;
        page_progress(index + 1, total);
    }

    if is_cancelled() {
        return Err(ComicOutputError::Cancelled);
    }
    Ok(archive.finish()?)
}

fn collect_page_images(
    nodes: &[Node],
    images: &mut Vec<ResourceId>,
) -> Result<(), ComicOutputError> {
    for node in nodes {
        match &node.kind {
            NodeKind::Image { resource, .. } if node.children.is_empty() => images.push(*resource),
            NodeKind::Image { .. } => {
                return Err(ComicOutputError::UnsupportedStructure(
                    "an image node contains nested content".to_owned(),
                ));
            }
            _ => {
                return Err(ComicOutputError::UnsupportedStructure(
                    "a page contains non-image Semantic IR content".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn image_extension(media_type: &str) -> Option<&'static str> {
    match media_type {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

fn image_bytes_match(bytes: &[u8], extension: &str) -> bool {
    let Ok(format) = image::guess_format(bytes) else {
        return false;
    };
    matches!(
        (extension, format),
        ("jpg", image::ImageFormat::Jpeg)
            | ("png", image::ImageFormat::Png)
            | ("gif", image::ImageFormat::Gif)
            | ("webp", image::ImageFormat::WebP)
    )
}
