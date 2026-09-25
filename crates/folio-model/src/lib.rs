//! Canonical, format-neutral book model used by every FolioForge frontend and
//! target writer.  This crate intentionally has no knowledge of EPUB, Kindle,
//! SwiftUI, HTTP, or Amazon services.

use std::{collections::BTreeMap, fmt, sync::Arc};

use serde::{Deserialize, Serialize};
use thiserror::Error;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            Deserialize,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
        )]
        pub struct $name(pub u32);

        impl $name {
            pub const fn new(value: u32) -> Self {
                Self(value)
            }

            pub const fn get(self) -> u32 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }
    };
}

id_type!(DocumentId);
id_type!(NodeId);
id_type!(ResourceId);
id_type!(StyleId);
id_type!(AnchorId);

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Metadata {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub language: Option<String>,
    /// Canonical author list. `creators` is retained as a backwards-compatible
    /// storage alias for older serialized books.
    #[serde(default)]
    pub authors: Vec<String>,
    pub creators: Vec<String>,
    pub contributors: Vec<String>,
    pub publisher: Option<String>,
    /// Canonical primary date. `dates` retains additional source dates.
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub series: Option<String>,
    #[serde(default)]
    pub series_index: Option<f64>,
    pub identifier: Option<String>,
    #[serde(default)]
    pub identifiers: Vec<String>,
    pub description: Option<String>,
    pub subjects: Vec<String>,
    pub dates: Vec<String>,
    pub rights: Option<String>,
}

impl Metadata {
    pub fn display_title(&self) -> &str {
        self.title.as_deref().unwrap_or("Untitled")
    }

    /// Return a de-duplicated author view while preserving the older
    /// `creators` storage field.
    pub fn author_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for name in self.authors.iter().chain(self.creators.iter()) {
            if !name.is_empty() && !names.iter().any(|item| item == name) {
                names.push(name.clone());
            }
        }
        names
    }

    pub fn add_author(&mut self, name: impl Into<String>) {
        let name = name.into();
        if name.is_empty() {
            return;
        }
        if !self.authors.iter().any(|item| item == &name) {
            self.authors.push(name.clone());
        }
        if !self.creators.iter().any(|item| item == &name) {
            self.creators.push(name);
        }
    }

    pub fn identifier_values(&self) -> Vec<String> {
        let mut values = Vec::new();
        for identifier in self.identifiers.iter().chain(self.identifier.iter()) {
            if !identifier.is_empty() && !values.iter().any(|item| item == identifier) {
                values.push(identifier.clone());
            }
        }
        values
    }

    pub fn add_identifier(&mut self, identifier: impl Into<String>) {
        let identifier = identifier.into();
        if identifier.is_empty() {
            return;
        }
        if self.identifier.is_none() {
            self.identifier = Some(identifier.clone());
        }
        if !self.identifiers.iter().any(|item| item == &identifier) {
            self.identifiers.push(identifier);
        }
    }

    pub fn date_values(&self) -> Vec<String> {
        let mut values = Vec::new();
        for date in self.date.iter().chain(self.dates.iter()) {
            if !date.is_empty() && !values.iter().any(|item| item == date) {
                values.push(date.clone());
            }
        }
        values
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum LayoutMode {
    #[default]
    Reflowable,
    Fixed,
}

/// Declared horizontal placement of one fixed-layout document in a
/// two-page presentation. This is semantic input, not a Reader's current
/// display arrangement.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageSide {
    Left,
    Right,
    Center,
}

