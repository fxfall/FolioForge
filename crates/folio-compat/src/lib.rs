//! The single compatibility and degradation authority.
//!
//! Format writers serialize the projected semantic book they receive.  They
//! do not decide whether a feature is supported and never silently discard a
//! node or style property.

use std::collections::{BTreeMap, BTreeSet};

use folio_capabilities::{profile_for, CapabilityLevel, CapabilityProfile, Feature, Format};
use folio_model::{
    Book, Confidence, Diagnostic, LayoutMode, Node, NodeId, NodeKind, PresentationFeature,
    ResourceId, SemanticRole, Severity, StyleId, StylePool, VariantTarget,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Comic archive target IDs exposed by the single compatibility authority.
/// This remains separate from ebook `Format`: a comic target is not a
/// reflowable-book capability profile.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ComicOutputTarget {
    Cbz,
}

impl ComicOutputTarget {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Cbz => "CBZ",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Cbz => "cbz",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Cbz => "Comic Book ZIP",
        }
    }
}

/// Comic targets whose output semantics have an implemented Core path.
/// Device-specific variants must be added only with a Core-owned profile.
pub const fn comic_output_targets() -> [ComicOutputTarget; 1] {
    [ComicOutputTarget::Cbz]
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum DegradationMode {
    Strict,
    #[default]
    Compatible,
    Readable,
}

impl DegradationMode {
    pub const fn allows(self, quality: QualityLevel) -> bool {
        match self {
            Self::Strict => matches!(quality, QualityLevel::Exact | QualityLevel::Equivalent),
            Self::Compatible => !matches!(quality, QualityLevel::Drop),
            Self::Readable => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum QualityLevel {
    #[default]
    Exact,
    Equivalent,
    CompatibleApproximation,
    StructuralFallback,
    Drop,
}

impl QualityLevel {
    pub const fn rank(self) -> u8 {
        match self {
            Self::Exact => 0,
            Self::Equivalent => 1,
            Self::CompatibleApproximation => 2,
            Self::StructuralFallback => 3,
            Self::Drop => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum CompatibilityQuality {
    #[default]
    Exact,
    High,
    Compatible,
    Reduced,
    SevereLoss,
}

impl CompatibilityQuality {
    pub const fn from_max(level: QualityLevel) -> Self {
        match level {
            QualityLevel::Exact => Self::Exact,
            QualityLevel::Equivalent => Self::High,
            QualityLevel::CompatibleApproximation => Self::Compatible,
            QualityLevel::StructuralFallback => Self::Reduced,
            QualityLevel::Drop => Self::SevereLoss,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DegradationOptions {
    /// When true, a table that the target cannot represent faithfully is
    /// converted into ordered row text.  Auto is the default behavior.
    #[serde(default)]
    pub linearize_complex_tables: bool,
    /// Reserved for a future deterministic rasterizer.  The planner records
    /// the preference but never turns it into an unverified image silently.
    #[serde(default)]
    pub prefer_rasterization: bool,
    #[serde(default)]
    pub strip_embedded_fonts: bool,
}

/// One ordered step in a feature's compatibility chain.  The planner still
/// chooses the step from the target capability; exposing the chain makes the
/// choice inspectable and keeps fallback policy testable outside writers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FallbackStep {
    pub quality: QualityLevel,
    pub representation: String,
}

/// Return the deterministic fallback chain for a semantic feature.
pub fn fallback_chain(feature: Feature) -> Vec<FallbackStep> {
    let entries: &[(QualityLevel, &str)] = match feature {
        Feature::Ruby => &[
            (QualityLevel::Exact, "native ruby"),
            (QualityLevel::Equivalent, "inline parenthetical annotation"),
            (
                QualityLevel::CompatibleApproximation,
                "plain base text plus annotation",
            ),
            (QualityLevel::Drop, "base text only"),
        ],
        Feature::Math => &[
            (QualityLevel::Exact, "native MathML"),
            (
                QualityLevel::CompatibleApproximation,
                "accessible mathematical text fallback",
            ),
            (QualityLevel::Drop, "alt text only"),
        ],
        Feature::Svg => &[
            (QualityLevel::Exact, "native SVG"),
            (
                QualityLevel::CompatibleApproximation,
                "deterministic raster image",
            ),
            (
                QualityLevel::StructuralFallback,
                "fallback image or alt text",
            ),
            (QualityLevel::Drop, "drop only without readable fallback"),
        ],
        Feature::EmbeddedFont => &[
            (QualityLevel::Exact, "native embedded font"),
            (QualityLevel::Equivalent, "compatible font encoding"),
            (QualityLevel::CompatibleApproximation, "generic font family"),
            (QualityLevel::Drop, "reader default font"),
        ],
        Feature::ComplexTable => &[
            (QualityLevel::Exact, "native table"),
            (QualityLevel::Equivalent, "simplified table"),
            (
                QualityLevel::StructuralFallback,
                "linearized rows and cells",
            ),
            (QualityLevel::Drop, "plain structured text"),
        ],
        Feature::Float | Feature::FixedPosition | Feature::FixedLayout => &[
            (QualityLevel::Exact, "native geometry"),
            (QualityLevel::Equivalent, "simplified target layout"),
            (QualityLevel::StructuralFallback, "inline flow order"),
            (QualityLevel::Drop, "drop unsupported decoration only"),
        ],
        Feature::VerticalWriting => &[
            (QualityLevel::Exact, "native vertical writing"),
            (
                QualityLevel::CompatibleApproximation,
                "horizontal writing with preserved direction",
            ),
            (QualityLevel::Drop, "plain horizontal text"),
        ],
        Feature::Footnote | Feature::Endnote => &[
            (QualityLevel::Exact, "native note relation"),
            (QualityLevel::Equivalent, "linked note marker"),
            (
                QualityLevel::CompatibleApproximation,
                "legacy linked note marker",
            ),
            (QualityLevel::Drop, "note marker only"),
        ],
        Feature::DropCap => &[
            (QualityLevel::Exact, "native drop cap"),
            (
                QualityLevel::CompatibleApproximation,
                "first-letter compatible styling",
            ),
            (QualityLevel::Drop, "ordinary paragraph text"),
        ],
        _ => &[
            (QualityLevel::Exact, "native target representation"),
            (QualityLevel::Equivalent, "compatible target representation"),
            (
                QualityLevel::CompatibleApproximation,
                "target-compatible visual approximation",
            ),
            (
                QualityLevel::StructuralFallback,
                "semantic structural fallback",
            ),
            (QualityLevel::Drop, "drop only as a last resort"),
        ],
    };
    entries
        .iter()
        .map(|(quality, representation)| FallbackStep {
            quality: *quality,
            representation: (*representation).to_owned(),
        })
        .collect()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DegradationItem {
    pub feature: Feature,
    pub source_representation: String,
    pub target_capability: CapabilityLevel,
    pub selected_fallback: String,
    pub quality: QualityLevel,
    pub reason: String,
    pub possible_alternatives: Vec<String>,
    pub node_id: Option<NodeId>,
    pub diagnostic: Diagnostic,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DegradationPlan {
    pub target: Format,
    pub mode: DegradationMode,
    #[serde(default)]
    pub options: DegradationOptions,
    pub capability: CapabilityProfile,
    pub items: Vec<DegradationItem>,
    pub diagnostics: Vec<Diagnostic>,
    pub quality: CompatibilityQuality,
    pub blocked: bool,
}

impl DegradationPlan {
    pub fn report(&self) -> DegradationReport {
        let mut report = DegradationReport {
            quality: self.quality,
            items: self.items.clone(),
            diagnostics: self.diagnostics.clone(),
            ..DegradationReport::default()
        };
        for item in &self.items {
            match item.quality {
                QualityLevel::Exact => report.exact += 1,
                QualityLevel::Equivalent => report.equivalent += 1,
                QualityLevel::CompatibleApproximation => report.approximation += 1,
                QualityLevel::StructuralFallback => report.structural_fallback += 1,
                QualityLevel::Drop => report.dropped += 1,
            }
        }
        report
    }

    pub fn validate_mode(&self) -> Result<(), CompatError> {
        if self.blocked {
            return Err(CompatError::ModeRejected {
                mode: self.mode,
                target: self.target,
                quality: self.quality,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DegradationReport {
    pub exact: usize,
    pub equivalent: usize,
    pub approximation: usize,
    pub structural_fallback: usize,
    pub dropped: usize,
    pub quality: CompatibilityQuality,
    pub items: Vec<DegradationItem>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Error)]
pub enum CompatError {
    #[error("{mode:?} mode rejects {quality:?} degradation for {target:?}")]
    ModeRejected {
        mode: DegradationMode,
        target: Format,
        quality: CompatibilityQuality,
    },
    #[error("compatibility projection failed: {0}")]
    Projection(String),
}

/// Analyze the complete semantic book before any target serialization.
pub fn plan(
    book: &Book,
    target: Format,
    mode: DegradationMode,
    options: &DegradationOptions,
) -> DegradationPlan {
    let capability = profile_for(target);
    let mut requests = Vec::new();
    if !book.navigation.toc.is_empty()
        || !book.navigation.landmarks.is_empty()
        || !book.navigation.page_list.is_empty()
        || book.navigation.start_location.is_some()
    {
        requests.push((Feature::Navigation, None, "Navigation".to_owned(), false));
    }
    if book.presentation.layout == LayoutMode::Fixed {
        requests.push((
            Feature::FixedLayout,
            None,
            "fixed-layout presentation".to_owned(),
            true,
        ));
    }
    for resource in &book.resources {
        match resource.kind {
            folio_model::ResourceKind::Svg => {
                requests.push((
                    Feature::Svg,
                    None,
                    format!("SVG resource {}", resource.path),
                    false,
                ));
            }
            folio_model::ResourceKind::Font => requests.push((
                Feature::EmbeddedFont,
                None,
                format!("embedded font {}", resource.path),
                false,
            )),
            _ => {}
        }
    }
    for document in &book.documents {
        collect_node_features(&document.nodes, book, &mut requests, options);
    }

    // BTree ordering is part of deterministic planning: the same source IR,
    // target, mode, and options always produce the same plan.
    requests.sort_by_key(|(feature, node_id, source, _)| {
        (
            *feature,
            node_id.map(NodeId::get).unwrap_or(u32::MAX),
            source.clone(),
        )
    });
    let mut items = Vec::with_capacity(requests.len());
    let mut max_quality = QualityLevel::Exact;
    for (feature, node_id, source, structural_hint) in requests {
        let capability_level = capability.level(feature);
        let (quality, fallback, reason, mut alternatives) = choose_fallback(
            feature,
            capability_level,
            book,
            node_id,
            structural_hint,
            options,
        );
        if quality != QualityLevel::Exact {
            for step in fallback_chain(feature) {
                if step.quality.rank() > quality.rank()
                    && !alternatives.contains(&step.representation)
                {
                    alternatives.push(step.representation);
                }
            }
        }
        max_quality = max_quality.max(quality);
        let diagnostic = diagnostic_for(feature, node_id, quality, &source, &fallback, &reason);
        items.push(DegradationItem {
            feature,
            source_representation: source,
            target_capability: capability_level,
            selected_fallback: fallback,
            quality,
            reason,
            possible_alternatives: alternatives,
            node_id,
            diagnostic,
        });
    }
    let quality = CompatibilityQuality::from_max(max_quality);
    let blocked = items.iter().any(|item| !mode.allows(item.quality));
    let diagnostics = items
        .iter()
        .filter(|item| item.quality != QualityLevel::Exact)
        .map(|item| item.diagnostic.clone())
        .collect();
    DegradationPlan {
        target,
        mode,
        options: options.clone(),
        capability,
        items,
        diagnostics,
        quality,
        blocked,
    }
}

fn collect_node_features(
    nodes: &[Node],
    book: &Book,
    requests: &mut Vec<(Feature, Option<NodeId>, String, bool)>,
    options: &DegradationOptions,
) {
    for node in nodes {
        let (feature, source, structural) = match node.role {
            SemanticRole::Heading => (Feature::Heading, "semantic heading", false),
            SemanticRole::Text | SemanticRole::Paragraph => (Feature::Text, "text content", false),
            SemanticRole::Footnote => (Feature::Footnote, "footnote relation", false),
            SemanticRole::Endnote => (Feature::Endnote, "endnote relation", false),
            SemanticRole::Poetry => (Feature::Poetry, "poetry semantics", false),
            SemanticRole::Aside => (Feature::Aside, "aside semantics", false),
            SemanticRole::Link => (Feature::Link, "hyperlink", false),
            SemanticRole::Image => (Feature::Image, "image", false),
            SemanticRole::Table => (Feature::ComplexTable, "table structure", true),
            SemanticRole::PageBreak => (Feature::PageBreak, "page break", false),
            SemanticRole::Ruby => (Feature::Ruby, "ruby annotation", false),
            SemanticRole::Math => (Feature::Math, "MathML expression", false),
            _ => (Feature::SemanticStructure, "semantic structure", false),
        };
        requests.push((feature, Some(node.id), source.to_owned(), structural));
        if matches!(node.kind, NodeKind::Svg { .. }) {
            requests.push((Feature::Svg, Some(node.id), "inline SVG".to_owned(), false));
        }
        if let NodeKind::Image { resource, .. } = node.kind {
            if book
                .resource(resource)
                .is_some_and(|resource| resource.kind == folio_model::ResourceKind::Svg)
            {
                requests.push((
                    Feature::Svg,
                    Some(node.id),
                    "SVG image resource".to_owned(),
                    false,
                ));
            }
        }
        if node
            .presentation
            .contains(PresentationFeature::VerticalText)
            || book
                .styles
                .get(node.style)
                .and_then(|style| style.get("writing-mode"))
                .is_some_and(|value| value != "horizontal-tb")
        {
            requests.push((
                Feature::VerticalWriting,
                Some(node.id),
                "vertical writing presentation".to_owned(),
                false,
            ));
        }
        if node.presentation.contains(PresentationFeature::DropCap) {
            requests.push((
                Feature::DropCap,
                Some(node.id),
                "drop cap presentation".to_owned(),
                false,
            ));
        }
        if node
            .presentation
            .contains(PresentationFeature::FixedPosition)
            || book
                .styles
                .get(node.style)
                .and_then(|style| style.get("position"))
                .is_some_and(|value| value == "fixed" || value == "absolute")
        {
            requests.push((
                Feature::FixedPosition,
                Some(node.id),
                "fixed-position presentation".to_owned(),
                true,
            ));
        }
        if book
            .styles
            .get(node.style)
            .and_then(|style| style.get("float"))
            .is_some_and(|value| value != "none")
        {
            requests.push((
                Feature::Float,
                Some(node.id),
                "floating layout".to_owned(),
                true,
            ));
        }
        let _ = options;
        collect_node_features(&node.children, book, requests, options);
    }
}

fn choose_fallback(
    feature: Feature,
    capability: CapabilityLevel,
    book: &Book,
    node_id: Option<NodeId>,
    structural_hint: bool,
    options: &DegradationOptions,
) -> (QualityLevel, String, String, Vec<String>) {
    use CapabilityLevel::*;
    if feature == Feature::EmbeddedFont && options.strip_embedded_fonts {
        return (
            QualityLevel::CompatibleApproximation,
            "generic family with retained weight/style".to_owned(),
            "embedded font stripping was requested explicitly".to_owned(),
            Vec::new(),
        );
    }
    if matches!(capability, Native) {
        return (
            QualityLevel::Exact,
            "native target representation".to_owned(),
            "target natively expresses the semantic feature".to_owned(),
            Vec::new(),
        );
    }
    if matches!(capability, Compatible) {
        return (
            QualityLevel::Equivalent,
            "compatible target representation".to_owned(),
            "target representation differs internally but preserves reading meaning".to_owned(),
            Vec::new(),
        );
    }
    if feature == Feature::Svg
        && matches!(capability, Approximate | Unsupported)
        && node_id
            .and_then(|id| find_node(book, id))
            .is_some_and(|node| match &node.kind {
                NodeKind::Svg { alt, .. } => alt.is_empty(),
                NodeKind::Image { resource, alt } => {
                    alt.is_empty()
                        && book
                            .resource(*resource)
                            .is_some_and(|resource| resource.kind == folio_model::ResourceKind::Svg)
                }
                _ => false,
            })
    {
        return (
            QualityLevel::StructuralFallback,
            "deterministic accessible image placeholder".to_owned(),
            "the SVG has no alt text and a binary rasterizer is not enabled".to_owned(),
            vec!["enable a future deterministic rasterizer".to_owned()],
        );
    }
    if matches!(capability, Unsupported) {
        let fallback = match feature {
            Feature::VerticalWriting => Some((
                QualityLevel::CompatibleApproximation,
                "horizontal writing with preserved direction".to_owned(),
                "the target has no vertical writing primitive, so reading order and text are retained"
                    .to_owned(),
            )),
            Feature::Svg => Some((
                QualityLevel::StructuralFallback,
                "accessible alt text or deterministic image placeholder".to_owned(),
                "the target cannot embed SVG directly".to_owned(),
            )),
            Feature::FixedLayout => Some((
                QualityLevel::StructuralFallback,
                "reflowable document order".to_owned(),
                "the target cannot express fixed-page geometry".to_owned(),
            )),
            _ => None,
        };
        if let Some((quality, fallback, reason)) = fallback {
            return (
                quality,
                fallback,
                reason,
                vec!["drop unsupported decoration".to_owned()],
            );
        }
    }
    if feature == Feature::Svg && matches!(capability, Approximate) {
        let has_alt = node_id
            .and_then(|id| find_node(book, id))
            .is_some_and(|node| match &node.kind {
                NodeKind::Svg { alt, .. } => !alt.is_empty(),
                NodeKind::Image { resource, alt } => {
                    !alt.is_empty()
                        && book
                            .resource(*resource)
                            .is_some_and(|resource| resource.kind == folio_model::ResourceKind::Svg)
                }
                _ => false,
            });
        return if has_alt {
            (
                QualityLevel::CompatibleApproximation,
                "accessible alt text fallback".to_owned(),
                "a deterministic rasterizer is not enabled, so the image meaning is retained through alt text"
                    .to_owned(),
                vec!["deterministic raster image when a rasterizer is available".to_owned()],
            )
        } else {
            (
                QualityLevel::StructuralFallback,
                "deterministic accessible image placeholder".to_owned(),
                "the SVG has no alt text and a binary rasterizer is not enabled".to_owned(),
                vec!["provide alt text or enable a future deterministic rasterizer".to_owned()],
            )
        };
    }
    if matches!(capability, Approximate) {
        let fallback = match feature {
            Feature::Svg => "rasterized image, then accessible alt text".to_owned(),
            Feature::Ruby => "inline annotation approximation".to_owned(),
            Feature::Math => "accessible mathematical text fallback".to_owned(),
            Feature::EmbeddedFont => "generic family with retained weight/style".to_owned(),
            Feature::Footnote | Feature::Endnote => "legacy linked note marker".to_owned(),
            Feature::DropCap => "first-letter compatible styling".to_owned(),
            _ => "target-compatible visual approximation".to_owned(),
        };
        return (
            QualityLevel::CompatibleApproximation,
            fallback,
            "the target cannot retain every presentation detail, but a verified compatible form exists"
                .to_owned(),
            vec!["plain semantic text".to_owned()],
        );
    }
    if matches!(capability, Flattenable) || structural_hint {
        let fallback = match feature {
            Feature::ComplexTable => "linearized rows and cells".to_owned(),
            Feature::Float | Feature::FixedPosition => "inline flow order".to_owned(),
            Feature::FixedLayout => "reflowable document order".to_owned(),
            _ => "structural semantic fallback".to_owned(),
        };
        return (
            QualityLevel::StructuralFallback,
            fallback,
            "the target cannot express the original layout, so readable structure is retained"
                .to_owned(),
            vec![
                "simplified native layout".to_owned(),
                "plain structured text".to_owned(),
            ],
        );
    }
    let has_text_fallback = node_id
        .and_then(|id| find_node(book, id))
        .is_some_and(|node| match &node.kind {
            NodeKind::Svg { alt, .. } => !alt.is_empty(),
            NodeKind::Ruby | NodeKind::Footnote { .. } => true,
            _ => false,
        });
    if has_text_fallback || options.prefer_rasterization {
        return (
            QualityLevel::CompatibleApproximation,
            "semantic text fallback".to_owned(),
            "a readable fallback is available even though native target support is absent"
                .to_owned(),
            vec!["drop feature".to_owned()],
        );
    }
    (
        QualityLevel::Drop,
        "drop only as a last resort".to_owned(),
        "no readable target representation was found".to_owned(),
        Vec::new(),
    )
}

fn diagnostic_for(
    feature: Feature,
    node_id: Option<NodeId>,
    quality: QualityLevel,
    source: &str,
    fallback: &str,
    reason: &str,
) -> Diagnostic {
    let severity = if quality == QualityLevel::Drop {
        Severity::Error
    } else {
        Severity::Warning
    };
    let mut diagnostic = Diagnostic::new(
        severity,
        format!("FF-COMPAT-{:?}-0001", feature).to_ascii_uppercase(),
        format!("{source}: selected {fallback} ({quality:?}); {reason}."),
    );
    if let Some(node_id) = node_id {
        diagnostic
            .context
            .insert("node_id".to_owned(), node_id.get().to_string());
    }
    diagnostic
        .context
        .insert("feature".to_owned(), feature.name().to_owned());
    diagnostic
        .context
        .insert("fallback".to_owned(), fallback.to_owned());
    diagnostic
}

/// Project the semantic superset into a target-capable semantic book.  The
/// writer sees only this output and therefore has no policy decisions left.
pub fn project(book: &Book, plan: &DegradationPlan) -> Result<Book, CompatError> {
    plan.validate_mode()?;
    let mut projected = book.clone();
    let mut style_map = BTreeMap::new();
    let old_styles: Vec<_> = book.styles.iter().collect();
    projected.styles = StylePool::new();
    for (id, style) in old_styles {
        let lowered = lower_style(style, plan.target, plan.options.strip_embedded_fonts);
        let new_id = projected.styles.intern(lowered);
        style_map.insert(id, new_id);
    }
    for document in &mut projected.documents {
        document.nodes = project_nodes(&document.nodes, book, &style_map, plan)?;
    }
    if plan.options.strip_embedded_fonts {
        strip_embedded_fonts(&mut projected);
    }
    projected.presentation.layout = if plan.target == Format::Kf7 {
        LayoutMode::Reflowable
    } else {
        projected.presentation.layout
    };
    Ok(projected)
}

fn lower_style(
    style: &folio_model::ComputedStyle,
    target: Format,
    strip_embedded_fonts: bool,
) -> folio_model::ComputedStyle {
    if target != Format::Kf7 && !strip_embedded_fonts {
        return style.clone();
    }
    let supported: BTreeSet<&str> = [
        "color",
        "background",
        "font-family",
        "font-size",
        "font-style",
        "font-weight",
        "line-height",
        "text-align",
        "text-indent",
        "text-decoration",
        "margin",
        "padding",
        "display",
        "visibility",
        "white-space",
        "direction",
        "list-style",
        "border",
        "width",
        "height",
        "max-width",
        "max-height",
        "page-break-before",
        "page-break-after",
        "page-break-inside",
        "break-before",
        "break-after",
        "break-inside",
    ]
    .into_iter()
    .collect();
    folio_model::ComputedStyle {
        properties: style
            .properties
            .iter()
            .filter(|(property, _)| supported.contains(property.as_str()))
            .map(|(property, value)| {
                if *property == "font-family" {
                    (property.clone(), legacy_font_family(value))
                } else {
                    (property.clone(), value.clone())
                }
            })
            .collect(),
    }
}

fn strip_embedded_fonts(book: &mut Book) {
    let mut resource_map = BTreeMap::new();
    let mut resources = Vec::with_capacity(book.resources.len());
    for resource in &book.resources {
        if resource.kind == folio_model::ResourceKind::Font {
            continue;
        }
        let new_id = ResourceId::new(resources.len() as u32);
        resource_map.insert(resource.id, new_id);
        let mut resource = resource.clone();
        resource.id = new_id;
        resources.push(resource);
    }
    for document in &mut book.documents {
        remap_node_resources(&mut document.nodes, &resource_map);
    }
    book.resources = resources;
}

fn remap_node_resources(nodes: &mut [Node], resource_map: &BTreeMap<ResourceId, ResourceId>) {
    for node in nodes {
        match &mut node.kind {
            NodeKind::Image { resource, .. } => {
                if let Some(mapped) = resource_map.get(resource) {
                    *resource = *mapped;
                }
            }
            NodeKind::Svg {
                resource: Some(resource),
                ..
            } => {
                if let Some(mapped) = resource_map.get(resource) {
                    *resource = *mapped;
                }
            }
            _ => {}
        }
        remap_node_resources(&mut node.children, resource_map);
    }
}

fn legacy_font_family(value: &str) -> String {
    let generic = value.split(',').map(str::trim).find(|value| {
        matches!(
            value.trim_matches('"').trim_matches('\''),
            "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy"
        )
    });
    generic.unwrap_or("serif").to_owned()
}

fn project_nodes(
    nodes: &[Node],
    book: &Book,
    style_map: &BTreeMap<StyleId, StyleId>,
    plan: &DegradationPlan,
) -> Result<Vec<Node>, CompatError> {
    let mut projected = Vec::with_capacity(nodes.len());
    for node in nodes {
        if let Some(node) = project_node(node, book, style_map, plan)? {
            projected.push(node);
        }
    }
    Ok(projected)
}

fn project_node(
    node: &Node,
    book: &Book,
    style_map: &BTreeMap<StyleId, StyleId>,
    plan: &DegradationPlan,
) -> Result<Option<Node>, CompatError> {
    let mut node = node.clone();
    node.style = *style_map.get(&node.style).unwrap_or(&node.style);
    for variant in &mut node.variants {
        variant.style = *style_map.get(&variant.style).unwrap_or(&variant.style);
    }
    if let Some(variant) = preferred_variant(&node.variants, plan.target) {
        node.style = variant.style;
        node.presentation
            .features
            .extend(variant.presentation.features.iter().copied());
    }
    node.children = project_nodes(&node.children, book, style_map, plan)?;
    if plan.target == Format::Kf7 {
        match &node.kind {
            NodeKind::Svg { ref alt, .. } if !alt.is_empty() => {
                node.kind = NodeKind::Text { value: alt.clone() };
                node.role = SemanticRole::Image;
                node.confidence = Confidence::Explicit;
            }
            NodeKind::Svg { .. } => {
                node.kind = NodeKind::Text {
                    value: "[SVG image]".to_owned(),
                };
                node.role = SemanticRole::Image;
            }
            NodeKind::Image { resource, alt }
                if book
                    .resource(*resource)
                    .is_some_and(|resource| resource.kind == folio_model::ResourceKind::Svg) =>
            {
                node.kind = NodeKind::Text {
                    value: if alt.is_empty() {
                        "[SVG image]".to_owned()
                    } else {
                        alt.clone()
                    },
                };
                node.role = SemanticRole::Image;
            }
            NodeKind::Image { resource, alt }
                if book.resource(*resource).is_some_and(|resource| {
                    !matches!(
                        resource.kind,
                        folio_model::ResourceKind::Jpeg
                            | folio_model::ResourceKind::Png
                            | folio_model::ResourceKind::Gif
                    )
                }) =>
            {
                node.kind = NodeKind::Text {
                    value: if alt.is_empty() {
                        "[image]".to_owned()
                    } else {
                        alt.clone()
                    },
                };
                node.role = SemanticRole::Image;
            }
            NodeKind::Ruby => {
                node.kind = NodeKind::Inline;
                node.presentation
                    .features
                    .remove(&PresentationFeature::RubyAnnotation);
            }
            NodeKind::Math { alt, .. } => {
                node.kind = NodeKind::Text {
                    value: alt.clone().unwrap_or_else(|| "[math]".to_owned()),
                };
                node.role = SemanticRole::Math;
            }
            NodeKind::Footnote { href } => {
                node.kind = NodeKind::Link {
                    href: href.clone().unwrap_or_else(|| "#".to_owned()),
                };
            }
            NodeKind::Table | NodeKind::TableRow | NodeKind::TableCell
                if plan
                    .items
                    .iter()
                    .any(|item| item.feature == Feature::ComplexTable) =>
            {
                // Keep every row and cell in reading order as block flow. The
                // KF7 writer receives this already-projected structure and
                // therefore does not need its own fallback policy.
                node.kind = NodeKind::GenericBlock {
                    tag: "div".to_owned(),
                };
            }
            _ => {}
        }
    }
    Ok(Some(node))
}

fn preferred_variant(
    variants: &[folio_model::PresentationVariant],
    target: Format,
) -> Option<&folio_model::PresentationVariant> {
    let preferred = match target {
        Format::Epub3 => [VariantTarget::Epub3, VariantTarget::Generic],
        Format::Kf7 => [VariantTarget::Kf7, VariantTarget::Legacy],
        Format::Kf8 => [VariantTarget::Kf8, VariantTarget::Modern],
        Format::Kfx => [VariantTarget::Kfx, VariantTarget::Generic],
    };
    preferred
        .iter()
        .find_map(|target| variants.iter().find(|variant| variant.target == *target))
}

fn find_node(book: &Book, id: NodeId) -> Option<&Node> {
    fn visit(nodes: &[Node], id: NodeId) -> Option<&Node> {
        for node in nodes {
            if node.id == id {
                return Some(node);
            }
            if let Some(found) = visit(&node.children, id) {
                return Some(found);
            }
        }
        None
    }
    for document in &book.documents {
        if let Some(found) = visit(&document.nodes, id) {
            return Some(found);
        }
    }
    None
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-compat/src/lib.rs"]
mod tests;
