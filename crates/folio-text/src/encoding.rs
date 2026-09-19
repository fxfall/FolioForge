use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextEncoding {
    #[serde(rename = "utf8")]
    Utf8,
    #[serde(rename = "utf16_le")]
    Utf16Le,
    #[serde(rename = "utf16_be")]
    Utf16Be,
    #[serde(rename = "gb18030")]
    Gb18030,
    #[serde(rename = "big5")]
    Big5,
    #[serde(rename = "shift_jis")]
    ShiftJis,
    #[serde(rename = "windows_1252")]
    Windows1252,
}

impl TextEncoding {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf16Le => "UTF-16LE",
            Self::Utf16Be => "UTF-16BE",
            Self::Gb18030 => "GB18030",
            Self::Big5 => "Big5",
            Self::ShiftJis => "Shift_JIS",
            Self::Windows1252 => "Windows-1252",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EncodingConfidence {
    High,
    Medium,
    #[default]
    Low,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EncodingEvidence {
    ByteOrderMark,
    StrictUtf8,
    Utf16NullBytePattern,
    NoDecodeErrors,
    LanguageScriptMatch,
    CompetingCandidates,
    TextLikeCharacterDistribution,
    UserOverride,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EncodingCandidate {
    pub encoding: TextEncoding,
    pub score: i32,
    pub evidence: Vec<EncodingEvidence>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct EncodingDetection {
    /// `None` means the byte evidence is ambiguous; callers must ask for an
    /// override instead of silently choosing a legacy encoding.
    pub selected: Option<TextEncoding>,
    pub confidence: EncodingConfidence,
    pub candidates: Vec<EncodingCandidate>,
    pub evidence: Vec<EncodingEvidence>,
}
