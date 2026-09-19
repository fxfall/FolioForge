//! FictionBook 2 XML input adapter.
//!
//! FB2's XML vocabulary is decoded into the format-neutral model.  Binary
//! payloads are validated and kept behind the model's resource loader; the
//! Base64 strings never become long-lived IR text nodes.

use std::{collections::BTreeMap, fs, sync::Arc};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use folio_format::{
    DetectionConfidence, DetectionResult, FormatAdapter, FormatError, FormatSupport, ImportContext,
    ImportedBook,
};
use folio_input::{BookSource, DetectedFormat, SourceKind};
use folio_model::{
    Anchor, AnchorId, Book, ComputedStyle, Confidence, Diagnostic, Document, DocumentId,
    MemoryResourceLoader, Metadata, NavPoint, Node, NodeId, NodeKind, Resource, ResourceId,
    ResourceKind, SemanticRole,
};
use roxmltree::{Document as XmlDocument, Node as XmlNode};

const MAX_BINARY_BYTES: usize = 256 << 20;
const MAX_XML_NODES: usize = 2_000_000;

#[derive(Debug, thiserror::Error)]
pub enum Fb2Error {
    #[error("FB2 source must be one regular .fb2 file")]
    Source,
    #[error("FB2 XML is invalid: {0}")]
    Xml(String),
    #[error("FB2 input is invalid: {0}")]
    Invalid(String),
    #[error("FB2 safety limit exceeded: {0}")]
    Limit(String),
    #[error("FB2 source I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

pub struct Fb2Adapter;

impl FormatAdapter for Fb2Adapter {
    fn format(&self) -> DetectedFormat {
        DetectedFormat::Fb2
    }

    fn support(&self) -> FormatSupport {
        FormatSupport {
            detect: true,
            inspect: true,
            import: true,
            metadata_read: true,
            metadata_write: true,
            edit: true,
            preview: true,
            ..FormatSupport::default()
        }
    }

    fn detect(&self, source: &BookSource) -> Result<DetectionResult, FormatError> {
        if source.kind() != SourceKind::SingleFile {
            return Err(FormatError::Unsupported(
                "FB2 adapter expects one .fb2 file".to_owned(),
            ));
        }
        let file = source
            .files()
            .first()
            .ok_or_else(|| FormatError::Unsupported("source contains no files".to_owned()))?;
        let extension = file
            .relative_path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        let bytes = fs::read(&file.path)?;
        if extension.as_deref() != Some("fb2") && !looks_like_fb2(&bytes) {
            return Err(FormatError::Unsupported(
                "FB2 extension or FictionBook root element is missing".to_owned(),
            ));
        }
        Ok(DetectionResult {
            format: DetectedFormat::Fb2.name().to_owned(),
            confidence: DetectionConfidence::Exact,
            evidence: vec!["FictionBook root signature or .fb2 extension".to_owned()],
            warnings: Vec::new(),
        })
    }

    fn import(
        &self,
        source: &BookSource,
        context: &ImportContext,
    ) -> Result<ImportedBook, FormatError> {
        let file = source
            .files()
            .first()
            .ok_or_else(|| FormatError::Unsupported("source contains no files".to_owned()))?;
        let bytes = fs::read(&file.path)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|error| FormatError::Invalid(Fb2Error::Xml(error.to_string()).to_string()))?;
        let xml = XmlDocument::parse(text)
            .map_err(|error| FormatError::Invalid(Fb2Error::Xml(error.to_string()).to_string()))?;
        let node_count = xml.descendants().count();
        if node_count > MAX_XML_NODES {
            return Err(FormatError::Invalid(
                Fb2Error::Limit(format!("FB2 XML has more than {MAX_XML_NODES} nodes")).to_string(),
            ));
        }
        let root = xml.root_element();
        if !local_name(root).eq_ignore_ascii_case("fictionbook") {
            return Err(FormatError::Invalid(
                "FB2 document root is not FictionBook".to_owned(),
            ));
        }
        let mut state = Decoder::new(context.strict);
        state.collect_binaries(root)?;
        state.mark_cover_image(root);
        state.metadata = metadata_from(root);
        state.decode_bodies(root)?;
        let input_loss = state
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.starts_with("FF-FB2-"))
            .map(|diagnostic| diagnostic.message.clone())
            .collect();
        let diagnostics = state.diagnostics.clone();
        let book = state.finish();
        Ok(ImportedBook {
            format: DetectedFormat::Fb2,
            parser: "folio-fb2/1".to_owned(),
            book,
            diagnostics,
            input_loss,
            text: None,
        })
    }
}

