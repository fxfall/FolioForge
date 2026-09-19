//! CommonMark-shaped Markdown input adapter.
//!
//! The parser intentionally produces a small native Markdown tree first.  It
//! is then decoded into the shared Folio Semantic IR; no exporter is aware
//! that a node originated in Markdown.

use std::{fs, path::PathBuf};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkdownInline {
    Text(String),
    Emphasis(Vec<MarkdownInline>),
    Strong(Vec<MarkdownInline>),
    Strike(Vec<MarkdownInline>),
    Link {
        label: Vec<MarkdownInline>,
        href: String,
    },
    FootnoteReference(String),
    Image {
        alt: String,
        source: String,
    },
    Code(String),
    RawHtml(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkdownBlock {
    Heading {
        level: u8,
        content: Vec<MarkdownInline>,
    },
    Paragraph(Vec<MarkdownInline>),
    Quote(Vec<MarkdownBlock>),
    OrderedList(Vec<MarkdownListItem>),
    UnorderedList(Vec<MarkdownListItem>),
    CodeBlock {
        language: Option<String>,
        value: String,
    },
    Table {
        headers: Vec<Vec<MarkdownInline>>,
        rows: Vec<Vec<Vec<MarkdownInline>>>,
    },
    HorizontalRule,
    RawHtml(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownListItem {
    pub content: Vec<MarkdownInline>,
    pub children: Vec<MarkdownBlock>,
}

#[derive(Clone, Debug, Default)]
pub struct MarkdownNativeDocument {
    pub metadata: Metadata,
    pub blocks: Vec<MarkdownBlock>,
    pub footnotes: std::collections::BTreeMap<String, Vec<MarkdownInline>>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, thiserror::Error)]
pub enum MarkdownError {
    #[error("Markdown source has no readable text: {0}")]
    Decode(String),
    #[error("Markdown source is not a regular file")]
    Source,
}

pub fn parse(source_name: Option<&str>, text: &str) -> MarkdownNativeDocument {
    let mut lines = text.lines().collect::<Vec<_>>();
    let mut metadata = Metadata::default();
    let mut footnotes = std::collections::BTreeMap::new();
    let mut diagnostics = Vec::new();
    let mut start = 0usize;
    if lines.first().is_some_and(|line| line.trim() == "---") {
        if let Some(end) = lines.iter().enumerate().skip(1).find_map(|(index, line)| {
            (line.trim() == "---" || line.trim() == "...").then_some(index)
        }) {
            parse_front_matter(&lines[1..end], &mut metadata, &mut diagnostics);
            start = end + 1;
        } else {
            diagnostics.push(Diagnostic::warning(
                "FF-MD-FRONTMATTER-0001",
                "front matter starts with --- but has no closing delimiter; it was kept as body text",
            ));
        }
    }
    lines = lines.split_off(start);
    let mut blocks = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];
        if line.trim().is_empty() {
            index += 1;
            continue;
        }
        if let Some((id, value)) = footnote_definition(line) {
            footnotes.insert(id, parse_inlines(value));
            index += 1;
            continue;
        }
        if let Some((level, value)) = heading_line(line) {
            blocks.push(MarkdownBlock::Heading {
                level,
                content: parse_inlines(value),
            });
            index += 1;
            continue;
        }
        if let Some(language) = fence_start(line) {
            let mut value = Vec::new();
            index += 1;
            while index < lines.len() && !lines[index].trim_start().starts_with("```") {
                value.push(lines[index]);
                index += 1;
            }
            if index == lines.len() {
                diagnostics.push(Diagnostic::warning(
                    "FF-MD-CODE-0001",
                    "unterminated fenced code block was closed at end of input",
                ));
            } else {
                index += 1;
            }
            blocks.push(MarkdownBlock::CodeBlock {
                language,
                value: value.join("\n"),
            });
            continue;
        }
        if is_horizontal_rule(line) {
            blocks.push(MarkdownBlock::HorizontalRule);
            index += 1;
            continue;
        }
        if line.trim_start().starts_with('>') {
            let mut quote_lines = Vec::new();
            while index < lines.len() && lines[index].trim_start().starts_with('>') {
                let value = lines[index].trim_start()[1..].trim_start();
                quote_lines.push(value);
                index += 1;
            }
            let nested = parse(None, &quote_lines.join("\n"));
            blocks.push(MarkdownBlock::Quote(nested.blocks));
            footnotes.extend(nested.footnotes);
            diagnostics.extend(nested.diagnostics);
            continue;
        }
        if list_line_with_indent(line).is_some_and(|(indent, _, _)| indent == 0) {
            let (list, next) = parse_list(&lines, index, 0);
            blocks.push(list);
            index = next;
            continue;
        }
        if index + 1 < lines.len()
            && looks_like_table_separator(lines[index + 1])
            && line.contains('|')
        {
            let headers = split_table_row(line)
                .into_iter()
                .map(parse_inlines)
                .collect::<Vec<_>>();
            index += 2;
            let mut rows = Vec::new();
            while index < lines.len()
                && lines[index].contains('|')
                && !lines[index].trim().is_empty()
            {
                rows.push(
                    split_table_row(lines[index])
                        .into_iter()
                        .map(parse_inlines)
                        .collect(),
                );
                index += 1;
            }
            blocks.push(MarkdownBlock::Table { headers, rows });
            continue;
        }
        if line.trim_start().starts_with('<') && line.trim_end().ends_with('>') {
            let mut raw = vec![line];
            index += 1;
            while index < lines.len()
                && !lines[index].trim().is_empty()
                && lines[index].trim_start().starts_with('<')
            {
                raw.push(lines[index]);
                index += 1;
            }
            blocks.push(MarkdownBlock::RawHtml(raw.join("\n")));
            continue;
        }
        let mut paragraph = vec![line.trim()];
        index += 1;
        while index < lines.len()
            && !lines[index].trim().is_empty()
            && !is_block_start(lines[index], lines.get(index + 1).copied())
        {
            paragraph.push(lines[index].trim());
            index += 1;
        }
        blocks.push(MarkdownBlock::Paragraph(parse_inlines(
            &paragraph.join(" "),
        )));
    }

    if metadata.title.is_none() {
        if let Some(name) = source_name {
            let (_, guess) = folio_text::infer_metadata(Some(name), "", None, None);
            metadata.title = guess.title;
            if let Some(author) = guess.author {
                metadata.add_author(author);
            }
        }
    }
    MarkdownNativeDocument {
        metadata,
        blocks,
        footnotes,
        diagnostics,
    }
}

fn footnote_definition(line: &str) -> Option<(String, &str)> {
    let value = line.trim_start();
    let rest = value.strip_prefix("[^")?;
    let separator = rest.find("]:")?;
    let id = rest[..separator].trim();
    let definition = rest[separator + 2..].trim_start();
    (!id.is_empty()).then(|| (id.to_owned(), definition))
}

fn parse_list(lines: &[&str], mut index: usize, indent: usize) -> (MarkdownBlock, usize) {
    let (_, ordered, _) = list_line_with_indent(lines[index]).unwrap_or((indent, false, ""));
    let mut items = Vec::new();
    while index < lines.len() {
        let Some((current_indent, current_ordered, value)) = list_line_with_indent(lines[index])
        else {
            break;
        };
        if current_indent != indent || current_ordered != ordered {
            break;
        }
        let mut item = MarkdownListItem {
            content: parse_inlines(value),
            children: Vec::new(),
        };
        index += 1;
        while index < lines.len() {
            let Some((nested_indent, _, _)) = list_line_with_indent(lines[index]) else {
                break;
            };
            if nested_indent <= indent {
                break;
            }
            let (nested, next) = parse_list(lines, index, nested_indent);
            item.children.push(nested);
            index = next;
        }
        items.push(item);
    }
    if ordered {
        (MarkdownBlock::OrderedList(items), index)
    } else {
        (MarkdownBlock::UnorderedList(items), index)
    }
}

fn parse_front_matter(lines: &[&str], metadata: &mut Metadata, diagnostics: &mut Vec<Diagnostic>) {
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            if !line.trim().is_empty() {
                diagnostics.push(Diagnostic::warning(
                    "FF-MD-FRONTMATTER-0002",
                    "front matter line without a key/value separator was ignored",
                ));
            }
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().trim_matches(['"', '\'']).to_owned();
        if value.is_empty() {
            continue;
        }
        match key.as_str() {
            "title" => metadata.title = Some(value),
            "subtitle" => metadata.subtitle = Some(value),
            "author" | "authors" => {
                for author in value
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                {
                    metadata.add_author(author.to_owned());
                }
            }
            "language" | "lang" => metadata.language = Some(value),
            "publisher" => metadata.publisher = Some(value),
            "date" => metadata.date = Some(value),
            "series" => metadata.series = Some(value),
            "description" | "summary" => metadata.description = Some(value),
            "rights" | "license" => metadata.rights = Some(value),
            "keywords" | "subjects" => metadata.subjects.extend(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(ToOwned::to_owned),
            ),
            "identifier" | "isbn" => metadata.add_identifier(value),
            "series_index" | "series-index" => {
                if let Ok(index) = value.parse::<f64>() {
                    metadata.series_index = Some(index);
                }
            }
            _ => diagnostics.push(Diagnostic::info(
                "FF-MD-FRONTMATTER-0003",
                format!("unsupported front matter key '{key}' was retained only as a diagnostic"),
            )),
        }
    }
}

