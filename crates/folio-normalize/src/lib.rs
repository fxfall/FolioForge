//! Format-neutral recovery of semantic structure from XHTML-like documents.
//!
//! EPUB, Kindle, and KFX adapters provide document content and resources here;
//! this crate deliberately owns neither an input container nor an output
//! package format.

use std::{collections::BTreeMap, sync::Arc};

use folio_model::{
    Anchor, AnchorEdge, AnchorGraph, AnchorId, AnchorRelation, Book, Confidence, Diagnostic,
    Document, DocumentId, Metadata, Node, NodeId, NodeKind, PresentationFeature,
    PresentationIntent, PresentationVariant, Resource, ResourceId, ResourceLoader, StylePool,
    VariantTarget,
};
use folio_style::{ElementContext, StyleResolver, TargetProfile};
use roxmltree::{Document as XmlDocument, Node as XmlNode};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum XhtmlError {
    #[error("invalid XML in {path}: {source}")]
    Xml {
        path: String,
        source: roxmltree::Error,
    },
    #[error("XHTML input is invalid: {0}")]
    Invalid(String),
    #[error("XHTML safety limit exceeded: {0}")]
    Limit(String),
}

#[derive(Clone, Debug)]
pub struct XhtmlDocumentInput {
    pub href: String,
    pub media_type: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct XhtmlImportReport {
    pub book: Book,
    pub diagnostics: Vec<Diagnostic>,
}

/// Result of the format-neutral normalization pass.
///
/// Importers are responsible for recovering source semantics; this pass only
/// makes equivalent IR values deterministic for every adapter.  It does not
/// choose an output format and never removes source nodes or resources.
#[derive(Clone, Debug, Default)]
pub struct NormalizationReport {
    pub changed_fields: usize,
    pub diagnostics: Vec<Diagnostic>,
}

/// Apply the shared, loss-averse normalization pass to Semantic IR.
///
/// Metadata whitespace/language spelling and duplicate list values are
/// normalized without changing document order, node kinds, links, anchors, or
/// resource identity.  Format adapters already normalize source text and
/// styles while decoding, so this function deliberately avoids rewriting
/// preformatted nodes or guessing missing metadata.
pub fn normalize_book(book: &mut Book) -> NormalizationReport {
    let mut report = NormalizationReport::default();
    let metadata = &mut book.metadata;
    normalize_optional(&mut metadata.title, &mut report);
    normalize_optional(&mut metadata.subtitle, &mut report);
    normalize_optional(&mut metadata.language, &mut report);
    normalize_optional(&mut metadata.publisher, &mut report);
    normalize_optional(&mut metadata.date, &mut report);
    normalize_optional(&mut metadata.series, &mut report);
    normalize_optional(&mut metadata.identifier, &mut report);
    normalize_optional(&mut metadata.description, &mut report);
    normalize_optional(&mut metadata.rights, &mut report);
    if let Some(language) = metadata.language.as_mut() {
        let normalized = normalize_language(language);
        if *language != normalized {
            *language = normalized;
            report.changed_fields += 1;
        }
    }
    for values in [
        &mut metadata.authors,
        &mut metadata.creators,
        &mut metadata.contributors,
        &mut metadata.identifiers,
        &mut metadata.subjects,
        &mut metadata.dates,
    ] {
        normalize_list(values, &mut report);
    }
    for resource in &mut book.resources {
        let original = resource.properties.len();
        let mut seen = std::collections::BTreeSet::new();
        resource
            .properties
            .retain(|property| seen.insert(property.clone()));
        if resource.properties.len() != original {
            report.changed_fields += 1;
        }
    }
    report
}

fn normalize_optional(value: &mut Option<String>, report: &mut NormalizationReport) {
    let Some(current) = value.as_mut() else {
        return;
    };
    let normalized = current.split_whitespace().collect::<Vec<_>>().join(" ");
    if *current != normalized {
        *current = normalized;
        report.changed_fields += 1;
    }
    if current.is_empty() {
        *value = None;
        report.changed_fields += 1;
    }
}

fn normalize_list(values: &mut Vec<String>, report: &mut NormalizationReport) {
    let original = values.clone();
    let mut normalized = Vec::new();
    for value in values.drain(..) {
        let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
        if !value.is_empty() && !normalized.iter().any(|item| item == &value) {
            normalized.push(value);
        }
    }
    *values = normalized;
    if *values != original {
        report.changed_fields += 1;
    }
}

fn normalize_language(value: &str) -> String {
    value
        .replace('_', "-")
        .split('-')
        .filter(|part| !part.is_empty())
        .enumerate()
        .map(|(index, part)| {
            if index == 0 {
                part.to_ascii_lowercase()
            } else if part.len() == 4 {
                let mut chars = part.chars();
                chars
                    .next()
                    .map(|first| first.to_ascii_uppercase().to_string())
                    .unwrap_or_default()
                    + &chars.as_str().to_ascii_lowercase()
            } else if (part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
                || (part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit()))
            {
                part.to_ascii_uppercase()
            } else {
                part.to_ascii_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

#[derive(Clone, Debug)]
pub struct XhtmlImportOptions {
    pub max_dom_nodes: usize,
    pub max_xml_depth: usize,
    pub require_body_element: bool,
    pub resolver: StyleResolver,
    pub variant_resolvers: Vec<(VariantTarget, StyleResolver)>,
}

impl Default for XhtmlImportOptions {
    fn default() -> Self {
        Self {
            max_dom_nodes: 2_000_000,
            max_xml_depth: 128,
            require_body_element: false,
            resolver: StyleResolver::new(TargetProfile::Generic),
            variant_resolvers: Vec::new(),
        }
    }
}

pub fn import_xhtml_documents(
    metadata: Metadata,
    documents: &[XhtmlDocumentInput],
    resources: Vec<Resource>,
    loader: Arc<dyn ResourceLoader>,
) -> Result<XhtmlImportReport, XhtmlError> {
    import_xhtml_documents_with_options(
        metadata,
        documents,
        resources,
        loader,
        XhtmlImportOptions::default(),
    )
}

pub fn import_xhtml_documents_with_options(
    metadata: Metadata,
    documents: &[XhtmlDocumentInput],
    resources: Vec<Resource>,
    loader: Arc<dyn ResourceLoader>,
    mut options: XhtmlImportOptions,
) -> Result<XhtmlImportReport, XhtmlError> {
    let mut book = Book::new().with_resource_loader(loader);
    book.metadata = metadata;
    book.resources = resources;
    let resources_by_path = book
        .resources
        .iter()
        .map(|resource| (resource.path.clone(), resource.id))
        .collect::<BTreeMap<_, _>>();
    let mut diagnostics = Vec::new();
    let mut anchors = Vec::new();
    let mut anchor_graph = AnchorGraph::default();
    let mut parsed_documents = Vec::with_capacity(documents.len());
    let mut next_node = 0u32;

    for (index, input) in documents.iter().enumerate() {
        let xml = XmlDocument::parse(&input.content).map_err(|source| XhtmlError::Xml {
            path: input.href.clone(),
            source,
        })?;
        enforce_xml_shape(
            &xml,
            &input.href,
            options.max_dom_nodes,
            options.max_xml_depth,
        )?;

        for style_node in xml
            .descendants()
            .filter(|node| node.is_element() && local_name(*node) == "style")
        {
            if let Some(css) = style_node.text() {
                options.resolver.add_stylesheet(css);
                for (_, variant_resolver) in &mut options.variant_resolvers {
                    variant_resolver.add_stylesheet(css);
                }
            }
        }

        let body = xml
            .descendants()
            .find(|node| node.is_element() && local_name(*node) == "body")
            .or_else(|| {
                (!options.require_body_element)
                    .then(|| xml.root().first_element_child())
                    .flatten()
            })
            .ok_or_else(|| XhtmlError::Invalid(format!("{} has no body element", input.href)))?;
        let document_id = DocumentId::new(index as u32);
        let next_anchor = anchors.len() as u32;
        let mut parser = NodeBuilder {
            resolver: &options.resolver,
            variant_resolvers: &options.variant_resolvers,
            styles: &mut book.styles,
            anchors: &mut anchors,
            anchor_graph: &mut anchor_graph,
            resources_by_path: &resources_by_path,
            diagnostics: &mut diagnostics,
            document: document_id,
            document_href: input.href.clone(),
            base_dir: parent_path(&input.href),
            next_node,
            next_anchor,
            max_dom_nodes: options.max_dom_nodes,
            node_count: 0,
        };
        let nodes = parser.parse_children(body, None, None, false)?;
        next_node = parser.next_node;
        let title = first_heading_text(&nodes);
        parsed_documents.push(Document {
            id: document_id,
            href: input.href.clone(),
            media_type: input.media_type.clone(),
            title,
            nodes,
        });
    }

    book.documents = parsed_documents;
    book.anchors = anchors;
    book.navigation.anchor_graph = anchor_graph;
    Ok(XhtmlImportReport { book, diagnostics })
}

fn enforce_xml_shape(
    xml: &XmlDocument<'_>,
    path: &str,
    max_dom_nodes: usize,
    max_xml_depth: usize,
) -> Result<(), XhtmlError> {
    let mut nodes = 0usize;
    let mut max_depth = 0usize;
    for node in xml.descendants() {
        nodes = nodes
            .checked_add(1)
            .ok_or_else(|| XhtmlError::Limit(format!("DOM node count overflow in {path}")))?;
        if nodes > max_dom_nodes {
            return Err(XhtmlError::Limit(format!(
                "DOM node count exceeds {max_dom_nodes} in {path}"
            )));
        }
        max_depth = max_depth.max(node.ancestors().count());
    }
    if max_depth > max_xml_depth {
        return Err(XhtmlError::Limit(format!(
            "XML nesting exceeds {max_xml_depth} in {path}"
        )));
    }
    Ok(())
}

struct NodeBuilder<'a> {
    resolver: &'a StyleResolver,
    variant_resolvers: &'a [(VariantTarget, StyleResolver)],
    styles: &'a mut StylePool,
    anchors: &'a mut Vec<Anchor>,
    anchor_graph: &'a mut AnchorGraph,
    resources_by_path: &'a BTreeMap<String, ResourceId>,
    diagnostics: &'a mut Vec<Diagnostic>,
    document: DocumentId,
    document_href: String,
    base_dir: String,
    next_node: u32,
    next_anchor: u32,
    max_dom_nodes: usize,
    node_count: usize,
}

impl<'a> NodeBuilder<'a> {
    fn parse_children(
        &mut self,
        parent: XmlNode<'_, '_>,
        parent_style: Option<folio_model::StyleId>,
        parent_variants: Option<&BTreeMap<VariantTarget, folio_model::StyleId>>,
        preformatted: bool,
    ) -> Result<Vec<Node>, XhtmlError> {
        let mut result = Vec::new();
        for child in parent.children() {
            if child.is_text() {
                let value = normalize_text(child.text().unwrap_or_default(), preformatted);
                if !value.is_empty() {
                    result.push(self.text_node(value, parent_style.unwrap_or_default()));
                }
            } else if child.is_element() {
                if let Some(node) =
                    self.parse_element(child, parent_style, parent_variants, preformatted)?
                {
                    result.push(node);
                }
            }
        }
        Ok(result)
    }

    fn parse_element(
        &mut self,
        element: XmlNode<'_, '_>,
        parent_style: Option<folio_model::StyleId>,
        parent_variants: Option<&BTreeMap<VariantTarget, folio_model::StyleId>>,
        parent_preformatted: bool,
    ) -> Result<Option<Node>, XhtmlError> {
        if self.node_count >= self.max_dom_nodes {
            return Err(XhtmlError::Limit(format!(
                "DOM node count exceeds {}",
                self.max_dom_nodes
            )));
        }
        let tag = local_name(element).to_ascii_lowercase();
        if matches!(
            tag.as_str(),
            "head" | "title" | "meta" | "link" | "style" | "script" | "template"
        ) {
            return Ok(None);
        }
        let mut context = ElementContext::new(&tag);
        context.id = element.attribute("id").map(ToOwned::to_owned);
        if let Some(classes) = element.attribute("class") {
            context
                .classes
                .extend(classes.split_whitespace().map(ToOwned::to_owned));
        }
        context.inline_style = element.attribute("style").map(ToOwned::to_owned);
        let parent_computed = parent_style.and_then(|id| self.styles.get(id).cloned());
        let computed = self.resolver.resolve(&context, parent_computed.as_ref());
        let style = self.styles.intern(computed);
        let mut variant_style_ids = BTreeMap::new();
        for (target, resolver) in self.variant_resolvers {
            let parent_style = parent_variants
                .and_then(|variants| variants.get(target).copied())
                .and_then(|id| self.styles.get(id).cloned());
            let variant_style = resolver.resolve(&context, parent_style.as_ref());
            variant_style_ids.insert(*target, self.styles.intern(variant_style));
        }
        let id = self.allocate_node();
        if let Some(name) = element
            .attribute("id")
            .or_else(|| element.attribute("name"))
        {
            if !name.is_empty() {
                let anchor_id = AnchorId::new(self.next_anchor);
                self.next_anchor = self.next_anchor.saturating_add(1);
                self.anchors.push(Anchor {
                    id: anchor_id,
                    document: self.document,
                    node: id,
                    name: name.to_owned(),
                });
            }
        }
        let preformatted = parent_preformatted || tag == "pre";
        let children = if tag == "img" || tag == "br" || tag == "hr" || tag == "math" {
            Vec::new()
        } else {
            self.parse_children(element, Some(style), Some(&variant_style_ids), preformatted)?
        };
        let kind = match tag.as_str() {
            "body" | "section" | "article" | "aside" | "header" | "footer" => NodeKind::Section,
            "h1" => NodeKind::Heading { level: 1 },
            "h2" => NodeKind::Heading { level: 2 },
            "h3" => NodeKind::Heading { level: 3 },
            "h4" => NodeKind::Heading { level: 4 },
            "h5" => NodeKind::Heading { level: 5 },
            "h6" => NodeKind::Heading { level: 6 },
            "p" => NodeKind::Paragraph,
            "em" | "i" => NodeKind::Emphasis,
            "strong" | "b" => NodeKind::Strong,
            "blockquote" => NodeKind::BlockQuote,
            "code" => NodeKind::Code,
            "pre" => NodeKind::Preformatted,
            "ol" => NodeKind::OrderedList,
            "ul" => NodeKind::UnorderedList,
            "li" => NodeKind::ListItem,
            "table" => NodeKind::Table,
            "tr" => NodeKind::TableRow,
            "td" | "th" => NodeKind::TableCell,
            "a" if element.attribute("href").is_none()
                && element
                    .attribute("name")
                    .or_else(|| element.attribute("id"))
                    .is_some() =>
            {
                NodeKind::Anchor {
                    name: element
                        .attribute("name")
                        .or_else(|| element.attribute("id"))
                        .unwrap_or_default()
                        .to_owned(),
                }
            }
            "a" => {
                let href = element.attribute("href").unwrap_or("");
                let href = resolve_xhtml_link_href(&self.document_href, &self.base_dir, href)
                    .unwrap_or_else(|_| href.to_owned());
                if attribute_value(element, "epub:type")
                    .is_some_and(|value| value.split_whitespace().any(|item| item == "footnote"))
                {
                    NodeKind::Footnote { href: Some(href) }
                } else {
                    NodeKind::Link { href }
                }
            }
            "img" => {
                let source = element.attribute("src").unwrap_or("");
                let source = resolve_xhtml_href(&self.base_dir, source)
                    .unwrap_or_else(|_| source.to_owned());
                let alt = element.attribute("alt").unwrap_or("").to_owned();
                if let Some(resource) = self.resources_by_path.get(&source) {
                    NodeKind::Image {
                        resource: *resource,
                        alt,
                    }
                } else {
                    self.diagnostics.push(Diagnostic::warning(
                        "FF-EPUB-RES-0002",
                        format!("image resource not found: {source}"),
                    ));
                    NodeKind::GenericInline {
                        tag: "img".to_owned(),
                    }
                }
            }
            "svg" => NodeKind::Svg {
                resource: None,
                alt: element.attribute("aria-label").unwrap_or("").to_owned(),
            },
            "ruby" => NodeKind::Ruby,
            "math" => match sanitize_mathml(element) {
                Some(mathml) => NodeKind::Math {
                    alt: element.attribute("alttext").map(ToOwned::to_owned),
                    display: element.attribute("display") == Some("block"),
                    mathml,
                },
                None => {
                    self.diagnostics.push(Diagnostic::warning(
                        "FF-XHTML-MATH-0001",
                        "MathML was rejected by the safe semantic parser; preserving its fallback text only.",
                    ));
                    NodeKind::GenericInline {
                        tag: "span".to_owned(),
                    }
                }
            },
            "br" => NodeKind::Text {
                value: "\n".to_owned(),
            },
            "hr" => NodeKind::PageBreak,
            "div" | "pagenum" | "figure" | "figcaption" | "dl" | "dt" | "dd" => {
                NodeKind::GenericBlock { tag: tag.clone() }
            }
            _ if is_inline_tag(&tag) => NodeKind::GenericInline { tag: tag.clone() },
            _ => NodeKind::GenericBlock { tag: tag.clone() },
        };
        let mut node = Node::new(id, kind, style, children);
        node.role = semantic_role(&tag, element);
        if node.role == folio_model::SemanticRole::PageBreak {
            node.kind = NodeKind::PageBreak;
        }
        node.presentation = presentation_intent(&node, self.styles, element);
        node.confidence = semantic_confidence(&tag, element);
        node.variants = variant_style_ids
            .into_iter()
            .map(|(target, style)| PresentationVariant {
                target,
                style,
                presentation: node.presentation.clone(),
            })
            .collect();
        if let NodeKind::Link { href } | NodeKind::Footnote { href: Some(href) } = &node.kind {
            self.anchor_graph.edges.push(AnchorEdge {
                source: format!("{}#node-{}", self.document.get(), id.get()),
                target: href.clone(),
                relation: if matches!(node.kind, NodeKind::Footnote { .. }) {
                    if node.role == folio_model::SemanticRole::Endnote {
                        AnchorRelation::Endnote
                    } else {
                        AnchorRelation::Footnote
                    }
                } else {
                    AnchorRelation::Link
                },
            });
        }
        Ok(Some(node))
    }

    fn text_node(&mut self, value: String, style: folio_model::StyleId) -> Node {
        let id = self.allocate_node();
        Node::new(id, NodeKind::Text { value }, style, Vec::new())
    }

    fn allocate_node(&mut self) -> NodeId {
        let id = NodeId::new(self.next_node);
        self.next_node = self.next_node.saturating_add(1);
        self.node_count = self.node_count.saturating_add(1);
        id
    }
}

fn semantic_role(tag: &str, element: XmlNode<'_, '_>) -> folio_model::SemanticRole {
    if element
        .attribute("class")
        .is_some_and(|value| value.split_whitespace().any(|item| item == "ff-pagebreak"))
    {
        return folio_model::SemanticRole::PageBreak;
    }
    let epub_type = attribute_value(element, "epub:type").unwrap_or("");
    if epub_type
        .split_whitespace()
        .any(|value| value.eq_ignore_ascii_case("chapter"))
    {
        return folio_model::SemanticRole::Chapter;
    }
    if epub_type
        .split_whitespace()
        .any(|value| value.eq_ignore_ascii_case("endnote"))
    {
        return folio_model::SemanticRole::Endnote;
    }
    if epub_type
        .split_whitespace()
        .any(|value| value.eq_ignore_ascii_case("footnote"))
    {
        return folio_model::SemanticRole::Footnote;
    }
    if epub_type.split_whitespace().any(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "separation" | "scene-break" | "scenebreak"
        )
    }) || element.attribute("class").is_some_and(|value| {
        value
            .split_whitespace()
            .any(|item| matches!(item, "scene-break" | "scenebreak" | "separator"))
    }) {
        return folio_model::SemanticRole::SceneBreak;
    }
    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => folio_model::SemanticRole::Heading,
        "p" => {
            if epub_type
                .split_whitespace()
                .any(|value| value.eq_ignore_ascii_case("poem"))
            {
                folio_model::SemanticRole::Poetry
            } else {
                folio_model::SemanticRole::Paragraph
            }
        }
        "blockquote" => folio_model::SemanticRole::Quote,
        "aside" => folio_model::SemanticRole::Aside,
        "figure" => folio_model::SemanticRole::Figure,
        "figcaption" | "caption" => folio_model::SemanticRole::Caption,
        "code" | "pre" => folio_model::SemanticRole::Code,
        "table" | "tr" | "td" | "th" => folio_model::SemanticRole::Table,
        "ruby" => folio_model::SemanticRole::Ruby,
        "math" => folio_model::SemanticRole::Math,
        "a" if epub_type
            .split_whitespace()
            .any(|value| value.eq_ignore_ascii_case("footnote")) =>
        {
            folio_model::SemanticRole::Footnote
        }
        "a" => folio_model::SemanticRole::Link,
        "img" | "svg" => folio_model::SemanticRole::Image,
        "br" | "hr" => folio_model::SemanticRole::PageBreak,
        "ol" | "ul" | "li" => folio_model::SemanticRole::List,
        "body" | "section" | "article" | "header" | "footer" => folio_model::SemanticRole::Section,
        _ => folio_model::SemanticRole::Generic,
    }
}

fn semantic_confidence(tag: &str, element: XmlNode<'_, '_>) -> Confidence {
    if attribute_value(element, "epub:type").is_some() || element.attribute("role").is_some() {
        return Confidence::Explicit;
    }
    if matches!(
        tag,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "figure"
    ) {
        Confidence::Explicit
    } else {
        Confidence::StronglyInferred
    }
}

fn presentation_intent(
    node: &Node,
    styles: &StylePool,
    element: XmlNode<'_, '_>,
) -> PresentationIntent {
    let mut intent = PresentationIntent::default();
    if let Some(style) = styles.get(node.style) {
        if style
            .get("text-align")
            .is_some_and(|value| value == "center")
        {
            intent = intent.with(PresentationFeature::Centered);
        }
        if style
            .get("writing-mode")
            .is_some_and(|value| value != "horizontal-tb")
        {
            intent = intent.with(PresentationFeature::VerticalText);
        }
        if style.get("float").is_some_and(|value| value != "none")
            || style
                .get("position")
                .is_some_and(|value| value == "fixed" || value == "absolute")
        {
            intent = intent.with(PresentationFeature::FixedPosition);
        }
        if style
            .get("page-break-inside")
            .is_some_and(|value| value == "avoid")
            || style
                .get("break-inside")
                .is_some_and(|value| value == "avoid")
        {
            intent = intent.with(PresentationFeature::KeepTogether);
            intent = intent.with(PresentationFeature::AvoidBreakInside);
        }
        if style
            .get("text-indent")
            .is_some_and(|value| value != "0" && value != "0px")
            || style
                .get("margin-left")
                .is_some_and(|value| value != "0" && value != "0px")
        {
            intent = intent.with(PresentationFeature::Indented);
        }
    }
    if element.attribute("class").is_some_and(|value| {
        value
            .split_whitespace()
            .any(|item| matches!(item, "dropcap" | "drop-cap"))
    }) {
        intent = intent.with(PresentationFeature::DropCap);
    }
    if element.attribute("class").is_some_and(|value| {
        value
            .split_whitespace()
            .any(|item| matches!(item, "fullbleed" | "full-bleed" | "bleed"))
    }) {
        intent = intent.with(PresentationFeature::FullBleed);
    }
    if matches!(node.role, folio_model::SemanticRole::Ruby) {
        intent = intent.with(PresentationFeature::RubyAnnotation);
    }
    intent
}

fn first_heading_text(nodes: &[Node]) -> Option<String> {
    for node in nodes {
        if matches!(node.kind, NodeKind::Heading { .. }) {
            let text = node.text_content().trim().to_owned();
            if !text.is_empty() {
                return Some(text);
            }
        }
        if let Some(text) = first_heading_text(&node.children) {
            return Some(text);
        }
    }
    None
}

fn is_inline_tag(tag: &str) -> bool {
    matches!(
        tag,
        "a" | "abbr"
            | "b"
            | "cite"
            | "em"
            | "i"
            | "img"
            | "kbd"
            | "q"
            | "rb"
            | "ruby"
            | "rt"
            | "s"
            | "small"
            | "span"
            | "strong"
            | "sub"
            | "sup"
            | "time"
            | "var"
    )
}

fn normalize_text(value: &str, preformatted: bool) -> String {
    let value = value.replace("\r\n", "\n").replace('\r', "\n");
    if preformatted {
        value
    } else {
        let parts = value.split_whitespace().collect::<Vec<_>>();
        if parts.is_empty() {
            return if value.chars().any(char::is_whitespace) {
                " ".to_owned()
            } else {
                String::new()
            };
        }
        let mut result = parts.join(" ");
        if value.chars().next().is_some_and(char::is_whitespace) {
            result.insert(0, ' ');
        }
        if value.chars().last().is_some_and(char::is_whitespace) {
            result.push(' ');
        }
        result
    }
}

fn local_name<'a, 'input>(node: XmlNode<'a, 'input>) -> &'input str {
    node.tag_name()
        .name()
        .rsplit(':')
        .next()
        .unwrap_or(node.tag_name().name())
}

