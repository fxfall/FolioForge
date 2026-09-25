//! Stateless preview requests and display results over prepared Folio IR.
//!
//! Core remains responsible for imports, edits, compatibility decisions and
//! target projection. This module only renders the Book supplied by Core.

use std::sync::Arc;

use folio_model::{Book, LayoutMode};
use serde::{Deserialize, Serialize};

use crate::{
    render_reflowable_documents, FixedPageRenderModel, ReaderContentMode, ReaderError,
    ReaderErrorCode, ReaderLocation, ReaderSession, ReaderSize, ReaderViewport,
    ReflowableDocumentRenderModel,
};

/// Semantic state already prepared by Core for the Reader to display.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderPreviewMode {
    SourceSemantic,
    EditedSemantic,
    Target,
}

/// Reflowable preview viewport. Dimensions and font scale are presentation
/// inputs; they never modify the Semantic IR.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReaderPreviewViewport {
    width: u16,
    height: u16,
    font_size_percent: u8,
}

impl ReaderPreviewViewport {
    pub fn new(width: u16, height: u16, font_size_percent: u8) -> Result<Self, ReaderError> {
        if width == 0 || height == 0 {
            return Err(ReaderError::new(ReaderErrorCode::InvalidViewport));
        }
        Ok(Self {
            width,
            height,
            font_size_percent: font_size_percent.clamp(60, 200),
        })
    }

    pub const fn width(self) -> u16 {
        self.width
    }

    pub const fn height(self) -> u16 {
        self.height
    }

    pub const fn font_size_percent(self) -> u8 {
        self.font_size_percent
    }
}

/// Generic, descriptive label for a Core-prepared target projection.
/// Reader does not interpret this as a compatibility or device profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReaderPreviewTargetContext {
    pub label: String,
    pub disclaimer: String,
}

/// One transient preview operation over source, edited, or target-projected
/// generic IR. Target context is mandatory only for an actual target preview.
#[derive(Clone, Debug)]
pub struct ReaderPreviewRequest {
    pub mode: ReaderPreviewMode,
    pub location: Option<ReaderLocation>,
    pub viewport: ReaderPreviewViewport,
    pub target_context: Option<ReaderPreviewTargetContext>,
}

impl ReaderPreviewRequest {
    pub fn validate(&self) -> Result<(), ReaderError> {
        if self.viewport.width == 0 || self.viewport.height == 0 {
            return Err(ReaderError::new(ReaderErrorCode::InvalidViewport));
        }
        if !(60..=200).contains(&self.viewport.font_size_percent) {
            return Err(ReaderError::new(ReaderErrorCode::InvalidViewport));
        }
        let valid = match self.mode {
            ReaderPreviewMode::Target => self.target_context.is_some(),
            ReaderPreviewMode::SourceSemantic | ReaderPreviewMode::EditedSemantic => {
                self.target_context.is_none()
            }
        };
        if valid {
            Ok(())
        } else {
            Err(ReaderError::new(ReaderErrorCode::InvalidPreviewRequest))
        }
    }
}

/// Preview content selected from generic IR layout semantics.
#[derive(Clone, Debug)]
pub enum ReaderPreviewContent {
    Reflowable {
        documents: Vec<ReflowableDocumentRenderModel>,
        html: String,
    },
    FixedPage(FixedPageRenderModel),
}

/// Stable Reader output for a single preview request.
#[derive(Clone, Debug)]
pub struct ReaderPreviewRenderModel {
    pub mode: ReaderPreviewMode,
    pub location: Option<ReaderLocation>,
    pub viewport: ReaderSize,
    pub target_context: Option<ReaderPreviewTargetContext>,
    pub content: ReaderPreviewContent,
}

/// Render an imported, edited, or Core-projected Book. No import/edit/compat
/// processing occurs here.
pub fn render_preview(
    book: Arc<Book>,
    request: &ReaderPreviewRequest,
) -> Result<ReaderPreviewRenderModel, ReaderError> {
    request.validate()?;
    let viewport_size = ReaderSize {
        width: f32::from(request.viewport.width),
        height: f32::from(request.viewport.height),
    };
    let reader_viewport = ReaderViewport::new(
        viewport_size.width,
        viewport_size.height,
        1.0,
        ReaderContentMode::Fit,
    )?;
    let (content, resolved_location) = match book.presentation.layout {
        LayoutMode::Reflowable => {
            let resolved_location = if let Some(location) = request.location {
                let mut session = ReaderSession::open(Arc::clone(&book), reader_viewport)?;
                Some(session.go_to(location)?)
            } else {
                None
            };
            let documents = render_reflowable_documents(&book);
            let html = render_reflowable_preview_html(
                &book,
                &documents,
                request.viewport,
                request.target_context.as_ref(),
            );
            (
                ReaderPreviewContent::Reflowable { documents, html },
                resolved_location,
            )
        }
        LayoutMode::Fixed => {
            let session = ReaderSession::open(Arc::clone(&book), reader_viewport)?;
            let location = request
                .location
                .unwrap_or_else(|| session.current_location());
            (
                ReaderPreviewContent::FixedPage(session.fixed_page(location)?),
                Some(location),
            )
        }
    };
    Ok(ReaderPreviewRenderModel {
        mode: request.mode,
        location: resolved_location,
        viewport: viewport_size,
        target_context: request.target_context.clone(),
        content,
    })
}

fn render_reflowable_preview_html(
    book: &Book,
    documents: &[ReflowableDocumentRenderModel],
    viewport: ReaderPreviewViewport,
    target: Option<&ReaderPreviewTargetContext>,
) -> String {
    let content_width = viewport.width.saturating_sub(48);
    let font_scale = f32::from(viewport.font_size_percent) / 100.0;
    let mut html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>html{{font-size:{}rem}}body{{box-sizing:border-box;width:100%;max-width:{}px;min-height:{}px;margin:0 auto;padding:1.25rem;font-family:system-ui,sans-serif;line-height:1.6;background:#fff;color:#292521}}article{{margin:2rem 0;padding-bottom:1rem;border-bottom:1px solid #ddd}}.notice{{padding:.8rem;border-radius:.5rem;background:#fff5dd;color:#6a4300}}</style></head><body>",
        escape(book.metadata.display_title()),
        font_scale,
        content_width,
        viewport.height,
    );
    if let Some(target) = target {
        html.push_str(&format!(
            "<p class=\"notice\">{} · compatibility preview</p>",
            escape(&target.disclaimer)
        ));
    }
    html.push_str(&format!(
        "<h1>{}</h1>",
        escape(book.metadata.display_title())
    ));
    for document in documents {
        html.push_str("<article>");
        if let Some(title) = &document.title {
            html.push_str(&format!("<h2>{}</h2>", escape(title)));
        }
        html.push_str(&document.html);
        html.push_str("</article>");
    }
    html.push_str("</body></html>");
    html
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
