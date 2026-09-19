//! Stable input-format adapter contracts.
//!
//! An adapter stops at the format-neutral semantic book.  It never chooses a
//! target fallback, applies an edit plan, writes a package, or talks to a
//! client.  The concrete adapters in this crate cover formats that already
//! are implemented by the format crates without adding parser branches to
//! `folio-core`.

use std::{collections::BTreeMap, fs, path::Path};

use folio_input::{BookSource, DetectedFormat, SourceKind};
use folio_model::{Book, Diagnostic, Metadata};
use folio_text::TextImportOptions;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable registry identity.  The input crate owns the canonical enum so
/// existing Core/FFI callers keep one source of truth while adapters can refer
/// to the contract using the specification's `FormatId` name.
pub type FormatId = DetectedFormat;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionConfidence {
    Exact,
    Strong,
    #[default]
    Heuristic,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetectionResult {
    pub format: String,
    pub confidence: DetectionConfidence,
    pub evidence: Vec<String>,
    pub warnings: Vec<String>,
}

impl DetectionResult {
    pub fn exact(format: DetectedFormat, evidence: impl Into<String>) -> Self {
        Self {
            format: format.name().to_owned(),
            confidence: DetectionConfidence::Exact,
            evidence: vec![evidence.into()],
            warnings: Vec::new(),
        }
    }
}

/// Capabilities describe an input adapter, not a target writer.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormatSupport {
    pub detect: bool,
    pub inspect: bool,
    pub import: bool,
    pub export: bool,
    pub metadata_read: bool,
    pub metadata_write: bool,
    pub edit: bool,
    pub preview: bool,
}

/// Stable input-side capability contract. This is intentionally separate
/// from target `CapabilityProfile`: it describes what an input adapter can
/// do, not what a target format can express.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AdapterCapabilities {
    pub detect: bool,
    pub inspect: bool,
    pub import: bool,
    pub export: bool,
    pub metadata_read: bool,
    pub metadata_write: bool,
    pub edit: bool,
    pub preview: bool,
}