fn attribute_value<'a, 'input>(node: XmlNode<'a, 'input>, name: &str) -> Option<&'a str> {
    node.attribute(name).or_else(|| {
        let local = name.rsplit(':').next().unwrap_or(name);
        node.attributes()
            .find(|attribute| attribute.name() == local)
            .map(|attribute| attribute.value())
    })
}

fn parent_path(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_owned())
        .unwrap_or_default()
}

fn resolve_xhtml_href(base_dir: &str, href: &str) -> Result<String, XhtmlError> {
    let href = href.split('#').next().unwrap_or(href);
    if href.is_empty() {
        return normalize_xhtml_path(base_dir);
    }
    let combined = if base_dir.is_empty() {
        href.to_owned()
    } else {
        format!("{base_dir}/{href}")
    };
    normalize_xhtml_path(&combined)
}

fn resolve_xhtml_link_href(
    document_href: &str,
    base_dir: &str,
    href: &str,
) -> Result<String, XhtmlError> {
    if is_external_href(href) {
        return Ok(href.to_owned());
    }
    let (path, fragment) = href.split_once('#').unwrap_or((href, ""));
    let path = if path.is_empty() {
        document_href.to_owned()
    } else {
        resolve_xhtml_href(base_dir, path)?
    };
    if fragment.is_empty() {
        Ok(path)
    } else {
        Ok(format!("{path}#{fragment}"))
    }
}

