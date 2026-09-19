use encoding_rs::{Encoding, BIG5, GB18030, SHIFT_JIS, WINDOWS_1252};

use crate::{
    EncodingCandidate, EncodingConfidence, EncodingDetection, EncodingEvidence, TextEncoding,
};

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum DetectionError {
    #[error("empty text input")]
    Empty,
    #[error("input contains binary NUL bytes or too many control characters")]
    BinaryLike,
    #[error("no supported text encoding produced plausible text")]
    NoPlausibleEncoding,
}

#[derive(Clone, Copy)]
struct LegacyEncoding {
    label: TextEncoding,
    codec: &'static Encoding,
}

const LEGACY_ENCODINGS: [LegacyEncoding; 4] = [
    LegacyEncoding {
        label: TextEncoding::Gb18030,
        codec: GB18030,
    },
    LegacyEncoding {
        label: TextEncoding::Big5,
        codec: BIG5,
    },
    LegacyEncoding {
        label: TextEncoding::ShiftJis,
        codec: SHIFT_JIS,
    },
    LegacyEncoding {
        label: TextEncoding::Windows1252,
        codec: WINDOWS_1252,
    },
];

pub fn detect(bytes: &[u8]) -> Result<EncodingDetection, DetectionError> {
    if bytes.is_empty() {
        return Err(DetectionError::Empty);
    }

    if bytes.starts_with(&[0xff, 0xfe]) {
        return Ok(explicit(TextEncoding::Utf16Le));
    }
    if bytes.starts_with(&[0xfe, 0xff]) {
        return Ok(explicit(TextEncoding::Utf16Be));
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Ok(explicit(TextEncoding::Utf8));
    }

    if let Some(encoding) = infer_utf16_without_bom(bytes) {
        return Ok(EncodingDetection {
            selected: Some(encoding),
            confidence: EncodingConfidence::Medium,
            candidates: vec![EncodingCandidate {
                encoding,
                score: 80,
                evidence: vec![EncodingEvidence::Utf16NullBytePattern],
            }],
            evidence: vec![EncodingEvidence::Utf16NullBytePattern],
        });
    }

    if bytes.contains(&0) || too_many_ascii_controls(bytes) {
        return Err(DetectionError::BinaryLike);
    }

    if std::str::from_utf8(bytes).is_ok() {
        return Ok(EncodingDetection {
            selected: Some(TextEncoding::Utf8),
            confidence: EncodingConfidence::High,
            candidates: vec![EncodingCandidate {
                encoding: TextEncoding::Utf8,
                score: 100,
                evidence: vec![EncodingEvidence::StrictUtf8],
            }],
            evidence: vec![EncodingEvidence::StrictUtf8],
        });
    }

    let mut candidates = LEGACY_ENCODINGS
        .iter()
        .filter_map(|item| score_candidate(bytes, *item))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.encoding.label().cmp(right.encoding.label()))
    });
    let Some(best) = candidates.first() else {
        return Err(DetectionError::NoPlausibleEncoding);
    };

    let margin = candidates
        .get(1)
        .map_or(i32::MAX, |second| best.score - second.score);
    let has_language_evidence = best
        .evidence
        .contains(&EncodingEvidence::LanguageScriptMatch);
    let (selected, confidence) = if margin >= 12 && has_language_evidence {
        (Some(best.encoding), EncodingConfidence::Medium)
    } else {
        (None, EncodingConfidence::Low)
    };
    let mut evidence = vec![EncodingEvidence::NoDecodeErrors];
    if best
        .evidence
        .contains(&EncodingEvidence::LanguageScriptMatch)
        && margin >= 12
    {
        evidence.push(EncodingEvidence::LanguageScriptMatch);
    }
    if margin < 12 {
        evidence.push(EncodingEvidence::CompetingCandidates);
    }
    if best
        .evidence
        .contains(&EncodingEvidence::TextLikeCharacterDistribution)
    {
        evidence.push(EncodingEvidence::TextLikeCharacterDistribution);
    }

    Ok(EncodingDetection {
        selected,
        confidence,
        candidates,
        evidence,
    })
}

fn explicit(encoding: TextEncoding) -> EncodingDetection {
    EncodingDetection {
        selected: Some(encoding),
        confidence: EncodingConfidence::High,
        candidates: vec![EncodingCandidate {
            encoding,
            score: 100,
            evidence: vec![EncodingEvidence::ByteOrderMark],
        }],
        evidence: vec![EncodingEvidence::ByteOrderMark],
    }
}

fn infer_utf16_without_bom(bytes: &[u8]) -> Option<TextEncoding> {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let pairs = bytes.as_chunks::<2>().0;
    if pairs.len() < 4 {
        return None;
    }
    let even_zeroes = pairs.iter().filter(|pair| pair[0] == 0).count();
    let odd_zeroes = pairs.iter().filter(|pair| pair[1] == 0).count();
    let threshold = pairs.len().saturating_mul(3).saturating_add(3) / 4;
    if odd_zeroes >= threshold && even_zeroes <= pairs.len() / 10 {
        Some(TextEncoding::Utf16Le)
    } else if even_zeroes >= threshold && odd_zeroes <= pairs.len() / 10 {
        Some(TextEncoding::Utf16Be)
    } else {
        None
    }
}

