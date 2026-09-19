use serde::{Deserialize, Serialize};

use crate::TextEncoding;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextImportMode {
    #[default]
    Auto,
    Novel,
    Markdown,
    Plain,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParagraphMode {
    #[default]
    Auto,
    BlankLine,
    EveryLine,
    Indented,
    HardWrap,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TextImportOptions {
    pub mode: TextImportMode,
    pub paragraph_mode: ParagraphMode,
    pub encoding_override: Option<TextEncoding>,
    pub title_override: Option<String>,
    pub author_override: Option<String>,
}
