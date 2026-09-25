use std::{fs::File, path::Path};

use folio_epub::{EpubLimits, EpubReader};
use folio_model::{LayoutMode, Node, NodeKind, Resource, ResourceId};
use zip::ZipArchive;

use crate::{
    ComicNativeBook, ComicSourceIdentity, ComicSourceKind, ComicSourcePageDraft, PageOrdering,
    SourceLocation,
};

use super::{
    check_cancelled, make_imported, raster, ComicContainerKind, ComicImportError,
    ComicImportLimits, ImportedComic,
};

pub(super) fn import_fixed_layout_epub<F>(
    path: &Path,
    stable_source_key: &str,
    limits: &ComicImportLimits,
    is_cancelled: &mut F,
) -> Result<ImportedComic, ComicImportError>
where
    F: FnMut() -> bool,
{
    check_cancelled(is_cancelled)?;
    let mut archive = super::archive::open_safe_zip(path, limits)?;
    let epub_limits = EpubLimits {
        max_entries: limits.max_entries,
        max_total_uncompressed: limits.max_total_uncompressed_bytes,
        max_entry_size: limits.max_member_bytes,
        max_xhtml_size: limits.max_member_bytes.min(32 * 1024 * 1024),
        max_css_size: limits.max_member_bytes.min(16 * 1024 * 1024),
        max_xml_depth: 128,
        max_dom_nodes: limits.max_entries.saturating_mul(32).min(2_000_000),
    };
    let report = EpubReader::new(epub_limits)
        .read(path)
        .map_err(|error| ComicImportError::IneligibleFixedLayoutEpub(error.to_string()))?;
    let book = &report.book;
    if book.presentation.layout != LayoutMode::Fixed {
        return Err(ComicImportError::IneligibleFixedLayoutEpub(
            "rendition:layout is not pre-paginated".to_owned(),
        ));
    }
    if book.documents.is_empty() || book.documents.len() > limits.max_pages {
        return Err(ComicImportError::LimitExceeded(format!(
            "Fixed Layout EPUB has {} spine documents; page limit is {}",
            book.documents.len(),
            limits.max_pages
        )));
    }

    let identity = ComicSourceIdentity::new(ComicSourceKind::FixedLayoutEpub, stable_source_key)?;
    let mut drafts = Vec::with_capacity(book.documents.len());
    let mut explicit_order = Vec::with_capacity(book.documents.len());
    for (index, document) in book.documents.iter().enumerate() {
        check_cancelled(is_cancelled)?;
        let mut image_ids = Vec::new();
        for node in &document.nodes {
            inspect_fixed_layout_node(node, &mut image_ids)?;
        }
        if image_ids.len() != 1 {
            return Err(ComicImportError::IneligibleFixedLayoutEpub(format!(
                "spine item {} contains {} raster images; exactly one is required",
                document.href,
                image_ids.len()
            )));
        }
        let resource = book.resource(image_ids[0]).ok_or_else(|| {
            ComicImportError::IneligibleFixedLayoutEpub(format!(
                "spine item {} references a missing image resource",
                document.href
            ))
        })?;
        let (format, encoded_size) =
            validate_image_resource(&mut archive, resource, limits, is_cancelled)?;
        let location = SourceLocation::fixed_layout_spine_item(&resource.path, index as u32)?;
        let page_id = identity.page_id(&location, "")?;
        explicit_order.push(page_id);
        drafts.push(
            ComicSourcePageDraft::new(location, format)
                .with_encoded_size(encoded_size)
                .with_source_name(resource.path.clone())?,
        );
    }

    let native = ComicNativeBook::new(
        identity,
        book.metadata.clone(),
        drafts,
        [("container".to_owned(), "fixed-layout-epub".to_owned())].into(),
        PageOrdering::Explicit(explicit_order),
    )?;
    let mut imported = make_imported(native, path, ComicContainerKind::FixedLayoutEpub, limits);
    imported.source_ir = Some(report);
    Ok(imported)
}

fn inspect_fixed_layout_node(
    node: &Node,
    images: &mut Vec<ResourceId>,
) -> Result<(), ComicImportError> {
    match &node.kind {
        NodeKind::Image { resource, .. } => images.push(*resource),
        NodeKind::Text { value } if value.trim().is_empty() => {}
        NodeKind::Section
        | NodeKind::Paragraph
        | NodeKind::Inline
        | NodeKind::Emphasis
        | NodeKind::Strong
        | NodeKind::Link { .. }
        | NodeKind::Anchor { .. }
        | NodeKind::GenericBlock { .. }
        | NodeKind::GenericInline { .. } => {
            for child in &node.children {
                inspect_fixed_layout_node(child, images)?;
            }
        }
        _ => {
            return Err(ComicImportError::IneligibleFixedLayoutEpub(
                "spine item contains non-image text or unsupported semantic content".to_owned(),
            ))
        }
    }
    Ok(())
}

fn validate_image_resource<F>(
    archive: &mut ZipArchive<File>,
    resource: &Resource,
    limits: &ComicImportLimits,
    is_cancelled: &mut F,
) -> Result<(crate::ComicSourceFormat, u64), ComicImportError>
where
    F: FnMut() -> bool,
{
    check_cancelled(is_cancelled)?;
    let media_type = resource.media_type.to_ascii_lowercase();
    if !matches!(
        media_type.as_str(),
        "image/jpeg" | "image/jpg" | "image/png" | "image/gif" | "image/webp"
    ) {
        return Err(ComicImportError::IneligibleFixedLayoutEpub(format!(
            "resource {} has unsupported media type {}",
            resource.path, resource.media_type
        )));
    }
    if resource
        .size
        .is_some_and(|size| size > limits.max_page_bytes)
    {
        return Err(ComicImportError::LimitExceeded(format!(
            "Fixed Layout EPUB page {} exceeds {} bytes",
            resource.path, limits.max_page_bytes
        )));
    }
    let bytes = super::archive::read_zip_archive_member(
        archive,
        &resource.path,
        limits.max_page_bytes,
        is_cancelled,
    )?;
    let format = raster::sniff_image_format(&bytes).ok_or(ComicImportError::InvalidRaster)?;
    let media_type_matches = matches!(
        (media_type.as_str(), format),
        ("image/jpeg" | "image/jpg", crate::ComicSourceFormat::Jpeg)
            | ("image/png", crate::ComicSourceFormat::Png)
            | ("image/gif", crate::ComicSourceFormat::Gif)
            | ("image/webp", crate::ComicSourceFormat::Webp)
    );
    if !media_type_matches {
        return Err(ComicImportError::IneligibleFixedLayoutEpub(format!(
            "resource {} media type disagrees with its image signature",
            resource.path
        )));
    }
    Ok((format, bytes.len() as u64))
}