fn heading_line(line: &str) -> Option<(u8, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&level) && trimmed.as_bytes().get(level) == Some(&b' ') {
        Some((level as u8, trimmed[level + 1..].trim()))
    } else {
        None
    }
}

fn fence_start(line: &str) -> Option<Option<String>> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("```") {
        return None;
    }
    let language = trimmed[3..].trim();
    Some((!language.is_empty()).then(|| language.to_owned()))
}

fn is_horizontal_rule(line: &str) -> bool {
    let value = line.trim();
    ["---", "***", "___"].contains(&value)
}

fn list_line(line: &str) -> Option<(bool, &str)> {
    list_line_with_indent(line).map(|(_, ordered, value)| (ordered, value))
}

fn list_line_with_indent(line: &str) -> Option<(usize, bool, &str)> {
    let indent = line
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t'))
        .map(|character| if character == '\t' { 4 } else { 1 })
        .sum();
    let trimmed = line.trim_start();
    if let Some(value) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        return Some((indent, false, value.trim()));
    }
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && trimmed.as_bytes().get(digits..digits + 2) == Some(b". ") {
        return Some((indent, true, trimmed[digits + 2..].trim()));
    }
    None
}

fn is_block_start(line: &str, next: Option<&str>) -> bool {
    heading_line(line).is_some()
        || fence_start(line).is_some()
        || is_horizontal_rule(line)
        || line.trim_start().starts_with('>')
        || list_line(line).is_some()
        || (line.contains('|') && next.is_some_and(looks_like_table_separator))
        || (line.trim_start().starts_with('<') && line.trim_end().ends_with('>'))
}