struct Decoder {
    strict: bool,
    book: Book,
    loader: MemoryResourceLoader,
    resources: Vec<Resource>,
    resource_by_binary: BTreeMap<String, ResourceId>,
    diagnostics: Vec<Diagnostic>,
    metadata: Metadata,
    anchors: Vec<Anchor>,
    navigation: Vec<NavPoint>,
    note_targets: std::collections::BTreeSet<String>,
    note_document: Option<DocumentId>,
    next_node: u32,
    next_anchor: u32,
}

impl Decoder {
    fn new(strict: bool) -> Self {
        let mut book = Book::new();
        book.styles.intern(ComputedStyle::default());
        Self {
            strict,
            book,
            loader: MemoryResourceLoader::default(),
            resources: Vec::new(),
            resource_by_binary: BTreeMap::new(),
            diagnostics: Vec::new(),
            metadata: Metadata::default(),
            anchors: Vec::new(),
            navigation: Vec::new(),
            note_targets: std::collections::BTreeSet::new(),
            note_document: None,
            next_node: 0,
            next_anchor: 0,
        }
    }

    fn collect_binaries(&mut self, root: XmlNode<'_, '_>) -> Result<(), FormatError> {
        for binary in root
            .descendants()
            .filter(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("binary"))
        {
            let Some(id) = binary.attribute("id").filter(|value| !value.is_empty()) else {
                self.diagnostics.push(Diagnostic::warning(
                    "FF-FB2-BINARY-0001",
                    "FB2 binary without an id was ignored",
                ));
                continue;
            };
            let encoded = binary.text().unwrap_or_default();
            let decoded = STANDARD
                .decode(
                    encoded
                        .bytes()
                        .filter(|byte| !byte.is_ascii_whitespace())
                        .collect::<Vec<_>>(),
                )
                .map_err(|error| {
                    FormatError::Invalid(
                        Fb2Error::Invalid(format!("binary {id} has invalid Base64: {error}"))
                            .to_string(),
                    )
                })?;
            if decoded.len() > MAX_BINARY_BYTES {
                return Err(FormatError::Invalid(
                    Fb2Error::Limit(format!("binary {id} exceeds {MAX_BINARY_BYTES} bytes"))
                        .to_string(),
                ));
            }
            let media_type = binary
                .attribute("content-type")
                .unwrap_or("application/octet-stream")
                .to_owned();
            let path = format!("fb2/{id}");
            let kind = resource_kind(&media_type, id);
            let resource_id = ResourceId::new(self.resources.len() as u32);
            self.loader.insert(path.clone(), decoded.clone());
            self.resources.push(Resource {
                id: resource_id,
                path,
                media_type,
                kind,
                properties: Vec::new(),
                size: Some(decoded.len() as u64),
            });
            self.resource_by_binary.insert(id.to_owned(), resource_id);
        }
        Ok(())
    }

