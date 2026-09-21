use folio_text::{
    analyze_paragraphs, analyze_structure, ParagraphMode, StructureNode, TextImportMode,
};

fn candidate_count(nodes: &[StructureNode]) -> usize {
    nodes
        .iter()
        .map(|node| 1 + candidate_count(&node.children))
        .sum()
}

const ZH_CN: &str = "第一卷\n\n第一章 初遇\n\n这是一个用于回归测试的合成段落。\n第二行属于同一段。\n\n第二章 再会\n\n这里有另一段简体中文测试文本。\n";
const ZH_TW: &str = "第一部\n\n第一回 初見\n\n這是一段合成繁體中文測試文字。\n\n第二回 再會\n\n這是第二段回歸資料。\n";
const JA: &str = "第一章 はじまり\n\nこれは合成された日本語のテスト本文です。\n\n第2章 つづき\n\n二つ目の段落です。\n\nあとがき\n";
const EN: &str = "Prologue\n\nSynthetic opening text for the parser regression corpus.\n\nChapter One: A Beginning\n\nThis is a non-copyrighted test paragraph.\n\nChapter 2: A Continuation\n\nThe second synthetic chapter follows.\n\nEpilogue\n";
const MIXED: &str = "序章\n\nThis synthetic mixed-language book begins here.\n\n第一章 约定\n\n中文段落 follows the opening.\n\nChapter 2: Return\n\nThe final synthetic section.\n";
const PLAIN: &str = "This short synthetic essay contains no chapter headings.\n\nIt mentions 第十二章 in the middle of a sentence, but that phrase is not a\nstandalone heading and must not create a false chapter split.\n";
const MALFORMED: &str = "第1章\n\nSynthetic first section.\n\n第92章\n\nThis unlikely number should be demoted by sequence evidence.\n\n第3章\n\nThe non-monotonic sequence should remain visible as rejected candidates.\n";
const HARD_WRAP: &str = "This synthetic paragraph is deliberately wrapped at a fairly regular width.\nThe following source line continues the same sentence without a blank line.\nAnother regular line helps the analyzer recognize old fixed-width wrapping.\nThe final source line closes this non-copyrighted regression paragraph.\n";

#[test]
fn language_fixtures_have_expected_structure_candidates() {
    let cases = [(ZH_CN, 3), (ZH_TW, 3), (JA, 3), (EN, 4), (MIXED, 3)];
    for (source, expected) in cases {
        let structure = analyze_structure(source, TextImportMode::Novel);
        assert_eq!(candidate_count(&structure.roots), expected, "{source}");
        assert!(structure.rejected.is_empty(), "{source}");
    }
}

#[test]
fn plain_and_malformed_fixtures_prefer_false_split_avoidance() {
    let plain = PLAIN;
    assert_eq!(
        candidate_count(&analyze_structure(plain, TextImportMode::Novel).roots),
        0
    );

    let malformed = MALFORMED;
    let structure = analyze_structure(malformed, TextImportMode::Novel);
    assert_eq!(candidate_count(&structure.roots), 1);
    assert_eq!(structure.rejected.len(), 2);
    assert!(structure.rejected[0].confidence_percent < 80);
}

#[test]
fn hard_wrap_fixture_is_detected_but_plain_lines_are_not() {
    let hard_wrap = HARD_WRAP;
    assert_eq!(
        analyze_paragraphs(hard_wrap, ParagraphMode::Auto).selected,
        ParagraphMode::HardWrap
    );
    let plain = PLAIN;
    assert_ne!(
        analyze_paragraphs(plain, ParagraphMode::Auto).selected,
        ParagraphMode::HardWrap
    );
}