fn looks_like_table_separator(line: &str) -> bool {
    let cells = split_table_row(line);
    !cells.is_empty()
        && cells
            .iter()
            .all(|cell| !cell.trim().is_empty() && cell.trim_matches('-').trim().is_empty())
}

fn split_table_row(line: &str) -> Vec<&str> {
    let value = line.trim().trim_matches('|');
    value.split('|').map(str::trim).collect()
}

fn parse_inlines(value: &str) -> Vec<MarkdownInline> {
    let mut output = Vec::new();
    let mut text = String::new();
    let mut cursor = 0usize;
    let bytes = value.as_bytes();
    let flush = |output: &mut Vec<MarkdownInline>, text: &mut String| {
        if !text.is_empty() {
            output.push(MarkdownInline::Text(std::mem::take(text)));
        }
    };
    while cursor < bytes.len() {
        let rest = &value[cursor..];
        if let Some(end) = rest.strip_prefix("[^").and_then(|rest| rest.find(']')) {
            let id_end = cursor + 2 + end;
            if id_end > cursor + 2 {
                flush(&mut output, &mut text);
                output.push(MarkdownInline::FootnoteReference(
                    value[cursor + 2..id_end].to_owned(),
                ));
                cursor = id_end + 1;
                continue;
            }
        }
        if let Some(end) = rest.strip_prefix("![").and_then(|rest| rest.find("](")) {
            let label_end = cursor + 2 + end;
            if let Some(close) = value[label_end + 2..].find(')') {
                flush(&mut output, &mut text);
                let source_end = label_end + 2 + close;
                output.push(MarkdownInline::Image {
                    alt: value[cursor + 2..label_end].to_owned(),
                    source: value[label_end + 2..source_end].trim().to_owned(),
                });
                cursor = source_end + 1;
                continue;
            }
        }
        if let Some(end) = rest.strip_prefix('[').and_then(|rest| rest.find("](")) {
            let label_end = cursor + 1 + end;
            if let Some(close) = value[label_end + 2..].find(')') {
                flush(&mut output, &mut text);
                let target_end = label_end + 2 + close;
                output.push(MarkdownInline::Link {
                    label: parse_inlines(&value[cursor + 1..label_end]),
                    href: value[label_end + 2..target_end].trim().to_owned(),
                });
                cursor = target_end + 1;
                continue;
            }
        }
        let pairs = [
            ("**", "**", 1u8),
            ("__", "__", 2),
            ("~~", "~~", 3),
            ("*", "*", 4),
            ("_", "_", 5),
            ("`", "`", 6),
        ];
        let mut matched = false;
        for (open, close, kind) in pairs {
            if !rest.starts_with(open) {
                continue;
            }
            let start = cursor + open.len();
            if let Some(end) = value[start..].find(close) {
                if end == 0 {
                    continue;
                }
                flush(&mut output, &mut text);
                let inner = &value[start..start + end];
                let node = match kind {
                    1 | 2 => MarkdownInline::Strong(parse_inlines(inner)),
                    3 => MarkdownInline::Strike(parse_inlines(inner)),
                    4 | 5 => MarkdownInline::Emphasis(parse_inlines(inner)),
                    _ => MarkdownInline::Code(inner.to_owned()),
                };
                output.push(node);
                cursor = start + end + close.len();
                matched = true;
                break;
            }
        }
        if matched {
            continue;
        }
        if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                let raw = &rest[..=end];
                if raw.contains('<') {
                    flush(&mut output, &mut text);
                    output.push(MarkdownInline::RawHtml(raw.to_owned()));
                    cursor += end + 1;
                    continue;
                }
            }
        }
        let character = rest.chars().next().unwrap_or_default();
        text.push(character);
        cursor += character.len_utf8();
    }
    flush(&mut output, &mut text);
    output
}