    fn mark_cover_image(&mut self, root: XmlNode<'_, '_>) {
        let Some(reference) = root
            .descendants()
            .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("coverpage"))
            .and_then(|coverpage| {
                coverpage.descendants().find_map(|node| {
                    (node.is_element() && local_name(node).eq_ignore_ascii_case("image"))
                        .then(|| attribute_local(node, "href"))
                        .flatten()
                })
            })
        else {
            return;
        };
        let binary_id = reference.trim_start_matches('#');
        let Some(resource_id) = self.resource_by_binary.get(binary_id).copied() else {
            self.diagnostics.push(Diagnostic::warning(
                "FF-FB2-COVER-0001",
                format!("FB2 cover references missing binary '{binary_id}'"),
            ));
            return;
        };
        if let Some(resource) = self
            .resources
            .iter_mut()
            .find(|item| item.id == resource_id)
        {
            if !resource
                .properties
                .iter()
                .any(|property| property == "cover-image")
            {
                resource.properties.push("cover-image".to_owned());
            }
        }
    }

    fn decode_bodies(&mut self, root: XmlNode<'_, '_>) -> Result<(), FormatError> {
        let bodies = root
            .children()
            .filter(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("body"))
            .collect::<Vec<_>>();
        if bodies.is_empty() {
            return Err(FormatError::Invalid(
                "FB2 document contains no body".to_owned(),
            ));
        }
        for (index, body) in bodies.iter().enumerate() {
            if body
                .attribute("name")
                .is_some_and(|name| name.eq_ignore_ascii_case("notes"))
            {
                self.note_document = Some(DocumentId::new(index as u32));
                for node in body.descendants().filter(|node| node.is_element()) {
                    if let Some(id) = node.attribute("id").filter(|id| !id.is_empty()) {
                        self.note_targets.insert(id.to_owned());
                    }
                }
            }
        }
        for (index, body) in bodies.into_iter().enumerate() {
            let document_id = DocumentId::new(index as u32);
            let mut nodes = Vec::new();
            let mut nav_points = Vec::new();
            for child in body.children().filter(|node| node.is_element()) {
                if local_name(child).eq_ignore_ascii_case("section") {
                    let (node, nav) = self.section_node(child, document_id)?;
                    nodes.push(node);
                    if let Some(nav) = nav {
                        nav_points.push(nav);
                    }
                } else if let Some(node) = self.block_node(child, document_id)? {
                    nodes.push(node);
                }
            }
            if index == 0 {
                self.navigation = nav_points;
            }
            let title = nodes.iter().find_map(|node| match node.kind {
                NodeKind::Heading { .. } => Some(node.text_content().trim().to_owned()),
                _ => None,
            });
            self.book.documents.push(Document {
                id: document_id,
                href: format!("body-{}.xhtml", index + 1),
                media_type: "application/xhtml+xml".to_owned(),
                title,
                nodes,
            });
        }
        Ok(())
    }

    fn section_node(
        &mut self,
        section: XmlNode<'_, '_>,
        document: DocumentId,
    ) -> Result<(Node, Option<NavPoint>), FormatError> {
        let id = self.allocate_node();
        let mut children = Vec::new();
        let mut nav_children = Vec::new();
        for child in section.children().filter(|node| node.is_element()) {
            if local_name(child).eq_ignore_ascii_case("section") {
                let (node, nav) = self.section_node(child, document)?;
                children.push(node);
                if let Some(nav) = nav {
                    nav_children.push(nav);
                }
            } else if let Some(node) = self.block_node(child, document)? {
                children.push(node);
            }
        }
        let label = section
            .children()
            .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("title"))
            .map(element_text)
            .filter(|value| !value.is_empty());
        let anchor_name = section
            .attribute("id")
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("fb2-section-{}", self.next_anchor));
        if section.attribute("id").is_some() || label.is_some() {
            self.anchors.push(Anchor {
                id: AnchorId::new(self.next_anchor),
                document,
                node: id,
                name: anchor_name.clone(),
            });
            self.next_anchor = self.next_anchor.saturating_add(1);
        }
        let node = Node::new(id, NodeKind::Section, self.style(), children).with_semantics(
            SemanticRole::Section,
            Default::default(),
            Confidence::Explicit,
        );
        let nav = label.map(|label| NavPoint {
            label,
            href: format!("body-{}.xhtml#{}", document.get() + 1, anchor_name),
            children: nav_children,
        });
        Ok((node, nav))
    }

    fn block_node(
        &mut self,
        element: XmlNode<'_, '_>,
        document: DocumentId,
    ) -> Result<Option<Node>, FormatError> {
        let tag = local_name(element).to_ascii_lowercase();
        match tag.as_str() {
            "title" => Ok(Some(self.heading_node(element, 2, document)?)),
            "subtitle" => Ok(Some(self.heading_node(element, 3, document)?)),
            "p" => Ok(Some(self.container_node(
                NodeKind::Paragraph,
                SemanticRole::Paragraph,
                element,
                document,
            )?)),
            "epigraph" | "cite" => Ok(Some(self.block_container_node(
                NodeKind::BlockQuote,
                SemanticRole::Quote,
                element,
                document,
            )?)),
            "poem" => Ok(Some(self.block_container_node(
                NodeKind::GenericBlock { tag },
                SemanticRole::Poetry,
                element,
                document,
            )?)),
            "stanza" => Ok(Some(self.block_container_node(
                NodeKind::GenericBlock { tag },
                SemanticRole::Poetry,
                element,
                document,
            )?)),
            "list" => Ok(Some(self.list_node(element, document)?)),
            "list-item" => Ok(Some(self.block_container_node(
                NodeKind::ListItem,
                SemanticRole::List,
                element,
                document,
            )?)),
            "v" => Ok(Some(self.container_node(
                NodeKind::GenericBlock {
                    tag: "verse".to_owned(),
                },
                SemanticRole::Poetry,
                element,
                document,
            )?)),
            "text-author" => Ok(Some(self.container_node(
                NodeKind::GenericBlock { tag },
                SemanticRole::Poetry,
                element,
                document,
            )?)),
            "empty-line" => Ok(Some(
                Node::new(
                    self.allocate_node(),
                    NodeKind::PageBreak,
                    self.style(),
                    Vec::new(),
                )
                .with_semantics(
                    SemanticRole::SceneBreak,
                    Default::default(),
                    Confidence::Explicit,
                ),
            )),
            "image" => Ok(Some(self.image_node(element)?)),
            "section" => Ok(Some(self.section_node(element, document)?.0)),
            "annotation" | "description" | "document-info" | "publish-info" | "title-info"
            | "coverpage" | "keywords" | "sequence" => Ok(None),
            _ => {
                if self.strict {
                    return Err(FormatError::Invalid(format!(
                        "unsupported important FB2 element: {tag}"
                    )));
                }
                self.diagnostics.push(Diagnostic::warning(
                    "FF-FB2-UNKNOWN-0001",
                    format!("unsupported FB2 element '{tag}' was preserved as generic content"),
                ));
                Ok(Some(self.container_node(
                    NodeKind::GenericBlock { tag },
                    SemanticRole::Generic,
                    element,
                    document,
                )?))
            }
        }
    }

    fn list_node(
        &mut self,
        element: XmlNode<'_, '_>,
        document: DocumentId,
    ) -> Result<Node, FormatError> {
        let mut children = Vec::new();
        for child in element.children().filter(|node| node.is_element()) {
            if local_name(child).eq_ignore_ascii_case("list-item") {
                if let Some(node) = self.block_node(child, document)? {
                    children.push(node);
                }
                continue;
            }
            // FB2 permits a compact list form with direct <p> children.
            // Promote each paragraph to a list item so reading order and
            // block boundaries survive the shared IR projection.
            if local_name(child).eq_ignore_ascii_case("p") {
                let paragraph = self.container_node(
                    NodeKind::Paragraph,
                    SemanticRole::Paragraph,
                    child,
                    document,
                )?;
                children.push(
                    Node::new(
                        self.allocate_node(),
                        NodeKind::ListItem,
                        self.style(),
                        vec![paragraph],
                    )
                    .with_semantics(
                        SemanticRole::List,
                        Default::default(),
                        Confidence::Explicit,
                    ),
                );
                continue;
            }
            if let Some(node) = self.block_node(child, document)? {
                children.push(node);
            }
        }
        if children.is_empty() {
            children = self.inline_children(element)?;
        }
        Ok(Node::new(
            self.allocate_node(),
            NodeKind::UnorderedList,
            self.style(),
            children,
        )
        .with_semantics(SemanticRole::List, Default::default(), Confidence::Explicit))
    }

    fn heading_node(
        &mut self,
        element: XmlNode<'_, '_>,
        level: u8,
        _document: DocumentId,
    ) -> Result<Node, FormatError> {
        let children = self.inline_children(element)?;
        Ok(Node::new(
            self.allocate_node(),
            NodeKind::Heading { level },
            self.style(),
            children,
        )
        .with_semantics(
            SemanticRole::Heading,
            Default::default(),
            Confidence::Explicit,
        ))
    }

    fn container_node(
        &mut self,
        kind: NodeKind,
        role: SemanticRole,
        element: XmlNode<'_, '_>,
        document: DocumentId,
    ) -> Result<Node, FormatError> {
        let node_id = self.allocate_node();
        self.register_anchor(element, document, node_id);
        let children = self.inline_children(element)?;
        Ok(
            Node::new(node_id, kind, self.style(), children).with_semantics(
                role,
                Default::default(),
                Confidence::Explicit,
            ),
        )
    }

    fn block_container_node(
        &mut self,
        kind: NodeKind,
        role: SemanticRole,
        element: XmlNode<'_, '_>,
        document: DocumentId,
    ) -> Result<Node, FormatError> {
        let node_id = self.allocate_node();
        self.register_anchor(element, document, node_id);
        let mut children = Vec::new();
        for child in element.children().filter(|node| node.is_element()) {
            if let Some(node) = self.block_node(child, document)? {
                children.push(node);
            }
        }
        if children.is_empty() {
            children = self.inline_children(element)?;
        }
        Ok(
            Node::new(node_id, kind, self.style(), children).with_semantics(
                role,
                Default::default(),
                Confidence::Explicit,
            ),
        )
    }

    fn inline_children(&mut self, element: XmlNode<'_, '_>) -> Result<Vec<Node>, FormatError> {
        let mut result = Vec::new();
        for child in element.children() {
            if child.is_text() {
                let value = normalize_text(child.text().unwrap_or_default());
                if !value.is_empty() {
                    result.push(Node::new(
                        self.allocate_node(),
                        NodeKind::Text { value },
                        self.style(),
                        Vec::new(),
                    ));
                }
                continue;
            }
            if !child.is_element() {
                continue;
            }
            let tag = local_name(child).to_ascii_lowercase();
            let id = self.allocate_node();
            let node = match tag.as_str() {
                "emphasis" => Node::new(
                    id,
                    NodeKind::Emphasis,
                    self.style(),
                    self.inline_children(child)?,
                )
                .with_semantics(
                    SemanticRole::Emphasis,
                    Default::default(),
                    Confidence::Explicit,
                ),
                "strong" => Node::new(
                    id,
                    NodeKind::Strong,
                    self.style(),
                    self.inline_children(child)?,
                )
                .with_semantics(
                    SemanticRole::Strong,
                    Default::default(),
                    Confidence::Explicit,
                ),
                "strikethrough" => Node::new(
                    id,
                    NodeKind::GenericInline {
                        tag: "s".to_owned(),
                    },
                    self.style(),
                    self.inline_children(child)?,
                ),
                "a" => {
                    let raw_href = attribute_local(child, "href")
                        .unwrap_or_default()
                        .to_owned();
                    let target = raw_href.strip_prefix('#');
                    let explicit_note = attribute_local(child, "type")
                        .is_some_and(|value| value.eq_ignore_ascii_case("note"));
                    let note = explicit_note
                        || target.is_some_and(|value| self.note_targets.contains(value));
                    let href = if note {
                        if let (Some(target), Some(document)) = (target, self.note_document) {
                            format!("body-{}.xhtml#{target}", document.get() + 1)
                        } else {
                            raw_href
                        }
                    } else {
                        raw_href
                    };
                    if note {
                        Node::new(
                            id,
                            NodeKind::Footnote { href: Some(href) },
                            self.style(),
                            self.inline_children(child)?,
                        )
                        .with_semantics(
                            SemanticRole::Footnote,
                            Default::default(),
                            Confidence::Explicit,
                        )
                    } else {
                        Node::new(
                            id,
                            NodeKind::Link { href },
                            self.style(),
                            self.inline_children(child)?,
                        )
                        .with_semantics(
                            SemanticRole::Link,
                            Default::default(),
                            Confidence::Explicit,
                        )
                    }
                }
                "image" => self.image_node(child)?,
                "code" => Node::new(
                    id,
                    NodeKind::Code,
                    self.style(),
                    vec![Node::new(
                        self.allocate_node(),
                        NodeKind::Text {
                            value: element_text(child),
                        },
                        self.style(),
                        Vec::new(),
                    )],
                ),
                _ => {
                    if self.strict && !matches!(tag.as_str(), "sub" | "sup" | "style") {
                        return Err(FormatError::Invalid(format!(
                            "unsupported FB2 inline element: {tag}"
                        )));
                    }
                    Node::new(
                        id,
                        NodeKind::GenericInline { tag },
                        self.style(),
                        self.inline_children(child)?,
                    )
                }
            };
            result.push(node);
        }
        Ok(result)
    }

    fn image_node(&mut self, element: XmlNode<'_, '_>) -> Result<Node, FormatError> {
        let id = self.allocate_node();
        let Some(reference) = attribute_local(element, "href") else {
            self.diagnostics.push(Diagnostic::warning(
                "FF-FB2-IMAGE-0001",
                "FB2 image without an xlink:href was preserved as a generic inline node",
            ));
            return Ok(Node::new(
                id,
                NodeKind::GenericInline {
                    tag: "image".to_owned(),
                },
                self.style(),
                Vec::new(),
            ));
        };
        let binary_id = reference.trim_start_matches('#');
        let Some(resource) = self.resource_by_binary.get(binary_id).copied() else {
            self.diagnostics.push(Diagnostic::warning(
                "FF-FB2-IMAGE-0002",
                format!("FB2 image references missing binary '{binary_id}'"),
            ));
            return Ok(Node::new(
                id,
                NodeKind::GenericInline {
                    tag: "image".to_owned(),
                },
                self.style(),
                Vec::new(),
            ));
        };
        Ok(Node::new(
            id,
            NodeKind::Image {
                resource,
                alt: String::new(),
            },
            self.style(),
            Vec::new(),
        )
        .with_semantics(
            SemanticRole::Image,
            Default::default(),
            Confidence::Explicit,
        ))
    }

    fn style(&self) -> folio_model::StyleId {
        folio_model::StyleId::new(0)
    }

    fn register_anchor(&mut self, element: XmlNode<'_, '_>, document: DocumentId, node: NodeId) {
        let Some(name) = element
            .attribute("id")
            .or_else(|| element.attribute("name"))
            .filter(|value| !value.is_empty())
        else {
            return;
        };
        if self
            .anchors
            .iter()
            .any(|anchor| anchor.document == document && anchor.name == name)
        {
            return;
        }
        self.anchors.push(Anchor {
            id: AnchorId::new(self.next_anchor),
            document,
            node,
            name: name.to_owned(),
        });
        self.next_anchor = self.next_anchor.saturating_add(1);
    }

    fn allocate_node(&mut self) -> NodeId {
        let id = NodeId::new(self.next_node);
        self.next_node = self.next_node.saturating_add(1);
        id
    }

    fn finish(mut self) -> Book {
        self.book.metadata = self.metadata;
        self.book.resources = self.resources;
        self.book.anchors = self.anchors;
        self.book.navigation.toc = self.navigation;
        self.book = self.book.with_resource_loader(Arc::new(self.loader));
        self.book
    }
}

