use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::SystemTime,
};

use folio_core::{self, DetectedFormat, InspectionReport};

use crate::{
    error::LibraryError,
    model::{BookMetadata, FileVariantDraft, FileVariantId, FileVariantState, StorageRootId},
    repository::Library,
};

#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub max_workers: usize,
    pub queue_capacity: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        let workers = thread::available_parallelism()
            .map(|value| value.get().min(8))
            .unwrap_or(2)
            .max(1);
        Self {
            max_workers: workers,
            queue_capacity: workers.saturating_mul(2).max(1),
        }
    }
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct ScanReport {
    pub unchanged: u64,
    pub moved: u64,
    pub modified: u64,
    pub new: u64,
    pub missing: u64,
    pub duplicate: u64,
    pub failed: u64,
    pub errors: Vec<String>,
}

struct ScanEvent {
    relative_path: PathBuf,
    size_bytes: u64,
    mtime_ns: i128,
    content_hash: String,
    detection: Result<DetectedFormat, String>,
    inspection: Result<InspectionReport, String>,
}

/// Incrementally inspect a storage root with bounded worker queues and one
/// database writer. Existing files are first filtered by size/mtime; only
/// new or changed files enter Core inspection. A content hash may prove a
/// move/duplicate, but never changes book identity by title similarity.
pub fn scan(
    library: &mut Library,
    root_id: StorageRootId,
    options: ScanOptions,
) -> Result<ScanReport, LibraryError> {
    let root = library
        .get_storage_root(root_id)?
        .ok_or_else(|| LibraryError::NotFound(format!("storage root {root_id}")))?;
    let existing = library.list_root_variants(root_id)?;
    let by_path = existing
        .iter()
        .map(|variant| (variant.relative_path.clone(), variant.clone()))
        .collect::<HashMap<_, _>>();
    let mut discovered = Vec::new();
    collect_supported_files(Path::new(&root.path), &mut discovered)?;
    let discovered_keys = discovered
        .iter()
        .filter_map(|path| path.strip_prefix(&root.path).ok())
        .filter_map(|path| normalize_relative_for_scan(path).ok())
        .collect::<HashSet<_>>();

    let mut report = ScanReport::default();
    let mut seen_ids = HashSet::<FileVariantId>::new();
    let mut work = Vec::new();
    for path in discovered {
        let relative = path
            .strip_prefix(&root.path)
            .map_err(|_| LibraryError::UnsafeRelativePath(path.clone()))?
            .to_path_buf();
        let relative_key = normalize_relative_for_scan(&relative)?;
        let metadata = fs::metadata(&path)?;
        let size_bytes = metadata.len();
        let mtime_ns = modified_nanos(&metadata);
        if let Some(variant) = by_path.get(&relative_key) {
            seen_ids.insert(variant.id);
            if variant.state == FileVariantState::Present
                && variant.size_bytes == size_bytes
                && variant.mtime_ns == mtime_ns
            {
                library.update_file_observation(
                    variant.id,
                    Path::new(&relative_key),
                    size_bytes,
                    mtime_ns,
                    variant.content_hash.as_deref(),
                    FileVariantState::Present,
                )?;
                report.unchanged += 1;
                continue;
            }
        }
        work.push((path, relative, size_bytes, mtime_ns));
    }

    let events = inspect_changed_files(work, &options);
    for event in events {
        let relative_key = normalize_relative_for_scan(&event.relative_path)?;
        let existing_variant = by_path.get(&relative_key).cloned();
        match (&event.detection, &event.inspection) {
            (Ok(detected), Ok(inspection)) => {
                if let Some(variant) = existing_variant {
                    seen_ids.insert(variant.id);
                    library.update_file_observation(
                        variant.id,
                        Path::new(&relative_key),
                        event.size_bytes,
                        event.mtime_ns,
                        Some(&event.content_hash),
                        FileVariantState::Present,
                    )?;
                    let file_metadata =
                        crate::metadata::file_metadata_from_inspection(inspection, unix_seconds());
                    library.upsert_file_metadata(variant.id, &file_metadata)?;
                    report.modified += 1;
                    continue;
                }

                if let Some(hash_variant) =
                    library.find_variant_by_hash(root_id, &event.content_hash)?
                {
                    if hash_variant.state == FileVariantState::Missing
                        || !discovered_keys.contains(&hash_variant.relative_path)
                    {
                        library.update_file_observation(
                            hash_variant.id,
                            Path::new(&relative_key),
                            event.size_bytes,
                            event.mtime_ns,
                            Some(&event.content_hash),
                            FileVariantState::Present,
                        )?;
                        seen_ids.insert(hash_variant.id);
                        let file_metadata = crate::metadata::file_metadata_from_inspection(
                            inspection,
                            unix_seconds(),
                        );
                        library.upsert_file_metadata(hash_variant.id, &file_metadata)?;
                        report.moved += 1;
                    } else {
                        let draft = FileVariantDraft {
                            book_id: hash_variant.book_id,
                            storage_root_id: root_id,
                            relative_path: relative_key.clone(),
                            format_id: folio_core::format_id(*detected).to_owned(),
                            size_bytes: event.size_bytes,
                            mtime_ns: event.mtime_ns,
                            content_hash: Some(event.content_hash.clone()),
                            origin: crate::model::FileVariantOrigin::Scan,
                            source_file_id: Some(hash_variant.id),
                        };
                        let variant = library.add_file_variant(&draft)?;
                        library.update_file_observation(
                            variant.id,
                            Path::new(&relative_key),
                            event.size_bytes,
                            event.mtime_ns,
                            Some(&event.content_hash),
                            FileVariantState::Duplicate,
                        )?;
                        let file_metadata = crate::metadata::file_metadata_from_inspection(
                            inspection,
                            unix_seconds(),
                        );
                        library.upsert_file_metadata(variant.id, &file_metadata)?;
                        seen_ids.insert(variant.id);
                        report.duplicate += 1;
                    }
                    continue;
                }

                let book_metadata = BookMetadata::from_core(&inspection.metadata);
                let book = library.create_book(&book_metadata)?;
                let draft = FileVariantDraft {
                    book_id: book.id,
                    storage_root_id: root_id,
                    relative_path: relative_key.clone(),
                    format_id: folio_core::format_id(*detected).to_owned(),
                    size_bytes: event.size_bytes,
                    mtime_ns: event.mtime_ns,
                    content_hash: Some(event.content_hash.clone()),
                    origin: crate::model::FileVariantOrigin::Scan,
                    source_file_id: None,
                };
                let variant = library.add_file_variant(&draft)?;
                let file_metadata =
                    crate::metadata::file_metadata_from_inspection(inspection, unix_seconds());
                library.upsert_file_metadata(variant.id, &file_metadata)?;
                seen_ids.insert(variant.id);
                report.new += 1;
            }
            (detection, inspection) => {
                let error = format_scan_error(detection, inspection);
                report.failed += 1;
                report.errors.push(format!("{relative_key}: {error}"));
                if let Some(variant) = existing_variant {
                    seen_ids.insert(variant.id);
                    library.update_file_observation(
                        variant.id,
                        Path::new(&relative_key),
                        event.size_bytes,
                        event.mtime_ns,
                        Some(&event.content_hash),
                        FileVariantState::Error,
                    )?;
                }
            }
        }
    }

    for variant in existing {
        if !seen_ids.contains(&variant.id) && variant.state != FileVariantState::Missing {
            library.update_file_observation(
                variant.id,
                Path::new(&variant.relative_path),
                variant.size_bytes,
                variant.mtime_ns,
                variant.content_hash.as_deref(),
                FileVariantState::Missing,
            )?;
            report.missing += 1;
        }
    }
    Ok(report)
}