fn is_external_href(href: &str) -> bool {
    href.starts_with("//")
        || href
            .find(':')
            .is_some_and(|index| index > 0 && href[..index].chars().all(is_uri_scheme_char))
}

fn is_uri_scheme_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
}

fn normalize_xhtml_path(path: &str) -> Result<String, XhtmlError> {
    let decoded = percent_decode(path.split('#').next().unwrap_or(path))?;
    let decoded = decoded.replace('\\', "/");
    let mut parts = Vec::new();
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(XhtmlError::Invalid(format!(
                        "path escapes document root: {path}"
                    )));
                }
            }
            value => parts.push(value),
        }
    }
    if parts.is_empty() {
        return Err(XhtmlError::Invalid(format!("empty document path: {path}")));
    }
    Ok(parts.join("/"))
}

fn percent_decode(value: &str) -> Result<String, XhtmlError> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let end = index
                .checked_add(3)
                .ok_or_else(|| XhtmlError::Invalid("percent escape overflow".to_owned()))?;
            let digits = bytes
                .get(index + 1..end)
                .ok_or_else(|| XhtmlError::Invalid(format!("invalid percent escape in {value}")))?;
            let high = hex_digit(digits[0])
                .ok_or_else(|| XhtmlError::Invalid(format!("invalid percent escape in {value}")))?;
            let low = hex_digit(digits[1])
                .ok_or_else(|| XhtmlError::Invalid(format!("invalid percent escape in {value}")))?;
            output.push((high << 4) | low);
            index = end;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output)
        .map_err(|error| XhtmlError::Invalid(format!("path is not UTF-8: {error}")))
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

