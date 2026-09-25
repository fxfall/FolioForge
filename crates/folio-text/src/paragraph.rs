use std::collections::BTreeSet;

use crate::{ParagraphAnalysis, ParagraphMode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedParagraph {
    pub start_line: usize,
    pub text: String,
    pub preformatted: bool,
}

pub fn parse_paragraphs(
    text: &str,
    analysis: &ParagraphAnalysis,
    excluded_lines: &BTreeSet<usize>,
) -> Vec<ParsedParagraph> {
    let lines = text.lines().collect::<Vec<_>>();
    let mode = analysis.selected;
    let protect_lines = analysis.protected_preformatted
        && matches!(mode, ParagraphMode::EveryLine | ParagraphMode::HardWrap);
    match mode {
        ParagraphMode::EveryLine => lines
            .iter()
            .enumerate()
            .filter(|(index, line)| !excluded_lines.contains(index) && !line.trim().is_empty())
            .map(|(index, line)| ParsedParagraph {
                start_line: index,
                text: if protect_lines {
                    (*line).to_owned()
                } else {
                    line.trim().to_owned()
                },
                preformatted: protect_lines,
            })
            .collect(),
        ParagraphMode::Indented => parse_indented(&lines, excluded_lines),
        ParagraphMode::Auto | ParagraphMode::BlankLine | ParagraphMode::HardWrap => {
            parse_blankline_groups(&lines, excluded_lines, protect_lines)
        }
    }
}

fn parse_blankline_groups(
    lines: &[&str],
    excluded_lines: &BTreeSet<usize>,
    preformatted: bool,
) -> Vec<ParsedParagraph> {
    let mut output = Vec::new();
    let mut start = None;
    let mut pieces = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if excluded_lines.contains(&index) || line.trim().is_empty() {
            flush_group(&mut output, &mut start, &mut pieces, preformatted);
            continue;
        }
        if start.is_none() {
            start = Some(index);
        }
        pieces.push(line.trim());
    }
    flush_group(&mut output, &mut start, &mut pieces, preformatted);
    output
}

fn flush_group(
    output: &mut Vec<ParsedParagraph>,
    start: &mut Option<usize>,
    pieces: &mut Vec<&str>,
    preformatted: bool,
) {
    if pieces.is_empty() {
        *start = None;
        return;
    }
    let mut text = String::with_capacity(pieces.iter().map(|piece| piece.len()).sum());
    let mut previous = None;
    for piece in pieces.iter() {
        if let Some(last) = previous {
            let next = piece.chars().next();
            if !next.is_some_and(|right| is_cjk(last) && is_cjk(right)) && !last.is_whitespace() {
                text.push(' ');
            }
        }
        text.push_str(piece);
        previous = piece.chars().last();
    }
    output.push(ParsedParagraph {
        start_line: start.take().unwrap_or_default(),
        text,
        preformatted,
    });
    pieces.clear();
}

fn parse_indented(lines: &[&str], excluded_lines: &BTreeSet<usize>) -> Vec<ParsedParagraph> {
    let mut output = Vec::new();
    let mut start = None;
    let mut current = String::new();
    let mut previous = None;
    for (index, line) in lines.iter().enumerate() {
        if excluded_lines.contains(&index) || line.trim().is_empty() {
            flush_indented(&mut output, &mut start, &mut current);
            previous = None;
            continue;
        }
        let begins_paragraph = line.starts_with([' ', '\t', '\u{3000}']);
        if begins_paragraph && !current.is_empty() {
            flush_indented(&mut output, &mut start, &mut current);
            previous = None;
        }
        if start.is_none() {
            start = Some(index);
        }
        let trimmed = line.trim();
        if let Some(last) = previous {
            let next = trimmed.chars().next();
            if !next.is_some_and(|right| is_cjk(last) && is_cjk(right)) {
                current.push(' ');
            }
        }
        current.push_str(trimmed);
        previous = trimmed.chars().last();
    }
    flush_indented(&mut output, &mut start, &mut current);
    output
}

fn flush_indented(
    output: &mut Vec<ParsedParagraph>,
    start: &mut Option<usize>,
    current: &mut String,
) {
    if current.is_empty() {
        *start = None;
        return;
    }
    output.push(ParsedParagraph {
        start_line: start.take().unwrap_or_default(),
        text: std::mem::take(current),
        preformatted: false,
    });
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x2e80..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x3134f)
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-text/src/paragraph.rs"]
mod tests;
