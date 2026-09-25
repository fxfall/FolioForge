//! Format-blind rendering of generic reflowable IR documents.

use folio_model::{Book, Node, NodeKind};
use serde::{Deserialize, Serialize};

/// A stable per-document Reader render result for the existing WebKit client.
/// It contains no source-format parser state, compatibility decision or file
/// path. Resource/image placeholders intentionally match the migrated Preview
/// A behavior until Reader resource rendering has its own validated contract.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReflowableDocumentRenderModel {
    pub href: String,
    pub title: Option<String>,
    pub html: String,
}

/// Render generic reflowable IR documents in their stored reading order.
///
/// The returned HTML is a display representation, not an export format. The
/// caller owns any source editing, target compatibility projection, outer
/// viewport and application-specific preview metadata.
pub fn render_reflowable_documents(book: &Book) -> Vec<ReflowableDocumentRenderModel> {
    book.documents
        .iter()
        .map(|document| ReflowableDocumentRenderModel {
            href: document.href.clone(),
            title: document.title.clone(),
            html: render_nodes(&document.nodes, book),
        })
        .collect()
}

fn render_nodes(nodes: &[Node], book: &Book) -> String {
    nodes.iter().map(|node| render_node(node, book)).collect()
}

fn render_node(node: &Node, book: &Book) -> String {
    let children = render_nodes(&node.children, book);
    let style = style_attribute(node, book);
    match &node.kind {
        NodeKind::Text { value } => tagged("span", &style, escape(value)),
        NodeKind::Section => tagged("section", &style, children),
        NodeKind::Heading { level } => {
            let level = (*level).clamp(1, 6);
            tagged(&format!("h{level}"), &style, children)
        }
        NodeKind::Paragraph => tagged("p", &style, children),
        NodeKind::BlockQuote => tagged("blockquote", &style, children),
        NodeKind::Preformatted | NodeKind::Code => tagged("pre", &style, children),
        NodeKind::OrderedList => tagged("ol", &style, children),
        NodeKind::UnorderedList => tagged("ul", &style, children),
        NodeKind::ListItem => tagged("li", &style, children),
        NodeKind::Emphasis => tagged("em", &style, children),
        NodeKind::Strong => tagged("strong", &style, children),
        NodeKind::Link { href } => {
            if safe_local_href(href) {
                format!("<a href=\"{}\"{}>{children}</a>", escape(href), style)
            } else {
                tagged("span", &style, children)
            }
        }
        NodeKind::Anchor { name } => format!("<a id=\"{}\"{}>{children}</a>", escape(name), style),
        NodeKind::Image { alt, .. } | NodeKind::Svg { alt, .. } => format!(
            "<figure{}><div aria-label=\"{}\">[image]</div>{children}</figure>",
            style,
            escape(alt)
        ),
        NodeKind::Math { alt, .. } => {
            tagged("span", &style, escape(alt.as_deref().unwrap_or("[math]")))
        }
        NodeKind::PageBreak => format!("<hr class=\"page-break\"{style}>"),
        NodeKind::Footnote { .. } => tagged("aside", &style, children),
        NodeKind::Table | NodeKind::TableRow | NodeKind::TableCell => {
            tagged("div", &style, children)
        }
        NodeKind::Inline
        | NodeKind::GenericInline { .. }
        | NodeKind::GenericBlock { .. }
        | NodeKind::Ruby => {
            if style.is_empty() {
                children
            } else {
                tagged("span", &style, children)
            }
        }
    }
}

fn tagged(tag: &str, style: &str, body: String) -> String {
    format!("<{tag}{style}>{body}</{tag}>")
}

fn style_attribute(node: &Node, book: &Book) -> String {
    let Some(style) = book.styles.get(node.style) else {
        return String::new();
    };
    let declarations = style
        .properties
        .iter()
        .filter(|(property, value)| safe_css_declaration(property, value))
        .map(|(property, value)| format!("{property}: {value}"))
        .collect::<Vec<_>>();
    if declarations.is_empty() {
        String::new()
    } else {
        format!(" style=\"{}\"", escape(&declarations.join("; ")))
    }
}

fn safe_css_declaration(property: &str, value: &str) -> bool {
    !property.is_empty()
        && property
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && !value.to_ascii_lowercase().contains("url(")
        && !value.to_ascii_lowercase().contains("expression(")
        && !value.to_ascii_lowercase().contains("javascript:")
}

fn safe_local_href(href: &str) -> bool {
    !href.starts_with("//")
        && !href.to_ascii_lowercase().starts_with("javascript:")
        && !href.to_ascii_lowercase().starts_with("data:")
        && (!href.contains(':') || href.starts_with('#'))
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
