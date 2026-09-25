//! Identity composition and one-way projection for source pages that need no
//! edits. This is the first, deliberately narrow Comic → Semantic IR slice;
//! analysis-driven composition and fixed-page geometry remain later gates.

use std::{collections::BTreeMap, sync::Arc};

use folio_model::{
    Book, ComputedStyle, Diagnostic, Document, DocumentId, LayoutMode, Metadata, NavPoint, Node,
    NodeId, NodeKind, Presentation, Resource, ResourceId, ResourceKind, ResourceLoadError,
    ResourceLoader, Severity,
};

use crate::{ComicImportError, ComicNativeBook, ComicSourceFormat, ImportedComic, SourcePageId};

/// Source-order identity composition. No crop, rotation, split, spread, or
/// other inferred edit is applied by this constructor.
#[derive(Clone, Debug)]
pub struct ComicComposition {
    metadata: Metadata,
    pages: Vec<CompositionPage>,
}

#[derive(Clone, Debug)]
struct CompositionPage {
    id: SourcePageId,
    source_format: ComicSourceFormat,
    encoded_size: Option<u64>,
}

impl ComicComposition {
    /// Build a no-op composition directly from the immutable, validated source
    /// order. This does not run image analysis or apply automatic suggestions.
    pub fn from_native(native: &ComicNativeBook) -> Self {
        Self {
            metadata: native.metadata().clone(),
            pages: native
                .source_pages()
                .map(|page| CompositionPage {
                    id: page.id().clone(),
                    source_format: page.source_format(),
                    encoded_size: page.encoded_size(),
                })
                .collect(),
        }
    }

    /// Project an unedited raster-page composition into generic Book/IR.
    /// Page resources remain lazy and are resolved through the originating
    /// Comic importer. Unknown direction, alt text and viewport geometry are
    /// reported instead of guessed.
    pub fn project_to_ir(
        &self,
        source: &ImportedComic,
    ) -> Result<ComicIrProjection, ComicImportError> {
        if self.pages.len() != source.native().source_pages().len()
            || self
                .pages
                .iter()
                .zip(source.native().source_pages())
                .any(|(composed, source_page)| &composed.id != source_page.id())
        {
            return Err(ComicImportError::InvalidSource(
                "composition page count no longer matches its imported source".to_owned(),
            ));
        }

        let mut book = Book::new();
        book.metadata = self.metadata.clone();
        book.presentation = Presentation {
            layout: LayoutMode::Fixed,
            direction: None,
            writing_mode: None,
            ..Presentation::default()
        };
        let default_style = book.styles.intern(ComputedStyle::default());
        let mut pages_by_locator = BTreeMap::new();
        let mut missing_alt_text = false;

        for (index, page) in self.pages.iter().enumerate() {
            let (extension, media_type, kind) = ir_image_type(page.source_format)?;
            let resource_id = ResourceId::new(index as u32);
            let document_id = DocumentId::new(index as u32);
            let number = index + 1;
            let resource_path = format!("comic-resources/page-{number:06}.{extension}");
            let document_href = format!("comic-pages/page-{number:06}.xhtml");
            pages_by_locator.insert(resource_path.clone(), page.id.clone());
            book.resources.push(Resource {
                id: resource_id,
                path: resource_path,
                media_type: media_type.to_owned(),
                kind,
                properties: Vec::new(),
                size: page.encoded_size,
            });
            book.documents.push(Document {
                id: document_id,
                href: document_href.clone(),
                media_type: "application/xhtml+xml".to_owned(),
                title: Some(format!("Page {number}")),
                nodes: vec![Node::new(
                    NodeId::new(index as u32),
                    NodeKind::Image {
                        resource: resource_id,
                        alt: String::new(),
                    },
                    default_style,
                    Vec::new(),
                )],
            });
            book.navigation.page_list.push(NavPoint {
                label: format!("Page {number}"),
                href: document_href,
                children: Vec::new(),
            });
            missing_alt_text = true;
        }

        book = book.with_resource_loader(Arc::new(ComicIrResourceLoader {
            source: source.clone(),
            pages_by_locator,
        }));

        let mut diagnostics = vec![Diagnostic::warning(
            "FF-COMIC-IR-0002",
            "This source-only projection preserves page order and image bytes, but does not yet represent viewport geometry, page-side/spread semantics, or manga reading direction; fixed-layout output fidelity is not established.",
        )];
        if missing_alt_text {
            diagnostics.push(Diagnostic::new(
                Severity::Warning,
                "FF-COMIC-IR-0001",
                "Comic page images have no source-provided alternative text. OCR or descriptions were not inferred.",
            ));
        }

        Ok(ComicIrProjection { book, diagnostics })
    }
}

/// Generic Semantic IR produced by a Comic source projection, with explicit
/// diagnostics for facts the current native model cannot prove.
#[derive(Clone, Debug)]
pub struct ComicIrProjection {
    pub book: Book,
    pub diagnostics: Vec<Diagnostic>,
}

fn ir_image_type(
    format: ComicSourceFormat,
) -> Result<(&'static str, &'static str, ResourceKind), ComicImportError> {
    match format {
        ComicSourceFormat::Jpeg => Ok(("jpg", "image/jpeg", ResourceKind::Jpeg)),
        ComicSourceFormat::Png => Ok(("png", "image/png", ResourceKind::Png)),
        ComicSourceFormat::Gif => Ok(("gif", "image/gif", ResourceKind::Gif)),
        ComicSourceFormat::Webp => Ok(("webp", "image/webp", ResourceKind::Unknown)),
        ComicSourceFormat::PdfPage => Err(ComicImportError::PdfPageRequiresRasterization),
        ComicSourceFormat::FixedLayoutResource | ComicSourceFormat::Unknown => {
            Err(ComicImportError::InvalidRaster)
        }
    }
}

#[derive(Clone, Debug)]
struct ComicIrResourceLoader {
    source: ImportedComic,
    pages_by_locator: BTreeMap<String, SourcePageId>,
}

impl ResourceLoader for ComicIrResourceLoader {
    fn load(&self, locator: &str, max_bytes: Option<u64>) -> Result<Vec<u8>, ResourceLoadError> {
        let page_id = self
            .pages_by_locator
            .get(locator)
            .ok_or_else(|| ResourceLoadError::NotFound(locator.to_owned()))?;
        self.source
            .read_page_bytes(page_id, max_bytes.unwrap_or(u64::MAX))
            .map_err(|error| ResourceLoadError::Other(error.to_string()))
    }
}

/// A convenience projection that retains direct EPUB Semantic IR when the
/// Comic adapter was built from a Fixed Layout EPUB; otherwise builds an
/// identity composition from the Comic native page manifest.
pub(crate) fn project_imported(
    source: &ImportedComic,
    epub_report: Option<&folio_epub::EpubReadReport>,
) -> Result<ComicIrProjection, ComicImportError> {
    if let Some(report) = epub_report {
        return Ok(ComicIrProjection {
            book: report.book.clone(),
            diagnostics: report.diagnostics.clone(),
        });
    }
    ComicComposition::from_native(source.native()).project_to_ir(source)
}