fn metadata_from(root: XmlNode<'_, '_>) -> Metadata {
    let mut metadata = Metadata::default();
    let Some(title_info) = root
        .descendants()
        .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("title-info"))
    else {
        return metadata;
    };
    metadata.title = title_info
        .children()
        .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("book-title"))
        .map(element_text)
        .filter(|value| !value.is_empty());
    if let Some(lang) = title_info
        .children()
        .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("lang"))
        .map(element_text)
        .filter(|value| !value.is_empty())
    {
        metadata.language = Some(lang);
    }
    for author in title_info
        .children()
        .filter(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("author"))
    {
        if let Some(name) = person_name(author) {
            metadata.add_author(name);
        }
    }
    for translator in title_info
        .children()
        .filter(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("translator"))
    {
        if let Some(name) = person_name(translator) {
            if !metadata.contributors.iter().any(|item| item == &name) {
                metadata.contributors.push(name);
            }
        }
    }
    if let Some(annotation) = title_info
        .children()
        .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("annotation"))
    {
        let value = element_text(annotation);
        if !value.is_empty() {
            metadata.description = Some(value);
        }
    }
    if let Some(keywords) = title_info
        .children()
        .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("keywords"))
    {
        metadata.subjects.extend(
            element_text(keywords)
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned),
        );
    }
    for genre in title_info
        .children()
        .filter(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("genre"))
    {
        let value = element_text(genre);
        if !value.is_empty() && !metadata.subjects.iter().any(|item| item == &value) {
            metadata.subjects.push(value);
        }
    }
    if metadata.date.is_none() {
        metadata.date = title_info
            .children()
            .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("date"))
            .map(element_text)
            .filter(|value| !value.is_empty());
    }
    if let Some(sequence) = title_info
        .children()
        .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("sequence"))
    {
        metadata.series = sequence.attribute("name").map(ToOwned::to_owned);
        metadata.series_index = sequence
            .attribute("number")
            .and_then(|value| value.parse().ok());
    }
    if let Some(publish) = root
        .descendants()
        .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("publish-info"))
    {
        metadata.publisher = publish
            .children()
            .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("publisher"))
            .map(element_text)
            .filter(|value| !value.is_empty());
        if let Some(isbn) = publish
            .children()
            .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("isbn"))
            .map(element_text)
            .filter(|value| !value.is_empty())
        {
            metadata.add_identifier(isbn);
        }
    }
    if let Some(document_info) = root
        .descendants()
        .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("document-info"))
    {
        metadata.date = document_info
            .children()
            .find(|node| node.is_element() && local_name(*node).eq_ignore_ascii_case("date"))
            .map(element_text)
            .filter(|value| !value.is_empty());
    }
    metadata
}

