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

#[cfg(test)]
mod tests {
    use encoding_rs::{BIG5, GB18030, SHIFT_JIS, WINDOWS_1252};

    use super::*;

    fn encoded(encoding: &'static encoding_rs::Encoding, value: &str) -> Vec<u8> {
        let (bytes, _, had_errors) = encoding.encode(value);
        assert!(!had_errors);
        bytes.into_owned()
    }

    #[test]
    fn strict_utf8_and_bom_have_high_confidence() {
        let utf8 = detect("这是一段中文正文".as_bytes()).unwrap();
        assert_eq!(utf8.selected, Some(TextEncoding::Utf8));
        assert_eq!(utf8.confidence, EncodingConfidence::High);
        let bom = detect(b"\xef\xbb\xbfhello").unwrap();
        assert_eq!(bom.selected, Some(TextEncoding::Utf8));
        assert!(bom.evidence.contains(&EncodingEvidence::ByteOrderMark));
    }

    #[test]
    fn text_encoding_json_names_are_a_stable_client_contract() {
        let values = [
            (TextEncoding::Utf8, "utf8"),
            (TextEncoding::Utf16Le, "utf16_le"),
            (TextEncoding::Utf16Be, "utf16_be"),
            (TextEncoding::Gb18030, "gb18030"),
            (TextEncoding::Big5, "big5"),
            (TextEncoding::ShiftJis, "shift_jis"),
            (TextEncoding::Windows1252, "windows_1252"),
        ];
        for (encoding, expected) in values {
            let json = serde_json::to_string(&encoding).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
            assert_eq!(
                serde_json::from_str::<TextEncoding>(&json).unwrap(),
                encoding
            );
        }
    }

    #[test]
    fn detects_utf16_endianness_from_bom_and_without_bom() {
        let le = [0xff, 0xfe, b'h', 0, b'i', 0];
        let be = [0xfe, 0xff, 0, b'h', 0, b'i'];
        assert_eq!(detect(&le).unwrap().selected, Some(TextEncoding::Utf16Le));
        assert_eq!(detect(&be).unwrap().selected, Some(TextEncoding::Utf16Be));
        let no_bom = [b'h', 0, b'e', 0, b'l', 0, b'o', 0];
        assert_eq!(
            detect(&no_bom).unwrap().selected,
            Some(TextEncoding::Utf16Le)
        );
    }

    #[test]
    fn decodes_required_legacy_encodings() {
        let cases = [
            (GB18030, TextEncoding::Gb18030, "这是一本小说"),
            (BIG5, TextEncoding::Big5, "這是一本小說"),
            (SHIFT_JIS, TextEncoding::ShiftJis, "これは小説です"),
            (WINDOWS_1252, TextEncoding::Windows1252, "Café déjà vu"),
        ];
        for (codec, label, source) in cases {
            let bytes = encoded(codec, source);
            let decoded = decode_as(&bytes, label).unwrap();
            assert_eq!(decoded, source);
            let report = detect(&bytes).unwrap();
            assert!(report.candidates.iter().any(|item| item.encoding == label));
        }
    }

    #[test]
    fn likely_gb18030_big5_and_shift_jis_are_reported_with_evidence() {
        let simplified = encoded(GB18030, "这是一本中文小说，讲述一个故事。第一章开始。");
        let traditional = encoded(BIG5, "這是一本中文小說，講述一個故事。第一章開始。");
        let japanese = encoded(SHIFT_JIS, "これは日本語の小説です。第一章の始まり。");
        let detected = detect(&simplified).unwrap();
        assert_eq!(
            detected.selected,
            Some(TextEncoding::Gb18030),
            "detection result: {detected:?}"
        );
        assert_eq!(
            detect(&traditional).unwrap().selected,
            Some(TextEncoding::Big5)
        );
        assert_eq!(
            detect(&japanese).unwrap().selected,
            Some(TextEncoding::ShiftJis)
        );
    }

    #[test]
    fn ambiguous_legacy_bytes_are_not_silently_selected() {
        let report = detect(&[0x81, 0x40, 0x81, 0x41, 0x81, 0x42]).unwrap();
        assert_eq!(
            report.confidence,
            EncodingConfidence::Low,
            "detection result: {report:?}"
        );
        assert_eq!(report.selected, None);
    }

    #[test]
    fn rejects_binary_nuls_and_invalid_forced_sequences() {
        assert!(matches!(
            detect(b"PK\x00\x01"),
            Err(DetectionError::BinaryLike)
        ));
        assert!(decode_as(&[0xff, 0xfe, 0x00], TextEncoding::Utf16Le).is_err());
    }

    #[test]
    fn normalization_only_changes_bom_and_newline_forms() {
        let (normalized, report) = normalize_text("\u{feff}a\r\nb\rc\u{2028}d\n\n ");
        assert_eq!(normalized, "a\nb\nc\nd\n\n ");
        assert!(report.removed_leading_bom);
        assert_eq!(report.newline_sequences_normalized, 3);
    }
}
