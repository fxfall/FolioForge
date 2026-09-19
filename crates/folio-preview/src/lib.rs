//! Compatibility preview generation from the same IR/edit/degradation inputs
//! used by conversion.  The output is intentionally called a compatibility
//! preview: it does not claim Kindle Exact Previewer/device equivalence.

use folio_capabilities::Format;
use folio_compat::{plan, project, DegradationMode, DegradationOptions, DegradationReport};
use folio_edit::{BookEditPlan, EditError};
use folio_model::{Book, Node, NodeKind};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PreviewTargetProfile {
    pub format: Format,
    pub label: String,
    pub disclaimer: String,
    pub device: PreviewDevice,
    pub orientation: PreviewOrientation,
    pub viewport_width: u16,
    pub viewport_height: u16,
    pub font_size_percent: u8,
}

impl PreviewTargetProfile {
    pub fn for_format(format: Format, settings: &PreviewSettings) -> Self {
        let (mut width, mut height) = match settings.device {
            PreviewDevice::EReader => (640, 960),
            PreviewDevice::Phone => (390, 844),
            PreviewDevice::Tablet => (820, 1180),
        };
        if settings.orientation == PreviewOrientation::Landscape {
            std::mem::swap(&mut width, &mut height);
        }
        Self {
            format,
            label: format!(
                "{} · {} · {}",
                format.name(),
                settings.device.label(),
                settings.orientation.label()
            ),
            disclaimer: "This is a FolioForge compatibility preview, not a Kindle Exact Preview or device renderer.".to_owned(),
            device: settings.device,
            orientation: settings.orientation,
            viewport_width: width,
            viewport_height: height,
            font_size_percent: settings.font_size_percent.clamp(60, 200),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewDevice {
    #[default]
    EReader,
    Phone,
    Tablet,
}

impl PreviewDevice {
    fn label(self) -> &'static str {
        match self {
            Self::EReader => "E-reader",
            Self::Phone => "Phone",
            Self::Tablet => "Tablet",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewOrientation {
    #[default]
    Portrait,
    Landscape,
}

impl PreviewOrientation {
    fn label(self) -> &'static str {
        match self {
            Self::Portrait => "Portrait",
            Self::Landscape => "Landscape",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct PreviewSettings {
    pub device: PreviewDevice,
    pub orientation: PreviewOrientation,
    pub font_size_percent: u8,
}

impl PreviewSettings {
    pub fn normalized(self) -> Self {
        Self {
            font_size_percent: self.font_size_percent.clamp(60, 200),
            ..self
        }
    }
}

impl Default for PreviewSettings {
    fn default() -> Self {
        Self {
            device: PreviewDevice::EReader,
            orientation: PreviewOrientation::Portrait,
            font_size_percent: 100,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PreviewDocument {
    pub href: String,
    pub title: Option<String>,
    pub html: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PreviewBundle {
    pub target: PreviewTargetProfile,
    pub source_title: String,
    pub documents: Vec<PreviewDocument>,
    pub html: String,
    pub degradation: DegradationReport,
    pub input_loss: Vec<String>,
    pub target_loss: Vec<String>,
    pub blocked: bool,
}

#[derive(Debug, Error)]
pub enum PreviewError {
    #[error("edit plan could not be applied: {0}")]
    Edit(#[from] EditError),
    #[error("compatibility projection failed: {0}")]
    Compatibility(#[from] folio_compat::CompatError),
}

pub fn build(
    source: &Book,
    edits: &BookEditPlan,
    target: Format,
    mode: DegradationMode,
    options: &DegradationOptions,
) -> Result<PreviewBundle, PreviewError> {
    build_with_settings(
        source,
        edits,
        target,
        mode,
        options,
        &PreviewSettings::default(),
    )
}

pub fn build_with_settings(
    source: &Book,
    edits: &BookEditPlan,
    target: Format,
    mode: DegradationMode,
    options: &DegradationOptions,
    settings: &PreviewSettings,
) -> Result<PreviewBundle, PreviewError> {
    let edited = edits.apply(source)?;
    let degradation = plan(&edited, target, mode, options);
    let preview_book = if degradation.blocked {
        edited.clone()
    } else {
        project(&edited, &degradation)?
    };
    let target_loss = degradation
        .items
        .iter()
        .filter(|item| {
            !matches!(
                item.quality,
                folio_compat::QualityLevel::Exact | folio_compat::QualityLevel::Equivalent
            )
        })
        .map(|item| format!("{}: {}", item.feature.name(), item.selected_fallback))
        .collect::<Vec<_>>();
    let documents = preview_book
        .documents
        .iter()
        .map(|document| PreviewDocument {
            href: document.href.clone(),
            title: document.title.clone(),
            html: render_nodes(&document.nodes, &preview_book),
        })
        .collect::<Vec<_>>();
    let settings = settings.normalized();
    let profile = PreviewTargetProfile::for_format(target, &settings);
    let html = render_bundle(&preview_book, &documents, &profile);
    Ok(PreviewBundle {
        target: profile,
        source_title: edited.metadata.display_title().to_owned(),
        documents,
        html,
        degradation: degradation.report(),
        input_loss: Vec::new(),
        target_loss,
        blocked: degradation.blocked,
    })
}

fn render_bundle(
    book: &Book,
    documents: &[PreviewDocument],
    profile: &PreviewTargetProfile,
) -> String {
    let content_width = profile.viewport_width.saturating_sub(48);
    let font_scale = f32::from(profile.font_size_percent) / 100.0;
    let mut html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>html{{font-size:{}rem}}body{{box-sizing:border-box;width:100%;max-width:{}px;min-height:{}px;margin:0 auto;padding:1.25rem;font-family:system-ui,sans-serif;line-height:1.6;background:#fff;color:#292521}}article{{margin:2rem 0;padding-bottom:1rem;border-bottom:1px solid #ddd}}.notice{{padding:.8rem;border-radius:.5rem;background:#fff5dd;color:#6a4300}}</style></head><body><p class=\"notice\">{} · compatibility preview</p><h1>{}</h1>",
        escape(book.metadata.display_title()),
        font_scale,
        content_width,
        profile.viewport_height,
        escape(&profile.disclaimer),
        escape(book.metadata.display_title())
    );
    for document in documents {
        html.push_str("<article>");
        if let Some(title) = &document.title {
            html.push_str(&format!("<h2>{}</h2>", escape(title)));
        }
        html.push_str(&document.html);
        html.push_str("</article>");
    }
    html.push_str("</body></html>");
    html
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

#[cfg(test)]
mod tests {
    use super::*;
    use folio_model::{ComputedStyle, Document, DocumentId, NodeId};

    #[test]
    fn preview_uses_the_same_degradation_plan_as_conversion() {
        let mut book = Book::new();
        let style = book.styles.intern(ComputedStyle::default());
        book.documents.push(Document {
            id: DocumentId::new(0),
            href: "book.xhtml".to_owned(),
            media_type: "application/xhtml+xml".to_owned(),
            title: None,
            nodes: vec![Node::new(
                NodeId::new(0),
                NodeKind::Paragraph,
                style,
                vec![Node::new(
                    NodeId::new(1),
                    NodeKind::Text {
                        value: "hello".to_owned(),
                    },
                    style,
                    Vec::new(),
                )],
            )],
        });
        let preview = build(
            &book,
            &BookEditPlan::default(),
            Format::Epub3,
            DegradationMode::Compatible,
            &DegradationOptions::default(),
        )
        .unwrap();
        assert!(preview.html.contains("hello"));
        assert_eq!(preview.target.format, Format::Epub3);
    }

    #[test]
    fn preview_renders_book_edit_styles_without_enabling_remote_content() {
        let mut book = Book::new();
        let style = book.styles.intern(ComputedStyle::default());
        book.documents.push(Document {
            id: DocumentId::new(0),
            href: "book.xhtml".to_owned(),
            media_type: "application/xhtml+xml".to_owned(),
            title: None,
            nodes: vec![Node::new(
                NodeId::new(0),
                NodeKind::Link {
                    href: "https://example.invalid/".to_owned(),
                },
                style,
                vec![Node::new(
                    NodeId::new(1),
                    NodeKind::Text {
                        value: "hello".to_owned(),
                    },
                    style,
                    Vec::new(),
                )],
            )],
        });
        let edits = BookEditPlan {
            styles: vec![folio_edit::StyleEdit {
                css: Some(
                    "color: #333; background-image: url(https://example.invalid/x)".to_owned(),
                ),
                ..folio_edit::StyleEdit::default()
            }],
            ..BookEditPlan::default()
        };
        let preview = build(
            &book,
            &edits,
            Format::Epub3,
            DegradationMode::Compatible,
            &DegradationOptions::default(),
        )
        .unwrap();
        assert!(preview.html.contains("color: #333"));
        assert!(!preview.html.contains("background-image"));
        assert!(!preview.html.contains("href=\"https://example.invalid/\""));
        assert!(preview.html.contains("hello"));
    }

    #[test]
    fn preview_profile_applies_device_orientation_and_font_size() {
        let settings = PreviewSettings {
            device: PreviewDevice::Phone,
            orientation: PreviewOrientation::Landscape,
            font_size_percent: 175,
        };
        let profile = PreviewTargetProfile::for_format(Format::Epub3, &settings);
        let html = render_bundle(&Book::new(), &[], &profile);

        assert_eq!(profile.viewport_width, 844);
        assert_eq!(profile.viewport_height, 390);
        assert_eq!(profile.font_size_percent, 175);
        assert!(html.contains("max-width:796px"));
        assert!(html.contains("min-height:390px"));
        assert!(html.contains("font-size:1.75rem"));
    }
}
