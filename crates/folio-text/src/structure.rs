use serde::{Deserialize, Serialize};

use crate::{ParagraphMode, TextImportMode};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureKind {
    Volume,
    Chapter,
    Prologue,
    Epilogue,
    Introduction,
    Afterword,
    Supplement,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StructureCandidate {
    pub line_index: usize,
    pub text: String,
    pub kind: StructureKind,
    pub level: u8,
    pub number: Option<u32>,
    pub confidence_percent: u8,
    pub detector: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StructureNode {
    pub candidate: StructureCandidate,
    pub children: Vec<StructureNode>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BookStructure {
    /// Volume/part roots contain chapter nodes; ungrouped chapters are roots.
    pub roots: Vec<StructureNode>,
    /// Pattern matches excluded by confidence and false-split safeguards.
    pub rejected: Vec<StructureCandidate>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParagraphAnalysis {
    pub selected: ParagraphMode,
    pub confidence_percent: u8,
    pub protected_preformatted: bool,
    pub evidence: Vec<String>,
}

pub fn analyze_structure(text: &str, mode: TextImportMode) -> BookStructure {
    if matches!(mode, TextImportMode::Plain | TextImportMode::Markdown) {
        return BookStructure::default();
    }

    let lines = text.lines().collect::<Vec<_>>();
    let mut candidates = lines
        .iter()
        .enumerate()
        .filter_map(|(line_index, line)| {
            parse_heading(line).map(|(kind, number, detector)| {
                score_heading(&lines, line_index, kind, number, detector)
            })
        })
        .collect::<Vec<_>>();
    score_sequences(&mut candidates);

    // Auto is deliberately more conservative than explicit Novel mode: a
    // single mid-book pattern is not enough evidence to split a plain essay.
    let auto_has_structure = candidates
        .iter()
        .filter(|candidate| candidate.confidence_percent >= 65)
        .count()
        > 1
        || candidates.first().is_some_and(|candidate| {
            candidate.line_index == 0 && candidate.confidence_percent >= 85
        });
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    for candidate in candidates {
        if candidate.confidence_percent >= 75
            && (mode == TextImportMode::Novel || auto_has_structure)
        {
            accepted.push(candidate);
        } else {
            rejected.push(candidate);
        }
    }

    let mut roots = Vec::<StructureNode>::new();
    let mut current_volume = None;
    for candidate in accepted {
        if candidate.kind == StructureKind::Volume {
            roots.push(StructureNode {
                candidate,
                children: Vec::new(),
            });
            current_volume = Some(roots.len() - 1);
        } else {
            let node = StructureNode {
                candidate,
                children: Vec::new(),
            };
            if let Some(volume_index) = current_volume {
                roots[volume_index].children.push(node);
            } else {
                roots.push(node);
            }
        }
    }
    BookStructure { roots, rejected }
}

pub fn analyze_paragraphs(text: &str, requested: ParagraphMode) -> ParagraphAnalysis {
    let lines = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return ParagraphAnalysis {
            selected: ParagraphMode::BlankLine,
            confidence_percent: 100,
            protected_preformatted: false,
            evidence: vec!["empty-or-whitespace-only text".to_owned()],
        };
    }

    let short_lines = lines
        .iter()
        .filter(|line| line.chars().count() <= 42)
        .count();
    let indented = lines
        .iter()
        .filter(|line| line.starts_with([' ', '\t', '\u{3000}']))
        .count();
    let nonblank_count = lines.len();
    let protected_preformatted = short_lines * 100 >= nonblank_count * 65
        && (indented * 100 >= nonblank_count * 10 || has_irregular_line_lengths(&lines));
    let (median, hard_wrap_share) = line_length_statistics(&lines);
    let blank_lines = text.lines().filter(|line| line.trim().is_empty()).count();
    let hardwrap_evidence = (42..=90).contains(&median)
        && hard_wrap_share >= 65
        && blank_lines * 100 < text.lines().count().max(1) * 12;

    let (selected, confidence_percent, evidence) = match requested {
        ParagraphMode::EveryLine => (
            ParagraphMode::EveryLine,
            100,
            vec!["explicit every-line mode".to_owned()],
        ),
        ParagraphMode::Indented => (
            ParagraphMode::Indented,
            85,
            vec!["explicit indentation-based mode".to_owned()],
        ),
        ParagraphMode::BlankLine => (
            ParagraphMode::BlankLine,
            90,
            vec!["explicit blank-line mode".to_owned()],
        ),
        ParagraphMode::HardWrap if protected_preformatted => (
            ParagraphMode::EveryLine,
            92,
            vec!["short/irregular or indented lines protected from unwrapping".to_owned()],
        ),
        ParagraphMode::HardWrap => (
            ParagraphMode::HardWrap,
            if hardwrap_evidence { 86 } else { 62 },
            vec!["hard-wrap requested; source line-shape evidence recorded".to_owned()],
        ),
        ParagraphMode::Auto if protected_preformatted => (
            ParagraphMode::EveryLine,
            88,
            vec!["short/irregular or indented lines suggest poetry/code".to_owned()],
        ),
        ParagraphMode::Auto if hardwrap_evidence => (
            ParagraphMode::HardWrap,
            82,
            vec!["line lengths and sparse blank lines support hard-wrap recovery".to_owned()],
        ),
        ParagraphMode::Auto => (
            ParagraphMode::BlankLine,
            76,
            vec!["no strong hard-wrap or preformatted pattern; use blank lines".to_owned()],
        ),
    };

    ParagraphAnalysis {
        selected,
        confidence_percent,
        protected_preformatted,
        evidence,
    }
}

fn score_heading(
    lines: &[&str],
    line_index: usize,
    kind: StructureKind,
    number: Option<u32>,
    detector: &'static str,
) -> StructureCandidate {
    let line = lines[line_index].trim();
    let mut score = if number.is_some() { 55i32 } else { 50i32 };
    let mut evidence = vec!["explicit heading pattern".to_owned()];
    let chars = line.chars().count();
    if chars <= 30 {
        score += 12;
        evidence.push("short standalone heading".to_owned());
    } else if chars <= 60 {
        score += 6;
    }
    let previous_blank = line_index == 0 || lines[line_index - 1].trim().is_empty();
    let next_blank = line_index + 1 == lines.len() || lines[line_index + 1].trim().is_empty();
    if previous_blank {
        score += if line_index == 0 { 8 } else { 14 };
        evidence.push("preceded by a paragraph boundary".to_owned());
    }
    if next_blank {
        score += 8;
        evidence.push("followed by a paragraph boundary".to_owned());
    }
    if !previous_blank && !next_blank {
        score -= 12;
        evidence.push("no surrounding paragraph boundary".to_owned());
    }
    if has_sentence_ending(line) {
        score -= 25;
        evidence.push("sentence-ending punctuation lowers split confidence".to_owned());
    }

    StructureCandidate {
        line_index,
        text: line.to_owned(),
        kind,
        level: if kind == StructureKind::Volume { 1 } else { 2 },
        number,
        confidence_percent: score.clamp(0, 100) as u8,
        detector: detector.to_owned(),
        evidence,
    }
}

fn score_sequences(candidates: &mut [StructureCandidate]) {
    let mut previous = None::<(StructureKind, u32, usize, String)>;
    for candidate in candidates.iter_mut() {
        let Some(number) = candidate.number else {
            continue;
        };
        let sequence_kind = candidate.kind;
        if let Some((previous_kind, previous_number, previous_index, previous_detector)) = previous
        {
            if previous_kind == sequence_kind && previous_detector == candidate.detector {
                if number == previous_number.saturating_add(1) {
                    candidate.confidence_percent =
                        candidate.confidence_percent.saturating_add(15).min(100);
                    candidate
                        .evidence
                        .push("continues adjacent numeric sequence".to_owned());
                } else if number <= previous_number || number.saturating_sub(previous_number) > 4 {
                    candidate.confidence_percent = candidate.confidence_percent.saturating_sub(18);
                    candidate
                        .evidence
                        .push("inconsistent chapter numbering".to_owned());
                }
            }
            if candidate.line_index.saturating_sub(previous_index) > 1 {
                candidate
                    .evidence
                    .push("candidate sequence is separated in source".to_owned());
            }
        }
        previous = Some((
            sequence_kind,
            number,
            candidate.line_index,
            candidate.detector.clone(),
        ));
    }
}

fn parse_heading(line: &str) -> Option<(StructureKind, Option<u32>, &'static str)> {
    let text = line.trim();
    if text.is_empty() || text.chars().count() > 70 || has_sentence_ending(text) {
        return None;
    }
    if let Some((kind, number)) = parse_japanese_heading(text) {
        return Some((kind, number, "japanese_detector"));
    }
    if let Some(rest) = text.strip_prefix("第") {
        let marker_index = rest.find(['章', '回', '节', '卷', '部', '篇', '幕', '集'])?;
        let numeral = &rest[..marker_index];
        let marker = rest[marker_index..].chars().next()?;
        let title_tail = rest[marker_index + marker.len_utf8()..]
            .trim_start_matches([' ', '\t', '：', ':', '、', '—', '-']);
        if !title_tail.is_empty() && title_tail.chars().count() > 50 {
            return None;
        }
        let number = parse_number(numeral);
        if number.is_none() || numeral.is_empty() {
            return None;
        }
        let kind = if matches!(marker, '卷' | '部') {
            StructureKind::Volume
        } else {
            StructureKind::Chapter
        };
        let detector = match marker {
            '章' => "cjk_explicit_chapter",
            '回' => "chinese_novel_hui",
            '节' => "cjk_explicit_section",
            '卷' | '部' => "cjk_volume_detector",
            '篇' => "cjk_explicit_essay",
            '幕' => "cjk_explicit_act",
            '集' => "cjk_explicit_episode",
            _ => "cjk_explicit_pattern",
        };
        return Some((kind, number, detector));
    }
    if let Some(rest) = text.strip_prefix("番外") {
        let number = parse_number(rest.trim());
        return Some((StructureKind::Supplement, number, "cjk_supplement"));
    }

    let folded = text.to_lowercase();
    let simple = folded.trim_matches([' ', '　', '.', ':', '：', '—', '-']);
    let special = match simple {
        "序" | "序章" | "序言" | "前言" | "引子" | "楔子" | "prologue" => {
            Some((StructureKind::Prologue, "prologue_detector"))
        }
        "尾声" | "终章" | "終章" | "epilogue" => {
            Some((StructureKind::Epilogue, "epilogue_detector"))
        }
        "后记" | "後記" | "afterword" => Some((StructureKind::Afterword, "afterword_detector")),
        "introduction" | "引言" => Some((StructureKind::Introduction, "introduction_detector")),
        _ => None,
    };
    if let Some((kind, detector)) = special {
        return Some((kind, None, detector));
    }

    for (word, kind) in [
        ("chapter", StructureKind::Chapter),
        ("part", StructureKind::Volume),
        ("book", StructureKind::Volume),
    ] {
        let Some(rest) = folded.strip_prefix(word) else {
            continue;
        };
        if !rest.starts_with([' ', '\t', ':', '：', 'Ⅰ', 'Ⅱ', 'Ⅲ', 'Ⅳ', 'Ⅴ']) {
            continue;
        }
        let token = rest.trim_start_matches([' ', '\t', ':', '：']);
        let token = token
            .split([' ', '\t', ':', '：', '.', '—', '-'])
            .next()
            .unwrap_or_default();
        let number = parse_number(token);
        if let Some(number) = number {
            let detector = match word {
                "chapter" => "english_chapter_detector",
                "part" => "english_part_detector",
                _ => "english_book_detector",
            };
            return Some((kind, Some(number), detector));
        }
    }
    None
}

fn parse_number(value: &str) -> Option<u32> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(number) = value.parse::<u32>() {
        return Some(number);
    }
    if let Some(number) = parse_roman(value) {
        return Some(number);
    }
    let english = match value.to_ascii_lowercase().as_str() {
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        _ => 0,
    };
    if english != 0 {
        return Some(english);
    }
    parse_chinese_number(value)
}

/// Japanese-specific forms are kept separate from the shared CJK chapter
/// pattern. `第…章` without kana is intentionally format-ambiguous.
fn parse_japanese_heading(text: &str) -> Option<(StructureKind, Option<u32>)> {
    match text.trim() {
        "あとがき" | "まえがき" => return Some((StructureKind::Afterword, None)),
        "終章" => return Some((StructureKind::Epilogue, None)),
        "序章" => return Some((StructureKind::Prologue, None)),
        _ => {}
    }
    if !text.chars().any(is_kana) {
        return None;
    }
    let rest = text.strip_prefix('第')?;
    let marker = rest.find('章')?;
    let number = parse_number(&rest[..marker])?;
    let suffix = rest[marker + '章'.len_utf8()..].trim();
    if suffix.chars().count() > 50 || has_sentence_ending(suffix) {
        return None;
    }
    Some((StructureKind::Chapter, Some(number)))
}

fn is_kana(ch: char) -> bool {
    matches!(ch as u32, 0x3040..=0x30ff | 0x31f0..=0x31ff)
}

fn parse_roman(value: &str) -> Option<u32> {
    let mut total = 0u32;
    let mut previous = 0u32;
    for ch in value.to_ascii_uppercase().chars().rev() {
        let current = match ch {
            'I' => 1,
            'V' => 5,
            'X' => 10,
            'L' => 50,
            'C' => 100,
            'D' => 500,
            'M' => 1000,
            _ => return None,
        };
        if current < previous {
            total = total.checked_sub(current)?;
        } else {
            total = total.checked_add(current)?;
            previous = current;
        }
    }
    (total > 0 && total <= 3999).then_some(total)
}

fn parse_chinese_number(value: &str) -> Option<u32> {
    if value.chars().all(|ch| ch.is_ascii_digit()) {
        return value.parse().ok();
    }
    if value.chars().all(|ch| {
        matches!(
            ch,
            '零' | '〇'
                | '一'
                | '二'
                | '三'
                | '四'
                | '五'
                | '六'
                | '七'
                | '八'
                | '九'
                | '两'
                | '兩'
        )
    }) {
        return value.chars().try_fold(0u32, |number, ch| {
            let next = match ch {
                '零' | '〇' => 0,
                '一' => 1,
                '二' | '两' | '兩' => 2,
                '三' => 3,
                '四' => 4,
                '五' => 5,
                '六' => 6,
                '七' => 7,
                '八' => 8,
                '九' => 9,
                _ => return None,
            };
            number.checked_mul(10)?.checked_add(next)
        });
    }
    let mut total = 0u32;
    let mut section = 0u32;
    let mut digit = 0u32;
    for ch in value.chars() {
        let current = match ch {
            '零' | '〇' => 0,
            '一' => 1,
            '二' | '两' | '兩' => 2,
            '三' => 3,
            '四' => 4,
            '五' => 5,
            '六' => 6,
            '七' => 7,
            '八' => 8,
            '九' => 9,
            '十' => {
                section = section.checked_add((if digit == 0 { 1 } else { digit }) * 10)?;
                digit = 0;
                continue;
            }
            '百' => {
                section = section.checked_add((if digit == 0 { 1 } else { digit }) * 100)?;
                digit = 0;
                continue;
            }
            '千' => {
                section = section.checked_add((if digit == 0 { 1 } else { digit }) * 1000)?;
                digit = 0;
                continue;
            }
            '万' | '萬' => {
                section = section.checked_add(digit)?;
                total = total.checked_add((if section == 0 { 1 } else { section }) * 10_000)?;
                section = 0;
                digit = 0;
                continue;
            }
            _ => return None,
        };
        digit = current;
    }
    total.checked_add(section)?.checked_add(digit)
}

fn has_sentence_ending(text: &str) -> bool {
    text.trim_end()
        .chars()
        .last()
        .is_some_and(|ch| matches!(ch, '。' | '！' | '？' | '!' | '?' | '；' | ';'))
}

fn line_length_statistics(lines: &[&str]) -> (usize, usize) {
    let mut lengths = lines
        .iter()
        .map(|line| line.chars().count())
        .collect::<Vec<_>>();
    lengths.sort_unstable();
    let median = lengths[lengths.len() / 2];
    let within_hardwrap = lengths
        .iter()
        .filter(|length| (35..=100).contains(*length))
        .count();
    (median, within_hardwrap * 100 / lengths.len())
}

fn has_irregular_line_lengths(lines: &[&str]) -> bool {
    let mut lengths = lines.iter().map(|line| line.chars().count());
    let Some(first) = lengths.next() else {
        return false;
    };
    lengths.any(|length| first.abs_diff(length) > first.max(length) / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_volume_and_chapter_sequence_form_hierarchy() {
        let text =
            "第一卷\n\n第一章 初遇\n正文。\n\n第二章 再会\n正文。\n\n第二卷\n\n第一章 新篇\n正文。";
        let structure = analyze_structure(text, TextImportMode::Novel);
        assert_eq!(structure.roots.len(), 2);
        assert_eq!(structure.roots[0].candidate.kind, StructureKind::Volume);
        assert_eq!(structure.roots[0].children.len(), 2);
        assert_eq!(structure.roots[1].children[0].candidate.number, Some(1));
        assert!(structure.roots[0].children[1].candidate.confidence_percent >= 90);
    }

    #[test]
    fn supports_large_chinese_arabic_and_english_chapter_numbers() {
        let lines = "第一百二十章\n\n第12回\n\nChapter One\n\nPart II";
        let structure = analyze_structure(lines, TextImportMode::Novel);
        let candidates = structure
            .roots
            .iter()
            .map(|node| &node.candidate)
            .collect::<Vec<_>>();
        assert_eq!(candidates[0].number, Some(120));
        assert_eq!(candidates[1].number, Some(12));
        assert_eq!(candidates[2].number, Some(1));
        assert_eq!(candidates[3].kind, StructureKind::Volume);
        assert_eq!(candidates[3].number, Some(2));
    }

    #[test]
    fn out_of_sequence_headings_are_demoted_and_rejected() {
        let structure = analyze_structure(
            "第1章\n\n正文\n\n第92章\n\n正文\n\n第3章\n\n正文",
            TextImportMode::Novel,
        );
        assert_eq!(structure.roots.len(), 1);
        assert_eq!(structure.rejected.len(), 2);
        assert!(
            structure.rejected[0].confidence_percent
                < structure.roots[0].candidate.confidence_percent
        );
    }

    #[test]
    fn sentence_mentions_do_not_become_splits_and_plain_mode_is_conservative() {
        let text = "正文提到第十二章发生的事情，但没有独立标题。\n还有正文。";
        assert!(analyze_structure(text, TextImportMode::Novel)
            .roots
            .is_empty());
        assert!(analyze_structure("第一章\n\n第二章", TextImportMode::Plain)
            .roots
            .is_empty());
    }

    #[test]
    fn auto_mode_requires_repeated_or_high_confidence_structure() {
        let false_positive = "正文。\n\n第一章\n接续正文。\n\n更多内容。";
        assert!(analyze_structure(false_positive, TextImportMode::Auto)
            .roots
            .is_empty());
        let novel = "第一章\n\n正文。\n\n第二章\n\n正文。";
        assert_eq!(
            analyze_structure(novel, TextImportMode::Auto).roots.len(),
            2
        );
    }

    #[test]
    fn auto_paragraph_mode_protects_poetry_and_detects_hardwrap() {
        let poem = "  春风\n  吹绿\n  江南\n\n  秋雨\n  入夜";
        let protected = analyze_paragraphs(poem, ParagraphMode::Auto);
        assert!(protected.protected_preformatted);
        assert_eq!(protected.selected, ParagraphMode::EveryLine);

        let wrapped = (0..12)
            .map(|_| "This is a line from a long paragraph that continues without a blank line.")
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            analyze_paragraphs(&wrapped, ParagraphMode::Auto).selected,
            ParagraphMode::HardWrap
        );
    }

    #[test]
    fn chinese_number_parser_handles_units() {
        assert_eq!(parse_chinese_number("一百二十"), Some(120));
        assert_eq!(parse_chinese_number("两千零三"), Some(2003));
        assert_eq!(parse_chinese_number("一二三"), Some(123));
    }

    #[test]
    fn japanese_specific_headings_use_a_separate_detector() {
        let structure =
            analyze_structure("第一章 はじまり\n\n本文\n\nあとがき", TextImportMode::Novel);
        assert_eq!(structure.roots[0].candidate.detector, "japanese_detector");
        assert_eq!(structure.roots[0].candidate.number, Some(1));
        assert_eq!(structure.roots[1].candidate.kind, StructureKind::Afterword);
    }
}
