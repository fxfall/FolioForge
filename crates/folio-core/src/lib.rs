//! Public, GUI-independent orchestration API.
//!
//! This crate is deliberately local-file oriented.  It has no networking,
//! GUI, Swift, AppKit, Foundation, or Amazon publishing dependencies.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

use folio_capabilities::{CapabilityProfile, Format};
pub use folio_compat::{
    CompatibilityQuality, DegradationMode, DegradationOptions, DegradationPlan, DegradationReport,
};
pub use folio_edit::{BookEditPlan, MetadataEdit};
use folio_epub::{EpubError, EpubReader};
pub use folio_format::{CoverSummary, InspectionReport, StructureSummary};
pub use folio_input::DetectedFormat;
use folio_kfx::amazon::{
    AmazonKfxError, InputReport as AmazonInputReport, KfxResourceDiagnostic, ParseMode,
};
use folio_kindle_common::Compression;
use folio_model::{Diagnostic, Metadata, Node, NodeKind, ResourceKind, Severity};
use folio_preview::PreviewBundle;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ABI_VERSION: u32 = 1;

static ROUND_TRIP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Target {
    EPUB,
    KF7,
    KF8,
    KF7KF8Combo,
    KFX,
}

impl Target {
    pub const fn all() -> [Self; 5] {
        [
            Self::EPUB,
            Self::KF7,
            Self::KF8,
            Self::KF7KF8Combo,
            Self::KFX,
        ]
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::EPUB => "epub",
            Self::KF7 | Self::KF7KF8Combo => "mobi",
            Self::KF8 => "azw3",
            Self::KFX => "kfx",
        }
    }

    pub const fn format(self) -> Format {
        match self {
            Self::EPUB => Format::Epub3,
            Self::KF7 | Self::KF7KF8Combo => Format::Kf7,
            Self::KF8 => Format::Kf8,
            Self::KFX => Format::Kfx,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "epub" | "epub3" => Some(Self::EPUB),
            "kf7" | "mobi" => Some(Self::KF7),
            "kf8" | "azw3" => Some(Self::KF8),
            "kf7kf8combo" | "combo" => Some(Self::KF7KF8Combo),
            "kfx" => Some(Self::KFX),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FormatDescriptor {
    pub id: String,
    pub canonical_name: String,
    pub extensions: Vec<String>,
    pub mime_types: Vec<String>,
    pub input_supported: bool,
    pub output_supported: bool,
    #[serde(default)]
    pub adapter_capabilities: Option<folio_format::AdapterCapabilities>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConversionOptions {
    pub deterministic: bool,
    pub compression: CompressionOption,
    /// Collect lightweight stage timings for benchmark builds. The default is
    /// false so normal release conversions pay only for the total timer.
    #[serde(default)]
    pub collect_metrics: bool,
    #[serde(default)]
    pub degradation_mode: DegradationMode,
    #[serde(default)]
    pub degradation: DegradationOptions,
    #[serde(default)]
    pub text: folio_text::TextImportOptions,
}

impl Default for ConversionOptions {
    fn default() -> Self {
        Self {
            deterministic: true,
            compression: CompressionOption::None,
            collect_metrics: false,
            degradation_mode: DegradationMode::Compatible,
            degradation: DegradationOptions::default(),
            text: folio_text::TextImportOptions::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CompressionOption {
    None,
    PalmDoc,
}

impl From<CompressionOption> for Compression {
    fn from(value: CompressionOption) -> Self {
        match value {
            CompressionOption::None => Compression::None,
            CompressionOption::PalmDoc => Compression::PalmDoc,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConversionRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub target: Target,
    pub options: ConversionOptions,
    #[serde(default)]
    pub edit: BookEditPlan,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProgressStage {
    Opening,
    Parsing,
    Normalizing,
    ResolvingStyles,
    ProcessingResources,
    Lowering,
    BuildingIndexes,
    Encoding,
    Writing,
    Validating,
    Finished,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProgressEvent {
    pub stage: ProgressStage,
    pub current: u64,
    pub total: Option<u64>,
    pub fraction: Option<f32>,
    pub message: String,
}

/// Lightweight, opt-in conversion measurements. Stage names are stable JSON
/// keys so benchmark tooling does not need to understand internal functions.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ConversionMetrics {
    pub stage_ms: BTreeMap<String, u128>,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub resource_bytes: u64,
}

#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn check(&self) -> Result<(), CoreError> {
        if self.is_cancelled() {
            Err(CoreError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConversionReport {
    pub output_path: PathBuf,
    pub output_size: u64,
    pub warnings: Vec<Diagnostic>,
    pub metadata: Metadata,
    pub features: BTreeMap<String, usize>,
    pub resource_summary: ResourceSummary,
    pub duration_ms: u128,
    #[serde(default)]
    pub metrics: Option<ConversionMetrics>,
    #[serde(default)]
    pub source_format: String,
    #[serde(default)]
    pub target_format: String,
    #[serde(default)]
    pub degradation_mode: DegradationMode,
    #[serde(default)]
    pub compatibility: CompatibilityQuality,
    #[serde(default)]
    pub degradation: DegradationReport,
    #[serde(default)]
    pub round_trip: RoundTripReport,
    #[serde(default)]
    pub input_report: InputReport,
    #[serde(default)]
    pub semantic_report: SemanticReport,
    #[serde(default)]
    pub compatibility_report: CompatibilityReport,
    #[serde(default)]
    pub output_report: OutputReport,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct InputReport {
    pub detected_format: String,
    pub parser: String,
    pub container_count: usize,
    pub entity_count: usize,
    pub fragment_count: usize,
    pub symbol_count: usize,
    pub resource_count: usize,
    pub document_count: usize,
    pub drm_detected: bool,
    pub warnings: Vec<String>,
    pub unknown_features: Vec<String>,
    pub recovery_actions: Vec<String>,
    pub input_loss: Vec<String>,
    #[serde(default)]
    pub resource_identity_diagnostics: Vec<KfxResourceDiagnostic>,
    /// Evidence collected from the normalized IR for a real Amazon KFX input.
    /// This is deliberately a coverage summary, not a fidelity claim and not
    /// a second KFX model exposed through the public IR.
    #[serde(default)]
    pub kfx_summary: Option<KfxInputSummary>,
    #[serde(default)]
    pub text: Option<folio_text::TextImportReport>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct KfxInputSummary {
    pub toc_entry_count: usize,
    pub landmark_entry_count: usize,
    pub page_list_entry_count: usize,
    pub has_start_location: bool,
    pub feature_counts: BTreeMap<String, usize>,
    pub image_count: usize,
    pub image_alt_present_count: usize,
    pub image_alt_empty_count: usize,
    pub link_count: usize,
    pub anchor_count: usize,
    pub anchor_graph_edge_count: usize,
    pub style_count: usize,
    pub non_default_style_node_count: usize,
    pub font_face_count: usize,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SemanticReport {
    pub valid: bool,
    pub document_count: usize,
    pub resource_count: usize,
    pub feature_count: usize,
    pub warnings: Vec<Diagnostic>,
    pub input_loss: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CompatibilityReport {
    pub target: String,
    pub quality: CompatibilityQuality,
    pub exact: usize,
    pub equivalent: usize,
    pub approximation: usize,
    pub structural_fallback: usize,
    pub dropped: usize,
    pub target_loss: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct OutputReport {
    pub format: String,
    pub path: PathBuf,
    pub size: u64,
    pub deterministic: bool,
    pub validated: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RoundTripReport {
    pub checked: bool,
    pub passed: bool,
    pub unexpected_losses: Vec<String>,
    #[serde(default)]
    pub allowed_losses: Vec<String>,
    #[serde(default)]
    pub ruby_source: Vec<folio_model::RubyProjection>,
    #[serde(default)]
    pub ruby_round_trip: Vec<folio_model::RubyProjection>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ResourceSummary {
    pub total: usize,
    pub images: usize,
    pub fonts: usize,
    pub stylesheets: usize,
    pub audio: usize,
    pub declared_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InspectReport {
    pub format: String,
    pub semantic: serde_json::Value,
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub input_report: InputReport,
    #[serde(default)]
    pub semantic_report: SemanticReport,
}

/// Complete, client-neutral analysis of one source book after applying the
/// requested edit plan. HTTP and desktop clients may format this report, but
/// must not repeat its import, validation, or compatibility decisions.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PreflightReport {
    pub detected_format: String,
    pub input_report: InputReport,
    pub source_metadata: Metadata,
    pub semantic_metadata: Metadata,
    pub semantic_report: SemanticReport,
    pub plan: DegradationPlan,
}

#[derive(Clone, Debug)]
pub struct ImportedBook {
    pub format: DetectedFormat,
    pub book: folio_model::Book,
    pub diagnostics: Vec<Diagnostic>,
    pub input_report: InputReport,
}

/// Format registry used by Core, CLI, batch, service, and FFI.  It is a
/// registry of adapters, not a second set of conversion rules.
#[derive(Clone, Copy, Debug, Default)]
pub struct FormatRegistry;

impl FormatRegistry {
    pub fn capabilities(&self) -> [CapabilityProfile; 4] {
        folio_capabilities::all_profiles()
    }

    /// The sole format authority exposed to clients. Input adapters and
    /// target writers are represented by the same descriptor identity.
    pub fn formats(&self) -> Vec<FormatDescriptor> {
        let adapters = self.adapters();
        DetectedFormat::all()
            .into_iter()
            .map(|format| {
                let adapter = adapters
                    .iter()
                    .find(|adapter| adapter.input_formats().contains(&format));
                let target = Target::all().into_iter().find(|target| match target {
                    Target::KF7KF8Combo => format == DetectedFormat::Kf7Kf8Combo,
                    _ => target.format().name() == format.name(),
                });
                FormatDescriptor {
                    id: canonical_format_id(format).to_owned(),
                    canonical_name: match format {
                        DetectedFormat::Kf7Kf8Combo => "KF7 + KF8 Combo".to_owned(),
                        _ => format.name().to_owned(),
                    },
                    extensions: format
                        .extensions()
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect(),
                    mime_types: format
                        .mime_types()
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect(),
                    input_supported: adapter.is_some(),
                    output_supported: target.is_some(),
                    adapter_capabilities: adapter.map(|value| value.capabilities()),
                }
            })
            .collect()
    }

    pub fn output_formats(&self) -> Vec<FormatDescriptor> {
        self.formats()
            .into_iter()
            .filter(|descriptor| descriptor.output_supported)
            .collect()
    }

    pub fn format_support(&self) -> Vec<(String, folio_format::FormatSupport)> {
        let adapters = self.adapters();
        self.formats()
            .into_iter()
            .filter(|descriptor| descriptor.input_supported)
            .filter_map(|descriptor| {
                let adapter = adapters.iter().find(|adapter| {
                    adapter
                        .input_formats()
                        .iter()
                        .any(|format| canonical_format_id(*format) == descriptor.id)
                })?;
                Some((descriptor.id, adapter.support()))
            })
            .collect()
    }

    /// Public adapter view used by contract tests and non-UI clients. The
    /// returned adapters are the registry's registered instances; callers do
    /// not assemble a private format list.
    pub fn input_adapters(&self) -> Vec<Box<dyn folio_format::FormatAdapter>> {
        self.adapters()
    }

    pub fn inspect_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<folio_format::InspectionReport, CoreError> {
        let path = path.as_ref();
        ensure_input(path)?;
        let source = source_for_path(path)?;
        let adapter = self.resolve(&source)?;
        let context = folio_format::ImportContext {
            strict: false,
            text: folio_text::TextImportOptions::default(),
        };
        adapter
            .inspect(&source, &context)
            .map_err(CoreError::Format)
    }

    pub fn detect_path(&self, path: impl AsRef<Path>) -> Result<DetectedFormat, CoreError> {
        let path = path.as_ref();
        ensure_input(path)?;
        let source = source_for_path(path)?;
        let (_, detection) = self.resolve_with_detection(&source)?;
        DetectedFormat::all()
            .into_iter()
            .find(|format| format.name() == detection.format)
            .ok_or_else(|| {
                CoreError::UnsupportedInput(format!(
                    "registered adapter returned unknown format identity {}",
                    detection.format
                ))
            })
    }

    pub fn import_path(&self, path: impl AsRef<Path>) -> Result<ImportedBook, CoreError> {
        self.import_source(
            &source_for_path(path.as_ref())?,
            DegradationMode::Compatible,
            &folio_text::TextImportOptions::default(),
        )
    }

    fn adapters(&self) -> Vec<Box<dyn folio_format::FormatAdapter>> {
        // Registration is the only place where Core names concrete format
        // adapters. Each adapter implementation lives beside its parser;
        // `folio-format` contains only the shared contract.
        vec![
            Box::new(folio_epub::EpubAdapter),
            Box::new(folio_format::TextAdapter),
            Box::new(folio_kf7::Kf7Adapter),
            Box::new(folio_kf8::Kf8Adapter),
            Box::new(folio_kfx::KfxAdapter),
            Box::new(folio_kfx::FfkfxAdapter),
            Box::new(folio_markdown::MarkdownAdapter),
            Box::new(folio_html::HtmlzAdapter),
            Box::new(folio_html::HtmlAdapter),
            Box::new(folio_fb2::Fb2Adapter),
            Box::new(folio_docx::DocxAdapter),
        ]
    }

    fn resolve(
        &self,
        source: &folio_input::BookSource,
    ) -> Result<Box<dyn folio_format::FormatAdapter>, CoreError> {
        self.resolve_with_detection(source)
            .map(|(adapter, _)| adapter)
    }

    fn resolve_with_detection(
        &self,
        source: &folio_input::BookSource,
    ) -> Result<
        (
            Box<dyn folio_format::FormatAdapter>,
            folio_format::DetectionResult,
        ),
        CoreError,
    > {
        let mut unsupported = Vec::new();
        for adapter in self.adapters() {
            match adapter.detect(source) {
                Ok(detection) => return Ok((adapter, detection)),
                Err(error) => unsupported.push(error.to_string()),
            }
        }
        Err(CoreError::UnsupportedInput(
            unsupported
                .last()
                .cloned()
                .unwrap_or_else(|| "no registered format adapter accepted the source".to_owned()),
        ))
    }

    fn import_source(
        &self,
        source: &folio_input::BookSource,
        mode: DegradationMode,
        text: &folio_text::TextImportOptions,
    ) -> Result<ImportedBook, CoreError> {
        let adapter = self.resolve(source)?;
        let context = folio_format::ImportContext {
            strict: mode == DegradationMode::Strict,
            text: text.clone(),
        };
        let imported = adapter.import(source, &context).map_err(map_format_error)?;
        Ok(imported_book_from_adapter(imported))
    }
}

const fn canonical_format_id(format: DetectedFormat) -> &'static str {
    match format {
        DetectedFormat::Kf7Kf8Combo => "KF7KF8Combo",
        _ => format.name(),
    }
}

/// Return the stable identifier used by Core's format registry and by
/// Library's physical-file records. These identifiers are part of the 0.1
/// contract; callers must not derive them from file extensions.
pub const fn format_id(format: DetectedFormat) -> &'static str {
    canonical_format_id(format)
}

pub fn capabilities() -> [CapabilityProfile; 4] {
    FormatRegistry.capabilities()
}

/// Detect a local source through the single Core format registry.
///
/// This facade keeps future consumers from naming an adapter crate while
/// preserving the existing `FormatRegistry` API for current callers.
pub fn detect(path: impl AsRef<Path>) -> Result<DetectedFormat, CoreError> {
    FormatRegistry.detect_path(path)
}

/// Inspect metadata, cover and structure without exporting the source.
///
/// The result is format-neutral and is derived from the same adapter import
/// contract used by conversion. It is intentionally smaller than
/// [`inspect`], which also exposes semantic diagnostics and IR summaries.
pub fn inspect_summary(path: impl AsRef<Path>) -> Result<InspectionReport, CoreError> {
    FormatRegistry.inspect_path(path)
}

pub fn analyze(
    path: impl AsRef<Path>,
    target: Target,
    mode: DegradationMode,
    options: &DegradationOptions,
) -> Result<DegradationPlan, CoreError> {
    analyze_with_edit(path, target, mode, options, &BookEditPlan::default())
}

pub fn analyze_with_edit(
    path: impl AsRef<Path>,
    target: Target,
    mode: DegradationMode,
    options: &DegradationOptions,
    edits: &BookEditPlan,
) -> Result<DegradationPlan, CoreError> {
    analyze_with_text_options(
        path,
        target,
        mode,
        options,
        edits,
        &folio_text::TextImportOptions::default(),
    )
}

pub fn analyze_with_text_options(
    path: impl AsRef<Path>,
    target: Target,
    mode: DegradationMode,
    options: &DegradationOptions,
    edits: &BookEditPlan,
    text: &folio_text::TextImportOptions,
) -> Result<DegradationPlan, CoreError> {
    preflight_with_text_options(path, target, mode, options, edits, text).map(|report| report.plan)
}

/// Import, edit, validate, and plan a source book once for a client preflight.
/// The report deliberately contains only Semantic IR-facing data and the
/// shared compatibility plan, never a format-native parser model.
pub fn preflight(
    path: impl AsRef<Path>,
    target: Target,
    mode: DegradationMode,
    options: &DegradationOptions,
    edits: &BookEditPlan,
) -> Result<PreflightReport, CoreError> {
    preflight_with_text_options(
        path,
        target,
        mode,
        options,
        edits,
        &folio_text::TextImportOptions::default(),
    )
}

pub fn preflight_with_text_options(
    path: impl AsRef<Path>,
    target: Target,
    mode: DegradationMode,
    options: &DegradationOptions,
    edits: &BookEditPlan,
    text: &folio_text::TextImportOptions,
) -> Result<PreflightReport, CoreError> {
    let imported = import_path_with_options(path.as_ref(), mode, text)?;
    let source_metadata = imported.book.metadata.clone();
    let mut book = imported.book;
    if !edits.is_empty() {
        book = edits.apply(&book)?;
    }
    let validation = folio_validator::validate_book(&book);
    let report = semantic_report(
        &book,
        validation.is_valid(),
        imported.input_report.input_loss.clone(),
        validation.diagnostics,
    );
    let plan = folio_compat::plan(&book, target.format(), mode, options);
    Ok(PreflightReport {
        detected_format: imported.format.name().to_owned(),
        input_report: imported.input_report,
        source_metadata,
        semantic_metadata: book.metadata,
        semantic_report: report,
        plan,
    })
}

/// Build the compatibility preview from the exact source IR, edit plan, and
/// degradation planner used by conversion.  This function never writes an
/// output format and never talks to a network service.
pub fn preview(
    path: impl AsRef<Path>,
    target: Target,
    mode: DegradationMode,
    options: &DegradationOptions,
    edits: &BookEditPlan,
) -> Result<PreviewBundle, CoreError> {
    preview_with_settings(
        path,
        target,
        mode,
        options,
        edits,
        &folio_preview::PreviewSettings::default(),
    )
}

pub fn preview_with_settings(
    path: impl AsRef<Path>,
    target: Target,
    mode: DegradationMode,
    options: &DegradationOptions,
    edits: &BookEditPlan,
    settings: &folio_preview::PreviewSettings,
) -> Result<PreviewBundle, CoreError> {
    preview_with_settings_and_text_options(
        path,
        target,
        mode,
        options,
        edits,
        settings,
        &folio_text::TextImportOptions::default(),
    )
}

pub fn preview_with_settings_and_text_options(
    path: impl AsRef<Path>,
    target: Target,
    mode: DegradationMode,
    options: &DegradationOptions,
    edits: &BookEditPlan,
    settings: &folio_preview::PreviewSettings,
    text: &folio_text::TextImportOptions,
) -> Result<PreviewBundle, CoreError> {
    let imported = import_path_with_options(path.as_ref(), mode, text)?;
    let mut bundle = folio_preview::build_with_settings(
        &imported.book,
        edits,
        target.format(),
        mode,
        options,
        settings,
    )
    .map_err(CoreError::from)?;
    bundle.input_loss = imported.input_report.input_loss;
    Ok(bundle)
}

pub use folio_preview::{PreviewDevice, PreviewOrientation, PreviewSettings};

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("input file does not exist: {0}")]
    MissingInput(PathBuf),
    #[error("unsupported input format: {0}")]
    UnsupportedInput(String),
    #[error("unsupported target: {0}")]
    UnsupportedTarget(String),
    #[error("EPUB error: {0}")]
    Epub(#[from] EpubError),
    #[error("KF7 error: {0}")]
    Kf7(#[from] folio_kf7::Kf7Error),
    #[error("KF8 error: {0}")]
    Kf8(#[from] folio_kf8::Kf8Error),
    #[error("KFX error: {0}")]
    Kfx(#[from] folio_kfx::KfxError),
    #[error("Amazon KFX error: {0}")]
    AmazonKfx(#[from] AmazonKfxError),
    #[error("edit plan error: {0}")]
    Edit(#[from] folio_edit::EditError),
    #[error("KF7/KF8 combo error: {0}")]
    Combo(#[from] folio_mobi::ComboError),
    #[error("validation failed: {0:?}")]
    ValidationFailed(Vec<Diagnostic>),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("conversion cancelled")]
    Cancelled,
    #[error("compatibility planning failed: {0}")]
    Compatibility(#[from] folio_compat::CompatError),
    #[error("preview failed: {0}")]
    Preview(#[from] folio_preview::PreviewError),
    #[error("format adapter failed: {0}")]
    Format(#[from] folio_format::FormatError),
}

/// Destination contract for future streaming exporters. Writers may still
/// return an in-memory artifact today, but commit/rollback belongs here rather
/// than in clients or each target writer.
pub trait OutputSink {
    fn write_chunk(&mut self, bytes: &[u8]) -> Result<(), CoreError>;
    fn finalize(&mut self) -> Result<(), CoreError>;
}

pub struct AtomicFileSink {
    output: PathBuf,
    temporary: PathBuf,
    file: File,
    finalized: bool,
}

impl AtomicFileSink {
    pub fn new(output: impl AsRef<Path>) -> Result<Self, CoreError> {
        let output = output.as_ref().to_owned();
        let parent = output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let name = output
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("output");
        let temporary = parent.join(format!(
            ".{name}.{}.{}.tmp",
            std::process::id(),
            ROUND_TRIP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        Ok(Self {
            output,
            temporary,
            file,
            finalized: false,
        })
    }
}

impl OutputSink for AtomicFileSink {
    fn write_chunk(&mut self, bytes: &[u8]) -> Result<(), CoreError> {
        self.file.write_all(bytes)?;
        Ok(())
    }

    fn finalize(&mut self) -> Result<(), CoreError> {
        self.file.sync_all()?;
        fs::rename(&self.temporary, &self.output)?;
        self.finalized = true;
        Ok(())
    }
}

impl Drop for AtomicFileSink {
    fn drop(&mut self) {
        if !self.finalized {
            let _ = fs::remove_file(&self.temporary);
        }
    }
}

pub fn inspect(path: impl AsRef<Path>) -> Result<InspectReport, CoreError> {
    let path = path.as_ref();
    ensure_input(path)?;
    if path.is_dir() {
        let imported = import_path_with_options(
            path,
            DegradationMode::Compatible,
            &folio_text::TextImportOptions::default(),
        )?;
        let validation = validate_imported_book(&imported.book);
        let mut diagnostics = imported.diagnostics;
        diagnostics.extend(validation.diagnostics.clone());
        return Ok(InspectReport {
            format: imported.format.name().to_owned(),
            semantic: imported.book.semantic_value(),
            diagnostics: diagnostics.clone(),
            input_report: imported.input_report,
            semantic_report: semantic_report(
                &imported.book,
                validation.is_valid(),
                Vec::new(),
                diagnostics,
            ),
        });
    }
    let bytes = read_file(path)?;
    let imported = match import_bytes(path, &bytes) {
        Ok(imported) => imported,
        Err(CoreError::AmazonKfx(AmazonKfxError::ProtectedContent)) => {
            let native = folio_kfx::amazon::inspect(&bytes, ParseMode::Compatible)?;
            let input_report = amazon_input_report(&native);
            let mut diagnostics = diagnostics_from_input(&input_report);
            diagnostics.push(Diagnostic::error(
                "FF-KFX-DRM-0001",
                "DRM-protected KFX was structurally inspected only; FolioForge does not decrypt protected content.",
            ));
            let semantic_report = SemanticReport {
                valid: false,
                warnings: diagnostics.clone(),
                input_loss: input_report.input_loss.clone(),
                ..SemanticReport::default()
            };
            return Ok(InspectReport {
                format: DetectedFormat::Kfx.name().to_owned(),
                semantic: serde_json::json!({
                    "protected_content": true,
                    "editable": false,
                    "decryption_attempted": false
                }),
                diagnostics,
                input_report,
                semantic_report,
            });
        }
        Err(error) => return Err(error),
    };
    let mut diagnostics = imported.diagnostics;
    let validation = validate_imported_book(&imported.book);
    diagnostics.extend(validation.diagnostics.clone());
    let semantic_report = semantic_report(
        &imported.book,
        validation.is_valid(),
        imported.input_report.input_loss.clone(),
        diagnostics.clone(),
    );
    Ok(InspectReport {
        format: imported.format.name().to_owned(),
        semantic: imported.book.semantic_value(),
        diagnostics,
        input_report: imported.input_report,
        semantic_report,
    })
}

pub fn validate(path: impl AsRef<Path>) -> Result<folio_validator::ValidationReport, CoreError> {
    let path = path.as_ref();
    ensure_input(path)?;
    if path.is_dir() {
        let imported = import_path_with_options(
            path,
            DegradationMode::Compatible,
            &folio_text::TextImportOptions::default(),
        )?;
        return Ok(folio_validator::validate_book(&imported.book));
    }
    let bytes = read_file(path)?;
    let detected = detect_bytes(path, &bytes)?;
    let mut report = match detected {
        DetectedFormat::Epub => {
            let read = EpubReader::default().read(path)?;
            let mut report = folio_validator::validate_book(&read.book);
            report.diagnostics.extend(read.diagnostics);
            report
        }
        DetectedFormat::Kf7 => folio_validator::validate_mobi(&bytes),
        DetectedFormat::Kf8 => folio_validator::validate_kf8(&bytes),
        DetectedFormat::Kf7Kf8Combo => folio_validator::validate_combo(&bytes),
        DetectedFormat::Kfx => match import_bytes(path, &bytes) {
            Ok(imported) => {
                let mut report = folio_validator::validate_book(&imported.book);
                report
                    .diagnostics
                    .extend(diagnostics_from_input(&imported.input_report));
                report
            }
            Err(CoreError::AmazonKfx(AmazonKfxError::ProtectedContent)) => {
                let input = folio_kfx::amazon::inspect(&bytes, ParseMode::Compatible)?;
                let mut report = folio_validator::ValidationReport::default();
                report
                    .diagnostics
                    .extend(diagnostics_from_input(&amazon_input_report(&input)));
                report.diagnostics.push(Diagnostic::error(
                    "FF-KFX-DRM-0001",
                    "DRM-protected KFX cannot be semantically validated; no decryption was attempted.",
                ));
                report
            }
            Err(error) => return Err(error),
        },
        DetectedFormat::Ffkfx => folio_validator::validate_kfx(&bytes),
        _ => {
            let imported = import_bytes(path, &bytes)?;
            let mut report = folio_validator::validate_book(&imported.book);
            report.diagnostics.extend(imported.diagnostics);
            report
        }
    };
    if matches!(
        detected,
        DetectedFormat::Kf7
            | DetectedFormat::Kf8
            | DetectedFormat::Kf7Kf8Combo
            | DetectedFormat::Ffkfx
    ) {
        if let Ok(imported) = import_bytes(path, &bytes) {
            report
                .diagnostics
                .extend(folio_validator::validate_book(&imported.book).diagnostics);
        }
    }
    Ok(report)
}

pub fn convert(request: &ConversionRequest) -> Result<ConversionReport, CoreError> {
    convert_with_progress(request, &CancellationToken::new(), |_| {})
}

pub fn convert_with_progress<F>(
    request: &ConversionRequest,
    cancellation: &CancellationToken,
    mut progress: F,
) -> Result<ConversionReport, CoreError>
where
    F: FnMut(ProgressEvent),
{
    ensure_input(&request.input)?;
    let started = Instant::now();
    let metrics_enabled = request.options.collect_metrics;
    let mut stage_ms = BTreeMap::new();
    emit(
        &mut progress,
        ProgressStage::Opening,
        0,
        None,
        "opening local input",
    );
    cancellation.check()?;
    let stage_started = Instant::now();
    let mut imported = import_path_with_options(
        &request.input,
        request.options.degradation_mode,
        &request.options.text,
    )?;
    if !request.edit.is_empty() {
        imported.book = request
            .edit
            .apply(&imported.book)
            .map_err(CoreError::Edit)?;
    }
    if metrics_enabled {
        stage_ms.insert(
            "parse_normalize_ms".to_owned(),
            stage_started.elapsed().as_millis(),
        );
    }
    emit(
        &mut progress,
        ProgressStage::Parsing,
        1,
        Some(1),
        format!("parsed {} input", imported.format.name()),
    );
    cancellation.check()?;
    let stage_started = Instant::now();
    let ir_validation = folio_validator::validate_book(&imported.book);
    let semantic = semantic_report(
        &imported.book,
        ir_validation.is_valid(),
        imported.input_report.input_loss.clone(),
        ir_validation.diagnostics.clone(),
    );
    if !semantic.valid {
        return Err(CoreError::ValidationFailed(ir_validation.diagnostics));
    }
    if metrics_enabled {
        stage_ms.insert(
            "semantic_validate_ms".to_owned(),
            stage_started.elapsed().as_millis(),
        );
    }
    emit(
        &mut progress,
        ProgressStage::Normalizing,
        1,
        Some(1),
        "Folio Semantic IR is ready",
    );
    emit(
        &mut progress,
        ProgressStage::ResolvingStyles,
        imported.book.styles.len() as u64,
        Some(imported.book.styles.len() as u64),
        "computed styles interned",
    );
    cancellation.check()?;
    emit(
        &mut progress,
        ProgressStage::ProcessingResources,
        imported.book.resources.len() as u64,
        Some(imported.book.resources.len() as u64),
        "resources mapped lazily",
    );
    let stage_started = Instant::now();
    let plan = folio_compat::plan(
        &imported.book,
        request.target.format(),
        request.options.degradation_mode,
        &request.options.degradation,
    );
    plan.validate_mode()?;
    let projected = folio_compat::project(&imported.book, &plan)?;
    let compatibility_report = compatibility_report(&plan, request.target.format());
    if metrics_enabled {
        stage_ms.insert(
            "compatibility_ms".to_owned(),
            stage_started.elapsed().as_millis(),
        );
    }
    emit(
        &mut progress,
        ProgressStage::Lowering,
        1,
        Some(1),
        format!(
            "applying {:?} compatibility plan for {:?}",
            request.options.degradation_mode, request.target
        ),
    );

    let stage_started = Instant::now();
    let (bytes, mut target_diagnostics) = match request.target {
        Target::EPUB => {
            let artifact = folio_epub::export(
                &projected,
                &folio_epub::EpubOptions {
                    deterministic: request.options.deterministic,
                    include_ncx: true,
                },
            )?;
            (artifact.bytes, artifact.diagnostics)
        }
        Target::KF7 => {
            let artifact = folio_kf7::convert(
                &projected,
                &folio_kf7::Kf7Options {
                    compression: request.options.compression.into(),
                    deterministic: request.options.deterministic,
                },
            )?;
            (artifact.bytes, artifact.diagnostics)
        }
        Target::KF8 => {
            let artifact = folio_kf8::convert(
                &projected,
                &folio_kf8::Kf8Options {
                    compression: request.options.compression.into(),
                    deterministic: request.options.deterministic,
                },
            )?;
            (artifact.bytes, artifact.diagnostics)
        }
        Target::KFX => {
            let artifact = folio_kfx::convert(
                &projected,
                &folio_kfx::KfxOptions {
                    deterministic: request.options.deterministic,
                },
            )?;
            (artifact.bytes, artifact.diagnostics)
        }
        Target::KF7KF8Combo => {
            let kf7 = folio_kf7::convert(
                &projected,
                &folio_kf7::Kf7Options {
                    compression: request.options.compression.into(),
                    deterministic: request.options.deterministic,
                },
            )?;
            let kf8 = folio_kf8::convert(
                &projected,
                &folio_kf8::Kf8Options {
                    compression: request.options.compression.into(),
                    deterministic: request.options.deterministic,
                },
            )?;
            let combo = folio_mobi::pack_combo(
                &folio_mobi::MobiArtifact {
                    bytes: kf7.bytes,
                    text_record_count: 0,
                    image_record_start: None,
                },
                &folio_mobi::MobiArtifact {
                    bytes: kf8.bytes,
                    text_record_count: 0,
                    image_record_start: None,
                },
                projected.metadata.display_title(),
                request.options.deterministic,
            )?;
            let mut diagnostics = kf7.diagnostics;
            diagnostics.extend(kf8.diagnostics);
            diagnostics.push(Diagnostic::warning(
                "FF-KF-COMBO-0001",
                "output uses FolioForge's explicit composite compatibility container; undocumented Amazon dual-format boundaries are not inferred.",
            ));
            (combo.bytes, diagnostics)
        }
    };
    if metrics_enabled {
        stage_ms.insert("export_ms".to_owned(), stage_started.elapsed().as_millis());
    }
    cancellation.check()?;
    emit(
        &mut progress,
        ProgressStage::BuildingIndexes,
        1,
        Some(1),
        "target indexes built",
    );
    emit(
        &mut progress,
        ProgressStage::Encoding,
        bytes.len() as u64,
        Some(bytes.len() as u64),
        "target bytes encoded",
    );
    emit(
        &mut progress,
        ProgressStage::Validating,
        1,
        Some(1),
        "validating generated binary before commit",
    );
    let stage_started = Instant::now();
    let generated_validation = validate_generated(request.target, &bytes);
    if !generated_validation.is_valid() {
        return Err(CoreError::ValidationFailed(
            generated_validation.diagnostics,
        ));
    }
    let round_trip = semantic_round_trip(&projected, request.target, &bytes);
    if !round_trip.passed && imported.format != DetectedFormat::Kfx {
        return Err(CoreError::ValidationFailed(vec![Diagnostic::error(
            "FF-ROUNDTRIP-0001",
            format!(
                "semantic round-trip for {} reported: {}",
                request.target.format().name(),
                round_trip.unexpected_losses.join("; ")
            ),
        )]));
    }
    if metrics_enabled {
        stage_ms.insert(
            "validate_round_trip_ms".to_owned(),
            stage_started.elapsed().as_millis(),
        );
    }
    let stage_started = Instant::now();
    write_atomic(&request.output, &bytes, cancellation, &mut progress)?;
    if metrics_enabled {
        stage_ms.insert("write_ms".to_owned(), stage_started.elapsed().as_millis());
    }
    let output_size = fs::metadata(&request.output)?.len();
    emit(
        &mut progress,
        ProgressStage::Finished,
        1,
        Some(1),
        "conversion finished",
    );
    target_diagnostics.extend(
        generated_validation
            .diagnostics
            .into_iter()
            .filter(|diagnostic| diagnostic.severity != Severity::Error),
    );
    let mut warnings = imported.diagnostics;
    if !round_trip.passed {
        warnings.push(Diagnostic::warning(
            "FF-ROUNDTRIP-KFX-0001",
            format!(
                "real Amazon KFX was converted from inferred semantic fragments; round-trip audit recorded: {}",
                round_trip.unexpected_losses.join("; ")
            ),
        ));
    }
    warnings.extend(plan.diagnostics.clone());
    warnings.append(&mut target_diagnostics);
    let resource_summary = resource_summary(&imported.book);
    let metrics = metrics_enabled.then(|| ConversionMetrics {
        stage_ms,
        input_bytes: fs::metadata(&request.input)
            .map(|value| value.len())
            .unwrap_or_default(),
        output_bytes: output_size,
        resource_bytes: resource_summary.declared_bytes,
    });
    Ok(ConversionReport {
        output_path: request.output.clone(),
        output_size,
        warnings,
        metadata: imported.book.metadata.clone(),
        features: imported.book.feature_summary(),
        resource_summary,
        duration_ms: started.elapsed().as_millis(),
        metrics,
        source_format: imported.format.name().to_owned(),
        target_format: request.target.format().name().to_owned(),
        degradation_mode: request.options.degradation_mode,
        compatibility: plan.quality,
        degradation: plan.report(),
        round_trip,
        input_report: imported.input_report,
        semantic_report: semantic,
        compatibility_report,
        output_report: OutputReport {
            format: request.target.format().name().to_owned(),
            path: request.output.clone(),
            size: output_size,
            deterministic: request.options.deterministic,
            validated: true,
        },
    })
}

fn import_path(path: &Path) -> Result<ImportedBook, CoreError> {
    import_path_with_mode(path, DegradationMode::Compatible)
}

fn import_path_with_mode(path: &Path, mode: DegradationMode) -> Result<ImportedBook, CoreError> {
    import_path_with_options(path, mode, &folio_text::TextImportOptions::default())
}

fn import_path_with_options(
    path: &Path,
    mode: DegradationMode,
    text_options: &folio_text::TextImportOptions,
) -> Result<ImportedBook, CoreError> {
    FormatRegistry.import_source(&source_for_path(path)?, mode, text_options)
}

fn import_bytes(path: &Path, bytes: &[u8]) -> Result<ImportedBook, CoreError> {
    import_bytes_with_mode(path, bytes, DegradationMode::Compatible)
}

fn import_bytes_with_mode(
    path: &Path,
    bytes: &[u8],
    mode: DegradationMode,
) -> Result<ImportedBook, CoreError> {
    import_bytes_with_options(path, bytes, mode, &folio_text::TextImportOptions::default())
}

fn import_bytes_with_options(
    path: &Path,
    bytes: &[u8],
    mode: DegradationMode,
    text_options: &folio_text::TextImportOptions,
) -> Result<ImportedBook, CoreError> {
    let temporary_root = folioforge_temp_root().join(format!(
        "import-{}-{}",
        std::process::id(),
        ROUND_TRIP_COUNTER.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(&temporary_root)?;
    let temporary = temporary_root.join(
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("input.bin"),
    );
    fs::write(&temporary, bytes)?;
    let result = import_path_with_options(&temporary, mode, text_options);
    let _ = fs::remove_dir_all(&temporary_root);
    result
}

fn detect_bytes(path: &Path, bytes: &[u8]) -> Result<DetectedFormat, CoreError> {
    folio_input::detect_format(path, bytes)
        .map_err(|error| CoreError::UnsupportedInput(error.to_string()))
}

fn source_for_path(path: &Path) -> Result<folio_input::BookSource, CoreError> {
    if path.is_dir() {
        folio_input::BookSource::directory(path, folio_input::SourceLimits::default())
            .map_err(|error| CoreError::UnsupportedInput(error.to_string()))
    } else {
        folio_input::BookSource::single_file(path)
            .map_err(|error| CoreError::UnsupportedInput(error.to_string()))
    }
}

fn map_format_error(error: folio_format::FormatError) -> CoreError {
    match error {
        folio_format::FormatError::AmazonKfx(error) => {
            CoreError::AmazonKfx(AmazonKfxError::Semantic(error))
        }
        folio_format::FormatError::AmazonKfxProtected => {
            CoreError::AmazonKfx(AmazonKfxError::ProtectedContent)
        }
        other => CoreError::Format(other),
    }
}

fn imported_book_from_adapter(imported: folio_format::ImportedBook) -> ImportedBook {
    let folio_format::ImportedBook {
        format,
        parser,
        mut book,
        mut diagnostics,
        input_loss,
        text,
    } = imported;
    let normalization = folio_normalize::normalize_book(&mut book);
    diagnostics.extend(normalization.diagnostics);
    let kfx_summary = matches!(format, DetectedFormat::Kfx).then(|| kfx_input_summary(&book));
    let mut input_report = generic_input_report(format, &book, &diagnostics);
    input_report.parser = parser;
    input_report.input_loss = input_loss;
    input_report.kfx_summary = kfx_summary;
    input_report.text = text;
    ImportedBook {
        format,
        book,
        diagnostics,
        input_report,
    }
}

fn kfx_input_summary(book: &folio_model::Book) -> KfxInputSummary {
    let mut summary = KfxInputSummary {
        toc_entry_count: count_navigation_points(&book.navigation.toc),
        landmark_entry_count: count_navigation_points(&book.navigation.landmarks),
        page_list_entry_count: count_navigation_points(&book.navigation.page_list),
        has_start_location: book.navigation.start_location.is_some(),
        feature_counts: book.feature_summary(),
        anchor_count: book.anchors.len(),
        anchor_graph_edge_count: book.navigation.anchor_graph.edges.len(),
        style_count: book.styles.len(),
        font_face_count: book.font_faces.len(),
        ..KfxInputSummary::default()
    };
    for document in &book.documents {
        for node in &document.nodes {
            collect_kfx_node_summary(node, book, &mut summary);
        }
    }
    summary.link_count = summary
        .feature_counts
        .get("link")
        .copied()
        .unwrap_or_default();
    summary
}

fn count_navigation_points(points: &[folio_model::NavPoint]) -> usize {
    points
        .iter()
        .map(|point| 1 + count_navigation_points(&point.children))
        .sum()
}

fn collect_kfx_node_summary(node: &Node, book: &folio_model::Book, summary: &mut KfxInputSummary) {
    if let NodeKind::Image { alt, .. } = &node.kind {
        summary.image_count += 1;
        if alt.is_empty() {
            summary.image_alt_empty_count += 1;
        } else {
            summary.image_alt_present_count += 1;
        }
    }
    if book
        .styles
        .get(node.style)
        .is_some_and(|style| !style.properties.is_empty())
    {
        summary.non_default_style_node_count += 1;
    }
    for child in &node.children {
        collect_kfx_node_summary(child, book, summary);
    }
}

fn validate_imported_book(book: &folio_model::Book) -> folio_validator::ValidationReport {
    folio_validator::validate_book(book)
}

fn amazon_input_report(report: &AmazonInputReport) -> InputReport {
    InputReport {
        detected_format: report.detected_format.clone(),
        parser: report.parser_version.clone(),
        container_count: report.container_count,
        entity_count: report.entity_count,
        fragment_count: report.fragment_count,
        symbol_count: report.symbol_count,
        resource_count: report.resource_count,
        document_count: report.document_count,
        drm_detected: report.drm_detected,
        warnings: report.warnings.clone(),
        unknown_features: report.unknown_features.clone(),
        recovery_actions: report.recovery_actions.clone(),
        input_loss: report.input_loss.clone(),
        resource_identity_diagnostics: report.resource_identity_diagnostics.clone(),
        kfx_summary: None,
        text: None,
    }
}

fn generic_input_report(
    format: DetectedFormat,
    book: &folio_model::Book,
    diagnostics: &[Diagnostic],
) -> InputReport {
    InputReport {
        detected_format: format.name().to_owned(),
        parser: format!("folioforge-{}-frontend", format.name().to_ascii_lowercase()),
        resource_count: book.resources.len(),
        document_count: book.documents.len(),
        warnings: diagnostics
            .iter()
            .map(|item| item.message.clone())
            .collect(),
        ..InputReport::default()
    }
}

fn diagnostics_from_input(report: &InputReport) -> Vec<Diagnostic> {
    let mut diagnostics = report
        .warnings
        .iter()
        .enumerate()
        .map(|(index, message)| Diagnostic::warning(format!("FF-INPUT-W{:04}", index + 1), message))
        .collect::<Vec<_>>();
    diagnostics.extend(report.unknown_features.iter().map(|feature| {
        Diagnostic::warning(
            "FF-INPUT-UNKNOWN-0001",
            format!("input feature preserved only in the native model: {feature}"),
        )
    }));
    diagnostics.extend(
        report
            .recovery_actions
            .iter()
            .map(|action| Diagnostic::warning("FF-INPUT-RECOVERY-0001", action)),
    );
    diagnostics.extend(
        report
            .input_loss
            .iter()
            .map(|loss| Diagnostic::warning("FF-INPUT-LOSS-0001", loss)),
    );
    diagnostics.extend(report.resource_identity_diagnostics.iter().map(|item| {
        let mut diagnostic = Diagnostic::warning(&item.code, &item.message);
        diagnostic
            .context
            .insert("input_index".to_owned(), item.input_index.to_string());
        diagnostic.context.insert(
            "container_origin".to_owned(),
            item.container_origin.to_string(),
        );
        diagnostic.context.insert(
            "external_resource_entity_id".to_owned(),
            item.external_resource_entity_id.to_string(),
        );
        diagnostic.context.insert(
            "location_field_id".to_owned(),
            item.location_field_id.to_string(),
        );
        diagnostic.context.insert(
            "body_reference_count".to_owned(),
            item.body_reference_count.to_string(),
        );
        diagnostic.context.insert(
            "visible_placement_count".to_owned(),
            item.visible_placement_count.to_string(),
        );
        if let Some(location) = &item.location {
            diagnostic
                .context
                .insert("location".to_owned(), location.clone());
        }
        diagnostic
            .context
            .insert("location_source".to_owned(), item.location_source.clone());
        if let Some(symbol_id) = item.location_symbol_id {
            diagnostic
                .context
                .insert("location_symbol_id".to_owned(), symbol_id.to_string());
        }
        if let Some(location) = &item.normalized_location {
            diagnostic
                .context
                .insert("normalized_location".to_owned(), location.clone());
        }
        diagnostic
    }));
    diagnostics
}

fn semantic_report(
    book: &folio_model::Book,
    valid: bool,
    input_loss: Vec<String>,
    warnings: Vec<Diagnostic>,
) -> SemanticReport {
    SemanticReport {
        valid,
        document_count: book.documents.len(),
        resource_count: book.resources.len(),
        feature_count: book.feature_summary().values().sum(),
        warnings,
        input_loss,
    }
}

fn compatibility_report(plan: &DegradationPlan, target: Format) -> CompatibilityReport {
    let report = plan.report();
    CompatibilityReport {
        target: target.name().to_owned(),
        quality: report.quality,
        exact: report.exact,
        equivalent: report.equivalent,
        approximation: report.approximation,
        structural_fallback: report.structural_fallback,
        dropped: report.dropped,
        target_loss: report
            .items
            .iter()
            .filter(|item| item.quality != folio_compat::QualityLevel::Exact)
            .map(|item| format!("{}: {}", item.feature.name(), item.selected_fallback))
            .collect(),
        diagnostics: report.diagnostics,
    }
}

fn semantic_round_trip(
    source: &folio_model::Book,
    target: Target,
    output_bytes: &[u8],
) -> RoundTripReport {
    let imported = match import_round_trip_bytes(target, output_bytes) {
        Ok(value) => value.book,
        Err(error) => {
            return RoundTripReport {
                checked: true,
                passed: false,
                unexpected_losses: vec![format!(
                    "output could not be imported for semantic round-trip: {error}"
                )],
                allowed_losses: Vec::new(),
                ruby_source: source.ruby_projections(),
                ruby_round_trip: Vec::new(),
            };
        }
    };
    let expected = normalized_semantic_text_for_target(source, target);
    let actual = normalized_semantic_text(&imported);
    let mut unexpected_losses = Vec::new();
    let mut allowed_losses = Vec::new();
    if matches!(target, Target::KF7 | Target::KF7KF8Combo) && contains_image_alt(source) {
        allowed_losses.push(
            "KF7 target projection omits image/SVG alt text from the strict visible-text token budget"
                .to_owned(),
        );
    }
    if matches!(target, Target::KF7 | Target::KF7KF8Combo | Target::KF8) && contains_math(source) {
        allowed_losses.push(
            "legacy Kindle target projection may expose MathML through an accessible text fallback"
                .to_owned(),
        );
    }
    if !expected.is_empty() && !tokens_preserved(&expected, &actual) {
        let missing =
            first_missing_token(&expected, &actual).unwrap_or_else(|| "unknown".to_owned());
        unexpected_losses.push(format!(
            "normalized text content changed unexpectedly (expected {} bytes, actual {} bytes, first missing token: {missing})",
            expected.len(), actual.len()
        ));
    }
    let ruby_source = source.ruby_projections();
    let ruby_round_trip = imported.ruby_projections();
    if ruby_source != ruby_round_trip {
        unexpected_losses.push(format!(
            "ruby semantic projection changed unexpectedly (source nodes: {}, round-trip nodes: {})",
            ruby_source.len(),
            ruby_round_trip.len()
        ));
    }
    if source.metadata.display_title() != imported.metadata.display_title() {
        unexpected_losses.push("metadata title changed unexpectedly".to_owned());
    }
    RoundTripReport {
        checked: true,
        passed: unexpected_losses.is_empty(),
        unexpected_losses,
        allowed_losses,
        ruby_source,
        ruby_round_trip,
    }
}

fn first_missing_token(expected: &str, actual: &str) -> Option<String> {
    let expected = expected.split_whitespace().collect::<Vec<_>>();
    let actual = actual.split_whitespace().collect::<Vec<_>>();
    let mut cursor = 0usize;
    for token in expected {
        let Some(relative) = actual
            .get(cursor..)?
            .iter()
            .position(|value| *value == token)
        else {
            return Some(token.to_owned());
        };
        cursor = cursor.saturating_add(relative + 1);
    }
    None
}

fn import_round_trip_bytes(target: Target, output_bytes: &[u8]) -> Result<ImportedBook, CoreError> {
    if target != Target::EPUB {
        let synthetic_path = PathBuf::from(format!("roundtrip.{}", target.extension()));
        return import_bytes(&synthetic_path, output_bytes);
    }
    let temporary_root = folioforge_temp_root();
    fs::create_dir_all(&temporary_root)?;
    let temporary = temporary_root.join(format!(
        "roundtrip-{}-{}.epub",
        std::process::id(),
        ROUND_TRIP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(output_bytes)?;
        file.sync_all()?;
        drop(file);
        import_path(&temporary)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

/// Conversion scratch data is controlled by the caller. The environment
/// override keeps CI and packaged clients deterministic; the fallback is the
/// host temporary directory for ordinary library callers.
fn folioforge_temp_root() -> PathBuf {
    if let Some(value) = std::env::var_os("FOLIOFORGE_TEMP_ROOT") {
        return PathBuf::from(value);
    }
    std::env::temp_dir().join(".folioforge-runtime")
}

fn tokens_preserved(expected: &str, actual: &str) -> bool {
    let expected = expected.split_whitespace().collect::<Vec<_>>();
    let actual = actual.split_whitespace().collect::<Vec<_>>();
    if expected.is_empty() {
        return true;
    }
    let mut cursor = 0usize;
    for token in expected {
        let Some(relative) = actual[cursor..].iter().position(|value| *value == token) else {
            return false;
        };
        cursor = cursor.saturating_add(relative + 1);
    }
    true
}

fn normalized_semantic_text(book: &folio_model::Book) -> String {
    normalized_semantic_text_with(book, Target::EPUB)
}

fn normalized_semantic_text_for_target(book: &folio_model::Book, target: Target) -> String {
    normalized_semantic_text_with(book, target)
}

fn normalized_semantic_text_with(book: &folio_model::Book, target: Target) -> String {
    let mut text = String::new();
    for document in &book.documents {
        for node in &document.nodes {
            append_target_visible_text(node, target, &mut text);
        }
        text.push('\n');
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn append_target_visible_text(node: &Node, target: Target, output: &mut String) {
    if matches!(target, Target::KF7 | Target::KF7KF8Combo)
        && matches!(node.kind, NodeKind::Image { .. } | NodeKind::Svg { .. })
    {
        return;
    }
    if let Some(ruby) = node.ruby_projection() {
        output.push_str(&ruby.base);
        output.push_str(&ruby.annotation);
        return;
    }
    match &node.kind {
        NodeKind::Text { value } => output.push_str(value),
        NodeKind::Image { alt, .. } | NodeKind::Svg { alt, .. } if !alt.is_empty() => {
            output.push_str(alt)
        }
        NodeKind::Math { alt: Some(alt), .. } => output.push_str(alt),
        NodeKind::PageBreak => output.push('\n'),
        _ => {}
    }
    for child in &node.children {
        append_target_visible_text(child, target, output);
    }
    if matches!(
        node.kind,
        NodeKind::Section
            | NodeKind::Paragraph
            | NodeKind::Heading { .. }
            | NodeKind::BlockQuote
            | NodeKind::Preformatted
            | NodeKind::ListItem
            | NodeKind::GenericBlock { .. }
    ) {
        output.push('\n');
    }
}

fn contains_image_alt(book: &folio_model::Book) -> bool {
    fn contains(nodes: &[Node]) -> bool {
        nodes.iter().any(|node| {
            matches!(
                &node.kind,
                NodeKind::Image { alt, .. } | NodeKind::Svg { alt, .. } if !alt.is_empty()
            ) || contains(&node.children)
        })
    }
    book.documents
        .iter()
        .any(|document| contains(&document.nodes))
}

fn contains_math(book: &folio_model::Book) -> bool {
    fn contains(nodes: &[Node]) -> bool {
        nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Math { .. }) || contains(&node.children))
    }
    book.documents
        .iter()
        .any(|document| contains(&document.nodes))
}

fn resource_summary(book: &folio_model::Book) -> ResourceSummary {
    let mut summary = ResourceSummary {
        total: book.resources.len(),
        ..ResourceSummary::default()
    };
    for resource in &book.resources {
        match &resource.kind {
            ResourceKind::Jpeg | ResourceKind::Png | ResourceKind::Gif | ResourceKind::Svg => {
                summary.images += 1
            }
            ResourceKind::Font => summary.fonts += 1,
            ResourceKind::Stylesheet => summary.stylesheets += 1,
            ResourceKind::Audio => summary.audio += 1,
            ResourceKind::Unknown => {}
        }
        summary.declared_bytes = summary
            .declared_bytes
            .saturating_add(resource.size.unwrap_or_default());
    }
    summary
}

fn write_atomic<F>(
    output: &Path,
    bytes: &[u8],
    cancellation: &CancellationToken,
    progress: &mut F,
) -> Result<(), CoreError>
where
    F: FnMut(ProgressEvent),
{
    let mut sink = AtomicFileSink::new(output)?;
    let total = bytes.len() as u64;
    let mut written = 0u64;
    for chunk in bytes.chunks(1 << 20) {
        cancellation.check()?;
        sink.write_chunk(chunk)?;
        written = written.saturating_add(chunk.len() as u64);
        emit(
            progress,
            ProgressStage::Writing,
            written,
            Some(total),
            "atomically writing output",
        );
    }
    if bytes.is_empty() {
        emit(
            progress,
            ProgressStage::Writing,
            0,
            Some(total),
            "atomically writing output",
        );
    }
    cancellation.check()?;
    sink.finalize()
}

fn emit<F>(
    progress: &mut F,
    stage: ProgressStage,
    current: u64,
    total: Option<u64>,
    message: impl Into<String>,
) where
    F: FnMut(ProgressEvent),
{
    let fraction = total
        .filter(|value| *value > 0)
        .map(|value| (current as f32 / value as f32).clamp(0.0, 1.0));
    progress(ProgressEvent {
        stage,
        current,
        total,
        fraction,
        message: message.into(),
    });
}

fn validate_generated(target: Target, bytes: &[u8]) -> folio_validator::ValidationReport {
    match target {
        Target::EPUB => match folio_epub::validate_bytes(bytes) {
            Ok(()) => folio_validator::ValidationReport::default(),
            Err(error) => folio_validator::ValidationReport {
                diagnostics: vec![Diagnostic::error("FF-EPUB-BIN-0001", error.to_string())],
            },
        },
        Target::KF7 => folio_validator::validate_mobi(bytes),
        Target::KF8 => folio_validator::validate_kf8(bytes),
        Target::KFX => folio_validator::validate_kfx(bytes),
        Target::KF7KF8Combo => folio_validator::validate_combo(bytes),
    }
}

fn ensure_input(path: &Path) -> Result<(), CoreError> {
    if !path.is_file() && !path.is_dir() {
        return Err(CoreError::MissingInput(path.to_owned()));
    }
    Ok(())
}

fn read_file(path: &Path) -> Result<Vec<u8>, CoreError> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_observable_without_a_gui() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
        assert!(matches!(token.check(), Err(CoreError::Cancelled)));
    }

    #[test]
    fn target_extensions_are_stable() {
        assert_eq!(Target::KF7.extension(), "mobi");
        assert_eq!(Target::KF8.extension(), "azw3");
        assert_eq!(Target::KFX.extension(), "kfx");
    }

    #[test]
    fn kfx_resource_identity_diagnostics_keep_their_codes_and_context() {
        let report = InputReport {
            resource_identity_diagnostics: vec![KfxResourceDiagnostic {
                code: "KFX-R002".to_owned(),
                message: "No exact raw media match.".to_owned(),
                location: Some("resource/example.jpg".to_owned()),
                location_symbol_id: Some(77),
                location_source: "container_symbol_table".to_owned(),
                normalized_location: Some("resource/example.jpg".to_owned()),
                location_field_id: 165,
                input_index: 1,
                container_origin: 3,
                external_resource_entity_id: 99,
                body_reference_count: 7,
                visible_placement_count: 2,
            }],
            ..InputReport::default()
        };

        let diagnostics = diagnostics_from_input(&report);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "KFX-R002");
        assert_eq!(
            diagnostics[0].context.get("visible_placement_count"),
            Some(&"2".to_owned())
        );
        assert_eq!(
            diagnostics[0].context.get("location"),
            Some(&"resource/example.jpg".to_owned())
        );
    }

    #[test]
    fn txt_import_enters_the_shared_ir_and_preserves_structure_evidence() {
        let imported = import_bytes(
            Path::new("《测试书》作者：测试作者.txt"),
            "第一卷\n\n第一章 初遇\n\n这是正文。\n\n第二章 再会\n\n这是下一段。".as_bytes(),
        )
        .unwrap();
        assert_eq!(imported.format, DetectedFormat::Text);
        assert_eq!(imported.book.metadata.title.as_deref(), Some("测试书"));
        assert_eq!(imported.book.navigation.toc.len(), 1);
        assert_eq!(imported.book.navigation.toc[0].children.len(), 2);
        assert_eq!(
            imported
                .input_report
                .text
                .as_ref()
                .unwrap()
                .encoding
                .selected,
            Some(folio_text::TextEncoding::Utf8)
        );
        assert!(folio_validator::validate_book(&imported.book).is_valid());
    }
}
