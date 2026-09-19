use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use folio_core::{self, ConversionOptions, ConversionRequest, Target};
use rusqlite::{params, types::Type, Connection, OptionalExtension, Transaction};
use uuid::Uuid;

use crate::{
    db::LibraryDb,
    error::LibraryError,
    metadata::{file_metadata_from_inspection, fingerprint, metadata_diff},
    model::{
        BookDetail, BookId, BookListItem, BookMetadata, BookRecord, FileMetadata, FileVariant,
        FileVariantDraft, FileVariantId, FileVariantOrigin, FileVariantState, Identifier,
        LibraryStats, StorageRoot, StorageRootId, SyncStatus,
    },
};

/// A statement budget used by contract tests and callers that need to keep
/// paginated/detail queries bounded. It counts logical SQL statements, not
/// rows returned by a statement.
#[derive(Clone, Debug)]
pub struct QueryBudget {
    limit: usize,
    used: usize,
}

impl QueryBudget {
    pub const fn new(limit: usize) -> Self {
        Self { limit, used: 0 }
    }

    fn unlimited() -> Self {
        Self::new(usize::MAX)
    }

    fn record(&mut self) -> Result<(), LibraryError> {
        self.used = self.used.saturating_add(1);
        if self.used > self.limit {
            return Err(LibraryError::QueryBudgetExceeded {
                used: self.used,
                budget: self.limit,
            });
        }
        Ok(())
    }

    pub const fn used(&self) -> usize {
        self.used
    }

    pub const fn limit(&self) -> usize {
        self.limit
    }

