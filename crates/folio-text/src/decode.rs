use encoding_rs::{BIG5, GB18030, SHIFT_JIS, WINDOWS_1252};

use crate::{detect, DetectionError, EncodingDetection, TextEncoding};

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum DecodeError {
    #[error("text input contains no characters")]
    EmptyText,
    #[error("text encoding is ambiguous; choose an encoding override")]
    AmbiguousEncoding,
    #[error("invalid byte sequence for {0}")]
    InvalidSequence(&'static str),
    #[error("UTF-16 input has an incomplete code unit")]
    OddUtf16Length,
    #[error("input is not plausible text: {0}")]
    Detection(String),
}

impl From<DetectionError> for DecodeError {
    fn from(value: DetectionError) -> Self {
        Self::Detection(value.to_string())
    }
}

pub fn decode(bytes: &[u8]) -> Result<(String, EncodingDetection), DecodeError> {
    let detection = detect(bytes)?;
    let encoding = detection.selected.ok_or(DecodeError::AmbiguousEncoding)?;
    let text = decode_as(bytes, encoding)?;
    Ok((text, detection))
}

pub fn decode_as(bytes: &[u8], encoding: TextEncoding) -> Result<String, DecodeError> {
    let bytes = strip_matching_bom(bytes, encoding);
    let text = match encoding {
        TextEncoding::Utf8 => std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| DecodeError::InvalidSequence(encoding.label())),
        TextEncoding::Utf16Le => decode_utf16(bytes, true),
        TextEncoding::Utf16Be => decode_utf16(bytes, false),
        TextEncoding::Gb18030 => decode_legacy(bytes, GB18030, encoding),
        TextEncoding::Big5 => decode_legacy(bytes, BIG5, encoding),
        TextEncoding::ShiftJis => decode_legacy(bytes, SHIFT_JIS, encoding),
        TextEncoding::Windows1252 => decode_legacy(bytes, WINDOWS_1252, encoding),
    }?;
    validate_decoded_text(&text)?;
    Ok(text)
}

fn validate_decoded_text(text: &str) -> Result<(), DecodeError> {
    if text.is_empty() {
        return Err(DecodeError::EmptyText);
    }
    let mut count = 0usize;
    let mut controls = 0usize;
    for ch in text.chars() {
        count += 1;
        if ch == '\0' {
            return Err(DecodeError::Detection(
                "decoded content contains NUL bytes".to_owned(),
            ));
        }
        if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
            controls += 1;
        }
    }
    if controls.saturating_mul(100) > count {
        return Err(DecodeError::Detection(
            "decoded content contains too many control characters".to_owned(),
        ));
    }
    Ok(())
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, DecodeError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(DecodeError::OddUtf16Length);
    }
    let units = bytes.chunks_exact(2).map(|pair| {
        if little_endian {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        }
    });
    char::decode_utf16(units)
        .map(|item| {
            item.map_err(|_| {
                DecodeError::InvalidSequence(if little_endian {
                    TextEncoding::Utf16Le.label()
                } else {
                    TextEncoding::Utf16Be.label()
                })
            })
        })
        .collect()
}

fn decode_legacy(
    bytes: &[u8],
    encoding: &'static encoding_rs::Encoding,
    label: TextEncoding,
) -> Result<String, DecodeError> {
    let (text, had_errors) = encoding.decode_without_bom_handling(bytes);
    if had_errors {
        return Err(DecodeError::InvalidSequence(label.label()));
    }
    Ok(text.into_owned())
}

fn strip_matching_bom(bytes: &[u8], encoding: TextEncoding) -> &[u8] {
    let bom: &[u8] = match encoding {
        TextEncoding::Utf8 => &[0xef, 0xbb, 0xbf],
        TextEncoding::Utf16Le => &[0xff, 0xfe],
        TextEncoding::Utf16Be => &[0xfe, 0xff],
        _ => &[],
    };
    bytes.strip_prefix(bom).unwrap_or(bytes)
}
