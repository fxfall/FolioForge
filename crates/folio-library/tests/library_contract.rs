use std::{
    fs,
    io::{Cursor, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use folio_library::{scan, BookMetadata, Library, QueryBudget, ScanOptions, SyncStatus};
use zip::{write::SimpleFileOptions, ZipWriter};

fn test_root(name: &str) -> PathBuf {
    let base = std::env::var_os("FOLIOFORGE_PHASE7_5_TEST_ROOT")
        .or_else(|| std::env::var_os("FOLIOFORGE_VALIDATION_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("library-tests");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    base.join(format!("{name}-{}-{stamp}", std::process::id()))
}

fn write_fixture(path: &PathBuf) {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive.start_file("mimetype", stored).expect("mimetype");
    archive
        .write_all(b"application/epub+zip")
        .expect("mimetype bytes");
    archive
        .start_file("META-INF/container.xml", stored)
        .expect("container");
    archive
        .write_all(br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#)
        .expect("container bytes");
    archive
        .start_file("OEBPS/content.opf", stored)
        .expect("opf");
    archive
        .write_all(br#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="book-id">urn:folioforge:library-fixture</dc:identifier><dc:title>Library fixture</dc:title><dc:creator>FolioForge Fixture</dc:creator><dc:language>en</dc:language></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="chapter"/></spine></package>"#)
        .expect("opf bytes");
    archive
        .start_file("OEBPS/chapter.xhtml", stored)
        .expect("chapter");
    archive
        .write_all(b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><h1>Library fixture</h1><p>Fixture text.</p></body></html>")
        .expect("chapter bytes");
    fs::write(path, archive.finish().expect("archive").into_inner()).expect("fixture write");
}

#[test]
fn repository_schema_relations_search_and_bounded_queries_work() {
    let root = test_root("library-contract");
    fs::create_dir_all(&root).expect("test root");
    let source = root.join("source.epub");
    write_fixture(&source);
    let database = root.join("library.sqlite3");

    let mut library = Library::open(&database).expect("open library");
    assert_eq!(library.schema_version().expect("schema"), 2);
    let storage_root = library.add_storage_root(&root).expect("root");
    let report = scan(&mut library, storage_root.id, ScanOptions::default()).expect("scan");
    assert_eq!(report.new, 1, "scan report: {report:?}");
    assert_eq!(library.stats().expect("stats").books, 1);

    let mut list_budget = QueryBudget::new(2);
    let page = library
        .list_books_page_with_budget(0, 20, &mut list_budget)
        .expect("page");
    assert_eq!(page.items.len(), 1);
    list_budget.finish().expect("list budget");

    let book_id = page.items[0].id;
    let mut detail_budget = QueryBudget::new(3);
    let detail = library
        .get_book_detail_with_budget(book_id, &mut detail_budget)
        .expect("detail")
        .expect("book detail");
    assert_eq!(detail.variants.len(), 1);
    detail_budget.finish().expect("detail budget");

    library.rebuild_search().expect("search index");
    let title = detail.book.metadata.title.clone().expect("title");
    let token = title.split_whitespace().next().expect("title token");
    assert!(!library.search(token, 10, 0).expect("search").is_empty());

    let file_id = detail.variants[0].id;
    assert_eq!(
        library.sync_status(file_id).expect("sync"),
        SyncStatus::Synced
    );

    fs::rename(&source, root.join("moved.epub")).expect("move");
    let moved = scan(&mut library, storage_root.id, ScanOptions::default()).expect("move scan");
    assert_eq!(moved.moved, 1, "move report: {moved:?}");
    assert_eq!(moved.missing, 0, "move report: {moved:?}");

    fs::copy(root.join("moved.epub"), root.join("duplicate.epub")).expect("duplicate");
    let duplicate =
        scan(&mut library, storage_root.id, ScanOptions::default()).expect("duplicate scan");
    assert_eq!(duplicate.duplicate, 1, "duplicate report: {duplicate:?}");

    fs::remove_file(root.join("moved.epub")).expect("remove moved");
    fs::remove_file(root.join("duplicate.epub")).expect("remove duplicate");
    let missing =
        scan(&mut library, storage_root.id, ScanOptions::default()).expect("missing scan");
    assert_eq!(missing.missing, 2, "missing report: {missing:?}");
    drop(library);
    let reopened = Library::open(&database).expect("reopen library");
    assert_eq!(reopened.schema_version().expect("schema"), 2);

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn metadata_write_back_is_atomic_and_reinspectable_for_epub() {
    let root = test_root("library-write-back");
    fs::create_dir_all(&root).expect("test root");
    let source = root.join("source.epub");
    write_fixture(&source);
    let mut library = Library::open(root.join("library.sqlite3")).expect("open library");
    let storage_root = library.add_storage_root(&root).expect("root");
    let report = scan(&mut library, storage_root.id, ScanOptions::default()).expect("scan");
    assert_eq!(report.new, 1, "scan report: {report:?}");
    let book = library
        .list_books_page(0, 1)
        .expect("page")
        .items
        .first()
        .expect("book")
        .id;
    let detail = library
        .get_book_detail(book)
        .expect("detail")
        .expect("book detail");
    let mut metadata = detail.book.metadata.clone();
    metadata.title = Some("Library Write Back Probe".to_owned());
    library
        .update_book_metadata(book, &BookMetadata { ..metadata })
        .expect("canonical update");
    let result = library
        .write_back_metadata(detail.variants[0].id)
        .expect("write back");
    assert!(result.changed);
    assert_eq!(result.status, SyncStatus::Synced);
    assert_eq!(
        library.sync_status(detail.variants[0].id).expect("sync"),
        SyncStatus::Synced
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn scanner_failure_is_recoverable_without_creating_a_partial_book() {
    let root = test_root("library-recovery");
    fs::create_dir_all(&root).expect("test root");
    let broken = root.join("broken.epub");
    fs::write(&broken, b"not an epub").expect("broken file");
    let mut library = Library::open(root.join("library.sqlite3")).expect("open library");
    let storage_root = library.add_storage_root(&root).expect("root");
    let failed = scan(&mut library, storage_root.id, ScanOptions::default()).expect("failed scan");
    assert_eq!(failed.failed, 1, "failure report: {failed:?}");
    assert_eq!(library.stats().expect("stats").books, 0);

    write_fixture(&broken);
    let recovered =
        scan(&mut library, storage_root.id, ScanOptions::default()).expect("recovery scan");
    assert_eq!(recovered.new, 1, "recovery report: {recovered:?}");
    assert_eq!(library.stats().expect("stats").books, 1);

    fs::remove_dir_all(root).expect("cleanup");
}
