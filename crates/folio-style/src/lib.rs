//! CSS discovery, cascade, inheritance, style interning helpers, and target
//! compatibility mapping.  The parser deliberately keeps unknown declarations
//! as normalized strings so a later target can support them without changing
//! the canonical model.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, RwLock},
};

use folio_model::{ComputedStyle, StylePool};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TargetProfile {
    #[default]
    Generic,
    Kf7,
    Kf8,
    Kfx,
}

#[derive(Clone, Debug, Default)]
pub struct SpecifiedStyle {
    pub properties: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
struct Rule {
    selector: String,
    declarations: BTreeMap<String, String>,
    specificity: (u16, u16, u16),
    order: usize,
}

#[derive(Clone, Debug, Default)]
pub struct ElementContext {
    pub tag: String,
    pub id: Option<String>,
    pub classes: BTreeSet<String>,
    pub inline_style: Option<String>,
}

impl ElementContext {
    pub fn new(tag: impl Into<String>) -> Self {
        Self {
            tag: tag.into().to_ascii_lowercase(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Default)]
pub struct StyleResolver {
    rules: Vec<Rule>,
    target: TargetProfile,
    matched_cache: Arc<RwLock<BTreeMap<ElementMatchKey, Vec<usize>>>>,
    candidate_index: RuleCandidateIndex,
}

impl Clone for StyleResolver {
    fn clone(&self) -> Self {
        Self {
            rules: self.rules.clone(),
            target: self.target,
            matched_cache: Arc::new(RwLock::new(BTreeMap::new())),
            candidate_index: self.candidate_index.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ElementMatchKey {
    tag: String,
    id: Option<String>,
    classes: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct RuleCandidateIndex {
    universal: Vec<usize>,
    by_tag: BTreeMap<String, Vec<usize>>,
    by_class: BTreeMap<String, Vec<usize>>,
    by_id: BTreeMap<String, Vec<usize>>,
}

impl StyleResolver {
    pub fn new(target: TargetProfile) -> Self {
        Self {
            rules: Vec::new(),
            target,
            matched_cache: Arc::new(RwLock::new(BTreeMap::new())),
            candidate_index: RuleCandidateIndex::default(),
        }
    }

    pub fn from_stylesheets<'a, I>(stylesheets: I, target: TargetProfile) -> Self
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut resolver = Self::new(target);
        for (_, stylesheet) in stylesheets {
            resolver.add_stylesheet(stylesheet);
        }
        resolver
    }

    pub fn add_stylesheet(&mut self, css: &str) {
        let mut order = self.rules.len();
        let previous_rule_count = self.rules.len();
        parse_scope(css, self.target, &mut self.rules, &mut order);
        for rule_index in previous_rule_count..self.rules.len() {
            self.candidate_index
                .insert(rule_index, &self.rules[rule_index].selector);
        }
        if let Ok(mut cache) = self.matched_cache.write() {
            cache.clear();
        }
    }

    pub fn resolve(
        &self,
        element: &ElementContext,
        parent: Option<&ComputedStyle>,
    ) -> ComputedStyle {
        let mut properties = BTreeMap::new();
        add_defaults(&mut properties, &element.tag);
        if let Some(parent) = parent {
            for property in INHERITED_PROPERTIES {
                if let Some(value) = parent.get(property) {
                    properties.insert((*property).to_owned(), value.to_owned());
                }
            }
        }

        let key = ElementMatchKey {
            tag: element.tag.clone(),
            id: element.id.clone(),
            classes: element.classes.iter().cloned().collect(),
        };
        let matching_rules = self.cached_matching_rules(&key, element);
        for rule_index in matching_rules {
            let rule = &self.rules[rule_index];
            for (property, value) in &rule.declarations {
                properties.insert(property.clone(), value.clone());
            }
        }
        if let Some(inline_style) = &element.inline_style {
            properties.extend(parse_declarations(inline_style));
        }
        normalize_properties(&mut properties);
        ComputedStyle { properties }
    }

    pub fn intern(&self, pool: &mut StylePool, style: ComputedStyle) -> folio_model::StyleId {
        pool.intern(style)
    }

    pub fn target(&self) -> TargetProfile {
        self.target
    }

    fn cached_matching_rules(&self, key: &ElementMatchKey, element: &ElementContext) -> Vec<usize> {
        if let Ok(cache) = self.matched_cache.read() {
            if let Some(indices) = cache.get(key) {
                return indices.clone();
            }
        }
        let mut indices = self
            .candidate_index
            .candidates(element)
            .into_iter()
            .filter(|index| matches_selector(&self.rules[*index].selector, element))
            .collect::<Vec<_>>();
        indices.sort_by_key(|index| {
            let rule = &self.rules[*index];
            (rule.specificity, rule.order)
        });
        if let Ok(mut cache) = self.matched_cache.write() {
            cache.insert(key.clone(), indices.clone());
        }
        indices
    }
}

impl RuleCandidateIndex {
    fn insert(&mut self, rule_index: usize, selector: &str) {
        let compound = rightmost_compound(selector);
        let tag_end = compound.find(['.', '#', '[']).unwrap_or(compound.len());
        let tag = compound[..tag_end]
            .split(':')
            .next()
            .unwrap_or_default()
            .trim();
        let mut indexed = false;
        if !tag.is_empty() && tag != "*" {
            self.by_tag
                .entry(tag.to_ascii_lowercase())
                .or_default()
                .push(rule_index);
            indexed = true;
        }
        for class in compound
            .split('.')
            .skip(1)
            .map(|value| value.split(['#', '[', ':']).next().unwrap_or(value))
            .filter(|value| !value.is_empty())
        {
            self.by_class
                .entry(class.to_owned())
                .or_default()
                .push(rule_index);
            indexed = true;
        }
        for id in compound
            .split('#')
            .skip(1)
            .map(|value| value.split(['.', '[', ':']).next().unwrap_or(value))
            .filter(|value| !value.is_empty())
        {
            self.by_id
                .entry(id.to_owned())
                .or_default()
                .push(rule_index);
            indexed = true;
        }
        if !indexed || compound.contains('[') || compound.contains(':') {
            self.universal.push(rule_index);
        }
    }

    fn candidates(&self, element: &ElementContext) -> Vec<usize> {
        let mut candidates = BTreeSet::new();
        candidates.extend(self.universal.iter().copied());
        if let Some(indices) = self.by_tag.get(&element.tag) {
            candidates.extend(indices.iter().copied());
        }
        for class in &element.classes {
            if let Some(indices) = self.by_class.get(class) {
                candidates.extend(indices.iter().copied());
            }
        }
        if let Some(id) = element.id.as_deref() {
            if let Some(indices) = self.by_id.get(id) {
                candidates.extend(indices.iter().copied());
            }
        }
        candidates.into_iter().collect()
    }
}

const INHERITED_PROPERTIES: &[&str] = &[
    "color",
    "direction",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "line-height",
    "text-align",
    "visibility",
    "white-space",
    "writing-mode",
];

fn add_defaults(properties: &mut BTreeMap<String, String>, tag: &str) {
    properties.insert(
        "display".to_owned(),
        if is_inline_tag(tag) {
            "inline"
        } else {
            "block"
        }
        .to_owned(),
    );
    properties.insert("font-size".to_owned(), "1em".to_owned());
    properties.insert(
        "font-weight".to_owned(),
        if matches!(
            tag,
            "strong" | "b" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        ) {
            "bold"
        } else {
            "normal"
        }
        .to_owned(),
    );
    properties.insert(
        "font-style".to_owned(),
        if matches!(tag, "em" | "i") {
            "italic"
        } else {
            "normal"
        }
        .to_owned(),
    );
    properties.insert("line-height".to_owned(), "normal".to_owned());
    properties.insert("color".to_owned(), "#000000".to_owned());
    properties.insert("visibility".to_owned(), "visible".to_owned());
    properties.insert("white-space".to_owned(), "normal".to_owned());
    properties.insert("direction".to_owned(), "ltr".to_owned());
    properties.insert("writing-mode".to_owned(), "horizontal-tb".to_owned());
}

fn is_inline_tag(tag: &str) -> bool {
    matches!(
        tag,
        "a" | "abbr"
            | "b"
            | "br"
            | "code"
            | "em"
            | "i"
            | "img"
            | "ruby"
            | "small"
            | "span"
            | "strong"
            | "sub"
            | "sup"
    )
}

fn normalize_properties(properties: &mut BTreeMap<String, String>) {
    let replacements: Vec<_> = properties
        .iter()
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), collapse_space(value)))
        .collect();
    properties.clear();
    properties.extend(replacements);
}

fn collapse_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_css_comments(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("/*") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find("*/") {
            rest = &after[end + 2..];
        } else {
            return output;
        }
    }
    output.push_str(rest);
    output
}

fn parse_scope(input: &str, target: TargetProfile, rules: &mut Vec<Rule>, order: &mut usize) {
    let input = strip_css_comments(input);
    let mut cursor = 0;
    while cursor < input.len() {
        let Some(open_relative) = input[cursor..].find('{') else {
            break;
        };
        let open = cursor + open_relative;
        let header = input[cursor..open].trim();
        let Some(close) = matching_brace(&input, open) else {
            break;
        };
        let body = &input[open + 1..close];
        if header.to_ascii_lowercase().starts_with("@media") {
            if media_applies(header, target) {
                parse_scope(body, target, rules, order);
            }
        } else if !header.starts_with('@') {
            let declarations = parse_declarations(body);
            if !declarations.is_empty() {
                for selector in split_selector_list(header) {
                    let selector = selector.trim();
                    if selector.is_empty() {
                        continue;
                    }
                    rules.push(Rule {
                        selector: selector.to_owned(),
                        declarations: declarations.clone(),
                        specificity: specificity(selector),
                        order: *order,
                    });
                    *order += 1;
                }
            }
        }
        cursor = close + 1;
    }
}

fn matching_brace(input: &str, open: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut depth = 0usize;
    let mut quote = None;
    let mut index = open;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(q) = quote {
            if byte == q && (index == 0 || bytes[index - 1] != b'\\') {
                quote = None;
            }
        } else if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
        } else if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

fn media_applies(header: &str, target: TargetProfile) -> bool {
    let lower = header.to_ascii_lowercase();
    if lower.contains("amzn-mobi") {
        return matches!(target, TargetProfile::Kf7);
    }
    if lower.contains("amzn-kf8") {
        return matches!(target, TargetProfile::Kf8);
    }
    if lower.contains("screen") || lower.contains("all") || lower.contains("print") {
        return true;
    }
    matches!(target, TargetProfile::Generic)
}

fn split_selector_list(header: &str) -> Vec<&str> {
    header.split(',').collect()
}

fn parse_declarations(body: &str) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for declaration in body.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        let value = collapse_space(value.trim());
        if property.is_empty() || value.is_empty() || property.starts_with("--") {
            continue;
        }
        result.insert(property, value);
    }
    result
}

