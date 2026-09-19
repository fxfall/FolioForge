use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use folio_core::{convert, ConversionOptions, ConversionRequest, FormatRegistry, Target};
use zip::{write::SimpleFileOptions, ZipWriter};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

fn fixture_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "folioforge-format-adapters-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn convert_through_public_core(input: &Path) {
    let registry = FormatRegistry;
    let imported = registry.import_path(input).unwrap();
    assert!(!imported.book.documents.is_empty());
    assert!(imported.input_report.document_count > 0);

    for target in [Target::EPUB, Target::KF7, Target::KF8] {
        let output = input.with_extension(target.extension());
        let report = convert(&ConversionRequest {
            input: input.to_owned(),
            output: output.clone(),
            target,
            options: ConversionOptions::default(),
            edit: Default::default(),
        })
        .unwrap();
        assert!(report.output_report.validated, "{target:?}");
        assert!(output.is_file(), "{target:?} did not write an output");
        fs::remove_file(output).unwrap();
    }
}

fn minimal_docx_bytes() -> Vec<u8> {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive.start_file("[Content_Types].xml", stored).unwrap();
    archive.write_all(br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#).unwrap();
    archive.start_file("_rels/.rels", stored).unwrap();
    archive.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#).unwrap();
    archive.start_file("word/document.xml", stored).unwrap();
    archive.write_all(br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>DOCX fixture</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
    archive.finish().unwrap().into_inner()
}

fn minimal_epub_bytes() -> Vec<u8> {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
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
        .write_all(br#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Contract fixture</dc:title><dc:language>en</dc:language></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="chapter"/></spine></package>"#)
        .unwrap();
    archive.start_file("OEBPS/chapter.xhtml", stored).unwrap();
    archive
        .write_all(b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><h1>Contract</h1><p>Fixture.</p></body></html>")
        .unwrap();
    archive.finish().unwrap().into_inner()
}

#[test]
fn markdown_is_registered_imported_and_lowered_to_all_non_kfx_targets() {
    let root = fixture_root();
    let input = root.join("example.md");
    fs::write(
        &input,
        "---\ntitle: Adapter Markdown\nauthor: Alice\nlanguage: zh_cn\n---\n# Chapter One\n\nA **bold** [link](https://example.invalid) and ![](cover.png).\n\n> 引文 with RTL אבג\n\n- one\n  - nested one\n  - nested two\n- two[^1]\n\n[^1]: footnote text\n\n`code` and <mark>raw HTML</mark>.\n\n| A | B |\n|---|---|\n| 1 | 2 |\n",
    )
    .unwrap();
    fs::write(root.join("cover.png"), b"format-adapter-test-image").unwrap();
    let registry = FormatRegistry;
    assert_eq!(
        registry.detect_path(&input).unwrap(),
        folio_core::DetectedFormat::Markdown
    );
    let imported = registry.import_path(&input).unwrap();
    assert_eq!(imported.book.metadata.language.as_deref(), Some("zh-CN"));
    assert!(imported.book.feature_summary().contains_key("heading"));
    assert!(imported.book.feature_summary().contains_key("table"));
    assert!(imported.book.feature_summary().contains_key("footnote"));
    assert!(imported
        .book
        .documents
        .iter()
        .flat_map(|document| document.nodes.iter())
        .any(|node| node.text_content().contains("footnote text")));
    convert_through_public_core(&input);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn html_directory_and_htmlz_use_the_shared_xhtml_ir_path() {
    let root = fixture_root();
    let directory = root.join("html-book");
    fs::create_dir_all(directory.join("images")).unwrap();
    fs::create_dir_all(directory.join("fonts")).unwrap();
    fs::write(
        directory.join("index.html"),
        "<!doctype html><html><head><title>HTML book</title><link rel=\"stylesheet\" href=\"style.css\"><style>@font-face{font-family:book;src:url('fonts/book.woff')}</style><script>alert('ignored')</script></head><body><h1 id=\"start\">Start</h1><p>First <strong>chapter</strong>. <a href=\"chapter-2.html#second\">Next</a></p><table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table><img src=\"images/cover.png\" alt=\"\"><img src=\"images/ornament.svg\" alt=\"ornament\"><img src=\"https://example.invalid/remote.png\" alt=\"remote\"></body></html>",
    )
    .unwrap();
    fs::write(
        directory.join("chapter-2.html"),
        "<html><body><h2 id=\"second\">Second</h2><p>More text and <a href=\"#note\">[1]</a>.</p><aside id=\"note\">A note.</aside></body></html>",
    )
    .unwrap();
    fs::write(
        directory.join("chapter-1.html"),
        "<html><body><h2>First chapter</h2><p>Before second.</p></body></html>",
    )
    .unwrap();
    fs::write(directory.join("style.css"), b"h1 { color: #a00; }").unwrap();
    fs::write(
        directory.join("images/cover.png"),
        b"format-adapter-test-image",
    )
    .unwrap();
    fs::write(
        directory.join("images/ornament.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>",
    )
    .unwrap();
    fs::write(directory.join("fonts/book.woff"), b"font").unwrap();
    let registry = FormatRegistry;
    assert_eq!(
        registry.detect_path(&directory).unwrap(),
        folio_core::DetectedFormat::Html
    );
    let imported = registry.import_path(&directory).unwrap();
    assert_eq!(imported.book.documents.len(), 3);
    assert_eq!(
        imported
            .book
            .documents
            .iter()
            .map(|document| document.href.as_str())
            .collect::<Vec<_>>(),
        ["index.html", "chapter-2.html", "chapter-1.html"]
    );
    assert_eq!(imported.book.resources.len(), 4);
    assert!(imported.book.navigation.toc.len() >= 3);
    assert!(imported
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "FF-HTML-SCRIPT-0001"));
    assert!(imported
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "FF-HTML-REMOTE-0002"));
    convert_through_public_core(&directory);

    fs::write(
        directory.join("folioforge-manifest.txt"),
        "chapter-1.html\nindex.html\nchapter-2.html\n",
    )
    .unwrap();
    let manifest_source = FormatRegistry.import_path(&directory).unwrap();
    assert_eq!(
        manifest_source
            .book
            .documents
            .iter()
            .map(|document| document.href.as_str())
            .collect::<Vec<_>>(),
        ["chapter-1.html", "index.html", "chapter-2.html"]
    );

    let legacy = root.join("legacy.html");
    fs::write(
        &legacy,
        b"<html><body><h1>Caf\xe9</h1><p>Legacy Windows text.</p></body></html>",
    )
    .unwrap();
    assert_eq!(
        registry.detect_path(&legacy).unwrap(),
        folio_core::DetectedFormat::Html
    );
    let legacy_imported = registry.import_path(&legacy).unwrap();
    assert!(legacy_imported.book.documents[0].nodes[0]
        .text_content()
        .contains("Café"));
    convert_through_public_core(&legacy);

    let htmlz = root.join("example.htmlz");
    let file = fs::File::create(&htmlz).unwrap();
    let mut archive = ZipWriter::new(file);
    archive
        .start_file("index.html", SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(
            b"<html><body><h1>Packaged</h1><p><a href=\"chapter-b.html\">B</a></p></body></html>",
        )
        .unwrap();
    archive
        .start_file("chapter-a.html", SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(b"<html><body><h1>A</h1></body></html>")
        .unwrap();
    archive
        .start_file("chapter-b.html", SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(b"<html><body><h1>B</h1></body></html>")
        .unwrap();
    archive
        .start_file("manifest.txt", SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(b"chapter-a.html\nindex.html\nchapter-b.html\n")
        .unwrap();
    archive
        .start_file("style.css", SimpleFileOptions::default())
        .unwrap();
    archive.write_all(b"p { color: #333; }").unwrap();
    archive.finish().unwrap();
    assert_eq!(
        registry.detect_path(&htmlz).unwrap(),
        folio_core::DetectedFormat::Htmlz
    );
    let imported = registry.import_path(&htmlz).unwrap();
    assert_eq!(imported.format, folio_core::DetectedFormat::Htmlz);
    assert_eq!(
        imported
            .book
            .documents
            .iter()
            .map(|document| document.href.as_str())
            .collect::<Vec<_>>(),
        ["chapter-a.html", "index.html", "chapter-b.html"]
    );
    convert_through_public_core(&htmlz);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fb2_preserves_metadata_sections_poetry_and_cover_identity() {
    let root = fixture_root();
    let input = root.join("example.fb2");
    let value = r##"<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0" xmlns:l="http://www.w3.org/1999/xlink"><description><title-info><book-title>FB2 book</book-title><author><first-name>Alice</first-name><last-name>Reader</last-name></author><author><nickname>Боб</nickname></author><translator><first-name>Тина</first-name></translator><genre>literature</genre><lang>en_us</lang><date>2026</date><annotation><p>Summary.</p></annotation><coverpage><image l:href="#cover"/></coverpage></title-info></description><body><section id="one"><title><p>One</p></title><epigraph><p>Quote</p><cite><p>Source</p></cite></epigraph><p>Text <emphasis>inside</emphasis><a l:href="#n1" type="note">[1]</a>.</p><poem><stanza><v>Line one</v><v>Line two</v></stanza></poem></section></body><body name="notes"><section id="n1"><title><p>Note</p></title><p>Note text.</p></section></body><binary id="cover" content-type="image/png">aGVsbG8=</binary></FictionBook>"##;
    fs::write(&input, value).unwrap();
    let registry = FormatRegistry;
    assert_eq!(
        registry.detect_path(&input).unwrap(),
        folio_core::DetectedFormat::Fb2
    );
    let imported = registry.import_path(&input).unwrap();
    assert_eq!(imported.book.metadata.language.as_deref(), Some("en-US"));
    assert_eq!(
        imported.book.metadata.author_names(),
        ["Alice Reader", "Боб"]
    );
    assert_eq!(
        imported.book.metadata.description.as_deref(),
        Some("Summary.")
    );
    assert_eq!(imported.book.metadata.contributors, ["Тина"]);
    assert_eq!(imported.book.metadata.subjects, ["literature"]);
    assert_eq!(imported.book.metadata.date.as_deref(), Some("2026"));
    assert_eq!(imported.book.documents.len(), 2);
    assert!(imported
        .book
        .anchors
        .iter()
        .any(|anchor| anchor.name == "n1"));
    fn contains_footnote(nodes: &[folio_model::Node]) -> bool {
        nodes.iter().any(|node| {
            matches!(node.kind, folio_model::NodeKind::Footnote { .. })
                || contains_footnote(&node.children)
        })
    }
    assert!(contains_footnote(&imported.book.documents[0].nodes));
    assert!(imported.book.resources[0]
        .properties
        .iter()
        .any(|property| property == "cover-image"));
    assert!(imported
        .book
        .feature_summary()
        .contains_key("generic-block"));
    convert_through_public_core(&input);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_format_is_rejected_without_falling_back_to_a_different_adapter() {
    let root = fixture_root();
    let input = root.join("broken.fb2");
    fs::write(&input, b"<FictionBook><body>").unwrap();
    let result = FormatRegistry.import_path(&input);
    assert!(result.is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn docx_is_registered_at_the_public_core_boundary_and_converts_to_non_kfx_targets() {
    let root = fixture_root();
    let input = root.join("docx-fixture.docx");
    fs::write(&input, minimal_docx_bytes()).unwrap();
    let registry = FormatRegistry;
    assert_eq!(
        registry.detect_path(&input).unwrap(),
        folio_core::DetectedFormat::Docx
    );
    let imported = registry.import_path(&input).unwrap();
    assert_eq!(imported.format, folio_core::DetectedFormat::Docx);
    assert_eq!(imported.input_report.parser, "folio-docx/1");
    assert!(imported.book.documents[0].nodes[0]
        .text_content()
        .contains("DOCX fixture"));
    convert_through_public_core(&input);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn standalone_html_and_epub_share_xhtml_semantics() {
    let root = fixture_root();
    let markup = r##"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Shared book</title><style>h1 { color: #a00; } em { font-style: italic; }</style></head><body><h1 id="intro">Intro</h1><p>正文 <em>emphasis</em> <a href="#intro">back</a></p></body></html>"##;
    let html = root.join("shared.html");
    fs::write(&html, markup).unwrap();

    let epub = root.join("shared.epub");
    let file = fs::File::create(&epub).unwrap();
    let mut archive = ZipWriter::new(file);
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive.start_file("mimetype", stored).unwrap();
    archive.write_all(b"application/epub+zip").unwrap();
    archive
        .start_file("META-INF/container.xml", stored)
        .unwrap();
    archive.write_all(br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#).unwrap();
    archive.start_file("OEBPS/content.opf", stored).unwrap();
    archive.write_all(br#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Shared book</dc:title><dc:language>en</dc:language></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest><spine><itemref idref="chapter"/></spine></package>"#).unwrap();
    archive.start_file("OEBPS/chapter.xhtml", stored).unwrap();
    archive.write_all(markup.as_bytes()).unwrap();
    archive.start_file("OEBPS/nav.xhtml", stored).unwrap();
    archive.write_all(br#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="chapter.xhtml#intro">Intro</a></li></ol></nav></body></html>"#).unwrap();
    archive.finish().unwrap();

    let registry = FormatRegistry;
    let html_book = registry.import_path(&html).unwrap().book;
    let epub_book = registry.import_path(&epub).unwrap().book;
    let html_text = html_book
        .documents
        .iter()
        .map(|document| {
            document
                .nodes
                .iter()
                .map(folio_model::Node::text_content)
                .collect::<Vec<_>>()
                .join("")
        })
        .collect::<Vec<_>>();
    let epub_text = epub_book
        .documents
        .iter()
        .map(|document| {
            document
                .nodes
                .iter()
                .map(folio_model::Node::text_content)
                .collect::<Vec<_>>()
                .join("")
        })
        .collect::<Vec<_>>();
    assert_eq!(html_book.metadata.title, epub_book.metadata.title);
    assert_eq!(html_text, epub_text);
    assert_eq!(html_book.feature_summary(), epub_book.feature_summary());
    assert_eq!(
        html_book
            .anchors
            .iter()
            .map(|anchor| anchor.name.as_str())
            .collect::<Vec<_>>(),
        epub_book
            .anchors
            .iter()
            .map(|anchor| anchor.name.as_str())
            .collect::<Vec<_>>()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn all_registered_input_adapters_satisfy_the_v1_contract() {
    let root = fixture_root();
    let epub = root.join("contract.epub");
    fs::write(&epub, minimal_epub_bytes()).unwrap();
    let registry = FormatRegistry;
    let epub_report = convert(&ConversionRequest {
        input: epub.clone(),
        output: root.join("contract.mobi"),
        target: Target::KF7,
        options: ConversionOptions::default(),
        edit: Default::default(),
    })
    .unwrap();
    assert!(epub_report.round_trip.passed);
    let kf7 = root.join("contract.mobi");
    let kf8 = root.join("contract.azw3");
    let kfx = root.join("contract.kfx");
    for (target, output) in [(Target::KF8, kf8.clone()), (Target::KFX, kfx.clone())] {
        convert(&ConversionRequest {
            input: epub.clone(),
            output,
            target,
            options: ConversionOptions::default(),
            edit: Default::default(),
        })
        .unwrap();
    }

    let text = root.join("contract.txt");
    fs::write(&text, "Contract fixture\n\nBody text.").unwrap();
    let markdown = root.join("contract.md");
    fs::write(&markdown, "# Contract\n\nBody text.").unwrap();
    let html = root.join("contract.html");
    fs::write(
        &html,
        "<html><body><h1>Contract</h1><p>Body text.</p></body></html>",
    )
    .unwrap();
    let htmlz = root.join("contract.htmlz");
    let mut archive = ZipWriter::new(fs::File::create(&htmlz).unwrap());
    archive
        .start_file("index.html", SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(b"<html><body><h1>Contract</h1><p>Body text.</p></body></html>")
        .unwrap();
    archive.finish().unwrap();
    let fb2 = root.join("contract.fb2");
    fs::write(
        &fb2,
        r#"<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0"><description><title-info><book-title>Contract</book-title></title-info></description><body><section><p>Body text.</p></section></body></FictionBook>"#,
    )
    .unwrap();
    let docx = root.join("contract.docx");
    fs::write(&docx, minimal_docx_bytes()).unwrap();

    let inputs = [
        (&text, folio_core::DetectedFormat::Text),
        (&markdown, folio_core::DetectedFormat::Markdown),
        (&html, folio_core::DetectedFormat::Html),
        (&htmlz, folio_core::DetectedFormat::Htmlz),
        (&fb2, folio_core::DetectedFormat::Fb2),
        (&docx, folio_core::DetectedFormat::Docx),
        (&epub, folio_core::DetectedFormat::Epub),
        (&kf7, folio_core::DetectedFormat::Kf7),
        (&kf8, folio_core::DetectedFormat::Kf8),
        // The local KFX target is the explicit FolioForge compatibility
        // container; real Amazon CONT input is exercised by folio-kfx tests
        // and the reference corpus, so detection is intentionally Ffkfx here.
        (&kfx, folio_core::DetectedFormat::Ffkfx),
    ];
    for (path, expected) in inputs {
        let report = registry.inspect_path(path).unwrap();
        assert_eq!(report.format, expected.name());
        assert!(report.structure_summary.document_count > 0, "{path:?}");
        assert!(!report.features.is_empty(), "{path:?}");
    }

    let adapters = registry.input_adapters();
    let ids = adapters
        .iter()
        .map(|adapter| adapter.id())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 11);
    assert_eq!(
        ids.iter()
            .map(|id| id.name())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        ids.len()
    );
    for adapter in adapters {
        let capabilities = adapter.capabilities();
        assert!(capabilities.detect, "{}", adapter.id().name());
        assert!(capabilities.inspect, "{}", adapter.id().name());
        assert!(capabilities.import, "{}", adapter.id().name());
        assert!(!capabilities.export, "{}", adapter.id().name());
    }
    fs::remove_dir_all(root).unwrap();
}
