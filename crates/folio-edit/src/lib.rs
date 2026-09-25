//! Non-destructive edits over Folio Semantic IR.
//!
//! A `BookEditPlan` is data, not a second document model.  Applying a plan
//! clones the source IR, copies its lazy resource view, and returns a new IR;
//! the importer and the original source are never modified in place.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use folio_model::{Book, MemoryResourceLoader, ResourceId, ResourceKind, SemanticRole};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct MetadataEdit {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub authors: Option<Vec<String>>,
    pub contributors: Option<Vec<String>>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub date: Option<String>,
    pub series: Option<String>,
    pub series_index: Option<f64>,
    pub description: Option<String>,
    pub subjects: Option<Vec<String>>,
    pub identifiers: Option<Vec<String>>,
    pub rights: Option<String>,
    /// Fields in this set are cleared when the plan is applied.  This keeps
    /// `None` available for the important distinction between “unchanged”
    /// and “clear this value”.
    #[serde(default)]
    pub clear_fields: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub enum CoverEdit {
    #[default]
    Keep,
    Replace {
        file_name: String,
        media_type: String,
        bytes: Vec<u8>,
        fit: CoverFit,
    },
    Remove,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum CoverFit {
    #[default]
    Fill,
    Fit,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TypographyEdit {
    pub font_family: Option<String>,
    pub body_font_size: Option<String>,
    pub line_height: Option<String>,
    pub letter_spacing: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct FontEdit {
    #[serde(default)]
    pub strip_embedded_fonts: bool,
    pub preferred_family: Option<String>,
    pub replacement: Option<FontReplacement>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FontReplacement {
    pub file_name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
    pub family: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct StyleEdit {
    pub node_id: Option<u32>,
    pub role: Option<SemanticRole>,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
    /// A compact declaration list (`font-weight: bold; color: #333`) that is
    /// converted to the same computed property map as explicit properties.
    pub css: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct StructureEdit {
    /// New document order expressed as stable `DocumentId` values.
    pub document_order: Option<Vec<u32>>,
    #[serde(default)]
    pub document_titles: BTreeMap<u32, String>,
    #[serde(default)]
    pub toc_labels: BTreeMap<String, String>,
    #[serde(default)]
    pub remove_navigation: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BookEditPlan {
    pub metadata: MetadataEdit,
    pub cover: CoverEdit,
    pub typography: TypographyEdit,
    pub fonts: FontEdit,
    #[serde(default)]
    pub styles: Vec<StyleEdit>,
    pub structure: StructureEdit,
}

impl BookEditPlan {
    pub fn is_empty(&self) -> bool {
        self.metadata == MetadataEdit::default()
            && matches!(self.cover, CoverEdit::Keep)
            && self.typography == TypographyEdit::default()
            && self.fonts == FontEdit::default()
            && self.styles.is_empty()
            && self.structure == StructureEdit::default()
    }

    pub fn apply(&self, source: &Book) -> Result<Book, EditError> {
        let mut book = source.clone();
        let mut loader = MemoryResourceLoader::default();
        for resource in &book.resources {
            if let Ok(bytes) = source.load_resource(resource.id, None) {
                loader.insert(resource.path.clone(), bytes);
            }
        }
        apply_metadata(&mut book, &self.metadata);
        apply_structure(&mut book, &self.structure);
        apply_typography(&mut book, &self.typography);
        apply_styles(&mut book, &self.styles)?;
        apply_fonts(&mut book, &self.fonts, &mut loader)?;
        apply_cover(&mut book, &self.cover, &mut loader)?;
        book = book.with_resource_loader(Arc::new(loader));
        Ok(book)
    }
}

#[derive(Debug, Error)]
pub enum EditError {
    #[error("style edit refers to an unknown node {0}")]
    UnknownNode(u32),
    #[error("replacement cover has an empty file name")]
    EmptyCoverName,
    #[error("replacement cover has an empty media type")]
    EmptyCoverMediaType,
    #[error("replacement font has an empty file name")]
    EmptyFontName,
    #[error("replacement font has an empty media type")]
    EmptyFontMediaType,
    #[error("replacement font data is invalid or exceeds 32 MiB")]
    InvalidFontData,
}

fn apply_metadata(book: &mut Book, edit: &MetadataEdit) {
    let metadata = &mut book.metadata;
    if let Some(value) = &edit.title {
        metadata.title = Some(value.clone());
    }
    if let Some(value) = &edit.subtitle {
        metadata.subtitle = Some(value.clone());
    }
    if let Some(values) = &edit.authors {
        metadata.authors = values.clone();
        metadata.creators = values.clone();
    }
    if let Some(values) = &edit.contributors {
        metadata.contributors = values.clone();
    }
    if let Some(value) = &edit.language {
        metadata.language = Some(value.clone());
    }
    if let Some(value) = &edit.publisher {
        metadata.publisher = Some(value.clone());
    }
    if let Some(value) = &edit.date {
        metadata.date = Some(value.clone());
        metadata.dates = vec![value.clone()];
    }
    if let Some(value) = &edit.series {
        metadata.series = Some(value.clone());
    }
    if let Some(value) = edit.series_index {
        metadata.series_index = Some(value);
    }
    if let Some(value) = &edit.description {
        metadata.description = Some(value.clone());
    }
    if let Some(values) = &edit.subjects {
        metadata.subjects = values.clone();
    }
    if let Some(values) = &edit.identifiers {
        metadata.identifiers = values.clone();
        metadata.identifier = values.first().cloned();
    }
    if let Some(value) = &edit.rights {
        metadata.rights = Some(value.clone());
    }

    for field in &edit.clear_fields {
        match field.as_str() {
            "title" => metadata.title = None,
            "subtitle" => metadata.subtitle = None,
            "authors" | "creators" => {
                metadata.authors.clear();
                metadata.creators.clear();
            }
            "contributors" => metadata.contributors.clear(),
            "language" => metadata.language = None,
            "publisher" => metadata.publisher = None,
            "date" | "dates" => {
                metadata.date = None;
                metadata.dates.clear();
            }
            "series" => metadata.series = None,
            "series_index" => metadata.series_index = None,
            "description" => metadata.description = None,
            "subjects" => metadata.subjects.clear(),
            "identifier" | "identifiers" => {
                metadata.identifier = None;
                metadata.identifiers.clear();
            }
            "rights" => metadata.rights = None,
            _ => {}
        }
    }
}

fn apply_structure(book: &mut Book, edit: &StructureEdit) {
    if let Some(order) = &edit.document_order {
        let mut reordered = Vec::with_capacity(book.documents.len());
        for id in order {
            if let Some(document) = book.documents.iter().find(|item| item.id.get() == *id) {
                reordered.push(document.clone());
            }
        }
        for document in &book.documents {
            if !reordered
                .iter()
                .any(|item: &folio_model::Document| item.id == document.id)
            {
                reordered.push(document.clone());
            }
        }
        book.documents = reordered;
    }
    for (id, title) in &edit.document_titles {
        if let Some(document) = book.documents.iter_mut().find(|item| item.id.get() == *id) {
            document.title = Some(title.clone());
        }
    }
    if !edit.toc_labels.is_empty() {
        rename_nav_points(&mut book.navigation.toc, &edit.toc_labels);
        rename_nav_points(&mut book.navigation.landmarks, &edit.toc_labels);
        rename_nav_points(&mut book.navigation.page_list, &edit.toc_labels);
    }
    if edit.remove_navigation {
        book.navigation.toc.clear();
        book.navigation.landmarks.clear();
        book.navigation.page_list.clear();
    }
}

fn rename_nav_points(points: &mut [folio_model::NavPoint], labels: &BTreeMap<String, String>) {
    for point in points {
        if let Some(label) = labels.get(&point.href).or_else(|| labels.get(&point.label)) {
            point.label = label.clone();
        }
        rename_nav_points(&mut point.children, labels);
    }
}

fn apply_typography(book: &mut Book, edit: &TypographyEdit) {
    if edit == &TypographyEdit::default() {
        return;
    }
    let mut ids = Vec::new();
    for (id, style) in book.styles.iter() {
        let mut style = style.clone();
        if let Some(value) = &edit.font_family {
            style
                .properties
                .insert("font-family".to_owned(), value.clone());
        }
        if let Some(value) = &edit.body_font_size {
            style
                .properties
                .insert("font-size".to_owned(), value.clone());
        }
        if let Some(value) = &edit.line_height {
            style
                .properties
                .insert("line-height".to_owned(), value.clone());
        }
        if let Some(value) = &edit.letter_spacing {
            style
                .properties
                .insert("letter-spacing".to_owned(), value.clone());
        }
        ids.push((id, style));
    }
    for (old_id, style) in ids {
        let new_id = book.styles.intern(style);
        replace_style_id(book, old_id, new_id);
    }
}

fn apply_styles(book: &mut Book, edits: &[StyleEdit]) -> Result<(), EditError> {
    for edit in edits {
        let declarations = edit
            .css
            .as_deref()
            .map(parse_css_declarations)
            .unwrap_or_default();
        let properties = edit.properties.iter().chain(declarations.iter());
        let mut matched = false;
        for document in &mut book.documents {
            for node in &mut document.nodes {
                matched |= apply_node_style(node, edit, properties.clone(), &mut book.styles);
            }
        }
        if edit.node_id.is_some() && !matched {
            return Err(EditError::UnknownNode(edit.node_id.unwrap_or_default()));
        }
    }
    Ok(())
}

fn apply_node_style<'a, I>(
    node: &mut folio_model::Node,
    edit: &StyleEdit,
    properties: I,
    pool: &mut folio_model::StylePool,
) -> bool
where
    I: Iterator<Item = (&'a String, &'a String)> + Clone,
{
    let matches = edit.node_id.is_none_or(|id| id == node.id.get())
        && edit.role.is_none_or(|role| role == node.role);
    let mut changed = false;
    if matches {
        let mut style = pool.get(node.style).cloned().unwrap_or_default();
        for (key, value) in properties.clone() {
            style
                .properties
                .insert(key.trim().to_ascii_lowercase(), value.trim().to_owned());
            changed = true;
        }
        if changed {
            node.style = pool.intern(style);
        }
    }
    for child in &mut node.children {
        changed |= apply_node_style(child, edit, properties.clone(), pool);
    }
    changed
}

fn apply_fonts(
    book: &mut Book,
    edit: &FontEdit,
    loader: &mut MemoryResourceLoader,
) -> Result<(), EditError> {
    if edit.strip_embedded_fonts {
        book.resources
            .retain(|resource| resource.kind != ResourceKind::Font);
        book.font_faces.clear();
    }
    if let Some(replacement) = &edit.replacement {
        if replacement.file_name.trim().is_empty() {
            return Err(EditError::EmptyFontName);
        }
        if replacement.media_type.trim().is_empty() {
            return Err(EditError::EmptyFontMediaType);
        }
        let detected_type =
            detect_font_media_type(&replacement.bytes).ok_or(EditError::InvalidFontData)?;
        if replacement.bytes.len() > 32 * 1024 * 1024
            || !font_media_types_compatible(&replacement.media_type, detected_type)
        {
            return Err(EditError::InvalidFontData);
        }
        let id = book
            .resources
            .iter()
            .map(|resource| resource.id.get())
            .max()
            .map_or(0, |value| value.saturating_add(1));
        let path = unique_resource_path(book, "fonts", &replacement.file_name);
        loader.insert(path.clone(), replacement.bytes.clone());
        book.resources.push(folio_model::Resource {
            id: ResourceId::new(id),
            path,
            media_type: detected_type.to_owned(),
            kind: ResourceKind::Font,
            properties: Vec::new(),
            size: Some(replacement.bytes.len() as u64),
        });
        if let Some(family) = edit
            .preferred_family
            .as_ref()
            .or(replacement.family.as_ref())
            .filter(|family| !family.trim().is_empty())
        {
            book.font_faces.push(folio_model::FontFace {
                resource: ResourceId::new(id),
                family: family.trim().to_owned(),
            });
        }
    }
    if let Some(family) = edit.preferred_family.as_ref().or_else(|| {
        edit.replacement
            .as_ref()
            .and_then(|font| font.family.as_ref())
    }) {
        let typography = TypographyEdit {
            font_family: Some(family.clone()),
            ..TypographyEdit::default()
        };
        apply_typography(book, &typography);
    }
    Ok(())
}

fn unique_resource_path(book: &Book, directory: &str, file_name: &str) -> String {
    let file_name = sanitize_name(file_name);
    let path = format!("{directory}/{file_name}");
    if !book.resources.iter().any(|resource| resource.path == path) {
        return path;
    }
    let stem = std::path::Path::new(&file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("asset");
    let extension = std::path::Path::new(&file_name)
        .extension()
        .and_then(|value| value.to_str());
    for suffix in 1u32.. {
        let name = extension.map_or_else(
            || format!("{stem}-{suffix}"),
            |extension| format!("{stem}-{suffix}.{extension}"),
        );
        let candidate = format!("{directory}/{name}");
        if !book
            .resources
            .iter()
            .any(|resource| resource.path == candidate)
        {
            return candidate;
        }
    }
    format!("{directory}/{file_name}")
}

fn detect_font_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"wOF2") {
        Some("font/woff2")
    } else if bytes.starts_with(b"wOFF") {
        Some("font/woff")
    } else if bytes.starts_with(b"OTTO") {
        Some("font/otf")
    } else if bytes.starts_with(b"ttcf") {
        Some("font/collection")
    } else if bytes.starts_with(&[0, 1, 0, 0]) || bytes.starts_with(b"true") {
        Some("font/ttf")
    } else {
        None
    }
}

fn font_media_types_compatible(declared: &str, detected: &str) -> bool {
    let declared = declared.to_ascii_lowercase();
    matches!(
        (declared.as_str(), detected),
        ("font/woff2", "font/woff2")
            | ("application/font-woff", "font/woff")
            | ("font/woff", "font/woff")
            | ("font/otf", "font/otf")
            | ("font/ttf", "font/ttf")
            | ("application/x-font-ttf", "font/ttf")
            | ("font/collection", "font/collection")
    )
}

fn apply_cover(
    book: &mut Book,
    edit: &CoverEdit,
    loader: &mut MemoryResourceLoader,
) -> Result<(), EditError> {
    match edit {
        CoverEdit::Keep => {}
        CoverEdit::Remove => {
            for resource in &mut book.resources {
                resource
                    .properties
                    .retain(|property| property != "cover-image");
            }
        }
        CoverEdit::Replace {
            file_name,
            media_type,
            bytes,
            fit,
        } => {
            if file_name.trim().is_empty() {
                return Err(EditError::EmptyCoverName);
            }
            if media_type.trim().is_empty() {
                return Err(EditError::EmptyCoverMediaType);
            }
            for resource in &mut book.resources {
                resource
                    .properties
                    .retain(|property| property != "cover-image");
            }
            let id = ResourceId::new(book.resources.len() as u32);
            let path = format!("cover/{}", sanitize_name(file_name));
            loader.insert(path.clone(), bytes.clone());
            book.resources.push(folio_model::Resource {
                id,
                path,
                media_type: media_type.clone(),
                kind: kind_for_media_type(media_type),
                properties: vec![
                    "cover-image".to_owned(),
                    format!("fit:{fit:?}").to_ascii_lowercase(),
                ],
                size: Some(bytes.len() as u64),
            });
        }
    }
    Ok(())
}

fn replace_style_id(book: &mut Book, old: folio_model::StyleId, new: folio_model::StyleId) {
    for document in &mut book.documents {
        for node in &mut document.nodes {
            replace_node_style(node, old, new);
        }
    }
}

fn replace_node_style(
    node: &mut folio_model::Node,
    old: folio_model::StyleId,
    new: folio_model::StyleId,
) {
    if node.style == old {
        node.style = new;
    }
    for child in &mut node.children {
        replace_node_style(child, old, new);
    }
}

fn parse_css_declarations(css: &str) -> BTreeMap<String, String> {
    css.split(';')
        .filter_map(|part| {
            let (key, value) = part.split_once(':')?;
            let key = key.trim().to_ascii_lowercase();
            let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
            (!key.is_empty() && !value.is_empty()).then_some((key, value))
        })
        .collect()
}

fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn kind_for_media_type(media_type: &str) -> ResourceKind {
    match media_type.to_ascii_lowercase().as_str() {
        "image/jpeg" => ResourceKind::Jpeg,
        "image/png" => ResourceKind::Png,
        "image/gif" => ResourceKind::Gif,
        "image/svg+xml" => ResourceKind::Svg,
        _ => ResourceKind::Unknown,
    }
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-edit/src/lib.rs"]
mod tests;
