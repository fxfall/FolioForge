use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct NormalizationReport {
    pub removed_leading_bom: bool,
    pub newline_sequences_normalized: usize,
}

/// Normalize only unambiguous byte/text transport conventions. Trailing
/// spaces, indentation, repeated blank lines, and NULs are deliberately left
/// untouched because they may carry poetry, code, or layout meaning.
pub fn normalize_text(source: &str) -> (String, NormalizationReport) {
    normalize_text_owned(source.to_owned())
}

/// Normalize a decoded buffer in place (apart from the owned `String` handle)
/// so large books do not require a second full-size output allocation.
pub fn normalize_text_owned(source: String) -> (String, NormalizationReport) {
    let mut bytes = source.into_bytes();
    let removed_leading_bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
    let mut read = if removed_leading_bom { 3 } else { 0 };
    let mut write = 0usize;
    let mut newline_sequences_normalized = 0usize;
    while read < bytes.len() {
        if bytes[read] == b'\r' {
            read += 1;
            if bytes.get(read) == Some(&b'\n') {
                read += 1;
            }
            bytes[write] = b'\n';
            write += 1;
            newline_sequences_normalized += 1;
        } else if bytes.get(read..read + 2) == Some(&[0xc2, 0x85]) {
            bytes[write] = b'\n';
            write += 1;
            read += 2;
            newline_sequences_normalized += 1;
        } else if bytes
            .get(read..read + 3)
            .is_some_and(|slice| slice == [0xe2, 0x80, 0xa8] || slice == [0xe2, 0x80, 0xa9])
        {
            bytes[write] = b'\n';
            write += 1;
            read += 3;
            newline_sequences_normalized += 1;
        } else {
            bytes[write] = bytes[read];
            write += 1;
            read += 1;
        }
    }
    bytes.truncate(write);
    let text = String::from_utf8(bytes).expect("normalization preserves valid UTF-8");
    (
        text,
        NormalizationReport {
            removed_leading_bom,
            newline_sequences_normalized,
        },
    )
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-text/src/normalize.rs"]
mod tests;
