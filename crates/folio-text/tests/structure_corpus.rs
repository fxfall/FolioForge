use folio_text::{
    analyze_paragraphs, analyze_structure, ParagraphMode, StructureNode, TextImportMode,
};

fn candidate_count(nodes: &[StructureNode]) -> usize {
    nodes
        .iter()
        .map(|node| 1 + candidate_count(&node.children))
        .sum()
}

#[test]
fn language_fixtures_have_expected_structure_candidates() {
    let cases = [
        (include_str!("../../../tests/text/zh-cn/novel.txt"), 3),
        (include_str!("../../../tests/text/zh-tw/novel.txt"), 3),
        (include_str!("../../../tests/text/ja/novel.txt"), 3),
        (include_str!("../../../tests/text/en/novel.txt"), 4),
        (include_str!("../../../tests/text/mixed/novel.txt"), 3),
    ];
    for (source, expected) in cases {
        let structure = analyze_structure(source, TextImportMode::Novel);
        assert_eq!(candidate_count(&structure.roots), expected, "{source}");
        assert!(structure.rejected.is_empty(), "{source}");
    }
}

#[test]
fn plain_and_malformed_fixtures_prefer_false_split_avoidance() {
    let plain = include_str!("../../../tests/text/plain/no-chapters.txt");
    assert_eq!(
        candidate_count(&analyze_structure(plain, TextImportMode::Novel).roots),
        0
    );

    let malformed = include_str!("../../../tests/text/malformed/sequence.txt");
    let structure = analyze_structure(malformed, TextImportMode::Novel);
    assert_eq!(candidate_count(&structure.roots), 1);
    assert_eq!(structure.rejected.len(), 2);
    assert!(structure.rejected[0].confidence_percent < 80);
}

#[test]
fn hard_wrap_fixture_is_detected_but_plain_lines_are_not() {
    let hard_wrap = include_str!("../../../tests/text/hard-wrap/paragraph.txt");
    assert_eq!(
        analyze_paragraphs(hard_wrap, ParagraphMode::Auto).selected,
        ParagraphMode::HardWrap
    );
    let plain = include_str!("../../../tests/text/plain/no-chapters.txt");
    assert_ne!(
        analyze_paragraphs(plain, ParagraphMode::Auto).selected,
        ParagraphMode::HardWrap
    );
}