/// Normalize the small XHTML dialects emitted by legacy MOBI/KF8 writers.
/// Record-index images become stable synthetic locators before semantic import.
pub fn normalize_legacy_xhtml(value: &str, image_paths: &BTreeMap<usize, String>) -> String {
    let mut value = normalize_xhtml_for_xml(value)
        .replace("<mbp:pagebreak/>", "<div class=\"ff-pagebreak\"></div>")
        .replace("<mbp:pagebreak />", "<div class=\"ff-pagebreak\"></div>")
        .replace("<meta charset=\"utf-8\">", "<meta charset=\"utf-8\"/>");
    for (record_index, path) in image_paths {
        for quote in ['"', '\''] {
            let needle = format!("recindex={quote}{record_index}{quote}");
            let replacement = format!("src=\"{}\"", escape_xml(path));
            value = value.replace(&needle, &replacement);
        }
    }
    if value.to_ascii_lowercase().contains("<html") {
        value
    } else {
        format!(
            "<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><body>{value}</body></html>"
        )
    }
}

/// Make an XHTML string safe for the shared XML parser without applying any
/// Kindle-specific record-image or page-break rewrites.  EPUB producers often
/// retain HTML named entities such as `&nbsp;` even though the package is
/// declared as XHTML/XML; the known entities are converted to numeric XML
/// references while unknown entities remain a hard parse error.
pub fn normalize_xhtml_for_xml(value: &str) -> String {
    normalize_common_html_entities(&strip_external_xhtml_doctype(value))
}