/// Generic source intent for grouping one fixed-layout document with an
/// adjacent document when a Reader explicitly enables synthetic spreads.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpreadHint {
    /// No source-specific grouping instruction; Reader display policy applies.
    #[default]
    Automatic,
    /// Keep this document out of a two-document spread.
    SinglePage,
    /// Pair this document with an adjacent document when its metadata permits.
    Pair,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Presentation {
    pub layout: LayoutMode,
    pub direction: Option<String>,
    pub writing_mode: Option<String>,
    /// Explicit per-document side declarations. Missing entries mean the
    /// importer could not prove a source-declared side.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub page_sides: BTreeMap<DocumentId, PageSide>,
    /// Explicit per-document spread intent. Missing entries are `Automatic`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub spread_hints: BTreeMap<DocumentId, SpreadHint>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum ResourceKind {
    Jpeg,
    Png,
    Gif,
    Svg,
    Font,
    Stylesheet,
    Audio,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Resource {
    pub id: ResourceId,
    /// Canonical POSIX path inside the source package.
    pub path: String,
    pub media_type: String,
    pub kind: ResourceKind,
    pub properties: Vec<String>,
    pub size: Option<u64>,
}

/// Associates an embedded font resource with the family used by computed
/// styles. Importers may leave this empty when a source package does not
/// provide enough evidence to map a font file to a family safely.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FontFace {
    pub resource: ResourceId,
    pub family: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Document {
    pub id: DocumentId,
    pub href: String,
    pub media_type: String,
    pub title: Option<String>,
    pub nodes: Vec<Node>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub style: StyleId,
    pub children: Vec<Node>,
    /// Format-neutral meaning of this node.  The kind is intentionally kept
    /// small and serialization-friendly; roles preserve meaning when a
    /// target cannot retain the source markup or layout.
    #[serde(default)]
    pub role: SemanticRole,
    /// Author-facing presentation wishes, separate from target encoding.
    #[serde(default)]
    pub presentation: PresentationIntent,
    /// How confidently the importer established this semantic meaning.
    #[serde(default)]
    pub confidence: Confidence,
    /// Target-specific style/presentation variants collected during import.
    #[serde(default)]
    pub variants: Vec<PresentationVariant>,
}

impl Node {
    pub fn new(id: NodeId, kind: NodeKind, style: StyleId, children: Vec<Node>) -> Self {
        let role = kind.default_role();
        Self {
            id,
            kind,
            style,
            children,
            role,
            presentation: PresentationIntent::default(),
            confidence: Confidence::Explicit,
            variants: Vec::new(),
        }
    }

    pub fn with_semantics(
        mut self,
        role: SemanticRole,
        presentation: PresentationIntent,
        confidence: Confidence,
    ) -> Self {
        self.role = role;
        self.presentation = presentation;
        self.confidence = confidence;
        self
    }

    pub fn text_content(&self) -> String {
        let mut result = String::new();
        self.write_text_content(&mut result);
        result
    }

    /// Return the canonical semantic projection of a ruby node.
    ///
    /// Ruby base text and pronunciation text are separate semantic fields;
    /// callers that need the visible reading-text projection should use
    /// `visible_text_content`, which applies the project-wide base-then-
    /// annotation rule without changing the semantic fields themselves.
    pub fn ruby_projection(&self) -> Option<RubyProjection> {
        if !matches!(self.kind, NodeKind::Ruby) {
            return None;
        }
        let mut projection = RubyProjection::default();
        for child in &self.children {
            match &child.kind {
                NodeKind::GenericInline { tag } | NodeKind::GenericBlock { tag }
                    if tag.eq_ignore_ascii_case("rt") =>
                {
                    projection.annotation.push_str(&child.text_content());
                }
                NodeKind::GenericInline { tag } | NodeKind::GenericBlock { tag }
                    if tag.eq_ignore_ascii_case("rp") => {}
                _ => projection.base.push_str(&child.text_content()),
            }
        }
        Some(projection)
    }

    /// The canonical visible reading-text projection used for text-oriented
    /// round-trip checks. Ruby is deliberately projected as base followed by
    /// annotation; semantic comparisons also compare the two fields
    /// separately through `ruby_projection`.
    pub fn visible_text_content(&self) -> String {
        let mut result = String::new();
        self.write_visible_text_content(&mut result);
        result
    }

    fn write_visible_text_content(&self, result: &mut String) {
        if let Some(ruby) = self.ruby_projection() {
            result.push_str(&ruby.base);
            result.push_str(&ruby.annotation);
            return;
        }
        match &self.kind {
            NodeKind::Text { value } => result.push_str(value),
            NodeKind::Image { alt, .. } | NodeKind::Svg { alt, .. } if !alt.is_empty() => {
                result.push_str(alt)
            }
            NodeKind::Math { alt: Some(alt), .. } => result.push_str(alt),
            NodeKind::PageBreak => result.push('\n'),
            _ => {}
        }
        for child in &self.children {
            child.write_visible_text_content(result);
        }
        if matches!(
            self.kind,
            NodeKind::Section
                | NodeKind::Paragraph
                | NodeKind::Heading { .. }
                | NodeKind::BlockQuote
                | NodeKind::Preformatted
                | NodeKind::ListItem
                | NodeKind::GenericBlock { .. }
        ) {
            result.push('\n');
        }
    }

    fn write_text_content(&self, result: &mut String) {
        match &self.kind {
            NodeKind::Text { value } => result.push_str(value),
            NodeKind::Image { alt, .. } | NodeKind::Svg { alt, .. } if !alt.is_empty() => {
                result.push_str(alt)
            }
            NodeKind::Math { alt: Some(alt), .. } => result.push_str(alt),
            NodeKind::PageBreak => result.push('\n'),
            _ => {}
        }
        for child in &self.children {
            child.write_text_content(result);
        }
        if matches!(
            self.kind,
            NodeKind::Section
                | NodeKind::Paragraph
                | NodeKind::Heading { .. }
                | NodeKind::BlockQuote
                | NodeKind::Preformatted
                | NodeKind::ListItem
                | NodeKind::GenericBlock { .. }
        ) {
            result.push('\n');
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RubyProjection {
    pub base: String,
    pub annotation: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum NodeKind {
    Section,
    Heading {
        level: u8,
    },
    Paragraph,
    Text {
        value: String,
    },
    Inline,
    Emphasis,
    Strong,
    BlockQuote,
    Code,
    Preformatted,
    OrderedList,
    UnorderedList,
    ListItem,
    Image {
        resource: ResourceId,
        alt: String,
    },
    Svg {
        resource: Option<ResourceId>,
        alt: String,
    },
    Table,
    TableRow,
    TableCell,
    Link {
        href: String,
    },
    Anchor {
        name: String,
    },
    Ruby,
    Math {
        mathml: String,
        alt: Option<String>,
        display: bool,
    },
    Footnote {
        href: Option<String>,
    },
    PageBreak,
    GenericBlock {
        tag: String,
    },
    GenericInline {
        tag: String,
    },
}

/// Shared reading semantics.  These names are deliberately independent of
/// EPUB, MOBI, KF8, and KFX implementation vocabulary.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum SemanticRole {
    Chapter,
    Section,
    Heading,
    Paragraph,
    Quote,
    Footnote,
    Endnote,
    Caption,
    Aside,
    Code,
    Poetry,
    SceneBreak,
    PageBreak,
    List,
    Table,
    Figure,
    Image,
    Link,
    Text,
    Emphasis,
    Strong,
    Ruby,
    Math,
    #[default]
    Generic,
}

/// Confidence is attached only to semantic interpretation.  It lets a
/// lower-to-higher conversion preserve known facts without presenting weak
/// heuristics as authorial intent.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Confidence {
    Explicit,
    StronglyInferred,
    Heuristic,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum PresentationFeature {
    KeepTogether,
    AvoidBreakInside,
    Centered,
    Indented,
    DropCap,
    FixedPosition,
    FullBleed,
    VerticalText,
    RubyAnnotation,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PresentationIntent {
    pub features: std::collections::BTreeSet<PresentationFeature>,
}

impl PresentationIntent {
    pub fn contains(&self, feature: PresentationFeature) -> bool {
        self.features.contains(&feature)
    }

    pub fn with(mut self, feature: PresentationFeature) -> Self {
        self.features.insert(feature);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum VariantTarget {
    #[default]
    Generic,
    Legacy,
    Modern,
    Epub3,
    Kf7,
    Kf8,
    Kfx,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PresentationVariant {
    pub target: VariantTarget,
    pub style: StyleId,
    #[serde(default)]
    pub presentation: PresentationIntent,
}

impl NodeKind {
    pub const fn default_role(&self) -> SemanticRole {
        match self {
            Self::Section => SemanticRole::Section,
            Self::Heading { .. } => SemanticRole::Heading,
            Self::Paragraph => SemanticRole::Paragraph,
            Self::Text { .. } => SemanticRole::Text,
            Self::Emphasis => SemanticRole::Emphasis,
            Self::Strong => SemanticRole::Strong,
            Self::BlockQuote => SemanticRole::Quote,
            Self::Code | Self::Preformatted => SemanticRole::Code,
            Self::OrderedList | Self::UnorderedList | Self::ListItem => SemanticRole::List,
            Self::Image { .. } | Self::Svg { .. } => SemanticRole::Image,
            Self::Table | Self::TableRow | Self::TableCell => SemanticRole::Table,
            Self::Link { .. } => SemanticRole::Link,
            Self::Ruby => SemanticRole::Ruby,
            Self::Math { .. } => SemanticRole::Math,
            Self::Footnote { .. } => SemanticRole::Footnote,
            Self::PageBreak => SemanticRole::PageBreak,
            Self::Inline | Self::GenericInline { .. } | Self::GenericBlock { .. } => {
                SemanticRole::Generic
            }
            Self::Anchor { .. } => SemanticRole::Generic,
        }
    }
}

impl NodeKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Section => "section",
            Self::Heading { .. } => "heading",
            Self::Paragraph => "paragraph",
            Self::Text { .. } => "text",
            Self::Inline => "inline",
            Self::Emphasis => "emphasis",
            Self::Strong => "strong",
            Self::BlockQuote => "blockquote",
            Self::Code => "code",
            Self::Preformatted => "preformatted",
            Self::OrderedList => "ordered-list",
            Self::UnorderedList => "unordered-list",
            Self::ListItem => "list-item",
            Self::Image { .. } => "image",
            Self::Svg { .. } => "svg",
            Self::Table => "table",
            Self::TableRow => "table-row",
            Self::TableCell => "table-cell",
            Self::Link { .. } => "link",
            Self::Anchor { .. } => "anchor",
            Self::Ruby => "ruby",
            Self::Math { .. } => "math",
            Self::Footnote { .. } => "footnote",
            Self::PageBreak => "page-break",
            Self::GenericBlock { .. } => "generic-block",
            Self::GenericInline { .. } => "generic-inline",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Anchor {
    pub id: AnchorId,
    pub document: DocumentId,
    pub node: NodeId,
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Navigation {
    pub toc: Vec<NavPoint>,
    pub landmarks: Vec<NavPoint>,
    pub page_list: Vec<NavPoint>,
    pub start_location: Option<String>,
    #[serde(default)]
    pub anchor_graph: AnchorGraph,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NavPoint {
    pub label: String,
    pub href: String,
    pub children: Vec<NavPoint>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AnchorGraph {
    pub edges: Vec<AnchorEdge>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnchorEdge {
    pub source: String,
    pub target: String,
    pub relation: AnchorRelation,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum AnchorRelation {
    #[default]
    Link,
    Footnote,
    Endnote,
    Navigation,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ComputedStyle {
    /// Canonical property/value pairs. Values are normalized strings so the
    /// pool can intern equal computed styles deterministically.
    pub properties: BTreeMap<String, String>,
}

impl ComputedStyle {
    pub fn get(&self, property: &str) -> Option<&str> {
        self.properties.get(property).map(String::as_str)
    }

    pub fn with(mut self, property: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties.insert(property.into(), value.into());
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct StylePool {
    styles: Vec<ComputedStyle>,
}

impl StylePool {
    pub fn new() -> Self {
        Self { styles: Vec::new() }
    }

    pub fn intern(&mut self, style: ComputedStyle) -> StyleId {
        if let Some((index, _)) = self
            .styles
            .iter()
            .enumerate()
            .find(|(_, item)| **item == style)
        {
            return StyleId::new(index as u32);
        }
        let id = StyleId::new(self.styles.len() as u32);
        self.styles.push(style);
        id
    }

    pub fn get(&self, id: StyleId) -> Option<&ComputedStyle> {
        self.styles.get(id.get() as usize)
    }

    pub fn iter(&self) -> impl Iterator<Item = (StyleId, &ComputedStyle)> {
        self.styles
            .iter()
            .enumerate()
            .map(|(i, style)| (StyleId::new(i as u32), style))
    }

    pub fn len(&self) -> usize {
        self.styles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.styles.is_empty()
    }
}

#[derive(Debug, Error)]
pub enum ResourceLoadError {
    #[error("resource I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("resource is not available: {0}")]
    NotFound(String),
    #[error("resource exceeds the configured limit: {0}")]
    TooLarge(String),
    #[error("resource loader error: {0}")]
    Other(String),
}

/// A loader is intentionally owned by the EPUB frontend rather than by the
/// model.  The model only knows how to ask for bytes by a stable package
/// locator, which keeps large images and fonts lazy.
pub trait ResourceLoader: Send + Sync {
    fn load(&self, locator: &str, max_bytes: Option<u64>) -> Result<Vec<u8>, ResourceLoadError>;
}

#[derive(Default)]
struct EmptyResourceLoader;

impl ResourceLoader for EmptyResourceLoader {
    fn load(&self, locator: &str, _max_bytes: Option<u64>) -> Result<Vec<u8>, ResourceLoadError> {
        Err(ResourceLoadError::NotFound(locator.to_owned()))
    }
}

#[derive(Clone, Default)]
pub struct MemoryResourceLoader {
    resources: BTreeMap<String, Arc<[u8]>>,
}

impl MemoryResourceLoader {
    pub fn insert(&mut self, locator: impl Into<String>, bytes: impl Into<Arc<[u8]>>) {
        self.resources.insert(locator.into(), bytes.into());
    }
}

impl ResourceLoader for MemoryResourceLoader {
    fn load(&self, locator: &str, max_bytes: Option<u64>) -> Result<Vec<u8>, ResourceLoadError> {
        let bytes = self
            .resources
            .get(locator)
            .ok_or_else(|| ResourceLoadError::NotFound(locator.to_owned()))?;
        if max_bytes.is_some_and(|limit| bytes.len() as u64 > limit) {
            return Err(ResourceLoadError::TooLarge(locator.to_owned()));
        }
        Ok(bytes.to_vec())
    }
}

pub struct Book {
    pub metadata: Metadata,
    pub documents: Vec<Document>,
    pub resources: Vec<Resource>,
    pub font_faces: Vec<FontFace>,
    pub navigation: Navigation,
    pub presentation: Presentation,
    pub anchors: Vec<Anchor>,
    pub styles: StylePool,
    resource_loader: Arc<dyn ResourceLoader>,
}

/// Compatibility alias for callers that use the explicit Semantic IR name.
/// `Book` remains the short public type name.
pub type FolioSemanticIr = Book;

impl Clone for Book {
    fn clone(&self) -> Self {
        Self {
            metadata: self.metadata.clone(),
            documents: self.documents.clone(),
            resources: self.resources.clone(),
            font_faces: self.font_faces.clone(),
            navigation: self.navigation.clone(),
            presentation: self.presentation.clone(),
            anchors: self.anchors.clone(),
            styles: self.styles.clone(),
            resource_loader: Arc::clone(&self.resource_loader),
        }
    }
}

impl fmt::Debug for Book {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Book")
            .field("metadata", &self.metadata)
            .field("documents", &self.documents)
            .field("resources", &self.resources)
            .field("font_faces", &self.font_faces)
            .field("navigation", &self.navigation)
            .field("presentation", &self.presentation)
            .field("anchors", &self.anchors)
            .field("styles", &self.styles)
            .finish()
    }
}

impl Default for Book {
    fn default() -> Self {
        Self::new()
    }
}

impl Book {
    pub fn new() -> Self {
        Self {
            metadata: Metadata::default(),
            documents: Vec::new(),
            resources: Vec::new(),
            font_faces: Vec::new(),
            navigation: Navigation::default(),
            presentation: Presentation::default(),
            anchors: Vec::new(),
            styles: StylePool::new(),
            resource_loader: Arc::new(EmptyResourceLoader),
        }
    }

    pub fn with_resource_loader(mut self, loader: Arc<dyn ResourceLoader>) -> Self {
        self.resource_loader = loader;
        self
    }

    pub fn load_resource(
        &self,
        id: ResourceId,
        max_bytes: Option<u64>,
    ) -> Result<Vec<u8>, ResourceLoadError> {
        let resource = self
            .resource(id)
            .ok_or_else(|| ResourceLoadError::NotFound(id.to_string()))?;
        self.resource_loader.load(&resource.path, max_bytes)
    }

    pub fn resource(&self, id: ResourceId) -> Option<&Resource> {
        self.resources.get(id.get() as usize)
    }

    pub fn semantic_value(&self) -> serde_json::Value {
        let styles: Vec<_> = self
            .styles
            .iter()
            .map(|(id, style)| {
                serde_json::json!({
                    "id": id,
                    "properties": style.properties,
                })
            })
            .collect();
        serde_json::json!({
            "metadata": self.metadata,
            "documents": self.documents,
            "navigation": self.navigation,
            "presentation": self.presentation,
            "resources": self.resources,
            "font_faces": self.font_faces,
            "anchors": self.anchors,
            "styles": styles,
        })
    }

    pub fn feature_summary(&self) -> BTreeMap<String, usize> {
        let mut result = BTreeMap::new();
        for document in &self.documents {
            for node in &document.nodes {
                count_node_features(node, &mut result);
            }
        }
        result
    }

    /// Return ruby projections in document and child order. This is a
    /// semantic test/comparison primitive, not a second representation of the
    /// IR.
    pub fn ruby_projections(&self) -> Vec<RubyProjection> {
        let mut result = Vec::new();
        for document in &self.documents {
            for node in &document.nodes {
                collect_ruby_projections(node, &mut result);
            }
        }
        result
    }
}

fn collect_ruby_projections(node: &Node, result: &mut Vec<RubyProjection>) {
    if let Some(projection) = node.ruby_projection() {
        result.push(projection);
    }
    for child in &node.children {
        collect_ruby_projections(child, result);
    }
}

fn count_node_features(node: &Node, result: &mut BTreeMap<String, usize>) {
    *result.entry(node.kind.name().to_owned()).or_default() += 1;
    for child in &node.children {
        count_node_features(child, result);
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DiagnosticSource {
    pub path: Option<String>,
    pub line: Option<u64>,
    pub column: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub source: Option<DiagnosticSource>,
    pub context: BTreeMap<String, String>,
}

impl Diagnostic {
    pub fn new(severity: Severity, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity,
            code: code.into(),
            message: message.into(),
            source: None,
            context: BTreeMap::new(),
        }
    }

    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, code, message)
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, code, message)
    }

    pub fn info(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Severity::Info, code, message)
    }
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-model/src/lib.rs"]
mod tests;
