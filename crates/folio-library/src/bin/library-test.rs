use std::{env, fs, path::PathBuf, time::Instant};

use folio_library::{Library, QueryBudget};

fn main() {
    if let Err(error) = run() {
        eprintln!("library-test: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);
    let command = arguments.next().unwrap_or_default();
    if command != "benchmark" {
        return Err("usage: library-test benchmark <database-path> <book-count>"
            .to_owned()
            .into());
    }
    let database = PathBuf::from(arguments.next().ok_or("missing database path")?);
    let count: u64 = arguments.next().ok_or("missing book count")?.parse()?;
    if arguments.next().is_some() {
        return Err("too many arguments".to_owned().into());
    }
    if let Some(parent) = database.parent() {
        fs::create_dir_all(parent)?;
    }
    if database.exists() {
        fs::remove_file(&database)?;
    }

    let open_started = Instant::now();
    let mut library = Library::open(&database)?;
    let open_ms = open_started.elapsed().as_millis();

    let insert_started = Instant::now();
    let storage_root = database.with_extension("synthetic-root");
    let inserted = library.insert_synthetic_books_with_variants(count, &storage_root)?;
    let insert_ms = insert_started.elapsed().as_millis();

    let index_started = Instant::now();
    library.rebuild_search()?;
    let index_ms = index_started.elapsed().as_millis();

    let list_started = Instant::now();
    let mut list_budget = QueryBudget::new(2);
    let page = library.list_books_page_with_budget(0, 100, &mut list_budget)?;
    list_budget.finish()?;
    let list_ms = list_started.elapsed().as_millis();

    let search_started = Instant::now();
    let search_count = library.search("Synthetic", 100, 0)?.len();
    let search_author_count = library.search("Author 1", 100, 0)?.len();
    let search_series_count = library.search("Series 1", 100, 0)?.len();
    let search_tag_count = library.search("tag-1", 100, 0)?.len();
    let search_ms = search_started.elapsed().as_millis();

    let identifier_started = Instant::now();
    let identifier_count = library
        .find_books_by_identifier("synthetic", "book-0000000", 10)?
        .len();
    let identifier_ms = identifier_started.elapsed().as_millis();

    let hash = blake3::hash(b"books/0000000.epub").to_hex().to_string();
    let hash_started = Instant::now();
    let hash_found = library.find_file_variant_by_hash(&hash)?.is_some();
    let hash_ms = hash_started.elapsed().as_millis();

    let detail_started = Instant::now();
    let (detail, sync_status, sync_status_ms) = if let Some(item) = page.items.first() {
        let mut budget = QueryBudget::new(3);
        let detail = library.get_book_detail_with_budget(item.id, &mut budget)?;
        budget.finish()?;
        let sync_started = Instant::now();
        let sync_status = detail
            .as_ref()
            .and_then(|detail| detail.variants.first())
            .map(|variant| library.sync_status(variant.id))
            .transpose()?;
        (
            detail.is_some(),
            sync_status.map(|status| format!("{status:?}")),
            sync_started.elapsed().as_millis(),
        )
    } else {
        (false, None, 0)
    };
    let detail_ms = detail_started.elapsed().as_millis();
    let stats = library.stats()?;
    let database_bytes = fs::metadata(&database)?.len();
    println!(
        "{}",
        serde_json::json!({
            "count_requested": count,
            "count_inserted": inserted,
            "database_bytes": database_bytes,
            "schema_version": library.schema_version()?,
            "open_ms": open_ms,
            "insert_ms": insert_ms,
            "rebuild_search_ms": index_ms,
            "list_page_ms": list_ms,
            "search_ms": search_ms,
            "search_results": search_count,
            "search_author_results": search_author_count,
            "search_series_results": search_series_count,
            "search_tag_results": search_tag_count,
            "identifier_ms": identifier_ms,
            "identifier_results": identifier_count,
            "hash_ms": hash_ms,
            "hash_found": hash_found,
            "detail_ms": detail_ms,
            "detail_found": detail,
            "sync_status_ms": sync_status_ms,
            "sync_status": sync_status,
            "stats": stats,
        })
    );
    Ok(())
}