    pub fn finish(&self) -> Result<(), LibraryError> {
        if self.used <= self.limit {
            Ok(())
        } else {
            Err(LibraryError::QueryBudgetNotSatisfied {
                used: self.used,
                budget: self.limit,
            })
        }
    }
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct BookPage {
    pub items: Vec<BookListItem>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct SearchResult {
    pub item: BookListItem,
    pub rank: f64,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct MetadataSyncResult {
    pub file_id: FileVariantId,
    pub changed: bool,
    pub status: SyncStatus,
    pub changed_fields: Vec<String>,
}

/// The durable Library facade. All SQL stays here; callers see domain types
/// and the Core contract rather than SQLite rows or format adapters.
pub struct Library {
    db: LibraryDb,
}

impl Library {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LibraryError> {
        Ok(Self {
            db: LibraryDb::open(path)?,
        })
    }

    pub fn in_memory() -> Result<Self, LibraryError> {
        Ok(Self {
            db: LibraryDb::in_memory()?,
        })
    }

    pub fn db(&self) -> &LibraryDb {
        &self.db
    }

    pub fn schema_version(&self) -> Result<i64, LibraryError> {
        self.db.schema_version()
    }

    pub fn add_storage_root(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<StorageRoot, LibraryError> {
        let path = fs::canonicalize(path)?;
        if !path.is_dir() {
            return Err(LibraryError::InvalidInput(format!(
                "storage root is not a directory: {}",
                path.display()
            )));
        }
        let path_string = path.to_string_lossy().into_owned();
        let now = unix_seconds();
        let uuid = Uuid::new_v4();
        let connection = self.db.connection_mut();
        connection.execute(
            "INSERT INTO storage_roots(uuid, path, created_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(path) DO UPDATE SET path = excluded.path",
            params![uuid.to_string(), path_string, now],
        )?;
        let row = connection.query_row(
            "SELECT id, uuid, path, created_at FROM storage_roots WHERE path = ?1",
            [path_string],
            storage_root_from_row,
        )?;
        Ok(row)
    }

    pub fn create_book(&mut self, metadata: &BookMetadata) -> Result<BookRecord, LibraryError> {
        let now = unix_seconds();
        let uuid = Uuid::new_v4();
        let connection = self.db.connection_mut();
        let transaction = connection.transaction()?;
        let book_id = insert_book_row(&transaction, uuid, metadata, now)?;
        replace_book_relations(&transaction, book_id, metadata)?;
        transaction.commit()?;
        self.get_book(book_id)?.ok_or_else(|| {
            LibraryError::NotFound(format!("book {book_id} was inserted but could not be read"))
        })
    }

    pub fn update_book_metadata(
        &mut self,
        book_id: BookId,
        metadata: &BookMetadata,
    ) -> Result<BookRecord, LibraryError> {
        let now = unix_seconds();
        let connection = self.db.connection_mut();
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE books SET title = ?1, sort_title = ?2, subtitle = ?3, language = ?4,\
             publisher = ?5, published_date = ?6, description = ?7, series_index = ?8,\
             updated_at = ?9 WHERE id = ?10",
            params![
                metadata.title,
                metadata.sort_key(),
                metadata.subtitle,
                metadata.language,
                metadata.publisher,
                metadata.published_date,
                metadata.description,
                metadata.series_index,
                now,
                book_id,
            ],
        )?;
        if changed == 0 {
            return Err(LibraryError::NotFound(format!("book {book_id}")));
        }
        replace_book_relations(&transaction, book_id, metadata)?;
        transaction.commit()?;
        self.get_book(book_id)?.ok_or_else(|| {
            LibraryError::NotFound(format!("book {book_id} disappeared after update"))
        })
    }

    pub fn get_book(&self, book_id: BookId) -> Result<Option<BookRecord>, LibraryError> {
        let mut budget = QueryBudget::unlimited();
        self.get_book_with_budget(book_id, &mut budget)
    }

    pub fn get_book_with_budget(
        &self,
        book_id: BookId,
        budget: &mut QueryBudget,
    ) -> Result<Option<BookRecord>, LibraryError> {
        let connection = self.db.connection();
        let Some(mut book) = load_book_base(connection, book_id, budget)? else {
            return Ok(None);
        };
        load_book_relations(connection, &mut book, budget)?;
        Ok(Some(book))
    }

    pub fn list_books_page(&self, offset: u64, limit: u64) -> Result<BookPage, LibraryError> {
        let mut budget = QueryBudget::unlimited();
        self.list_books_page_with_budget(offset, limit, &mut budget)
    }

    pub fn list_books_page_with_budget(
        &self,
        offset: u64,
        limit: u64,
        budget: &mut QueryBudget,
    ) -> Result<BookPage, LibraryError> {
        let limit = limit.min(1_000);
        budget.record()?;
        let total: u64 =
            self.db
                .connection()
                .query_row("SELECT COUNT(*) FROM books", [], |row| {
                    row.get::<_, i64>(0).map(|value| value as u64)
                })?;
        budget.record()?;
        let mut statement = self.db.connection().prepare(
            "SELECT id, uuid, title, sort_title, language, updated_at \
             FROM books ORDER BY sort_title COLLATE NOCASE, id LIMIT ?1 OFFSET ?2",
        )?;
        let rows =
            statement.query_map(params![u64_to_i64(limit)?, u64_to_i64(offset)?], |row| {
                Ok(BookListItem {
                    id: row.get(0)?,
                    uuid: parse_uuid(row.get::<_, String>(1)?, 1)?,
                    title: row.get(2)?,
                    sort_title: row.get(3)?,
                    language: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })?;
        let items = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(BookPage {
            items,
            total,
            offset,
            limit,
        })
    }

    pub fn get_book_detail(&self, book_id: BookId) -> Result<Option<BookDetail>, LibraryError> {
        let mut budget = QueryBudget::unlimited();
        self.get_book_detail_with_budget(book_id, &mut budget)
    }

    pub fn get_book_detail_with_budget(
        &self,
        book_id: BookId,
        budget: &mut QueryBudget,
    ) -> Result<Option<BookDetail>, LibraryError> {
        let connection = self.db.connection();
        let Some(mut book) = load_book_base(connection, book_id, budget)? else {
            return Ok(None);
        };
        load_book_relations(connection, &mut book, budget)?;
        budget.record()?;
        let mut variants_statement = connection.prepare(
            "SELECT fv.id, fv.uuid, fv.book_id, fv.storage_root_id, fv.relative_path,\
             fv.format_id, fv.size_bytes, fv.mtime_ns, fv.content_hash, fv.origin,\
             fv.source_file_id, fv.state, fv.added_at, fv.last_seen_at,\
             fm.title, fm.subtitle, fm.language, fm.publisher, fm.published_date,\
             fm.metadata_fingerprint, fm.inspected_at, fm.inspection_contract_version \
             FROM file_variants fv LEFT JOIN file_metadata fm ON fm.file_id = fv.id \
             WHERE fv.book_id = ?1 ORDER BY fv.id",
        )?;
        let mut variants = Vec::new();
        let mut file_metadata = Vec::new();
        let rows = variants_statement.query_map([book_id], |row| {
            let variant = file_variant_from_row(row, 0)?;
            let metadata = if let Some(fingerprint) = row.get::<_, Option<String>>(19)? {
                Some(FileMetadata {
                    title: row.get(14)?,
                    subtitle: row.get(15)?,
                    language: row.get(16)?,
                    publisher: row.get(17)?,
                    published_date: row.get(18)?,
                    metadata_fingerprint: fingerprint,
                    inspected_at: row.get(20)?,
                    inspection_contract_version: row.get(21)?,
                })
            } else {
                None
            };
            Ok((variant, metadata))
        })?;
        for row in rows {
            let (variant, metadata) = row?;
            if let Some(metadata) = metadata {
                file_metadata.push((variant.id, metadata));
            }
            variants.push(variant);
        }
        Ok(Some(BookDetail {
            book,
            variants,
            file_metadata,
        }))
    }

    pub fn add_file_variant(
        &mut self,
        draft: &FileVariantDraft,
    ) -> Result<FileVariant, LibraryError> {
        let relative_path = normalize_relative_path(Path::new(&draft.relative_path))?;
        let now = unix_seconds();
        let uuid = Uuid::new_v4();
        let connection = self.db.connection_mut();
        connection.execute(
            "INSERT INTO file_variants(\
             uuid, book_id, storage_root_id, relative_path, format_id, size_bytes, mtime_ns,\
             content_hash, origin, source_file_id, state, added_at, last_seen_at\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
            params![
                uuid.to_string(),
                draft.book_id,
                draft.storage_root_id,
                relative_path,
                draft.format_id,
                u64_to_i64(draft.size_bytes)?,
                i128_to_i64(draft.mtime_ns)?,
                draft.content_hash,
                draft.origin.as_str(),
                draft.source_file_id,
                FileVariantState::Present.as_str(),
                now,
            ],
        )?;
        let id = connection.last_insert_rowid();
        self.get_file_variant(id)?.ok_or_else(|| {
            LibraryError::NotFound(format!("file variant {id} was inserted but not readable"))
        })
    }

    pub fn get_file_variant(
        &self,
        file_id: FileVariantId,
    ) -> Result<Option<FileVariant>, LibraryError> {
        self.db
            .connection()
            .query_row(
                "SELECT id, uuid, book_id, storage_root_id, relative_path, format_id,\
                 size_bytes, mtime_ns, content_hash, origin, source_file_id, state,\
                 added_at, last_seen_at FROM file_variants WHERE id = ?1",
                [file_id],
                |row| file_variant_from_row(row, 0),
            )
            .optional()
            .map_err(LibraryError::from)
    }

    pub fn get_file_metadata(
        &self,
        file_id: FileVariantId,
    ) -> Result<Option<FileMetadata>, LibraryError> {
        self.db
            .connection()
            .query_row(
                "SELECT title, subtitle, language, publisher, published_date,\
                 metadata_fingerprint, inspected_at, inspection_contract_version \
                 FROM file_metadata WHERE file_id = ?1",
                [file_id],
                |row| {
                    Ok(FileMetadata {
                        title: row.get(0)?,
                        subtitle: row.get(1)?,
                        language: row.get(2)?,
                        publisher: row.get(3)?,
                        published_date: row.get(4)?,
                        metadata_fingerprint: row.get(5)?,
                        inspected_at: row.get(6)?,
                        inspection_contract_version: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(LibraryError::from)
    }

    pub fn upsert_file_metadata(
        &mut self,
        file_id: FileVariantId,
        metadata: &FileMetadata,
    ) -> Result<(), LibraryError> {
        let changed = self.db.connection_mut().execute(
            "INSERT INTO file_metadata(\
             file_id, title, subtitle, language, publisher, published_date,\
             metadata_fingerprint, inspected_at, inspection_contract_version\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(file_id) DO UPDATE SET title = excluded.title,\
             subtitle = excluded.subtitle, language = excluded.language,\
             publisher = excluded.publisher, published_date = excluded.published_date,\
             metadata_fingerprint = excluded.metadata_fingerprint,\
             inspected_at = excluded.inspected_at,\
             inspection_contract_version = excluded.inspection_contract_version",
            params![
                file_id,
                metadata.title,
                metadata.subtitle,
                metadata.language,
                metadata.publisher,
                metadata.published_date,
                metadata.metadata_fingerprint,
                metadata.inspected_at,
                metadata.inspection_contract_version,
            ],
        )?;
        if changed == 0 {
            return Err(LibraryError::NotFound(format!("file variant {file_id}")));
        }
        Ok(())
    }

    pub(crate) fn update_file_observation(
        &mut self,
        file_id: FileVariantId,
        relative_path: &Path,
        size_bytes: u64,
        mtime_ns: i128,
        content_hash: Option<&str>,
        state: FileVariantState,
    ) -> Result<(), LibraryError> {
        let relative_path = normalize_relative_path(relative_path)?;
        let changed = self.db.connection_mut().execute(
            "UPDATE file_variants SET relative_path = ?1, size_bytes = ?2, mtime_ns = ?3,\
             content_hash = ?4, state = ?5, last_seen_at = ?6 WHERE id = ?7",
            params![
                relative_path,
                u64_to_i64(size_bytes)?,
                i128_to_i64(mtime_ns)?,
                content_hash,
                state.as_str(),
                unix_seconds(),
                file_id,
            ],
        )?;
        if changed == 0 {
            return Err(LibraryError::NotFound(format!("file variant {file_id}")));
        }
        Ok(())
    }

    pub(crate) fn find_variant_by_hash(
        &self,
        root_id: StorageRootId,
        content_hash: &str,
    ) -> Result<Option<FileVariant>, LibraryError> {
        self.db
            .connection()
            .query_row(
                "SELECT id, uuid, book_id, storage_root_id, relative_path, format_id,\
                 size_bytes, mtime_ns, content_hash, origin, source_file_id, state,\
                 added_at, last_seen_at FROM file_variants \
                 WHERE storage_root_id = ?1 AND content_hash = ?2 \
                 ORDER BY state = 'present' DESC, id LIMIT 1",
                params![root_id, content_hash],
                |row| file_variant_from_row(row, 0),
            )
            .optional()
            .map_err(LibraryError::from)
    }

    pub fn find_file_variant_by_hash(
        &self,
        content_hash: &str,
    ) -> Result<Option<FileVariant>, LibraryError> {
        self.db
            .connection()
            .query_row(
                "SELECT id, uuid, book_id, storage_root_id, relative_path, format_id,\
                 size_bytes, mtime_ns, content_hash, origin, source_file_id, state,\
                 added_at, last_seen_at FROM file_variants \
                 WHERE content_hash = ?1 ORDER BY id LIMIT 1",
                [content_hash],
                |row| file_variant_from_row(row, 0),
            )
            .optional()
            .map_err(LibraryError::from)
    }

    pub fn find_books_by_identifier(
        &self,
        scheme: &str,
        value: &str,
        limit: u64,
    ) -> Result<Vec<BookListItem>, LibraryError> {
        let mut statement = self.db.connection().prepare(
            "SELECT b.id, b.uuid, b.title, b.sort_title, b.language, b.updated_at \
             FROM book_identifiers bi JOIN books b ON b.id = bi.book_id \
             WHERE bi.scheme = ?1 AND bi.value = ?2 ORDER BY b.sort_title, b.id LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![scheme, value, u64_to_i64(limit.min(1_000))?],
            |row| {
                Ok(BookListItem {
                    id: row.get(0)?,
                    uuid: parse_uuid(row.get::<_, String>(1)?, 1)?,
                    title: row.get(2)?,
                    sort_title: row.get(3)?,
                    language: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            },
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn list_root_variants(
        &self,
        root_id: StorageRootId,
    ) -> Result<Vec<FileVariant>, LibraryError> {
        let mut statement = self.db.connection().prepare(
            "SELECT id, uuid, book_id, storage_root_id, relative_path, format_id,\
             size_bytes, mtime_ns, content_hash, origin, source_file_id, state,\
             added_at, last_seen_at FROM file_variants WHERE storage_root_id = ?1",
        )?;
        let rows = statement.query_map([root_id], |row| file_variant_from_row(row, 0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn get_storage_root(
        &self,
        root_id: StorageRootId,
    ) -> Result<Option<StorageRoot>, LibraryError> {
        self.db
            .connection()
            .query_row(
                "SELECT id, uuid, path, created_at FROM storage_roots WHERE id = ?1",
                [root_id],
                storage_root_from_row,
            )
            .optional()
            .map_err(LibraryError::from)
    }

    pub fn sync_status(&self, file_id: FileVariantId) -> Result<SyncStatus, LibraryError> {
        let variant = self
            .get_file_variant(file_id)?
            .ok_or_else(|| LibraryError::NotFound(format!("file variant {file_id}")))?;
        let root = self
            .get_storage_root(variant.storage_root_id)?
            .ok_or_else(|| {
                LibraryError::NotFound(format!("storage root {}", variant.storage_root_id))
            })?;
        let path = root_path(&root, &variant)?;
        if !path.is_file() {
            return Ok(SyncStatus::Missing);
        }
        let inspection = folio_core::inspect_summary(&path)?;
        let observed = BookMetadata::from_core(&inspection.metadata);
        let canonical = self
            .get_book(variant.book_id)?
            .ok_or_else(|| LibraryError::NotFound(format!("book {}", variant.book_id)))?;
        if fingerprint(&canonical.metadata) == fingerprint(&observed) {
            Ok(SyncStatus::Synced)
        } else {
            Ok(SyncStatus::Different)
        }
    }

    /// Apply the canonical Book metadata to an EPUB FileVariant. The output
    /// is converted to an adjacent temporary file, validated, atomically
    /// replaced, re-inspected, and only then reflected in File Metadata.
    /// Other formats remain read-only until their Core write-back contract is
    /// explicitly specified.
    pub fn write_back_metadata(
        &mut self,
        file_id: FileVariantId,
    ) -> Result<MetadataSyncResult, LibraryError> {
        let variant = self
            .get_file_variant(file_id)?
            .ok_or_else(|| LibraryError::NotFound(format!("file variant {file_id}")))?;
        if variant.format_id != "EPUB" {
            return Err(LibraryError::InvalidInput(format!(
                "metadata write-back is currently supported only for EPUB, got {}",
                variant.format_id
            )));
        }
        let root = self
            .get_storage_root(variant.storage_root_id)?
            .ok_or_else(|| {
                LibraryError::NotFound(format!("storage root {}", variant.storage_root_id))
            })?;
        let path = root_path(&root, &variant)?;
        let inspection = folio_core::inspect_summary(&path)?;
        let observed = BookMetadata::from_core(&inspection.metadata);
        let canonical = self
            .get_book(variant.book_id)?
            .ok_or_else(|| LibraryError::NotFound(format!("book {}", variant.book_id)))?;
        let diff = metadata_diff(&canonical.metadata, &observed);
        if diff.is_empty() {
            let file_metadata = file_metadata_from_inspection(&inspection, unix_seconds());
            self.upsert_file_metadata(file_id, &file_metadata)?;
            return Ok(MetadataSyncResult {
                file_id,
                changed: false,
                status: SyncStatus::Synced,
                changed_fields: Vec::new(),
            });
        }

        let temporary = temporary_output_path(&path);
        let request = ConversionRequest {
            input: path.clone(),
            output: temporary.clone(),
            target: Target::EPUB,
            options: ConversionOptions::default(),
            edit: diff.edit,
        };
        let conversion = folio_core::convert(&request)?;
        if !conversion.output_path.is_file() {
            return Err(LibraryError::InvalidInput(format!(
                "Core write-back did not create {}",
                temporary.display()
            )));
        }
        let validation = folio_core::validate(&temporary)?;
        if !validation.is_valid() {
            let _ = fs::remove_file(&temporary);
            return Err(LibraryError::InvalidInput(
                "Core write-back output failed validation".to_owned(),
            ));
        }
        fs::rename(&temporary, &path)?;
        let reinspection = folio_core::inspect_summary(&path)?;
        let new_file_metadata = file_metadata_from_inspection(&reinspection, unix_seconds());
        self.upsert_file_metadata(file_id, &new_file_metadata)?;
        let metadata = BookMetadata::from_core(&reinspection.metadata);
        let status = if fingerprint(&canonical.metadata) == fingerprint(&metadata) {
            SyncStatus::Synced
        } else {
            SyncStatus::Different
        };
        Ok(MetadataSyncResult {
            file_id,
            changed: true,
            status,
            changed_fields: diff.changed_fields,
        })
    }

    pub fn rebuild_search(&mut self) -> Result<(), LibraryError> {
        let connection = self.db.connection_mut();
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM library_search", [])?;
        transaction.execute(
            "INSERT INTO library_search(book_id, title, authors, series, tags, description, identifiers) \
             SELECT b.id, COALESCE(b.title, ''), \
             COALESCE((SELECT group_concat(p.name, ' ') FROM book_people bp \
                       JOIN people p ON p.id = bp.person_id WHERE bp.book_id = b.id), ''), \
             COALESCE((SELECT group_concat(s.name, ' ') FROM book_series bs \
                       JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id), ''), \
             COALESCE((SELECT group_concat(t.name, ' ') FROM book_tags bt \
                       JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = b.id), ''), \
             COALESCE(b.description, ''), \
             COALESCE((SELECT group_concat(bi.scheme || ':' || bi.value, ' ') \
                       FROM book_identifiers bi WHERE bi.book_id = b.id), '') \
             FROM books b",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn search(
        &self,
        query: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<SearchResult>, LibraryError> {
        let query = escape_fts_query(query);
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.min(1_000);
        let mut statement = self.db.connection().prepare(
            "SELECT b.id, b.uuid, b.title, b.sort_title, b.language, b.updated_at, \
             bm25(library_search) FROM library_search \
             JOIN books b ON CAST(library_search.book_id AS INTEGER) = b.id \
             WHERE library_search MATCH ?1 ORDER BY bm25(library_search), b.id \
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = statement.query_map(
            params![query, u64_to_i64(limit)?, u64_to_i64(offset)?],
            |row| {
                Ok(SearchResult {
                    item: BookListItem {
                        id: row.get(0)?,
                        uuid: parse_uuid(row.get::<_, String>(1)?, 1)?,
                        title: row.get(2)?,
                        sort_title: row.get(3)?,
                        language: row.get(4)?,
                        updated_at: row.get(5)?,
                    },
                    rank: row.get(6)?,
                })
            },
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn stats(&self) -> Result<LibraryStats, LibraryError> {
        self.db
            .connection()
            .query_row(
                "SELECT \
                 (SELECT COUNT(*) FROM books),\
                 (SELECT COUNT(*) FROM file_variants),\
                 (SELECT COUNT(*) FROM file_variants WHERE state = 'present'),\
                 (SELECT COUNT(*) FROM file_variants WHERE state = 'missing'),\
                 (SELECT COUNT(*) FROM storage_roots)",
                [],
                |row| {
                    Ok(LibraryStats {
                        books: row.get::<_, i64>(0)? as u64,
                        file_variants: row.get::<_, i64>(1)? as u64,
                        present_file_variants: row.get::<_, i64>(2)? as u64,
                        missing_file_variants: row.get::<_, i64>(3)? as u64,
                        storage_roots: row.get::<_, i64>(4)? as u64,
                    })
                },
            )
            .map_err(LibraryError::from)
    }

    /// Insert deterministic synthetic rows in one transaction for scale
    /// measurements. It is intentionally a development/benchmark helper and
    /// never reads or writes book content.
    pub fn insert_synthetic_books(&mut self, count: u64) -> Result<u64, LibraryError> {
        self.insert_synthetic_books_internal(count, None)
    }

    pub fn insert_synthetic_books_with_variants(
        &mut self,
        count: u64,
        storage_root: impl AsRef<Path>,
    ) -> Result<u64, LibraryError> {
        let storage_root = storage_root.as_ref();
        fs::create_dir_all(storage_root)?;
        let storage_root = fs::canonicalize(storage_root)?;
        self.insert_synthetic_books_internal(count, Some(storage_root))
    }

    fn insert_synthetic_books_internal(
        &mut self,
        count: u64,
        storage_root: Option<PathBuf>,
    ) -> Result<u64, LibraryError> {
        let now = unix_seconds();
        let connection = self.db.connection_mut();
        let transaction = connection.transaction()?;
        let storage_root_id = if let Some(path) = storage_root {
            transaction.execute(
                "INSERT INTO storage_roots(uuid, path, created_at) VALUES (?1, ?2, ?3)",
                params![Uuid::new_v4().to_string(), path.to_string_lossy(), now],
            )?;
            Some(transaction.last_insert_rowid())
        } else {
            None
        };
        let mut inserted = 0_u64;
        for index in 0..count {
            let title = format!("Synthetic Book {index:07}");
            let metadata = BookMetadata {
                title: Some(title),
                sort_title: None,
                authors: vec![format!("Author {}", index % 1_000)],
                series: Some(format!("Series {}", index % 100)),
                tags: vec![format!("tag-{}", index % 50)],
                identifiers: vec![Identifier {
                    scheme: "synthetic".to_owned(),
                    value: format!("book-{index:07}"),
                }],
                ..BookMetadata::default()
            };
            let book_id = insert_book_row(&transaction, Uuid::new_v4(), &metadata, now)?;
            replace_book_relations(&transaction, book_id, &metadata)?;
            if let Some(storage_root_id) = storage_root_id {
                let relative_path = format!("books/{index:07}.epub");
                let content_hash = blake3::hash(relative_path.as_bytes()).to_hex().to_string();
                transaction.execute(
                    "INSERT INTO file_variants(\
                     uuid, book_id, storage_root_id, relative_path, format_id, size_bytes,\
                     mtime_ns, content_hash, origin, source_file_id, state, added_at, last_seen_at\
                     ) VALUES (?1, ?2, ?3, ?4, 'EPUB', 1024, 0, ?5, 'scan', NULL, 'missing', ?6, ?6)",
                    params![
                        Uuid::new_v4().to_string(),
                        book_id,
                        storage_root_id,
                        relative_path,
                        content_hash,
                        now,
                    ],
                )?;
            }
            inserted += 1;
        }
        transaction.commit()?;
        Ok(inserted)
    }
}

fn insert_book_row(
    transaction: &Transaction<'_>,
    uuid: Uuid,
    metadata: &BookMetadata,
    now: i64,
) -> Result<BookId, LibraryError> {
    transaction.execute(
        "INSERT INTO books(uuid, title, sort_title, subtitle, language, publisher,\
         published_date, description, series_index, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
        params![
            uuid.to_string(),
            metadata.title,
            metadata.sort_key(),
            metadata.subtitle,
            metadata.language,
            metadata.publisher,
            metadata.published_date,
            metadata.description,
            metadata.series_index,
            now,
        ],
    )?;
    Ok(transaction.last_insert_rowid())
}

fn replace_book_relations(
    transaction: &Transaction<'_>,
    book_id: BookId,
    metadata: &BookMetadata,
) -> Result<(), LibraryError> {
    transaction.execute("DELETE FROM book_people WHERE book_id = ?1", [book_id])?;
    transaction.execute("DELETE FROM book_series WHERE book_id = ?1", [book_id])?;
    transaction.execute("DELETE FROM book_tags WHERE book_id = ?1", [book_id])?;
    transaction.execute("DELETE FROM book_identifiers WHERE book_id = ?1", [book_id])?;
    for (position, name) in metadata.authors.iter().enumerate() {
        let person_id = ensure_named_row(transaction, "people", name)?;
        transaction.execute(
            "INSERT INTO book_people(book_id, person_id, role, position) VALUES (?1, ?2, 'author', ?3)",
            params![book_id, person_id, position as i64],
        )?;
    }
    if let Some(series_name) = &metadata.series {
        let series_id = ensure_named_row(transaction, "series", series_name)?;
        transaction.execute(
            "INSERT INTO book_series(book_id, series_id, series_index) VALUES (?1, ?2, ?3)",
            params![book_id, series_id, metadata.series_index],
        )?;
    }
    let mut seen_tags = BTreeSet::new();
    for tag in &metadata.tags {
        if seen_tags.insert(tag) {
            let tag_id = ensure_named_row(transaction, "tags", tag)?;
            transaction.execute(
                "INSERT INTO book_tags(book_id, tag_id) VALUES (?1, ?2)",
                params![book_id, tag_id],
            )?;
        }
    }
    for identifier in &metadata.identifiers {
        transaction.execute(
            "INSERT OR IGNORE INTO book_identifiers(book_id, scheme, value) VALUES (?1, ?2, ?3)",
            params![book_id, identifier.scheme, identifier.value],
        )?;
    }
    Ok(())
}

fn ensure_named_row(
    transaction: &Transaction<'_>,
    table: &str,
    name: &str,
) -> Result<i64, LibraryError> {
    let sql = match table {
        "people" => "INSERT OR IGNORE INTO people(uuid, name) VALUES (?1, ?2)",
        "series" => "INSERT OR IGNORE INTO series(uuid, name) VALUES (?1, ?2)",
        "tags" => "INSERT OR IGNORE INTO tags(uuid, name) VALUES (?1, ?2)",
        _ => {
            return Err(LibraryError::InvalidInput(format!(
                "unknown relation table {table}"
            )))
        }
    };
    transaction.execute(sql, params![Uuid::new_v4().to_string(), name])?;
    let id = transaction.query_row(
        &format!("SELECT id FROM {table} WHERE name = ?1"),
        [name],
        |row| row.get(0),
    )?;
    Ok(id)
}

fn load_book_base(
    connection: &Connection,
    book_id: BookId,
    budget: &mut QueryBudget,
) -> Result<Option<BookRecord>, LibraryError> {
    budget.record()?;
    connection
        .query_row(
            "SELECT id, uuid, title, sort_title, subtitle, language, publisher,\
             published_date, description, series_index, created_at, updated_at \
             FROM books WHERE id = ?1",
            [book_id],
            |row| {
                Ok(BookRecord {
                    id: row.get(0)?,
                    uuid: parse_uuid(row.get::<_, String>(1)?, 1)?,
                    metadata: BookMetadata {
                        title: row.get(2)?,
                        sort_title: row.get(3)?,
                        subtitle: row.get(4)?,
                        language: row.get(5)?,
                        publisher: row.get(6)?,
                        published_date: row.get(7)?,
                        description: row.get(8)?,
                        series_index: row.get(9)?,
                        ..BookMetadata::default()
                    },
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                })
            },
        )
        .optional()
        .map_err(LibraryError::from)
}

fn load_book_relations(
    connection: &Connection,
    book: &mut BookRecord,
    budget: &mut QueryBudget,
) -> Result<(), LibraryError> {
    budget.record()?;
    let mut statement = connection.prepare(
        "SELECT kind, value, secondary, scheme FROM ( \
         SELECT 'author' AS kind, p.name AS value, CAST(bp.position AS TEXT) AS secondary, NULL AS scheme \
         FROM book_people bp JOIN people p ON p.id = bp.person_id WHERE bp.book_id = ?1 \
         UNION ALL SELECT 'series', s.name, CAST(bs.series_index AS TEXT), NULL \
         FROM book_series bs JOIN series s ON s.id = bs.series_id WHERE bs.book_id = ?1 \
         UNION ALL SELECT 'tag', t.name, NULL, NULL \
         FROM book_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = ?1 \
         UNION ALL SELECT 'identifier', bi.value, NULL, bi.scheme \
         FROM book_identifiers bi WHERE bi.book_id = ?1 \
         ) ORDER BY kind, secondary, value",
    )?;
    let rows = statement.query_map([book.id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    for row in rows {
        let (kind, value, secondary, scheme) = row?;
        match kind.as_str() {
            "author" => book.metadata.authors.push(value),
            "series" => {
                book.metadata.series = Some(value);
                book.metadata.series_index = secondary.and_then(|value| value.parse().ok());
            }
            "tag" => book.metadata.tags.push(value),
            "identifier" => book.metadata.identifiers.push(Identifier {
                scheme: scheme.unwrap_or_else(|| "identifier".to_owned()),
                value,
            }),
            _ => {}
        }
    }
    Ok(())
}

fn storage_root_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StorageRoot> {
    Ok(StorageRoot {
        id: row.get(0)?,
        uuid: parse_uuid(row.get::<_, String>(1)?, 1)?,
        path: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn file_variant_from_row(row: &rusqlite::Row<'_>, start: usize) -> rusqlite::Result<FileVariant> {
    Ok(FileVariant {
        id: row.get(start)?,
        uuid: parse_uuid(row.get::<_, String>(start + 1)?, start + 1)?,
        book_id: row.get(start + 2)?,
        storage_root_id: row.get(start + 3)?,
        relative_path: row.get(start + 4)?,
        format_id: row.get(start + 5)?,
        size_bytes: row.get::<_, i64>(start + 6)? as u64,
        mtime_ns: i64_to_i128(row.get(start + 7)?),
        content_hash: row.get(start + 8)?,
        origin: FileVariantOrigin::try_from(row.get::<_, String>(start + 9)?.as_str()).map_err(
            |error| {
                rusqlite::Error::FromSqlConversionFailure(
                    start + 9,
                    Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
                )
            },
        )?,
        source_file_id: row.get(start + 10)?,
        state: FileVariantState::try_from(row.get::<_, String>(start + 11)?.as_str()).map_err(
            |error| {
                rusqlite::Error::FromSqlConversionFailure(
                    start + 11,
                    Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
                )
            },
        )?,
        added_at: row.get(start + 12)?,
        last_seen_at: row.get(start + 13)?,
    })
}

fn parse_uuid(value: String, column: usize) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        )
    })
}

fn normalize_relative_path(path: &Path) -> Result<String, LibraryError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(LibraryError::UnsafeRelativePath(path.to_path_buf()));
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => components.push(value.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(LibraryError::UnsafeRelativePath(path.to_path_buf()))
            }
        }
    }
    if components.is_empty() {
        return Err(LibraryError::UnsafeRelativePath(path.to_path_buf()));
    }
    Ok(components.join("/"))
}

fn root_path(root: &StorageRoot, variant: &FileVariant) -> Result<PathBuf, LibraryError> {
    let relative = normalize_relative_path(Path::new(&variant.relative_path))?;
    Ok(Path::new(&root.path).join(relative))
}

fn temporary_output_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("book.epub");
    path.with_file_name(format!(
        ".{file_name}.folio-library-{}.tmp",
        std::process::id()
    ))
}

fn i128_to_i64(value: i128) -> Result<i64, LibraryError> {
    i64::try_from(value).map_err(|_| {
        LibraryError::InvalidInput(format!("nanosecond timestamp out of range: {value}"))
    })
}

fn u64_to_i64(value: u64) -> Result<i64, LibraryError> {
    i64::try_from(value)
        .map_err(|_| LibraryError::InvalidInput(format!("integer out of SQLite range: {value}")))
}

const fn i64_to_i128(value: i64) -> i128 {
    value as i128
}

fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or_default()
}

fn escape_fts_query(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    format!("\"{}\"", trimmed.replace('"', "\"\""))
}
