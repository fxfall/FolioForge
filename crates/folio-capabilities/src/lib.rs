//! Target capability declarations for the universal conversion pipeline.
//!
//! This crate is intentionally declarative.  It knows what a target can
//! express, but it does not parse or write any ebook format and it never
//! performs a fallback itself.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub enum Format {
    #[default]
    Epub3,
    Kf7,
    Kf8,
    Kfx,
}

impl Format {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Epub3 => "EPUB3",
            Self::Kf7 => "KF7",
            Self::Kf8 => "KF8",
            Self::Kfx => "KFX",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Feature {
    Text,
    Heading,
    Navigation,
    Image,
    Svg,
    EmbeddedFont,
    Ruby,
    Math,
    VerticalWriting,
    ComplexTable,
    Float,
    FixedPosition,
    FixedLayout,
    Footnote,
    Endnote,
    PageBreak,
    DropCap,
    Poetry,
    Aside,
    Link,
    SemanticStructure,
}

impl Feature {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Heading => "heading",
            Self::Navigation => "navigation",
            Self::Image => "image",
            Self::Svg => "svg",
            Self::EmbeddedFont => "embedded-font",
            Self::Ruby => "ruby",
            Self::Math => "math",
            Self::VerticalWriting => "vertical-writing",
            Self::ComplexTable => "complex-table",
            Self::Float => "float",
            Self::FixedPosition => "fixed-position",
            Self::FixedLayout => "fixed-layout",
            Self::Footnote => "footnote",
            Self::Endnote => "endnote",
            Self::PageBreak => "page-break",
            Self::DropCap => "drop-cap",
            Self::Poetry => "poetry",
            Self::Aside => "aside",
            Self::Link => "link",
            Self::SemanticStructure => "semantic-structure",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum CapabilityLevel {
    Native,
    Compatible,
    Approximate,
    Flattenable,
    Unsupported,
}

impl CapabilityLevel {
    pub const fn quality_rank(self) -> u8 {
        match self {
            Self::Native => 0,
            Self::Compatible => 1,
            Self::Approximate => 2,
            Self::Flattenable => 3,
            Self::Unsupported => 4,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CapabilityProfile {
    pub format: Format,
    pub levels: BTreeMap<Feature, CapabilityLevel>,
}

impl CapabilityProfile {
    pub fn level(&self, feature: Feature) -> CapabilityLevel {
        self.levels
            .get(&feature)
            .copied()
            .unwrap_or(CapabilityLevel::Unsupported)
    }

    pub fn supports(&self, feature: Feature) -> bool {
        !matches!(self.level(feature), CapabilityLevel::Unsupported)
    }

    pub fn as_matrix_row(&self) -> BTreeMap<String, String> {
        self.levels
            .iter()
            .map(|(feature, level)| (feature.name().to_owned(), format!("{level:?}")))
            .collect()
    }
}

fn profile(format: Format, entries: &[(Feature, CapabilityLevel)]) -> CapabilityProfile {
    CapabilityProfile {
        format,
        levels: entries.iter().copied().collect(),
    }
}

/// The single source of truth for the public compatibility matrix.
pub fn profile_for(format: Format) -> CapabilityProfile {
    use CapabilityLevel::{Approximate, Compatible, Flattenable, Native, Unsupported};
    use Feature::*;
    match format {
        Format::Epub3 => profile(
            format,
            &[
                (Text, Native),
                (Heading, Native),
                (Navigation, Native),
                (Image, Native),
                (Svg, Native),
                (EmbeddedFont, Native),
                (Ruby, Native),
                (Math, Native),
                (VerticalWriting, Native),
                (ComplexTable, Native),
                (Float, Native),
                (FixedPosition, Native),
                (FixedLayout, Native),
                (Footnote, Native),
                (Endnote, Native),
                (PageBreak, Native),
                (DropCap, Native),
                (Poetry, Native),
                (Aside, Native),
                (Link, Native),
                (SemanticStructure, Native),
            ],
        ),
        Format::Kf7 => profile(
            format,
            &[
                (Text, Native),
                (Heading, Native),
                (Navigation, Compatible),
                (Image, Compatible),
                (Svg, Approximate),
                (EmbeddedFont, Approximate),
                (Ruby, Approximate),
                (Math, Approximate),
                (VerticalWriting, Unsupported),
                (ComplexTable, Flattenable),
                (Float, Flattenable),
                (FixedPosition, Flattenable),
                (FixedLayout, Flattenable),
                (Footnote, Approximate),
                (Endnote, Approximate),
                (PageBreak, Native),
                (DropCap, Approximate),
                (Poetry, Compatible),
                (Aside, Approximate),
                (Link, Compatible),
                (SemanticStructure, Compatible),
            ],
        ),
        Format::Kf8 => profile(
            format,
            &[
                (Text, Native),
                (Heading, Native),
                (Navigation, Compatible),
                (Image, Native),
                (Svg, Native),
                (EmbeddedFont, Native),
                (Ruby, Compatible),
                (Math, Approximate),
                (VerticalWriting, Compatible),
                (ComplexTable, Compatible),
                (Float, Compatible),
                (FixedPosition, Compatible),
                (FixedLayout, Compatible),
                (Footnote, Compatible),
                (Endnote, Compatible),
                (PageBreak, Native),
                (DropCap, Compatible),
                (Poetry, Compatible),
                (Aside, Compatible),
                (Link, Native),
                (SemanticStructure, Native),
            ],
        ),
        Format::Kfx => profile(
            format,
            &[
                (Text, Native),
                (Heading, Native),
                (Navigation, Native),
                (Image, Native),
                (Svg, Native),
                (EmbeddedFont, Native),
                (Ruby, Native),
                (Math, Native),
                (VerticalWriting, Native),
                (ComplexTable, Compatible),
                (Float, Native),
                (FixedPosition, Native),
                (FixedLayout, Native),
                (Footnote, Native),
                (Endnote, Native),
                (PageBreak, Native),
                (DropCap, Native),
                (Poetry, Native),
                (Aside, Native),
                (Link, Native),
                (SemanticStructure, Native),
            ],
        ),
    }
}

pub fn all_profiles() -> [CapabilityProfile; 4] {
    [
        profile_for(Format::Epub3),
        profile_for(Format::Kf7),
        profile_for(Format::Kf8),
        profile_for(Format::Kfx),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_has_one_profile_per_target() {
        let profiles = all_profiles();
        assert_eq!(profiles.len(), 4);
        assert_eq!(
            profile_for(Format::Kf7).level(Feature::Ruby),
            CapabilityLevel::Approximate
        );
        assert_eq!(
            profile_for(Format::Epub3).level(Feature::Svg),
            CapabilityLevel::Native
        );
    }
}