impl From<FormatSupport> for AdapterCapabilities {
    fn from(value: FormatSupport) -> Self {
        Self {
            detect: value.detect,
            inspect: value.inspect,
            import: value.import,
            export: value.export,
            metadata_read: value.metadata_read,
            metadata_write: value.metadata_write,
            edit: value.edit,
            preview: value.preview,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ImportContext {
    pub strict: bool,
    #[serde(default)]
    pub text: TextImportOptions,
}

pub struct ImportedBook {
    pub format: DetectedFormat,
    pub parser: String,
    pub book: Book,
    pub diagnostics: Vec<Diagnostic>,
    pub input_loss: Vec<String>,
    pub text: Option<folio_text::TextImportReport>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct StructureSummary {
    pub document_count: usize,
    pub node_count: usize,
    pub resource_count: usize,
    pub anchor_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CoverSummary {
    pub resource_id: u32,
    pub path: String,
    pub media_type: String,
}

/// Format-neutral inspection result shared by Core and clients. It is derived
/// from the same imported semantic book as `import()` and never runs an
/// exporter.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct InspectionReport {
    pub format: String,
    pub metadata: Metadata,
    pub cover: Option<CoverSummary>,
    pub structure_summary: StructureSummary,
    pub features: BTreeMap<String, usize>,
    pub diagnostics: Vec<Diagnostic>,
}

impl InspectionReport {
    fn from_imported(imported: &ImportedBook) -> Self {
        Self {
            format: imported.format.name().to_owned(),
            metadata: imported.book.metadata.clone(),
            cover: find_cover(&imported.book),
            structure_summary: structure_summary(&imported.book),
            features: imported.book.feature_summary(),
            diagnostics: imported.diagnostics.clone(),
        }
    }
}

pub trait FormatAdapter: Send + Sync {
    /// Stable registry identity. `format()` remains the source-compatible
    /// spelling used by older callers.
    fn id(&self) -> FormatId {
        self.format()
    }
    fn input_formats(&self) -> Vec<FormatId> {
        vec![self.id()]
    }
    fn format(&self) -> DetectedFormat;
    fn support(&self) -> FormatSupport;
    fn capabilities(&self) -> AdapterCapabilities {
        self.support().into()
    }
    fn detect(&self, source: &BookSource) -> Result<DetectionResult, FormatError>;
    /// Inspect without exporting. The default implementation deliberately
    /// reuses the adapter's import path so there is no second parser; format
    /// crates may override it when a bounded native inspection is available.
    fn inspect(
        &self,
        source: &BookSource,
        context: &ImportContext,
    ) -> Result<InspectionReport, FormatError> {
        let imported = self.import(source, context)?;
        Ok(InspectionReport::from_imported(&imported))
    }
    fn import(
        &self,
        source: &BookSource,
        context: &ImportContext,
    ) -> Result<ImportedBook, FormatError>;
}

fn structure_summary(book: &Book) -> StructureSummary {
    StructureSummary {
        document_count: book.documents.len(),
        node_count: book
            .documents
            .iter()
            .flat_map(|document| document.nodes.iter())
            .map(count_nodes)
            .sum(),
        resource_count: book.resources.len(),
        anchor_count: book.anchors.len(),
    }
}

fn count_nodes(node: &folio_model::Node) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

fn find_cover(book: &Book) -> Option<CoverSummary> {
    book.resources
        .iter()
        .find(|resource| {
            resource
                .properties
                .iter()
                .any(|value| value == "cover-image")
        })
        .map(|resource| CoverSummary {
            resource_id: resource.id.get(),
            path: resource.path.clone(),
            media_type: resource.media_type.clone(),
        })
}

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("format adapter does not support this source: {0}")]
    Unsupported(String),
    #[error("format input is invalid: {0}")]
    Invalid(String),
    #[error("format source I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Amazon KFX adapter failed: {0}")]
    AmazonKfx(String),
    #[error("Amazon KFX input is DRM protected")]
    AmazonKfxProtected,
}

pub fn primary_file(source: &BookSource) -> Result<&Path, FormatError> {
    source
        .files()
        .first()
        .map(|file| file.path.as_path())
        .ok_or_else(|| FormatError::Unsupported("source contains no files".to_owned()))
}

pub fn detect_single(source: &BookSource) -> Result<DetectedFormat, FormatError> {
    if source.kind() != SourceKind::SingleFile {
        return Err(FormatError::Unsupported(
            "adapter expects a single input file".to_owned(),
        ));
    }
    let file = source
        .files()
        .first()
        .ok_or_else(|| FormatError::Unsupported("source contains no files".to_owned()))?;
    let bytes = fs::read(&file.path)?;
    folio_input::detect_format(&file.relative_path, &bytes)
        .map_err(|error| FormatError::Unsupported(error.to_string()))
}

pub fn supports_detected(
    source: &BookSource,
    expected: impl IntoIterator<Item = DetectedFormat>,
) -> Result<DetectionResult, FormatError> {
    let detected = detect_single(source)?;
    if expected.into_iter().any(|format| format == detected) {
        Ok(DetectionResult::exact(
            detected,
            "container signature and format detector",
        ))
    } else {
        Err(FormatError::Unsupported(format!(
            "detected {} but adapter expects another format",
            detected.name()
        )))
    }
}

fn text_import(source: &BookSource, context: &ImportContext) -> Result<ImportedBook, FormatError> {
    let file = source
        .files()
        .first()
        .ok_or_else(|| FormatError::Unsupported("source contains no files".to_owned()))?;
    let bytes = fs::read(&file.path)?;
    let source_name = file.relative_path.to_str();
    let imported = folio_text::import_bytes(&bytes, source_name, &context.text)
        .map_err(|error| FormatError::Invalid(error.to_string()))?;
    let folio_text::TextImportResult { book, report } = imported;
    let input_loss = report.input_loss.clone();
    Ok(ImportedBook {
        format: DetectedFormat::Text,
        parser: "folio-text/1".to_owned(),
        book,
        diagnostics: report
            .diagnostics
            .iter()
            .map(|message| Diagnostic::warning("FF-TXT-IMPORT-0001", message.clone()))
            .collect(),
        input_loss,
        text: Some(report),
    })
}

pub struct TextAdapter;

impl FormatAdapter for TextAdapter {
    fn format(&self) -> DetectedFormat {
        DetectedFormat::Text
    }

    fn support(&self) -> FormatSupport {
        FormatSupport {
            detect: true,
            inspect: true,
            import: true,
            metadata_read: true,
            metadata_write: true,
            edit: true,
            preview: true,
            ..FormatSupport::default()
        }
    }

    fn detect(&self, source: &BookSource) -> Result<DetectionResult, FormatError> {
        supports_detected(source, [DetectedFormat::Text])
    }

    fn import(
        &self,
        source: &BookSource,
        context: &ImportContext,
    ) -> Result<ImportedBook, FormatError> {
        text_import(source, context)
    }
}
