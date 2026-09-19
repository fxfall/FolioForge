use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::error::LibraryError;

pub const CURRENT_SCHEMA_VERSION: i64 = 2;

const MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "initial_library_schema", INITIAL_SCHEMA),
    (2, "rebuildable_library_search", SEARCH_SCHEMA),
];

const INITIAL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS library_meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS storage_roots (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    path TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS books (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    title TEXT,
    sort_title TEXT NOT NULL,
    subtitle TEXT,
    language TEXT,
    publisher TEXT,
    published_date TEXT,
    description TEXT,
    series_index REAL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS people (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS book_people (
    book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    person_id INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'author',
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(book_id, person_id, role)
);

CREATE TABLE IF NOT EXISTS series (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS book_series (
    book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
    series_index REAL,
    PRIMARY KEY(book_id, series_id)
);

CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS book_tags (
    book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY(book_id, tag_id)
);

CREATE TABLE IF NOT EXISTS book_identifiers (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    scheme TEXT NOT NULL,
    value TEXT NOT NULL,
    UNIQUE(book_id, scheme, value)
);

CREATE TABLE IF NOT EXISTS file_variants (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    storage_root_id INTEGER NOT NULL REFERENCES storage_roots(id) ON DELETE RESTRICT,
    relative_path TEXT NOT NULL,
    format_id TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL,
    content_hash TEXT,
    origin TEXT NOT NULL,
    source_file_id INTEGER REFERENCES file_variants(id) ON DELETE SET NULL,
    state TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    UNIQUE(storage_root_id, relative_path)
);

CREATE TABLE IF NOT EXISTS file_metadata (
    file_id INTEGER PRIMARY KEY REFERENCES file_variants(id) ON DELETE CASCADE,
    title TEXT,
    subtitle TEXT,
    language TEXT,
    publisher TEXT,
    published_date TEXT,
    metadata_fingerprint TEXT NOT NULL,
    inspected_at INTEGER NOT NULL,
    inspection_contract_version TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS conversions (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    source_file_id INTEGER REFERENCES file_variants(id) ON DELETE SET NULL,
    output_file_id INTEGER REFERENCES file_variants(id) ON DELETE SET NULL,
    target_format_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    error TEXT
);

CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_books_sort_title ON books(sort_title, id);
CREATE INDEX IF NOT EXISTS idx_file_variants_book ON file_variants(book_id, id);
CREATE INDEX IF NOT EXISTS idx_file_variants_hash ON file_variants(content_hash);
CREATE INDEX IF NOT EXISTS idx_file_variants_root_path ON file_variants(storage_root_id, relative_path);
CREATE INDEX IF NOT EXISTS idx_book_people_book ON book_people(book_id, position);
CREATE INDEX IF NOT EXISTS idx_book_people_person ON book_people(person_id, book_id);
CREATE INDEX IF NOT EXISTS idx_book_tags_book ON book_tags(book_id, tag_id);
CREATE INDEX IF NOT EXISTS idx_book_tags_tag ON book_tags(tag_id, book_id);
CREATE INDEX IF NOT EXISTS idx_book_series_book ON book_series(book_id, series_id);
CREATE INDEX IF NOT EXISTS idx_book_series_series ON book_series(series_id, book_id);
CREATE INDEX IF NOT EXISTS idx_book_identifiers_value ON book_identifiers(scheme, value, book_id);
"#;

const SEARCH_SCHEMA: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS library_search USING fts5(
    book_id UNINDEXED,
    title,
    authors,
    series,
    tags,
    description,
    identifiers
);
"#;

pub fn apply(connection: &mut Connection) -> Result<(), LibraryError> {
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (\
            version INTEGER PRIMARY KEY,\
            name TEXT NOT NULL,\
            applied_at INTEGER NOT NULL\
        );",
    )?;
    let transaction = connection.transaction()?;
    for (version, name, sql) in MIGRATIONS {
        let applied = transaction.query_row(
            "SELECT 1 FROM schema_migrations WHERE version = ?1",
            [version],
            |row| row.get::<_, i64>(0),
        );
        if applied.is_ok() {
            continue;
        }
        transaction
            .execute_batch(sql)
            .map_err(|source| LibraryError::Migration {
                version: *version,
                name: (*name).to_owned(),
                source,
            })?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![version, name, unix_seconds()],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or_default()
}