/// Normalize an NCX document that declares the canonical DAISY 2005 NCX DTD.
/// The external subset is never fetched or expanded: only that exact public
/// and system identifier pair is removed. Internal subsets and other external
/// identifiers remain intact so the strict XML parser rejects them.
pub fn normalize_ncx_for_xml(value: &str) -> String {
    normalize_common_html_entities(&strip_standard_ncx_doctype(value))
}

/// Remove only the canonical external DAISY NCX 2005 DTD declaration, and
/// only when it has no internal subset.
pub fn strip_standard_ncx_doctype(value: &str) -> String {
    const PUBLIC_ID: &str = "-//NISO//DTD ncx 2005-1//EN";
    const SYSTEM_ID: &str = "http://www.daisy.org/z3986/2005/ncx-2005-1.dtd";

    let lower = value.to_ascii_lowercase();
    let Some(start) = lower.find("<!doctype") else {
        return value.to_owned();
    };
    let Some(end) = doctype_end(value, start) else {
        return value.to_owned();
    };
    let declaration = &value[start..end];
    let Some(body) = declaration.get("<!DOCTYPE".len()..declaration.len().saturating_sub(1)) else {
        return value.to_owned();
    };
    let mut tokens = body.trim_start().splitn(2, char::is_whitespace);
    let Some(root_name) = tokens.next() else {
        return value.to_owned();
    };
    let Some(external_id) = tokens.next().map(str::trim_start) else {
        return value.to_owned();
    };
    if root_name != "ncx"
        || !external_id
            .get(..6)
            .is_some_and(|id| id.eq_ignore_ascii_case("PUBLIC"))
    {
        return value.to_owned();
    }

    let Some((public_id, after_public)) = take_quoted(external_id[6..].trim_start()) else {
        return value.to_owned();
    };
    let Some((system_id, trailing)) = take_quoted(after_public.trim_start()) else {
        return value.to_owned();
    };
    if public_id != PUBLIC_ID || system_id != SYSTEM_ID || !trailing.trim().is_empty() {
        return value.to_owned();
    }

    let mut normalized = String::with_capacity(value.len() - (end - start));
    normalized.push_str(&value[..start]);
    normalized.push_str(&value[end..]);
    normalized
}

