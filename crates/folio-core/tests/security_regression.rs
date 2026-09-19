use std::{
    fs,
    io::Write,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use folio_core::FormatRegistry;
use zip::{write::SimpleFileOptions, ZipWriter};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

fn fixture_root() -> PathBuf {
    let root = std::env::var_os("FOLIOFORGE_VALIDATION_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("security-regression")
        .join(format!(
            "{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = fs::File::create(path).unwrap();
    let mut archive = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        archive.start_file(*name, options).unwrap();
        archive.write_all(bytes).unwrap();
    }
    archive.finish().unwrap();
}

fn no_panic_import(registry: FormatRegistry, path: &Path) -> bool {
    catch_unwind(AssertUnwindSafe(|| registry.import_path(path))).is_ok()
}

#[test]
fn package_traversal_and_macro_inputs_fail_closed() {
    let root = fixture_root();
    let registry = FormatRegistry;
    let epub = root.join("traversal.epub");
    write_zip(
        &epub,
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#),
            ("OEBPS/content.opf", br#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Unsafe</dc:title></metadata><manifest/><spine/></package>"#),
            ("../escape.txt", b"must not be read"),
        ],
    );
    let epub_result = catch_unwind(AssertUnwindSafe(|| registry.import_path(&epub)));
    assert!(epub_result.is_ok());
    assert!(epub_result.unwrap().is_err());

    let htmlz = root.join("traversal.htmlz");
    write_zip(
        &htmlz,
        &[
            ("index.html", b"<html><body><p>Safe entry</p></body></html>"),
            ("../escape.png", b"must not be read"),
        ],
    );
    let htmlz_result = catch_unwind(AssertUnwindSafe(|| registry.import_path(&htmlz)));
    assert!(htmlz_result.is_ok());
    assert!(htmlz_result.unwrap().is_err());

    let macro_docx = root.join("macro.docx");
    write_zip(
        &macro_docx,
        &[
            (
                "[Content_Types].xml",
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.ms-word.document.macroEnabled.main+xml"/></Types>"#,
            ),
            ("word/document.xml", br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#),
        ],
    );
    let macro_result = catch_unwind(AssertUnwindSafe(|| registry.import_path(&macro_docx)));
    assert!(macro_result.is_ok());
    assert!(macro_result.unwrap().is_err());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_and_mutated_input_corpus_never_panics() {
    let root = fixture_root();
    let registry = FormatRegistry;
    let seeds = [
        ("broken.epub", b"PK\x03\x04not-an-epub".as_slice()),
        ("broken.fb2", b"<FictionBook><body>".as_slice()),
        ("broken.html", b"<html><body><script>".as_slice()),
        ("broken.docx", b"not-a-docx".as_slice()),
    ];
    for (name, seed) in seeds {
        let path = root.join(name);
        fs::write(&path, seed).unwrap();
        assert!(no_panic_import(registry, &path), "seed panicked: {name}");
        for mutation in 0..32usize {
            let mut bytes = seed.to_vec();
            if bytes.is_empty() {
                bytes.push(mutation as u8);
            } else {
                let index = mutation % bytes.len();
                bytes[index] ^= (mutation as u8).wrapping_mul(17).wrapping_add(1);
            }
            if mutation % 3 == 0 {
                bytes.push(mutation as u8);
            }
            let path = root.join(format!("mutated-{mutation}-{name}"));
            fs::write(&path, bytes).unwrap();
            assert!(
                no_panic_import(registry, &path),
                "mutation panicked: {name} #{mutation}"
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
}