fn inspect_changed_files(
    work: Vec<(PathBuf, PathBuf, u64, i128)>,
    options: &ScanOptions,
) -> Vec<ScanEvent> {
    if work.is_empty() {
        return Vec::new();
    }
    let worker_count = options.max_workers.max(1).min(work.len());
    let queue_capacity = options.queue_capacity.max(1);
    let (work_sender, work_receiver) =
        mpsc::sync_channel::<(PathBuf, PathBuf, u64, i128)>(queue_capacity);
    let receiver = Arc::new(Mutex::new(work_receiver));
    let (event_sender, event_receiver) = mpsc::channel();
    let producer = thread::spawn(move || {
        for item in work {
            if work_sender.send(item).is_err() {
                break;
            }
        }
    });
    let mut workers = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let receiver = Arc::clone(&receiver);
        let event_sender = event_sender.clone();
        workers.push(thread::spawn(move || {
            while let Some(item) = receiver.lock().ok().and_then(|guard| guard.recv().ok()) {
                let (path, relative_path, size_bytes, mtime_ns) = item;
                let content_hash = hash_file(&path);
                let (content_hash, detection, inspection) = match content_hash {
                    Ok(content_hash) => {
                        let detection =
                            folio_core::detect(&path).map_err(|error| error.to_string());
                        let inspection =
                            folio_core::inspect_summary(&path).map_err(|error| error.to_string());
                        (content_hash, detection, inspection)
                    }
                    Err(error) => (
                        String::new(),
                        Err(error.to_string()),
                        Err(error.to_string()),
                    ),
                };
                let _ = event_sender.send(ScanEvent {
                    relative_path,
                    size_bytes,
                    mtime_ns,
                    content_hash,
                    detection,
                    inspection,
                });
            }
        }));
    }
    drop(event_sender);
    let _ = producer.join();
    for worker in workers {
        let _ = worker.join();
    }
    event_receiver.into_iter().collect()
}

fn collect_supported_files(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), LibraryError> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() && is_supported_path(&path) {
                output.push(path);
            }
        }
    }
    output.sort();
    Ok(())
}

fn is_supported_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "epub"
                    | "mobi"
                    | "azw"
                    | "azw3"
                    | "kfx"
                    | "ffkfx"
                    | "docx"
                    | "fb2"
                    | "md"
                    | "markdown"
                    | "txt"
                    | "text"
                    | "html"
                    | "htm"
                    | "xhtml"
                    | "htmlz"
            )
        })
        .unwrap_or(false)
}

fn hash_file(path: &Path) -> Result<String, std::io::Error> {
    let mut file = fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

fn modified_nanos(metadata: &fs::Metadata) -> i128 {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|value| value.as_nanos() as i128)
        .unwrap_or_default()
}

fn normalize_relative_for_scan(path: &Path) -> Result<String, LibraryError> {
    if path.is_absolute() {
        return Err(LibraryError::UnsafeRelativePath(path.to_path_buf()));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(value) => parts.push(value.to_string_lossy().into_owned()),
            std::path::Component::CurDir => {}
            _ => return Err(LibraryError::UnsafeRelativePath(path.to_path_buf())),
        }
    }
    if parts.is_empty() {
        return Err(LibraryError::UnsafeRelativePath(path.to_path_buf()));
    }
    Ok(parts.join("/"))
}

fn format_scan_error(
    detection: &Result<DetectedFormat, String>,
    inspection: &Result<InspectionReport, String>,
) -> String {
    match (detection, inspection) {
        (Err(detection), Err(inspection)) => format!("detect: {detection}; inspect: {inspection}"),
        (Err(error), _) => format!("detect: {error}"),
        (_, Err(error)) => format!("inspect: {error}"),
        _ => "unknown scan failure".to_owned(),
    }
}

fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or_default()
}