fn take_quoted(value: &str) -> Option<(&str, &str)> {
    let quote = value.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let rest = &value[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some((&rest[..end], &rest[end + quote.len_utf8()..]))
}

fn doctype_end(value: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    let mut subset_depth = 0usize;
    for (offset, character) in value[start..].char_indices() {
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '[' => subset_depth = subset_depth.checked_add(1)?,
            ']' => subset_depth = subset_depth.checked_sub(1)?,
            '>' if subset_depth == 0 => return Some(start + offset + character.len_utf8()),
            _ => {}
        }
    }
    None
}

/// Kindle's legacy XHTML commonly uses HTML named entities even though its
/// document is declared as XML.  XML only defines five named entities, so
/// translate the common typographic/spacing entities to numeric references and
/// leave unknown names untouched for the XML parser to report explicitly.
fn normalize_common_html_entities(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0usize;
    while let Some(relative) = value[cursor..].find('&') {
        let start = cursor + relative;
        output.push_str(&value[cursor..start]);
        let tail = &value[start + 1..];
        let Some(end_relative) = tail.find(';') else {
            output.push('&');
            cursor = start + 1;
            continue;
        };
        let end = start + 1 + end_relative;
        let name = &value[start + 1..end];
        let replacement = match name {
            "nbsp" => Some("&#160;"),
            "ensp" => Some("&#8194;"),
            "emsp" => Some("&#8195;"),
            "thinsp" => Some("&#8201;"),
            "ndash" => Some("&#8211;"),
            "mdash" => Some("&#8212;"),
            "lsquo" => Some("&#8216;"),
            "rsquo" => Some("&#8217;"),
            "sbquo" => Some("&#8218;"),
            "ldquo" => Some("&#8220;"),
            "rdquo" => Some("&#8221;"),
            "bdquo" => Some("&#8222;"),
            "dagger" => Some("&#8224;"),
            "Dagger" => Some("&#8225;"),
            "hellip" => Some("&#8230;"),
            "permil" => Some("&#8240;"),
            "prime" => Some("&#8242;"),
            "Prime" => Some("&#8243;"),
            "lsaquo" => Some("&#8249;"),
            "rsaquo" => Some("&#8250;"),
            "bull" => Some("&#8226;"),
            "copy" => Some("&#169;"),
            "reg" => Some("&#174;"),
            "trade" => Some("&#8482;"),
            "euro" => Some("&#8364;"),
            "yen" => Some("&#165;"),
            "pound" => Some("&#163;"),
            "cent" => Some("&#162;"),
            "sect" => Some("&#167;"),
            "para" => Some("&#182;"),
            "middot" => Some("&#183;"),
            "plusmn" => Some("&#177;"),
            "times" => Some("&#215;"),
            "divide" => Some("&#247;"),
            "ne" => Some("&#8800;"),
            "le" => Some("&#8804;"),
            "ge" => Some("&#8805;"),
            "infin" => Some("&#8734;"),
            "alpha" => Some("&#945;"),
            "beta" => Some("&#946;"),
            "gamma" => Some("&#947;"),
            "Delta" => Some("&#916;"),
            "omega" => Some("&#969;"),
            "auml" => Some("&#228;"),
            "Auml" => Some("&#196;"),
            "euml" => Some("&#235;"),
            "Euml" => Some("&#203;"),
            "iuml" => Some("&#239;"),
            "Iuml" => Some("&#207;"),
            "ouml" => Some("&#246;"),
            "Ouml" => Some("&#214;"),
            "uuml" => Some("&#252;"),
            "Uuml" => Some("&#220;"),
            "yuml" => Some("&#255;"),
            "Yuml" => Some("&#376;"),
            "aring" => Some("&#229;"),
            "Aring" => Some("&#197;"),
            "ccedil" => Some("&#231;"),
            "Ccedil" => Some("&#199;"),
            "ntilde" => Some("&#241;"),
            "Ntilde" => Some("&#209;"),
            "szlig" => Some("&#223;"),
            "agrave" => Some("&#224;"),
            "Agrave" => Some("&#192;"),
            "eacute" => Some("&#233;"),
            "Eacute" => Some("&#201;"),
            "iacute" => Some("&#237;"),
            "Iacute" => Some("&#205;"),
            "oacute" => Some("&#243;"),
            "Oacute" => Some("&#211;"),
            "uacute" => Some("&#250;"),
            "Uacute" => Some("&#218;"),
            "sim" => Some("&#8764;"),
            "asymp" => Some("&#8776;"),
            "cong" => Some("&#8773;"),
            "micro" => Some("&#181;"),
            "deg" => Some("&#176;"),
            "sup2" => Some("&#178;"),
            "sup3" => Some("&#179;"),
            "frac12" => Some("&#189;"),
            "frac14" => Some("&#188;"),
            "frac34" => Some("&#190;"),
            "not" => Some("&#172;"),
            "there4" => Some("&#8756;"),
            "forall" => Some("&#8704;"),
            "exist" => Some("&#8707;"),
            "isin" => Some("&#8712;"),
            "notin" => Some("&#8713;"),
            "sum" => Some("&#8721;"),
            "prod" => Some("&#8719;"),
            "radic" => Some("&#8730;"),
            _ => None,
        };
        if let Some(replacement) = replacement {
            output.push_str(replacement);
            cursor = end + 1;
        } else {
            output.push('&');
            cursor = start + 1;
        }
    }
    output.push_str(&value[cursor..]);
    output
}

