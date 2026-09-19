use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use folio_core::{
    convert, detect, inspect_summary, validate, ConversionOptions, ConversionRequest,
    FormatRegistry, Target,
};
use folio_model::{Book, Node, NodeKind};
use zip::{write::SimpleFileOptions, ZipWriter};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Eq, PartialEq)]
struct SemanticSnapshot {
    title: Option<String>,
    language: Option<String>,
    headings: Vec<String>,
    paragraphs: Vec<String>,
    links: Vec<(String, String)>,
    resource_media_types: Vec<String>,
    feature_counts: BTreeMap<String, usize>,
    ruby: Vec<folio_model::RubyProjection>,
}

fn snapshot(book: &Book) -> SemanticSnapshot {
    let mut headings = Vec::new();
    let mut paragraphs = Vec::new();
    let mut links = Vec::new();
    let mut feature_counts = BTreeMap::new();
    for document in &book.documents {
        for node in &document.nodes {
            collect_nodes(
                node,
                &mut headings,
                &mut paragraphs,
                &mut links,
                &mut feature_counts,
            );
        }
    }
    let mut resource_media_types = book
        .resources
        .iter()
        .map(|resource| resource.media_type.clone())
        .collect::<Vec<_>>();
    resource_media_types.sort();
    SemanticSnapshot {
        title: book.metadata.title.clone(),
        language: book.metadata.language.clone(),
        headings,
        paragraphs,
        links,
        resource_media_types,
        feature_counts,
        ruby: book.ruby_projections(),
    }
}

fn collect_nodes(
    node: &Node,
    headings: &mut Vec<String>,
    paragraphs: &mut Vec<String>,
    links: &mut Vec<(String, String)>,
    feature_counts: &mut BTreeMap<String, usize>,
) {
    *feature_counts
        .entry(node.kind.name().to_owned())
        .or_default() += 1;
    match &node.kind {
        NodeKind::Heading { .. } => headings.push(node.text_content().trim().to_owned()),
        NodeKind::Paragraph => paragraphs.push(node.text_content().trim().to_owned()),
        NodeKind::Link { href } | NodeKind::Footnote { href: Some(href) } => {
            links.push((href.clone(), node.text_content().trim().to_owned()))
        }
        _ => {}
    }
    for child in &node.children {
        collect_nodes(child, headings, paragraphs, links, feature_counts);
    }
}