pub struct MarkdownAdapter;

impl FormatAdapter for MarkdownAdapter {
    fn format(&self) -> DetectedFormat {
        DetectedFormat::Markdown
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
                "Markdown adapter expects one .md or .markdown file".to_owned(),
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
        if !matches!(extension.as_deref(), Some("md" | "markdown")) {
            return Err(FormatError::Unsupported(
                "Markdown extension is not .md or .markdown".to_owned(),
            ));
        }
        Ok(DetectionResult {
            format: DetectedFormat::Markdown.name().to_owned(),
            confidence: DetectionConfidence::Exact,
            evidence: vec!["Markdown filename extension".to_owned()],
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
        let detection = context
            .text
            .encoding_override
            .map(|encoding| {
                folio_text::decode_as(&bytes, encoding).map_err(|error| error.to_string())
            })
            .transpose()
            .map_err(|error| FormatError::Invalid(MarkdownError::Decode(error).to_string()))?;
        let decoded = if let Some(decoded) = detection {
            decoded
        } else {
            let encoding = folio_text::detect(&bytes)
                .map_err(|error| FormatError::Invalid(error.to_string()))?
                .selected
                .ok_or_else(|| FormatError::Invalid("Markdown encoding is ambiguous".to_owned()))?;
            folio_text::decode_as(&bytes, encoding)
                .map_err(|error| FormatError::Invalid(error.to_string()))?
        };
        let (text, _) = folio_text::normalize_text_owned(decoded);
        let native = parse(file.relative_path.to_str(), &text);
        let (book, decode_diagnostics) =
            decode_native(source, file.relative_path.to_str(), native.clone())?;
        let mut diagnostics = native.diagnostics;
        diagnostics.extend(decode_diagnostics);
        let input_loss = diagnostics
            .iter()
            .filter(|item| item.code.starts_with("FF-MD-"))
            .map(|item| item.message.clone())
            .collect();
        Ok(ImportedBook {
            format: DetectedFormat::Markdown,
            parser: "folio-markdown/1".to_owned(),
            book,
            diagnostics,
            input_loss,
            text: None,
        })
    }
}

fn decode_native(
    source: &BookSource,
    source_name: Option<&str>,
    native: MarkdownNativeDocument,
) -> Result<(Book, Vec<Diagnostic>), FormatError> {
    let mut loader = MemoryResourceLoader::default();
    let mut resources = Vec::new();
    let mut diagnostics = native.diagnostics.clone();
    let base = source_name
        .and_then(|name| PathBuf::from(name).parent().map(PathBuf::from))
        .unwrap_or_default();
    let mut next_node = 0u32;
    let mut next_resource = 0u32;
    let mut next_anchor = 0u32;
    let mut book = Book::new();
    book.metadata = native.metadata;
    let style = book.styles.intern(ComputedStyle::default());
    let mut navigation = Vec::new();
    let mut anchors = Vec::new();
    let mut nodes = Vec::new();
    for block in &native.blocks {
        let node = block_node(
            block,
            source,
            &base,
            &mut loader,
            &mut resources,
            &mut diagnostics,
            &mut next_node,
            &mut next_resource,
            &mut next_anchor,
            style,
            &mut navigation,
            &mut anchors,
        )?;
        nodes.push(node);
    }
    for (footnote_id, content) in &native.footnotes {
        let node_id = allocate(&mut next_node);
        let children = inline_nodes(
            content,
            source,
            &base,
            &mut loader,
            &mut resources,
            &mut diagnostics,
            &mut next_node,
            &mut next_resource,
            style,
        )?;
        let anchor_name = footnote_anchor_name(footnote_id);
        anchors.push(Anchor {
            id: AnchorId::new(next_anchor),
            document: DocumentId::new(0),
            node: node_id,
            name: anchor_name,
        });
        next_anchor = next_anchor.saturating_add(1);
        nodes.push(
            Node::new(
                node_id,
                NodeKind::GenericBlock {
                    tag: "aside".to_owned(),
                },
                style,
                children,
            )
            .with_semantics(
                SemanticRole::Footnote,
                Default::default(),
                Confidence::Explicit,
            ),
        );
    }
    book.resources = resources;
    book.anchors = anchors;
    book.navigation.toc = navigation;
    let title = nodes.iter().find_map(|node| match &node.kind {
        NodeKind::Heading { .. } => Some(node.text_content().trim().to_owned()),
        _ => None,
    });
    book.documents.push(Document {
        id: DocumentId::new(0),
        href: "markdown.xhtml".to_owned(),
        media_type: "application/xhtml+xml".to_owned(),
        title,
        nodes,
    });
    Ok((
        book.with_resource_loader(std::sync::Arc::new(loader)),
        diagnostics,
    ))
}

#[allow(clippy::too_many_arguments)]
fn block_node(
    block: &MarkdownBlock,
    source: &BookSource,
    base: &std::path::Path,
    loader: &mut MemoryResourceLoader,
    resources: &mut Vec<Resource>,
    diagnostics: &mut Vec<Diagnostic>,
    next_node: &mut u32,
    next_resource: &mut u32,
    next_anchor: &mut u32,
    style: folio_model::StyleId,
    navigation: &mut Vec<NavPoint>,
    anchors: &mut Vec<Anchor>,
) -> Result<Node, FormatError> {
    let node_id = allocate(next_node);
    let mut node = match block {
        MarkdownBlock::Heading { level, content } => {
            let children = inline_nodes(
                content,
                source,
                base,
                loader,
                resources,
                diagnostics,
                next_node,
                next_resource,
                style,
            )?;
            let anchor_name = format!("md-heading-{}", *next_anchor);
            anchors.push(Anchor {
                id: AnchorId::new(*next_anchor),
                document: DocumentId::new(0),
                node: node_id,
                name: anchor_name.clone(),
            });
            *next_anchor = next_anchor.saturating_add(1);
            navigation.push(NavPoint {
                label: inline_text(content),
                href: format!("markdown.xhtml#{anchor_name}"),
                children: Vec::new(),
            });
            Node::new(
                node_id,
                NodeKind::Heading { level: *level },
                style,
                children,
            )
            .with_semantics(
                SemanticRole::Heading,
                Default::default(),
                Confidence::Explicit,
            )
        }
        MarkdownBlock::Paragraph(content) => Node::new(
            node_id,
            NodeKind::Paragraph,
            style,
            inline_nodes(
                content,
                source,
                base,
                loader,
                resources,
                diagnostics,
                next_node,
                next_resource,
                style,
            )?,
        )
        .with_semantics(
            SemanticRole::Paragraph,
            Default::default(),
            Confidence::Explicit,
        ),
        MarkdownBlock::Quote(blocks) => {
            let children = blocks
                .iter()
                .map(|block| {
                    block_node(
                        block,
                        source,
                        base,
                        loader,
                        resources,
                        diagnostics,
                        next_node,
                        next_resource,
                        next_anchor,
                        style,
                        navigation,
                        anchors,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            Node::new(node_id, NodeKind::BlockQuote, style, children).with_semantics(
                SemanticRole::Quote,
                Default::default(),
                Confidence::Explicit,
            )
        }
        MarkdownBlock::OrderedList(items) | MarkdownBlock::UnorderedList(items) => {
            let ordered = matches!(block, MarkdownBlock::OrderedList(_));
            let children = items
                .iter()
                .map(|item| {
                    let mut item_children = inline_nodes(
                        &item.content,
                        source,
                        base,
                        loader,
                        resources,
                        diagnostics,
                        next_node,
                        next_resource,
                        style,
                    )?;
                    for nested in &item.children {
                        item_children.push(block_node(
                            nested,
                            source,
                            base,
                            loader,
                            resources,
                            diagnostics,
                            next_node,
                            next_resource,
                            next_anchor,
                            style,
                            navigation,
                            anchors,
                        )?);
                    }
                    let item_node = Node::new(
                        allocate(next_node),
                        NodeKind::ListItem,
                        style,
                        item_children,
                    )
                    .with_semantics(
                        SemanticRole::List,
                        Default::default(),
                        Confidence::Explicit,
                    );
                    Ok(item_node)
                })
                .collect::<Result<Vec<_>, FormatError>>()?;
            Node::new(
                node_id,
                if ordered {
                    NodeKind::OrderedList
                } else {
                    NodeKind::UnorderedList
                },
                style,
                children,
            )
            .with_semantics(
                SemanticRole::List,
                Default::default(),
                Confidence::Explicit,
            )
        }
        MarkdownBlock::CodeBlock { value, .. } => Node::new(
            node_id,
            NodeKind::Preformatted,
            style,
            vec![text_node(value, next_node, style)],
        )
        .with_semantics(SemanticRole::Code, Default::default(), Confidence::Explicit),
        MarkdownBlock::Table { headers, rows } => {
            let mut table_rows = Vec::new();
            let mut all_rows = vec![headers.clone()];
            all_rows.extend(rows.clone());
            for row in all_rows {
                let mut cells = Vec::new();
                for cell in row {
                    let children = inline_nodes(
                        &cell,
                        source,
                        base,
                        loader,
                        resources,
                        diagnostics,
                        next_node,
                        next_resource,
                        style,
                    )?;
                    cells.push(Node::new(
                        allocate(next_node),
                        NodeKind::TableCell,
                        style,
                        children,
                    ));
                }
                table_rows.push(Node::new(
                    allocate(next_node),
                    NodeKind::TableRow,
                    style,
                    cells,
                ));
            }
            Node::new(node_id, NodeKind::Table, style, table_rows).with_semantics(
                SemanticRole::Table,
                Default::default(),
                Confidence::Explicit,
            )
        }
        MarkdownBlock::HorizontalRule => Node::new(node_id, NodeKind::PageBreak, style, Vec::new())
            .with_semantics(
                SemanticRole::SceneBreak,
                Default::default(),
                Confidence::Explicit,
            ),
        MarkdownBlock::RawHtml(value) => {
            diagnostics.push(Diagnostic::warning(
                "FF-MD-RAW-HTML-0001",
                "raw HTML tags are preserved as a generic block boundary; scripts are discarded and never executed",
            ));
            Node::new(
                node_id,
                NodeKind::GenericBlock {
                    tag: "raw-html".to_owned(),
                },
                style,
                vec![text_node(&raw_html_text(value), next_node, style)],
            )
        }
    };
    if matches!(node.kind, NodeKind::PageBreak) {
        node.role = SemanticRole::SceneBreak;
    }
    Ok(node)
}

#[allow(clippy::too_many_arguments)]
fn inline_nodes(
    inlines: &[MarkdownInline],
    source: &BookSource,
    base: &std::path::Path,
    loader: &mut MemoryResourceLoader,
    resources: &mut Vec<Resource>,
    diagnostics: &mut Vec<Diagnostic>,
    next_node: &mut u32,
    next_resource: &mut u32,
    style: folio_model::StyleId,
) -> Result<Vec<Node>, FormatError> {
    inlines
        .iter()
        .map(|inline| {
            let id = allocate(next_node);
            let node = match inline {
                MarkdownInline::Text(value) => Node::new(
                    id,
                    NodeKind::Text {
                        value: value.clone(),
                    },
                    style,
                    Vec::new(),
                ),
                MarkdownInline::Emphasis(children) => Node::new(
                    id,
                    NodeKind::Emphasis,
                    style,
                    inline_nodes(
                        children,
                        source,
                        base,
                        loader,
                        resources,
                        diagnostics,
                        next_node,
                        next_resource,
                        style,
                    )?,
                ),
                MarkdownInline::Strong(children) => Node::new(
                    id,
                    NodeKind::Strong,
                    style,
                    inline_nodes(
                        children,
                        source,
                        base,
                        loader,
                        resources,
                        diagnostics,
                        next_node,
                        next_resource,
                        style,
                    )?,
                ),
                MarkdownInline::Strike(children) => Node::new(
                    id,
                    NodeKind::GenericInline {
                        tag: "s".to_owned(),
                    },
                    style,
                    inline_nodes(
                        children,
                        source,
                        base,
                        loader,
                        resources,
                        diagnostics,
                        next_node,
                        next_resource,
                        style,
                    )?,
                ),
                MarkdownInline::Link { label, href } => Node::new(
                    id,
                    NodeKind::Link { href: href.clone() },
                    style,
                    inline_nodes(
                        label,
                        source,
                        base,
                        loader,
                        resources,
                        diagnostics,
                        next_node,
                        next_resource,
                        style,
                    )?,
                )
                .with_semantics(
                    SemanticRole::Link,
                    Default::default(),
                    Confidence::Explicit,
                ),
                MarkdownInline::FootnoteReference(reference) => Node::new(
                    id,
                    NodeKind::Footnote {
                        href: Some(format!(
                            "markdown.xhtml#{}",
                            footnote_anchor_name(reference)
                        )),
                    },
                    style,
                    vec![text_node(&footnote_label(reference), next_node, style)],
                )
                .with_semantics(
                    SemanticRole::Footnote,
                    Default::default(),
                    Confidence::Explicit,
                ),
                MarkdownInline::Image {
                    alt,
                    source: image_source,
                } => {
                    let Some(resource) = load_image(
                        source,
                        base,
                        image_source,
                        loader,
                        resources,
                        next_resource,
                        diagnostics,
                    )?
                    else {
                        return Ok(Node::new(
                            id,
                            NodeKind::GenericInline {
                                tag: "img".to_owned(),
                            },
                            style,
                            Vec::new(),
                        ));
                    };
                    Node::new(
                        id,
                        NodeKind::Image {
                            resource,
                            alt: alt.clone(),
                        },
                        style,
                        Vec::new(),
                    )
                    .with_semantics(
                        SemanticRole::Image,
                        Default::default(),
                        Confidence::Explicit,
                    )
                }
                MarkdownInline::Code(value) => Node::new(
                    id,
                    NodeKind::Code,
                    style,
                    vec![text_node(value, next_node, style)],
                ),
                MarkdownInline::RawHtml(value) => Node::new(
                    id,
                    NodeKind::GenericInline {
                        tag: raw_html_tag(value),
                    },
                    style,
                    Vec::new(),
                ),
            };
            Ok(node)
        })
        .collect()
}

fn load_image(
    source: &BookSource,
    base: &std::path::Path,
    image_source: &str,
    loader: &mut MemoryResourceLoader,
    resources: &mut Vec<Resource>,
    next_resource: &mut u32,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<ResourceId>, FormatError> {
    if image_source.contains("://") || image_source.starts_with("data:") {
        diagnostics.push(Diagnostic::warning(
            "FF-MD-REMOTE-0001",
            format!("remote or data image was not fetched: {image_source}"),
        ));
        return Ok(None);
    }
    let relative = safe_relative(base, image_source).ok_or_else(|| {
        FormatError::Invalid(format!(
            "image path escapes Markdown source root: {image_source}"
        ))
    })?;
    let root = source.root();
    let path = root.join(&relative);
    if !path.is_file() {
        diagnostics.push(Diagnostic::warning(
            "FF-MD-RES-0001",
            format!("image resource not found: {}", relative.display()),
        ));
        return Ok(None);
    }
    let canonical = std::fs::canonicalize(&path)?;
    if !canonical.starts_with(root) {
        return Err(FormatError::Invalid(format!(
            "image path escapes Markdown source root: {image_source}"
        )));
    }
    let locator = relative.to_string_lossy().replace('\\', "/");
    let bytes = fs::read(&canonical)?;
    loader.insert(locator.clone(), bytes.clone());
    let id = ResourceId::new(*next_resource);
    *next_resource = next_resource.saturating_add(1);
    resources.push(Resource {
        id,
        path: locator.clone(),
        media_type: mime_for_path(&locator).to_owned(),
        kind: kind_for_path(&locator),
        properties: Vec::new(),
        size: Some(bytes.len() as u64),
    });
    Ok(Some(id))
}

fn safe_relative(base: &std::path::Path, value: &str) -> Option<PathBuf> {
    let value = value
        .split('#')
        .next()
        .unwrap_or(value)
        .split('?')
        .next()
        .unwrap_or(value);
    let mut result = PathBuf::new();
    for component in base.join(value).components() {
        match component {
            std::path::Component::Normal(part) => result.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !result.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(result)
}

fn mime_for_path(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

fn kind_for_path(path: &str) -> ResourceKind {
    match mime_for_path(path) {
        "image/jpeg" => ResourceKind::Jpeg,
        "image/png" => ResourceKind::Png,
        "image/gif" => ResourceKind::Gif,
        "image/svg+xml" => ResourceKind::Svg,
        value if value.starts_with("font/") => ResourceKind::Font,
        _ => ResourceKind::Unknown,
    }
}

fn inline_text(inlines: &[MarkdownInline]) -> String {
    inlines
        .iter()
        .map(|inline| match inline {
            MarkdownInline::Text(value)
            | MarkdownInline::Code(value)
            | MarkdownInline::RawHtml(value) => value.clone(),
            MarkdownInline::Emphasis(value)
            | MarkdownInline::Strong(value)
            | MarkdownInline::Strike(value) => inline_text(value),
            MarkdownInline::Link { label, .. } => inline_text(label),
            MarkdownInline::Image { alt, .. } => alt.clone(),
            MarkdownInline::FootnoteReference(reference) => footnote_label(reference),
        })
        .collect()
}

fn footnote_anchor_name(reference: &str) -> String {
    let mut anchor = String::from("fn-");
    for character in reference.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            anchor.push(character);
        } else {
            anchor.push('-');
        }
    }
    anchor.trim_end_matches('-').to_owned()
}

fn footnote_label(reference: &str) -> String {
    reference
        .parse::<usize>()
        .map(|value| value.to_string())
        .unwrap_or_else(|_| reference.to_owned())
}

fn raw_html_tag(value: &str) -> String {
    let value = value.trim_start_matches('<').trim_start_matches('/');
    let name = value
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    match name.as_str() {
        "abbr" | "b" | "cite" | "div" | "i" | "kbd" | "q" | "s" | "small" | "span" | "sub"
        | "sup" | "time" | "var" | "address" | "main" => name,
        _ => "span".to_owned(),
    }
}

fn raw_html_text(value: &str) -> String {
    let mut output = String::new();
    let mut remaining = value;
    loop {
        let lower = remaining.to_ascii_lowercase();
        let Some(start) = lower.find("<script") else {
            output.push_str(&strip_html_tags(remaining));
            break;
        };
        output.push_str(&strip_html_tags(&remaining[..start]));
        let Some(end) = lower[start..].find("</script>") else {
            break;
        };
        remaining = &remaining[start + end + "</script>".len()..];
    }
    output.trim().to_owned()
}

fn strip_html_tags(value: &str) -> String {
    let mut output = String::new();
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    output
}

fn text_node(value: &str, next_node: &mut u32, style: folio_model::StyleId) -> Node {
    Node::new(
        allocate(next_node),
        NodeKind::Text {
            value: value.to_owned(),
        },
        style,
        Vec::new(),
    )
}

fn allocate(next_node: &mut u32) -> NodeId {
    let id = NodeId::new(*next_node);
    *next_node = next_node.saturating_add(1);
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_front_matter_and_commonmark_blocks() {
        let document = parse(Some("book.md"), "---\ntitle: Example\nauthor: Alice, Bob\nlanguage: zh-CN\n---\n# Title\n\n**strong** and [link](next.html)\n\n- one\n- two\n\n```rust\nlet x = 1;\n```\n");
        assert_eq!(document.metadata.title.as_deref(), Some("Example"));
        assert_eq!(document.metadata.author_names(), ["Alice", "Bob"]);
        assert!(matches!(
            document.blocks[0],
            MarkdownBlock::Heading { level: 1, .. }
        ));
        assert!(document
            .blocks
            .iter()
            .any(|block| matches!(block, MarkdownBlock::UnorderedList(_))));
        assert!(document
            .blocks
            .iter()
            .any(|block| matches!(block, MarkdownBlock::CodeBlock { .. })));
    }

    #[test]
    fn parses_inline_semantics_without_treating_tokens_as_plain_text() {
        let inlines = parse_inlines("**bold** *em* ~~strike~~ [go](#x) ![cover](cover.png)");
        assert!(inlines
            .iter()
            .any(|item| matches!(item, MarkdownInline::Strong(_))));
        assert!(inlines
            .iter()
            .any(|item| matches!(item, MarkdownInline::Emphasis(_))));
        assert!(inlines
            .iter()
            .any(|item| matches!(item, MarkdownInline::Strike(_))));
        assert!(inlines
            .iter()
            .any(|item| matches!(item, MarkdownInline::Link { .. })));
        assert!(inlines
            .iter()
            .any(|item| matches!(item, MarkdownInline::Image { .. })));
    }
}
