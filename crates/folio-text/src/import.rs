use std::collections::{BTreeMap, BTreeSet};

use folio_model::Book;
use serde::{Deserialize, Serialize};

use crate::{
    analyze_paragraphs, analyze_structure, decode_as, detect, infer_metadata, normalize_text_owned,
    parse_paragraphs, BookStructure, DecodeError, EncodingCandidate, EncodingConfidence,
    EncodingDetection, EncodingEvidence, NormalizationReport, ParagraphAnalysis, TextEncoding,
    TextImportMode, TextImportOptions, TextMetadataGuess,
};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextImportReport {
    pub encoding: EncodingDetection,
    pub normalization: NormalizationReport,
    pub mode: TextImportMode,
    pub paragraph_analysis: Option<ParagraphAnalysis>,
    pub structure: BookStructure,
    pub metadata_guess: TextMetadataGuess,
    pub markdown_like: bool,
    pub diagnostics: Vec<String>,
    pub input_loss: Vec<String>,
}

pub struct TextImportResult {
    pub book: Book,
    pub report: TextImportReport,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum TextImportError {
    #[error(transparent)]
    Decode(#[from] DecodeError),
}

pub fn import_bytes(
    bytes: &[u8],
    source_name: Option<&str>,
    options: &TextImportOptions,
) -> Result<TextImportResult, TextImportError> {
    let detection = match options.encoding_override {
        Some(encoding) => override_detection(encoding),
        None => detect(bytes).map_err(DecodeError::from)?,
    };
    let encoding = detection.selected.ok_or(DecodeError::AmbiguousEncoding)?;
    let decoded = decode_as(bytes, encoding)?;
    let (normalized, normalization) = normalize_text_owned(decoded);
    let structure = analyze_structure(&normalized, options.mode);
    let paragraph_analysis = analyze_paragraphs(&normalized, options.paragraph_mode);
    let candidates = flatten_candidates(&structure);
    let excluded_lines = candidates
        .iter()
        .map(|candidate| candidate.line_index)
        .collect::<BTreeSet<_>>();
    let paragraphs = parse_paragraphs(&normalized, &paragraph_analysis, &excluded_lines);
    let (metadata, metadata_guess) = infer_metadata(
        source_name,
        &normalized,
        options.title_override.as_deref(),
        options.author_override.as_deref(),
    );
    let markdown_like = looks_markdown_like(&normalized);
    let mut report = TextImportReport {
        encoding: detection,
        normalization,
        mode: options.mode,
        paragraph_analysis: Some(paragraph_analysis),
        structure,
        metadata_guess,
        markdown_like,
        ..TextImportReport::default()
    };
    if markdown_like
        && matches!(
            options.mode,
            TextImportMode::Auto | TextImportMode::Markdown
        )
    {
        report.diagnostics.push(
            "Markdown-like syntax detected while importing plain text; use the Markdown adapter when semantic Markdown nodes are required.".to_owned(),
        );
        report.input_loss.push(
            "Markdown tokens are not yet converted to semantic headings, lists, emphasis, or links.".to_owned(),
        );
    }
    if !report.structure.rejected.is_empty() {
        report.diagnostics.push(format!(
            "{} possible chapter/volume headings were rejected by confidence safeguards.",
            report.structure.rejected.len()
        ));
    }
    if report.encoding.confidence == EncodingConfidence::Medium {
        report.diagnostics.push(format!(
            "Text encoding {} was inferred from byte/language evidence; the user may override it.",
            encoding.label()
        ));
    }
    if report.metadata_guess.confidence.is_some() {
        report.diagnostics.push(
            "Title/author values are filename or opening-line heuristics; explicit user edits take precedence.".to_owned(),
        );
    }

    let book = build_book(metadata, paragraphs, &report.structure);
    Ok(TextImportResult { book, report })
}

fn override_detection(encoding: TextEncoding) -> EncodingDetection {
    EncodingDetection {
        selected: Some(encoding),
        confidence: EncodingConfidence::High,
        candidates: vec![EncodingCandidate {
            encoding,
            score: 100,
            evidence: vec![EncodingEvidence::UserOverride],
        }],
        evidence: vec![EncodingEvidence::UserOverride],
    }
}

fn flatten_candidates(structure: &BookStructure) -> Vec<crate::StructureCandidate> {
    fn visit(node: &crate::StructureNode, output: &mut Vec<crate::StructureCandidate>) {
        output.push(node.candidate.clone());
        for child in &node.children {
            visit(child, output);
        }
    }
    let mut output = Vec::new();
    for node in &structure.roots {
        visit(node, &mut output);
    }
    output.sort_by_key(|candidate| candidate.line_index);
    output
}

fn looks_markdown_like(text: &str) -> bool {
    let mut markers = 0usize;
    let mut nonblank = 0usize;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        nonblank += 1;
        let line = line.trim_start();
        if line.starts_with("# ")
            || line.starts_with("> ")
            || line.starts_with("- ")
            || line.starts_with("* ")
            || line.starts_with("+ ")
            || line.starts_with("```")
            || line.starts_with("---")
            || line.contains("**")
            || line.contains("[ ]")
            || line.contains("[x]")
        {
            markers += 1;
        }
    }
    markers >= 2 && markers.saturating_mul(100) >= nonblank.saturating_mul(8)
}

#[derive(Default)]
struct SectionBuilder {
    candidate: Option<crate::StructureCandidate>,
    paragraphs: Vec<crate::ParsedParagraph>,
}

#[derive(Default)]
struct VolumeBuilder {
    candidate: Option<crate::StructureCandidate>,
    paragraphs: Vec<crate::ParsedParagraph>,
    materialized: Vec<(
        folio_model::Node,
        folio_model::NavPoint,
        folio_model::Anchor,
    )>,
}

fn build_book(
    metadata: folio_model::Metadata,
    paragraphs: Vec<crate::ParsedParagraph>,
    structure: &BookStructure,
) -> Book {
    use folio_model::{Book, ComputedStyle, Document, DocumentId, Navigation};

    let candidates = flatten_candidates(structure);
    let mut events = candidates
        .into_iter()
        .map(|candidate| (candidate.line_index, ImportEvent::Heading(candidate)))
        .chain(
            paragraphs
                .into_iter()
                .map(|paragraph| (paragraph.start_line, ImportEvent::Paragraph(paragraph))),
        )
        .collect::<Vec<_>>();
    events.sort_by_key(|(line_index, event)| {
        let is_heading = matches!(event, ImportEvent::Heading(_));
        (*line_index, !is_heading)
    });

    let mut book = Book::new();
    book.metadata = metadata;
    let default_style = book.styles.intern(ComputedStyle::default());
    let mut next_node = 0u32;
    let mut next_anchor = 0u32;
    let mut document_anchors = Vec::new();
    let mut navigation = Vec::new();
    let mut root_nodes = Vec::new();
    let mut current_section = None::<SectionBuilder>;
    let mut current_volume = None::<VolumeBuilder>;

    for (_, event) in events {
        match event {
            ImportEvent::Heading(candidate) if candidate.kind == crate::StructureKind::Volume => {
                flush_section(
                    &mut current_section,
                    &mut current_volume,
                    &mut root_nodes,
                    &mut navigation,
                    &mut document_anchors,
                    &mut next_node,
                    &mut next_anchor,
                    default_style,
                );
                flush_volume(
                    &mut current_volume,
                    &mut root_nodes,
                    &mut navigation,
                    &mut document_anchors,
                    &mut next_node,
                    &mut next_anchor,
                    default_style,
                );
                current_volume = Some(VolumeBuilder {
                    candidate: Some(candidate),
                    ..VolumeBuilder::default()
                });
            }
            ImportEvent::Heading(candidate) => {
                flush_section(
                    &mut current_section,
                    &mut current_volume,
                    &mut root_nodes,
                    &mut navigation,
                    &mut document_anchors,
                    &mut next_node,
                    &mut next_anchor,
                    default_style,
                );
                current_section = Some(SectionBuilder {
                    candidate: Some(candidate),
                    ..SectionBuilder::default()
                });
            }
            ImportEvent::Paragraph(paragraph) => {
                if let Some(section) = current_section.as_mut() {
                    section.paragraphs.push(paragraph);
                } else if let Some(volume) = current_volume.as_mut() {
                    volume.paragraphs.push(paragraph);
                } else {
                    root_nodes.push(make_paragraph_node(
                        paragraph,
                        &mut next_node,
                        default_style,
                    ));
                }
            }
        }
    }
    flush_section(
        &mut current_section,
        &mut current_volume,
        &mut root_nodes,
        &mut navigation,
        &mut document_anchors,
        &mut next_node,
        &mut next_anchor,
        default_style,
    );
    flush_volume(
        &mut current_volume,
        &mut root_nodes,
        &mut navigation,
        &mut document_anchors,
        &mut next_node,
        &mut next_anchor,
        default_style,
    );

    book.anchors = document_anchors;
    book.navigation = Navigation {
        toc: navigation,
        ..Navigation::default()
    };
    book.documents.push(Document {
        id: DocumentId::new(0),
        href: "text.xhtml".to_owned(),
        media_type: "application/xhtml+xml".to_owned(),
        title: book.metadata.title.clone(),
        nodes: root_nodes,
    });
    split_large_text_document(&mut book);
    book
}

const MAX_TEXT_DOCUMENT_BYTES: usize = 32 << 20;

/// Keep large TXT imports within the per-XHTML safety budget used by the EPUB
/// reader.  Splitting occurs only at already-established top-level semantic
/// nodes, so it does not invent chapter boundaries or reorder content.
fn split_large_text_document(book: &mut Book) {
    let Some(document) = book.documents.pop() else {
        return;
    };
    let mut groups = Vec::<Vec<folio_model::Node>>::new();
    let mut current = Vec::new();
    let mut current_bytes = 0usize;
    for node in document.nodes {
        let node_bytes = estimated_text_bytes(&node);
        if !current.is_empty() && current_bytes.saturating_add(node_bytes) > MAX_TEXT_DOCUMENT_BYTES
        {
            groups.push(std::mem::take(&mut current));
            current_bytes = 0;
        }
        current_bytes = current_bytes.saturating_add(node_bytes);
        current.push(node);
    }
    if !current.is_empty() {
        groups.push(current);
    }
    if groups.len() <= 1 {
        book.documents.push(folio_model::Document {
            id: folio_model::DocumentId::new(0),
            href: "text.xhtml".to_owned(),
            media_type: document.media_type,
            title: document.title,
            nodes: groups.into_iter().next().unwrap_or_default(),
        });
        return;
    }

    let mut node_documents = BTreeMap::new();
    let mut documents = Vec::with_capacity(groups.len());
    for (index, nodes) in groups.into_iter().enumerate() {
        for node in &nodes {
            index_nodes(node, index, &mut node_documents);
        }
        documents.push(folio_model::Document {
            id: folio_model::DocumentId::new(index as u32),
            href: if index == 0 {
                "text.xhtml".to_owned()
            } else {
                format!("text-{:04}.xhtml", index + 1)
            },
            media_type: document.media_type.clone(),
            title: if index == 0 {
                document.title.clone()
            } else {
                first_heading(&nodes).or_else(|| document.title.clone())
            },
            nodes,
        });
    }
    for anchor in &mut book.anchors {
        anchor.document = node_documents
            .get(&anchor.node)
            .copied()
            .map(|index| folio_model::DocumentId::new(index as u32))
            .unwrap_or_default();
    }
    let anchor_documents = book
        .anchors
        .iter()
        .map(|anchor| {
            (
                anchor.name.clone(),
                documents
                    .get(anchor.document.get() as usize)
                    .map(|document| document.href.clone())
                    .unwrap_or_else(|| "text.xhtml".to_owned()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for point in &mut book.navigation.toc {
        rewrite_nav_href(point, &anchor_documents);
    }
    book.documents = documents;
}

fn estimated_text_bytes(node: &folio_model::Node) -> usize {
    let own = match &node.kind {
        folio_model::NodeKind::Text { value } => value.len(),
        folio_model::NodeKind::Image { alt, .. } | folio_model::NodeKind::Svg { alt, .. } => {
            alt.len()
        }
        _ => 0,
    };
    own.saturating_add(
        node.children
            .iter()
            .map(estimated_text_bytes)
            .sum::<usize>(),
    )
}

fn index_nodes(
    node: &folio_model::Node,
    document: usize,
    output: &mut BTreeMap<folio_model::NodeId, usize>,
) {
    output.insert(node.id, document);
    for child in &node.children {
        index_nodes(child, document, output);
    }
}

fn first_heading(nodes: &[folio_model::Node]) -> Option<String> {
    nodes.iter().find_map(|node| match &node.kind {
        folio_model::NodeKind::Heading { .. } => Some(node.text_content().trim().to_owned()),
        _ => first_heading(&node.children),
    })
}

fn rewrite_nav_href(point: &mut folio_model::NavPoint, documents: &BTreeMap<String, String>) {
    if let Some((_, fragment)) = point.href.split_once('#') {
        if let Some(document) = documents.get(fragment) {
            point.href = format!("{document}#{fragment}");
        }
    }
    for child in &mut point.children {
        rewrite_nav_href(child, documents);
    }
}

enum ImportEvent {
    Heading(crate::StructureCandidate),
    Paragraph(crate::ParsedParagraph),
}

#[allow(clippy::too_many_arguments)]
fn flush_section(
    current_section: &mut Option<SectionBuilder>,
    current_volume: &mut Option<VolumeBuilder>,
    root_nodes: &mut Vec<folio_model::Node>,
    navigation: &mut Vec<folio_model::NavPoint>,
    anchors: &mut Vec<folio_model::Anchor>,
    next_node: &mut u32,
    next_anchor: &mut u32,
    style: folio_model::StyleId,
) {
    let Some(section) = current_section.take() else {
        return;
    };
    let Some(candidate) = section.candidate else {
        return;
    };
    let (node, nav, anchor) = make_section_node(
        &candidate,
        section.paragraphs,
        next_node,
        next_anchor,
        style,
    );
    if let Some(volume) = current_volume.as_mut() {
        volume.materialized.push((node, nav, anchor));
    } else {
        root_nodes.push(node);
        navigation.push(nav);
        anchors.push(anchor);
    }
}

#[allow(clippy::too_many_arguments)]
fn flush_volume(
    current_volume: &mut Option<VolumeBuilder>,
    root_nodes: &mut Vec<folio_model::Node>,
    navigation: &mut Vec<folio_model::NavPoint>,
    anchors: &mut Vec<folio_model::Anchor>,
    next_node: &mut u32,
    next_anchor: &mut u32,
    style: folio_model::StyleId,
) {
    let Some(mut volume) = current_volume.take() else {
        return;
    };
    let Some(candidate) = volume.candidate.take() else {
        return;
    };
    let (mut node, mut nav, anchor) =
        make_section_node(&candidate, volume.paragraphs, next_node, next_anchor, style);
    for (child, child_nav, child_anchor) in volume.materialized.drain(..) {
        node.children.push(child);
        nav.children.push(child_nav);
        anchors.push(child_anchor);
    }
    root_nodes.push(node);
    navigation.push(nav);
    anchors.push(anchor);
}

fn make_section_node(
    candidate: &crate::StructureCandidate,
    paragraphs: Vec<crate::ParsedParagraph>,
    next_node: &mut u32,
    next_anchor: &mut u32,
    style: folio_model::StyleId,
) -> (
    folio_model::Node,
    folio_model::NavPoint,
    folio_model::Anchor,
) {
    use folio_model::{Anchor, AnchorId, DocumentId, NavPoint, Node, NodeKind, SemanticRole};
    let section_id = allocate(next_node);
    let heading_id = allocate(next_node);
    let text_id = allocate(next_node);
    let heading = Node::new(
        heading_id,
        NodeKind::Heading {
            level: candidate.level,
        },
        style,
        vec![Node::new(
            text_id,
            NodeKind::Text {
                value: candidate.text.clone(),
            },
            style,
            Vec::new(),
        )],
    )
    .with_semantics(
        SemanticRole::Heading,
        Default::default(),
        semantic_confidence(candidate.confidence_percent),
    );
    let mut children = vec![heading];
    for paragraph in paragraphs {
        children.push(make_paragraph_node(paragraph, next_node, style));
    }
    let anchor_name = format!("text-section-{}", *next_anchor);
    let anchor_id = AnchorId::new(*next_anchor);
    *next_anchor = next_anchor.saturating_add(1);
    let anchor = Anchor {
        id: anchor_id,
        document: DocumentId::new(0),
        node: section_id,
        name: anchor_name.clone(),
    };
    let nav = NavPoint {
        label: candidate.text.clone(),
        href: format!("text.xhtml#{anchor_name}"),
        children: Vec::new(),
    };
    let role = if candidate.kind == crate::StructureKind::Volume {
        SemanticRole::Section
    } else {
        SemanticRole::Chapter
    };
    let node = Node::new(section_id, NodeKind::Section, style, children).with_semantics(
        role,
        Default::default(),
        semantic_confidence(candidate.confidence_percent),
    );
    (node, nav, anchor)
}

fn make_paragraph_node(
    paragraph: crate::ParsedParagraph,
    next_node: &mut u32,
    style: folio_model::StyleId,
) -> folio_model::Node {
    use folio_model::{Confidence, Node, NodeKind, SemanticRole};
    let preformatted = paragraph.preformatted;
    let text_value = paragraph.text;
    let text = Node::new(
        allocate(next_node),
        NodeKind::Text { value: text_value },
        style,
        Vec::new(),
    );
    if preformatted {
        let role = if let NodeKind::Text { value } = &text.kind {
            if looks_like_code(value) {
                SemanticRole::Code
            } else {
                SemanticRole::Poetry
            }
        } else {
            SemanticRole::Poetry
        };
        Node::new(
            allocate(next_node),
            NodeKind::Preformatted,
            style,
            vec![text],
        )
        .with_semantics(role, Default::default(), Confidence::Heuristic)
    } else {
        Node::new(allocate(next_node), NodeKind::Paragraph, style, vec![text]).with_semantics(
            SemanticRole::Paragraph,
            Default::default(),
            Confidence::StronglyInferred,
        )
    }
}

fn semantic_confidence(percent: u8) -> folio_model::Confidence {
    if percent >= 85 {
        folio_model::Confidence::StronglyInferred
    } else {
        folio_model::Confidence::Heuristic
    }
}

fn looks_like_code(text: &str) -> bool {
    ["{", "}", "=>", "fn ", "let ", "class ", "#include", "</"]
        .iter()
        .any(|marker| text.contains(marker))
}

fn allocate(next: &mut u32) -> folio_model::NodeId {
    let id = folio_model::NodeId::new(*next);
    *next = next.saturating_add(1);
    id
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-text/src/import.rs"]
mod tests;