fn rightmost_compound(selector: &str) -> &str {
    selector
        .split(|character: char| character.is_whitespace() || character == '>')
        .rfind(|part| !part.is_empty())
        .unwrap_or(selector)
        .trim()
}

fn matches_selector(selector: &str, element: &ElementContext) -> bool {
    // The canonical model does not retain a DOM.  Matching the right-most
    // compound selector gives deterministic and useful support for EPUB CSS
    // while avoiding a hidden HTML representation in folio-model.
    let compound = rightmost_compound(selector);
    if compound.is_empty() {
        return false;
    }
    let compound = compound.split(':').next().unwrap_or(compound);
    let tag_end = compound.find(['.', '#', '[']).unwrap_or(compound.len());
    let tag = &compound[..tag_end];
    if !tag.is_empty() && tag != "*" && !tag.eq_ignore_ascii_case(&element.tag) {
        return false;
    }
    for id in compound
        .split('#')
        .skip(1)
        .map(|value| value.split(['.', '[']).next().unwrap_or(value))
    {
        if element.id.as_deref() != Some(id) {
            return false;
        }
    }
    for class in compound
        .split('.')
        .skip(1)
        .map(|value| value.split(['#', '[']).next().unwrap_or(value))
    {
        if !element.classes.contains(class) {
            return false;
        }
    }
    true
}

fn specificity(selector: &str) -> (u16, u16, u16) {
    let ids = selector.matches('#').count() as u16;
    let classes = selector.matches('.').count() as u16 + selector.matches('[').count() as u16;
    let elements = selector
        .split(|character: char| {
            character.is_whitespace()
                || character == '>'
                || character == '.'
                || character == '#'
                || character == '['
        })
        .filter(|part| !part.is_empty() && *part != "*")
        .count() as u16;
    (ids, classes, elements)
}

pub fn inline_css(style: &ComputedStyle) -> String {
    style
        .properties
        .iter()
        .map(|(property, value)| format!("{property}:{value}"))
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-style/src/lib.rs"]
mod tests;