fn person_name(node: XmlNode<'_, '_>) -> Option<String> {
    let parts = ["first-name", "middle-name", "last-name", "nickname"]
        .into_iter()
        .filter_map(|name| {
            node.children()
                .find(|child| child.is_element() && local_name(*child).eq_ignore_ascii_case(name))
                .map(element_text)
        })
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn looks_like_fb2(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).ok().is_some_and(|text| {
        text.trim_start()
            .to_ascii_lowercase()
            .contains("<fictionbook")
    })
}

fn local_name<'a, 'input>(node: XmlNode<'a, 'input>) -> &'input str {
    node.tag_name()
        .name()
        .rsplit(':')
        .next()
        .unwrap_or(node.tag_name().name())
}

fn attribute_local<'a>(node: XmlNode<'a, '_>, name: &str) -> Option<&'a str> {
    node.attribute(name).or_else(|| {
        node.attributes()
            .find(|attribute| {
                attribute
                    .name()
                    .rsplit(':')
                    .next()
                    .unwrap_or(attribute.name())
                    .eq_ignore_ascii_case(name)
            })
            .map(|attribute| attribute.value())
    })
}

fn element_text(node: XmlNode<'_, '_>) -> String {
    node.descendants()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_text(value: &str) -> String {
    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.is_empty() {
        return if value.chars().any(char::is_whitespace) {
            " ".to_owned()
        } else {
            String::new()
        };
    }
    let mut normalized = parts.join(" ");
    if value.chars().next().is_some_and(char::is_whitespace) {
        normalized.insert(0, ' ');
    }
    if value.chars().last().is_some_and(char::is_whitespace) {
        normalized.push(' ');
    }
    normalized
}

fn resource_kind(media_type: &str, id: &str) -> ResourceKind {
    match media_type.to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => ResourceKind::Jpeg,
        "image/png" => ResourceKind::Png,
        "image/gif" => ResourceKind::Gif,
        "image/svg+xml" => ResourceKind::Svg,
        _ if id.to_ascii_lowercase().ends_with(".jpg")
            || id.to_ascii_lowercase().ends_with(".jpeg") =>
        {
            ResourceKind::Jpeg
        }
        _ if id.to_ascii_lowercase().ends_with(".png") => ResourceKind::Png,
        _ => ResourceKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> BookSource {
        let path = std::env::temp_dir().join(format!("folioforge-fb2-{}.fb2", std::process::id()));
        let value = r##"<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0" xmlns:l="http://www.w3.org/1999/xlink"><description><title-info><book-title>Example</book-title><author><first-name>Alice</first-name><last-name>Smith</last-name></author><lang>en</lang><sequence name="Series" number="2"/></title-info></description><body><section id="one"><title><p>Chapter One</p></title><p>Hello <emphasis>world</emphasis><a l:href="#n1" type="note">[1]</a>.</p><poem><stanza><v>Line one</v><v>Line two</v></stanza></poem></section></body><body name="notes"><section id="n1"><title><p>Note</p></title><p>Note text</p></section></body><binary id="cover.jpg" content-type="image/jpeg">aGVsbG8=</binary></FictionBook>"##;
        fs::write(&path, value).unwrap();
        BookSource::single_file(&path).unwrap()
    }

    #[test]
    fn imports_fb2_metadata_sections_poetry_and_binary() {
        let source = source();
        let imported = Fb2Adapter
            .import(&source, &ImportContext::default())
            .unwrap();
        assert_eq!(imported.book.metadata.title.as_deref(), Some("Example"));
        assert_eq!(imported.book.metadata.author_names(), ["Alice Smith"]);
        assert_eq!(imported.book.metadata.series.as_deref(), Some("Series"));
        assert_eq!(imported.book.resources.len(), 1);
        assert!(imported.book.documents[0]
            .nodes
            .iter()
            .any(|node| matches!(node.role, SemanticRole::Section)));
        assert_eq!(imported.book.documents.len(), 2);
        assert!(imported
            .book
            .anchors
            .iter()
            .any(|anchor| anchor.name == "n1"));
        fn contains_footnote(nodes: &[Node]) -> bool {
            nodes.iter().any(|node| {
                matches!(node.kind, NodeKind::Footnote { .. }) || contains_footnote(&node.children)
            })
        }
        assert!(contains_footnote(&imported.book.documents[0].nodes));
        let _ = fs::remove_file(source.files()[0].path.clone());
    }
}
