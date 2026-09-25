//! Conservative text-input foundation for plain-text books.
//!
//! This crate owns text byte classification, encoding evidence, decoding, and
//! byte-order/newline normalization and conservative plain-text structure
//! analysis. It does not access the network or write target formats.

mod decode;
mod detect;
mod encoding;
mod import;
mod metadata;
mod normalize;
mod options;
mod paragraph;
mod structure;

pub use decode::{decode, decode_as, DecodeError};
pub use detect::{detect, DetectionError};
pub use encoding::{
    EncodingCandidate, EncodingConfidence, EncodingDetection, EncodingEvidence, TextEncoding,
};
pub use import::{import_bytes, TextImportError, TextImportReport, TextImportResult};
pub use metadata::{infer_metadata, MetadataSource, TextMetadataGuess};
pub use normalize::{normalize_text, normalize_text_owned, NormalizationReport};
pub use options::{ParagraphMode, TextImportMode, TextImportOptions};
pub use paragraph::{parse_paragraphs, ParsedParagraph};
pub use structure::{
    analyze_paragraphs, analyze_structure, BookStructure, ParagraphAnalysis, StructureCandidate,
    StructureKind, StructureNode,
};

/// Diagnostic detail that can be passed to clients without exposing source
/// paths or book contents.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct TextDecodeReport {
    pub detection: EncodingDetection,
    pub normalization: NormalizationReport,
    pub decoded_characters: usize,
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-text/src/lib.rs"]
mod tests;