fn fixture_root() -> PathBuf {
    let root = std::env::var_os("FOLIOFORGE_VALIDATION_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("semantic-equivalence")
        .join(format!(
            "folioforge-semantic-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_epub(path: &Path) {
    let file = fs::File::create(path).unwrap();
    let mut archive = ZipWriter::new(file);
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive.start_file("mimetype", stored).unwrap();
    archive.write_all(b"application/epub+zip").unwrap();
    archive
        .start_file("META-INF/container.xml", stored)
        .unwrap();
    archive
        .write_all(br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#)
        .unwrap();
    archive.start_file("OEBPS/content.opf", stored).unwrap();
    archive
        .write_all(br#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Canonical Book</dc:title><dc:creator>FolioForge Test</dc:creator><dc:language>en</dc:language></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/><item id="cover" href="cover.png" media-type="image/png" properties="cover-image"/></manifest><spine><itemref idref="chapter"/></spine></package>"#)
        .unwrap();
    archive.start_file("OEBPS/chapter.xhtml", stored).unwrap();
    archive
        .write_all(b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><h1 id=\"chapter-one\">Chapter One</h1><p>Shared paragraph.</p><ul><li>Item one</li><li>Item two</li></ul><p><a href=\"#target\">Internal target</a>.</p><p><img src=\"cover.png\" alt=\"Shared image\"/></p><h2 id=\"target\">Target</h2><p>Target paragraph.</p></body></html>")
        .unwrap();
    archive.start_file("OEBPS/cover.png", stored).unwrap();
    archive.write_all(b"image fixture").unwrap();
    archive.finish().unwrap();
}

fn write_rich_epub(path: &Path) {
    let file = fs::File::create(path).unwrap();
    let mut archive = ZipWriter::new(file);
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive.start_file("mimetype", stored).unwrap();
    archive.write_all(b"application/epub+zip").unwrap();
    archive
        .start_file("META-INF/container.xml", stored)
        .unwrap();
    archive
        .write_all(br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#)
        .unwrap();
    archive.start_file("OEBPS/content.opf", stored).unwrap();
    archive
        .write_all(br#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Rich Book</dc:title><dc:creator>FolioForge Test</dc:creator><dc:language>ja</dc:language></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/><item id="notes" href="notes.xhtml" media-type="application/xhtml+xml"/><item id="cover" href="cover.png" media-type="image/png" properties="cover-image"/></manifest><spine><itemref idref="chapter"/><itemref idref="notes"/></spine></package>"#)
        .unwrap();
    archive.start_file("OEBPS/chapter.xhtml", stored).unwrap();
    archive
        .write_all(r##"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><h1 id="rich-chapter">Rich Chapter</h1><p id="note-ref">Ruby <ruby><rb>漢</rb><rt>かん</rt></ruby> and <a href="notes.xhtml#note-1" epub:type="footnote">[1]</a>.</p><figure><img src="cover.png" alt="Figure alt"/><figcaption>Caption</figcaption></figure><p>Math <math xmlns="http://www.w3.org/1998/Math/MathML" display="block"><mi>x</mi><mo>=</mo><mn>1</mn></math></p><table><tr><td>Cell</td></tr></table><p><a href="#rich-chapter">Back to chapter</a></p></body></html>"##.as_bytes())
        .unwrap();
    archive.start_file("OEBPS/notes.xhtml", stored).unwrap();
    archive
        .write_all(r##"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><aside id="note-1" epub:type="footnote"><p>Note text</p><a href="chapter.xhtml#note-ref" epub:type="backlink">↩</a></aside></body></html>"##.as_bytes())
        .unwrap();
    archive.start_file("OEBPS/cover.png", stored).unwrap();
    archive.write_all(b"rich image").unwrap();
    archive.finish().unwrap();
}

fn write_docx(path: &Path) {
    let mut archive = ZipWriter::new(fs::File::create(path).unwrap());
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive.start_file("[Content_Types].xml", stored).unwrap();
    archive
        .write_all(br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/></Types>"#)
        .unwrap();
    archive.start_file("_rels/.rels", stored).unwrap();
    archive
        .write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#)
        .unwrap();
    archive.start_file("docProps/core.xml", stored).unwrap();
    archive
        .write_all(br#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Canonical Book</dc:title><dc:creator>FolioForge Test</dc:creator><dc:language>en</dc:language></cp:coreProperties>"#)
        .unwrap();
    archive.start_file("word/document.xml", stored).unwrap();
    archive
        .write_all(br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Chapter One</w:t></w:r></w:p><w:p><w:r><w:t>Shared paragraph.</w:t></w:r></w:p><w:p><w:r><w:t>Target paragraph.</w:t></w:r></w:p></w:body></w:document>"#)
        .unwrap();
    archive.finish().unwrap();
}

fn write_sources(root: &Path) -> BTreeMap<&'static str, PathBuf> {
    let source_root = root.join("sources");
    fs::create_dir_all(&source_root).unwrap();
    let basic_root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/semantic-equivalence/basic");
    let mut paths = BTreeMap::new();
    let markdown = source_root.join("basic.md");
    fs::copy(basic_root.join("basic.md"), &markdown).unwrap();
    fs::write(source_root.join("cover.png"), b"image fixture").unwrap();
    paths.insert("markdown", markdown);
    let html = source_root.join("basic.html");
    fs::copy(basic_root.join("basic.html"), &html).unwrap();
    paths.insert("html", html);
    let text = source_root.join("Canonical Book.txt");
    fs::copy(basic_root.join("basic.txt"), &text).unwrap();
    paths.insert("text", text);
    let fb2 = source_root.join("basic.fb2");
    fs::copy(basic_root.join("basic.fb2"), &fb2).unwrap();
    paths.insert("fb2", fb2);
    let docx = source_root.join("basic.docx");
    write_docx(&docx);
    paths.insert("docx", docx);
    let epub = source_root.join("basic.epub");
    write_epub(&epub);
    paths.insert("epub", epub);
    paths
}

#[test]
fn canonical_basic_sources_share_public_semantics_and_round_trip_to_all_targets() {
    let root = fixture_root();
    let sources = write_sources(&root);
    let registry = FormatRegistry;
    let mut canonical = Vec::new();
    for (name, path) in &sources {
        let detected = detect(path).unwrap();
        let summary = inspect_summary(path).unwrap();
        assert_eq!(detected.name(), summary.format);
        assert_eq!(summary.metadata.title.as_deref(), Some("Canonical Book"));
        assert!(summary.structure_summary.document_count > 0);
        let imported = registry.import_path(path).unwrap();
        let view = snapshot(&imported.book);
        assert!(
            view.title.as_deref() == Some("Canonical Book"),
            "source {name}"
        );
        assert!(
            view.paragraphs
                .iter()
                .any(|paragraph| paragraph.contains("Shared paragraph.")),
            "source {name}"
        );
        assert!(
            view.headings.iter().any(|heading| heading == "Chapter One"),
            "source {name}"
        );
        canonical.push((name, view));

        for target in Target::all() {
            let output = root.join(format!(
                "{name}-{}.{}",
                target.format().name(),
                target.extension()
            ));
            let report = convert(&ConversionRequest {
                input: path.clone(),
                output: output.clone(),
                target,
                options: ConversionOptions::default(),
                edit: Default::default(),
            });
            let report = report.unwrap_or_else(|error| panic!("{error}"));
            assert!(
                report.round_trip.passed,
                "{name} -> {target:?}: {:?}",
                report.round_trip
            );
            assert!(
                validate(&output).unwrap().is_valid(),
                "{name} -> {target:?}"
            );
            let round_trip = registry.import_path(&output).unwrap();
            let view = snapshot(&round_trip.book);
            assert!(
                view.paragraphs
                    .iter()
                    .any(|paragraph| paragraph.contains("Shared paragraph.")),
                "round-trip {name} -> {target:?}"
            );
            assert!(view.headings.iter().any(|heading| heading == "Chapter One"));
        }
    }
    let first = &canonical[0].1;
    for (name, view) in canonical.iter().skip(1) {
        assert_eq!(view.title, first.title, "canonical title {name}");
        assert!(
            view.paragraphs
                .iter()
                .any(|paragraph| paragraph.contains("Shared paragraph.")),
            "canonical shared text {name}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rich_probe_preserves_footnote_ruby_math_figure_and_table_semantics() {
    let root = fixture_root();
    let input = root.join("rich.epub");
    write_rich_epub(&input);
    let registry = FormatRegistry;
    let imported = registry.import_path(&input).unwrap();
    let source = snapshot(&imported.book);
    assert_eq!(source.title.as_deref(), Some("Rich Book"));
    assert_eq!(
        source.ruby,
        [folio_model::RubyProjection {
            base: "漢".to_owned(),
            annotation: "かん".to_owned(),
        }]
    );
    for feature in ["footnote", "ruby", "math", "image", "table"] {
        assert!(
            source
                .feature_counts
                .get(feature)
                .copied()
                .unwrap_or_default()
                > 0,
            "missing rich feature {feature}: {:?}",
            source.feature_counts
        );
    }
    assert!(
        source
            .links
            .iter()
            .any(|(href, _)| href.ends_with("notes.xhtml#note-1")),
        "links: {:?}",
        source.links
    );
    assert!(
        source
            .links
            .iter()
            .any(|(href, _)| href.ends_with("chapter.xhtml#rich-chapter")),
        "links: {:?}",
        source.links
    );

    for target in Target::all() {
        let output = root.join(format!(
            "rich-{}.{}",
            target.format().name(),
            target.extension()
        ));
        let report = convert(&ConversionRequest {
            input: input.clone(),
            output: output.clone(),
            target,
            options: ConversionOptions::default(),
            edit: Default::default(),
        })
        .unwrap_or_else(|error| panic!("rich -> {target:?}: {error}"));
        assert!(
            report.round_trip.passed,
            "rich -> {target:?}: {:?}",
            report.round_trip
        );
        assert!(validate(&output).unwrap().is_valid(), "rich -> {target:?}");
        let round_trip = snapshot(&registry.import_path(&output).unwrap().book);
        if matches!(target, Target::KF7 | Target::KF7KF8Combo) {
            assert!(round_trip.ruby.is_empty(), "rich Ruby -> {target:?}");
        } else {
            assert_eq!(round_trip.ruby, source.ruby, "rich Ruby -> {target:?}");
        }
        assert!(
            round_trip
                .paragraphs
                .iter()
                .any(|paragraph| paragraph.contains("Note text")),
            "rich note -> {target:?}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}
