//! Core orchestration for compatibility preview requests.
//!
//! Target planning and IR projection stay in Core/folio-compat. Generic
//! reflowable IR display models come from folio-reader.

use folio_capabilities::Format;
use folio_compat::{
    plan, project, DegradationMode, DegradationOptions, DegradationPlan, DegradationReport,
};
use folio_edit::{BookEditPlan, EditError};
use folio_model::Book;
use folio_reader::{
    render_preview, ReaderLocation, ReaderPreviewContent, ReaderPreviewMode, ReaderPreviewRequest,
    ReaderPreviewTargetContext, ReaderPreviewViewport, ReaderSize,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PreviewTargetProfile {
    pub format: Format,
    pub label: String,
    pub disclaimer: String,
    pub device: PreviewDevice,
    pub orientation: PreviewOrientation,
    pub viewport_width: u16,
    pub viewport_height: u16,
    pub font_size_percent: u8,
}

impl PreviewTargetProfile {
    pub fn for_format(format: Format, settings: &PreviewSettings) -> Self {
        let (width, height) = settings.viewport_dimensions();
        Self {
            format,
            label: format!(
                "{} · {} · {}",
                format.name(),
                settings.device.label(),
                settings.orientation.label()
            ),
            disclaimer: "This is a FolioForge compatibility preview, not a Kindle Exact Preview or device renderer.".to_owned(),
            device: settings.device,
            orientation: settings.orientation,
            viewport_width: width,
            viewport_height: height,
            font_size_percent: settings.font_size_percent.clamp(60, 200),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewDevice {
    #[default]
    EReader,
    Phone,
    Tablet,
}

impl PreviewDevice {
    fn label(self) -> &'static str {
        match self {
            Self::EReader => "E-reader",
            Self::Phone => "Phone",
            Self::Tablet => "Tablet",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewOrientation {
    #[default]
    Portrait,
    Landscape,
}

impl PreviewOrientation {
    fn label(self) -> &'static str {
        match self {
            Self::Portrait => "Portrait",
            Self::Landscape => "Landscape",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct PreviewSettings {
    pub device: PreviewDevice,
    pub orientation: PreviewOrientation,
    pub font_size_percent: u8,
}

impl PreviewSettings {
    pub fn normalized(self) -> Self {
        Self {
            font_size_percent: self.font_size_percent.clamp(60, 200),
            ..self
        }
    }

    pub fn viewport_dimensions(self) -> (u16, u16) {
        let (mut width, mut height) = match self.device {
            PreviewDevice::EReader => (640, 960),
            PreviewDevice::Phone => (390, 844),
            PreviewDevice::Tablet => (820, 1180),
        };
        if self.orientation == PreviewOrientation::Landscape {
            std::mem::swap(&mut width, &mut height);
        }
        (width, height)
    }
}

impl Default for PreviewSettings {
    fn default() -> Self {
        Self {
            device: PreviewDevice::EReader,
            orientation: PreviewOrientation::Portrait,
            font_size_percent: 100,
        }
    }
}

/// Frozen 0.1 response name for the canonical Reader document render model.
pub type PreviewDocument = folio_reader::ReflowableDocumentRenderModel;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PreviewBundle {
    pub target: PreviewTargetProfile,
    pub source_title: String,
    pub documents: Vec<PreviewDocument>,
    pub html: String,
    pub degradation: DegradationReport,
    pub input_loss: Vec<String>,
    pub target_loss: Vec<String>,
    pub blocked: bool,
}

/// Core-side inputs for a Reader preview. The target is required only for the
/// `Target` mode; semantic modes intentionally cannot request compatibility
/// projection.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CoreReaderPreviewRequest {
    pub input: PathBuf,
    pub mode: ReaderPreviewMode,
    #[serde(default)]
    pub location: Option<ReaderLocation>,
    #[serde(default)]
    pub target: Option<crate::Target>,
    #[serde(default)]
    pub degradation_mode: DegradationMode,
    #[serde(default)]
    pub degradation: DegradationOptions,
    #[serde(default)]
    pub edit: BookEditPlan,
    #[serde(default)]
    pub settings: PreviewSettings,
    #[serde(default)]
    pub text: folio_text::TextImportOptions,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoreReaderPreviewContent {
    Reflowable {
        documents: Vec<PreviewDocument>,
        html: String,
    },
    FixedPage {
        page: crate::ReaderPageModelDto,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CoreReaderPreviewBundle {
    pub mode: ReaderPreviewMode,
    pub location: Option<ReaderLocation>,
    pub viewport: ReaderSize,
    pub source_title: String,
    pub content: CoreReaderPreviewContent,
    pub target: Option<PreviewTargetProfile>,
    pub degradation: Option<DegradationReport>,
    pub input_loss: Vec<String>,
    pub diagnostics: Vec<folio_model::Diagnostic>,
    pub target_loss: Vec<String>,
    pub blocked: bool,
}

#[derive(Debug, Error)]
pub enum PreviewError {
    #[error("edit plan could not be applied: {0}")]
    Edit(#[from] EditError),
    #[error("compatibility projection failed: {0}")]
    Compatibility(#[from] folio_compat::CompatError),
    #[error("Reader preview rendering failed: {0}")]
    Reader(#[from] folio_reader::ReaderError),
}

struct PreparedTargetPreview {
    edited_book: Book,
    display_book: Book,
    profile: PreviewTargetProfile,
    degradation: DegradationPlan,
    target_loss: Vec<String>,
    blocked: bool,
}

fn prepare_target_preview(
    source: &Book,
    edits: &BookEditPlan,
    target: Format,
    mode: DegradationMode,
    options: &DegradationOptions,
    settings: &PreviewSettings,
) -> Result<PreparedTargetPreview, PreviewError> {
    let edited_book = edits.apply(source)?;
    let degradation = plan(&edited_book, target, mode, options);
    let blocked = degradation.blocked;
    let display_book = if blocked {
        edited_book.clone()
    } else {
        project(&edited_book, &degradation)?
    };
    let target_loss = degradation
        .items
        .iter()
        .filter(|item| {
            !matches!(
                item.quality,
                folio_compat::QualityLevel::Exact | folio_compat::QualityLevel::Equivalent
            )
        })
        .map(|item| format!("{}: {}", item.feature.name(), item.selected_fallback))
        .collect();
    Ok(PreparedTargetPreview {
        edited_book,
        display_book,
        profile: PreviewTargetProfile::for_format(target, &settings.normalized()),
        degradation,
        target_loss,
        blocked,
    })
}

/// Import and prepare one semantic state, then render it using the generic
/// Reader. Compatibility planning remains entirely in Core.
pub fn reader_preview(
    request: &CoreReaderPreviewRequest,
) -> Result<CoreReaderPreviewBundle, crate::CoreError> {
    let (source, input_loss, diagnostics) = match crate::import_path_with_options(
        &request.input,
        request.degradation_mode,
        &request.text,
    ) {
        Ok(imported) => (
            Arc::new(imported.book),
            imported.input_report.input_loss,
            imported.diagnostics,
        ),
        Err(_)
            if request.input.is_dir()
                || request
                    .input
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        extension.eq_ignore_ascii_case("zip")
                            || extension.eq_ignore_ascii_case("cbz")
                    }) =>
        {
            let (book, diagnostics) = crate::reader_runtime::comic_book(
                &request.input,
                &crate::CancellationToken::new(),
            )?;
            (book, Vec::new(), diagnostics)
        }
        Err(error) => return Err(error),
    };
    reader_preview_from_source(request, source, input_loss, diagnostics)
}

pub(crate) fn reader_preview_from_imported(
    request: &CoreReaderPreviewRequest,
    imported: crate::ImportedBook,
) -> Result<CoreReaderPreviewBundle, crate::CoreError> {
    reader_preview_from_source(
        request,
        Arc::new(imported.book),
        imported.input_report.input_loss,
        imported.diagnostics,
    )
}

fn reader_preview_from_source(
    request: &CoreReaderPreviewRequest,
    source: Arc<Book>,
    input_loss: Vec<String>,
    diagnostics: Vec<folio_model::Diagnostic>,
) -> Result<CoreReaderPreviewBundle, crate::CoreError> {
    let raw_title = source.metadata.display_title().to_owned();
    let settings = request.settings.normalized();
    let mut target_profile = None;
    let mut degradation_report = None;
    let mut target_loss = Vec::new();
    let mut blocked = false;

    let (prepared, source_title) = match request.mode {
        ReaderPreviewMode::SourceSemantic if request.target.is_none() => {
            (Arc::clone(&source), raw_title)
        }
        ReaderPreviewMode::EditedSemantic if request.target.is_none() => {
            let edited = Arc::new(request.edit.apply(&source)?);
            let title = edited.metadata.display_title().to_owned();
            (edited, title)
        }
        ReaderPreviewMode::Target => {
            let target = request
                .target
                .ok_or_else(folio_reader::ReaderError::invalid_preview_request)?;
            let target_preview = prepare_target_preview(
                &source,
                &request.edit,
                target.format(),
                request.degradation_mode,
                &request.degradation,
                &settings,
            )?;
            let title = target_preview
                .edited_book
                .metadata
                .display_title()
                .to_owned();
            blocked = target_preview.blocked;
            target_loss = target_preview.target_loss;
            degradation_report = Some(target_preview.degradation.report());
            target_profile = Some(target_preview.profile);
            (Arc::new(target_preview.display_book), title)
        }
        ReaderPreviewMode::SourceSemantic | ReaderPreviewMode::EditedSemantic => {
            return Err(folio_reader::ReaderError::invalid_preview_request().into())
        }
    };

    let (width, height) = target_profile
        .as_ref()
        .map(|profile| (profile.viewport_width, profile.viewport_height))
        .unwrap_or_else(|| settings.viewport_dimensions());
    let viewport = ReaderPreviewViewport::new(width, height, settings.font_size_percent)?;
    let reader_request = ReaderPreviewRequest {
        mode: request.mode,
        location: request.location,
        viewport,
        target_context: target_profile
            .as_ref()
            .map(|profile| ReaderPreviewTargetContext {
                label: profile.label.clone(),
                disclaimer: profile.disclaimer.clone(),
            }),
    };
    let render_model = render_preview(Arc::clone(&prepared), &reader_request)?;
    let content = match render_model.content {
        ReaderPreviewContent::Reflowable { documents, html } => {
            CoreReaderPreviewContent::Reflowable { documents, html }
        }
        ReaderPreviewContent::FixedPage(page) => {
            let document_index = prepared
                .documents
                .iter()
                .position(|document| document.id == page.location.document_id)
                .ok_or_else(folio_reader::ReaderError::invalid_preview_request)?;
            CoreReaderPreviewContent::FixedPage {
                page: super::reader_runtime::page_model_dto(page, document_index)?,
            }
        }
    };
    Ok(CoreReaderPreviewBundle {
        mode: request.mode,
        location: render_model.location,
        viewport: render_model.viewport,
        source_title,
        content,
        target: target_profile,
        degradation: degradation_report,
        input_loss,
        diagnostics,
        target_loss,
        blocked,
    })
}