/// Standard Kindle XHTML frequently carries an external XHTML DTD.  The
/// shared XML parser intentionally does not fetch or process external DTDs;
/// remove only a declaration without an internal subset and leave all other
/// declarations untouched so malformed or potentially semantic content still
/// fails closed.
pub fn strip_external_xhtml_doctype(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let Some(start) = lower.find("<!doctype") else {
        return value.to_owned();
    };
    let Some(end_relative) = value[start..].find('>') else {
        return value.to_owned();
    };
    let end = start + end_relative + 1;
    let declaration = &value[start..end];
    if declaration.contains('[') {
        return value.to_owned();
    }
    let body = declaration
        .trim_start_matches(|character: char| character.is_ascii_whitespace())
        .to_ascii_lowercase();
    if !body.starts_with("<!doctype html") {
        return value.to_owned();
    }
    let mut output = String::with_capacity(value.len().saturating_sub(declaration.len()));
    output.push_str(&value[..start]);
    output.push_str(&value[end..]);
    output
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

const MATHML_NAMESPACE: &str = "http://www.w3.org/1998/Math/MathML";

/// Serialize only a bounded MathML subtree.  The normalized IR never carries
/// source XML verbatim: namespace, element depth, element count, and
/// attributes are checked here so exporters do not become an XML injection or
/// external-entity surface.
fn sanitize_mathml(element: XmlNode<'_, '_>) -> Option<String> {
    if element.tag_name().name() != "math"
        || element.tag_name().namespace() != Some(MATHML_NAMESPACE)
    {
        return None;
    }
    let mut output = String::new();
    let mut count = 0usize;
    serialize_mathml(element, 0, &mut count, &mut output).ok()?;
    Some(output)
}

fn serialize_mathml(
    element: XmlNode<'_, '_>,
    depth: usize,
    count: &mut usize,
    output: &mut String,
) -> Result<(), ()> {
    if depth > 64 {
        return Err(());
    }
    *count = count.saturating_add(1);
    if *count > 4096 || element.tag_name().namespace() != Some(MATHML_NAMESPACE) {
        return Err(());
    }
    let name = element.tag_name().name();
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err(());
    }
    output.push('<');
    output.push_str(name);
    if depth == 0 {
        output.push_str(" xmlns=\"");
        output.push_str(MATHML_NAMESPACE);
        output.push('"');
    }
    for attribute in element.attributes() {
        let name = attribute.name();
        let allowed = matches!(
            name,
            "alttext"
                | "display"
                | "encoding"
                | "id"
                | "mathvariant"
                | "displaystyle"
                | "scriptlevel"
                | "stretchy"
                | "fence"
                | "separator"
                | "open"
                | "close"
                | "movablelimits"
                | "accent"
                | "rowalign"
                | "columnalign"
                | "columnspacing"
                | "rowspacing"
        );
        if allowed {
            output.push(' ');
            output.push_str(name);
            output.push_str("=\"");
            output.push_str(&escape_xml(attribute.value()));
            output.push('"');
        }
    }
    let has_element_children = element.children().any(|child| child.is_element());
    let text = element
        .children()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect::<String>();
    if !has_element_children && text.is_empty() {
        output.push_str("/>");
        return Ok(());
    }
    output.push('>');
    if !text.is_empty() {
        output.push_str(&escape_xml(&text));
    }
    for child in element.children().filter(|child| child.is_element()) {
        serialize_mathml(child, depth + 1, count, output)?;
    }
    output.push_str("</");
    output.push_str(name);
    output.push('>');
    Ok(())
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-normalize/src/lib.rs"]
mod tests;