fn too_many_ascii_controls(bytes: &[u8]) -> bool {
    let controls = bytes
        .iter()
        .filter(|byte| matches!(**byte, 0x00..=0x08 | 0x0b | 0x0c | 0x0e..=0x1f))
        .count();
    controls.saturating_mul(100) > bytes.len().max(1)
}

fn score_candidate(bytes: &[u8], candidate: LegacyEncoding) -> Option<EncodingCandidate> {
    let (decoded, had_errors) = candidate.codec.decode_without_bom_handling(bytes);
    if had_errors || decoded.is_empty() {
        return None;
    }
    let mut total = 0usize;
    let mut controls = 0usize;
    let mut letters = 0usize;
    let mut han = 0usize;
    let mut kana = 0usize;
    let mut simplified = 0usize;
    let mut traditional = 0usize;
    let mut latin_extended = 0usize;
    let mut latin_letters = 0usize;
    let mut spaces = 0usize;
    for ch in decoded.chars() {
        total += 1;
        if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
            controls += 1;
        }
        if ch.is_whitespace() {
            spaces += 1;
        }
        if ch.is_alphabetic() || ch.is_numeric() {
            letters += 1;
        }
        if is_han(ch) {
            han += 1;
        }
        if is_kana(ch) {
            kana += 1;
        }
        if SIMPLIFIED_HINTS.contains(ch) {
            simplified += 1;
        }
        if TRADITIONAL_HINTS.contains(ch) {
            traditional += 1;
        }
        if ('\u{00c0}'..='\u{024f}').contains(&ch) {
            latin_extended += 1;
        }
        if ch.is_alphabetic() && ch.is_ascii()
            || ch.is_alphabetic() && ('\u{00c0}'..='\u{024f}').contains(&ch)
        {
            latin_letters += 1;
        }
    }
    if controls.saturating_mul(100) > total
        || letters.saturating_mul(100) < total.saturating_mul(35)
    {
        return None;
    }

    let mut score = 50i32;
    let mut evidence = vec![
        EncodingEvidence::NoDecodeErrors,
        EncodingEvidence::TextLikeCharacterDistribution,
    ];
    match candidate.label {
        TextEncoding::Gb18030 => {
            if simplified > traditional {
                score += (simplified - traditional).min(8) as i32 * 8;
                evidence.push(EncodingEvidence::LanguageScriptMatch);
            }
            if han > 0 {
                score += 8;
            }
        }
        TextEncoding::Big5 => {
            if traditional > simplified {
                score += (traditional - simplified).min(8) as i32 * 8;
                evidence.push(EncodingEvidence::LanguageScriptMatch);
            }
            if han > 0 {
                score += 8;
            }
        }
        TextEncoding::ShiftJis => {
            if kana > 0 {
                score += (kana.min(30) as i32) * 2;
                evidence.push(EncodingEvidence::LanguageScriptMatch);
            }
            if han > 0 {
                score += 4;
            }
        }
        TextEncoding::Windows1252 => {
            if latin_extended > 0 && latin_letters.saturating_mul(100) >= total.saturating_mul(45) {
                score += (latin_extended.min(5) as i32) * 2;
                evidence.push(EncodingEvidence::LanguageScriptMatch);
            }
            if spaces.saturating_mul(100) >= total.saturating_mul(4) {
                score += 14;
            } else if han > 0 || kana > 0 || latin_extended > 2 {
                score -= 16;
            }
            if han > 0 || kana > 0 {
                score -= 20;
            }
        }
        TextEncoding::Utf8 | TextEncoding::Utf16Le | TextEncoding::Utf16Be => {}
    }

    Some(EncodingCandidate {
        encoding: candidate.label,
        score,
        evidence,
    })
}

fn is_han(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff)
}

fn is_kana(ch: char) -> bool {
    matches!(ch as u32, 0x3040..=0x30ff | 0x31f0..=0x31ff)
}

const SIMPLIFIED_HINTS: &str =
    "这们为与汉国门书说来时个后里发见学会对开关长东万实点听写读话体当样从现过无应还进业结号报";
const TRADITIONAL_HINTS: &str =
    "這們為與漢國門書說來時個後裡發見學會對開關長東萬實點聽寫讀話體當樣從現過無應還進業結號報";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_bomless_utf16_only_for_clear_lane_pattern() {
        assert_eq!(
            infer_utf16_without_bom(&[b'a', 0, b'b', 0, b'c', 0, b'd', 0]),
            Some(TextEncoding::Utf16Le)
        );
        assert_eq!(
            infer_utf16_without_bom(&[0, b'a', 0, b'b', 0, b'c', 0, b'd']),
            Some(TextEncoding::Utf16Be)
        );
        assert_eq!(infer_utf16_without_bom(b"abcd"), None);
    }
}
