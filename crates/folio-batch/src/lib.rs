//! Directory and multi-file orchestration.  This crate owns file naming and
//! batch policy; all format and compatibility decisions remain in Core.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Condvar, Mutex},
    thread,
};

use folio_core::{
    CancellationToken, ConversionOptions, ConversionReport, ConversionRequest, ProgressEvent,
    Target,
};
use folio_edit::BookEditPlan;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum BatchMode {
    #[default]
    BestEffort,
    Strict,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum CollisionPolicy {
    #[default]
    Rename,
    Error,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BatchOptions {
    pub target: Target,
    pub conversion: ConversionOptions,
    pub batch_mode: BatchMode,
    pub collision_policy: CollisionPolicy,
    pub preserve_tree: bool,
    /// Zero selects a conservative CPU/memory-aware automatic worker count.
    #[serde(default)]
    pub max_concurrent_jobs: usize,
    /// Reserved for future single-book parallel stages; v1 keeps it at one.
    #[serde(default = "default_per_job_parallelism")]
    pub per_job_parallelism: usize,
    /// Optional upper bound for estimated in-flight input bytes.
    #[serde(default)]
    pub memory_budget_bytes: Option<u64>,
    #[serde(default)]
    pub fail_fast: bool,
    #[serde(default)]
    pub edit: BookEditPlan,
}

const fn default_per_job_parallelism() -> usize {
    1
}

impl Default for BatchOptions {
    fn default() -> Self {
        Self {
            target: Target::KFX,
            conversion: ConversionOptions::default(),
            batch_mode: BatchMode::BestEffort,
            collision_policy: CollisionPolicy::Rename,
            preserve_tree: true,
            max_concurrent_jobs: 0,
            per_job_parallelism: 1,
            memory_budget_bytes: None,
            fail_fast: false,
            edit: BookEditPlan::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BatchInput {
    pub source: PathBuf,
    pub relative_path: Option<PathBuf>,
    /// Optional per-source destination root for clients that select files
    /// from several folders. Naming and collision resolution remain here.
    #[serde(default)]
    pub output_root: Option<PathBuf>,
    /// Optional per-book edit plan; when absent, `BatchOptions::edit` applies.
    #[serde(default)]
    pub edit: Option<BookEditPlan>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BatchItemReport {
    pub source: PathBuf,
    pub output: Option<PathBuf>,
    #[serde(default)]
    pub relative_output: Option<PathBuf>,
    pub success: bool,
    pub report: Option<ConversionReport>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BatchReport {
    pub items: Vec<BatchItemReport>,
    pub succeeded: usize,
    pub failed: usize,
    pub aborted: bool,
}

#[derive(Debug, Error)]
pub enum BatchError {
    #[error("batch input directory does not exist: {0}")]
    MissingDirectory(PathBuf),
    #[error("batch output collision: {0}")]
    Collision(PathBuf),
    #[error("archive failed: {0}")]
    Archive(#[from] folio_archive::ArchiveError),
    #[error("batch I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("batch source is invalid: {0}")]
    Source(#[from] folio_input::SourceError),
    #[error("batch executor failed: {0}")]
    Executor(String),
}

struct PlannedJob {
    index: usize,
    input: BatchInput,
    output: PathBuf,
    estimated_bytes: u64,
}

enum WorkerMessage {
    Progress {
        index: usize,
        event: ProgressEvent,
    },
    Finished {
        index: usize,
        result: Box<Result<ConversionReport, String>>,
    },
}

struct MemoryGate {
    budget: Option<u64>,
    used: Mutex<u64>,
    wake: Condvar,
}

impl MemoryGate {
    fn new(budget: Option<u64>) -> Self {
        Self {
            budget,
            used: Mutex::new(0),
            wake: Condvar::new(),
        }
    }

    fn acquire(self: &Arc<Self>, requested: u64) -> MemoryPermit {
        let Some(budget) = self.budget else {
            return MemoryPermit {
                gate: Arc::clone(self),
                amount: 0,
            };
        };
        let requested = requested.max(1);
        let mut used = self.used.lock().expect("memory gate mutex poisoned");
        loop {
            let fits = used
                .checked_add(requested)
                .is_some_and(|total| total <= budget);
            let oversize_alone = requested > budget && *used == 0;
            if fits || oversize_alone {
                *used = used.saturating_add(requested);
                return MemoryPermit {
                    gate: Arc::clone(self),
                    amount: requested,
                };
            }
            used = self.wake.wait(used).expect("memory gate mutex poisoned");
        }
    }

    fn release(&self, amount: u64) {
        if self.budget.is_some() {
            let mut used = self.used.lock().expect("memory gate mutex poisoned");
            *used = used.saturating_sub(amount);
            self.wake.notify_all();
        }
    }
}

struct MemoryPermit {
    gate: Arc<MemoryGate>,
    amount: u64,
}

impl Drop for MemoryPermit {
    fn drop(&mut self) {
        self.gate.release(self.amount);
    }
}

fn worker_count(requested: usize, jobs: usize) -> usize {
    if jobs == 0 {
        return 0;
    }
    let automatic = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(8);
    let limit = if requested == 0 {
        automatic
    } else {
        requested.max(1)
    };
    limit.min(jobs)
}

fn run_worker(
    receiver: Arc<Mutex<mpsc::Receiver<PlannedJob>>>,
    sender: mpsc::Sender<WorkerMessage>,
    cancellation: CancellationToken,
    memory: Arc<MemoryGate>,
    target: Target,
    conversion: ConversionOptions,
    default_edit: BookEditPlan,
) {
    loop {
        let job = {
            let receiver = receiver.lock().expect("batch job mutex poisoned");
            receiver.recv()
        };
        let Ok(job) = job else { break };
        let _permit = memory.acquire(job.estimated_bytes);
        let request = ConversionRequest {
            input: job.input.source.clone(),
            output: job.output.clone(),
            target,
            options: conversion.clone(),
            edit: job.input.edit.as_ref().unwrap_or(&default_edit).clone(),
        };
        let result = folio_core::convert_with_progress(&request, &cancellation, |event| {
            let _ = sender.send(WorkerMessage::Progress {
                index: job.index,
                event,
            });
        })
        .map_err(|error| error.to_string());
        let _ = sender.send(WorkerMessage::Finished {
            index: job.index,
            result: Box::new(result),
        });
    }
}

pub fn convert_files<F>(
    inputs: &[PathBuf],
    output_dir: &Path,
    options: &BatchOptions,
    progress: F,
) -> Result<BatchReport, BatchError>
where
    F: FnMut(usize, usize, &ProgressEvent),
{
    convert_files_with_cancellation(
        inputs,
        output_dir,
        options,
        &CancellationToken::new(),
        progress,
    )
}

pub fn convert_files_with_cancellation<F>(
    inputs: &[PathBuf],
    output_dir: &Path,
    options: &BatchOptions,
    cancellation: &CancellationToken,
    progress: F,
) -> Result<BatchReport, BatchError>
where
    F: FnMut(usize, usize, &ProgressEvent),
{
    let jobs = inputs
        .iter()
        .cloned()
        .map(|source| BatchInput {
            relative_path: source.file_name().map(PathBuf::from),
            source,
            output_root: None,
            edit: None,
        })
        .collect::<Vec<_>>();
    convert_items_with_cancellation(&jobs, output_dir, options, cancellation, progress)
}

pub fn convert_directory<F>(
    input_dir: &Path,
    output_dir: &Path,
    options: &BatchOptions,
    progress: F,
) -> Result<BatchReport, BatchError>
where
    F: FnMut(usize, usize, &ProgressEvent),
{
    convert_directory_with_cancellation(
        input_dir,
        output_dir,
        options,
        &CancellationToken::new(),
        progress,
    )
}

pub fn convert_directory_with_cancellation<F>(
    input_dir: &Path,
    output_dir: &Path,
    options: &BatchOptions,
    cancellation: &CancellationToken,
    progress: F,
) -> Result<BatchReport, BatchError>
where
    F: FnMut(usize, usize, &ProgressEvent),
{
    let jobs = list_directory_inputs(input_dir)?;
    convert_items_with_cancellation(&jobs, output_dir, options, cancellation, progress)
}

/// List supported files under a directory using the shared input and path
/// safety rules. This is also the discovery API used by the native client.
pub fn list_directory_inputs(input_dir: &Path) -> Result<Vec<BatchInput>, BatchError> {
    if !input_dir.is_dir() {
        return Err(BatchError::MissingDirectory(input_dir.to_owned()));
    }
    let source =
        folio_input::BookSource::directory(input_dir, folio_input::SourceLimits::default())?;
    Ok(source
        .supported_book_files()
        .map(|file| BatchInput {
            relative_path: Some(file.relative_path.clone()),
            source: file.path.clone(),
            output_root: None,
            edit: None,
        })
        .collect())
}

/// Convert explicit source items through the same batch owner used by
/// directory conversion. Callers may vary an item's edit plan or output root;
/// output filenames and collision handling are still resolved centrally.
pub fn convert_items_with_cancellation<F>(
    inputs: &[BatchInput],
    output_dir: &Path,
    options: &BatchOptions,
    cancellation: &CancellationToken,
    mut progress: F,
) -> Result<BatchReport, BatchError>
where
    F: FnMut(usize, usize, &ProgressEvent),
{
    fs::create_dir_all(output_dir)?;
    let mut used = BTreeSet::new();
    let mut planned_jobs = Vec::with_capacity(inputs.len());
    let mut planned_by_index: Vec<Option<(BatchInput, PathBuf, PathBuf)>> =
        (0..inputs.len()).map(|_| None).collect();
    for (index, input) in inputs.iter().enumerate() {
        let relative_path = input
            .relative_path
            .as_deref()
            .map(folio_input::normalize_relative_path)
            .transpose()?;
        let safe_input = BatchInput {
            source: input.source.clone(),
            relative_path,
            output_root: input.output_root.clone(),
            edit: input.edit.clone(),
        };
        let source_output_root = safe_input
            .output_root
            .clone()
            .unwrap_or_else(|| output_dir.to_owned());
        fs::create_dir_all(&source_output_root)?;
        let planned = output_path(
            &safe_input,
            &source_output_root,
            options.target,
            options.preserve_tree,
        );
        let output = resolve_collision(planned, &mut used, options.collision_policy)?;
        let estimated_bytes = fs::metadata(&safe_input.source)
            .map(|metadata| metadata.len())
            .unwrap_or(1)
            .max(1);
        planned_by_index[index] = Some((
            safe_input.clone(),
            source_output_root.clone(),
            output.clone(),
        ));
        planned_jobs.push(PlannedJob {
            index,
            input: safe_input,
            output,
            estimated_bytes,
        });
    }

    let worker_limit = worker_count(options.max_concurrent_jobs, planned_jobs.len());
    let (job_sender, job_receiver) = mpsc::channel();
    for job in planned_jobs {
        job_sender
            .send(job)
            .map_err(|error| BatchError::Executor(error.to_string()))?;
    }
    drop(job_sender);
    let job_receiver = Arc::new(Mutex::new(job_receiver));
    let (message_sender, message_receiver) = mpsc::channel();
    let memory = Arc::new(MemoryGate::new(options.memory_budget_bytes));
    let mut item_reports: Vec<Option<BatchItemReport>> = (0..inputs.len()).map(|_| None).collect();
    let mut aborted = cancellation.is_cancelled();
    let stop_on_error = options.fail_fast || options.batch_mode == BatchMode::Strict;
    let planned_count = planned_by_index
        .iter()
        .filter(|item| item.is_some())
        .count();

    thread::scope(|scope| {
        for _ in 0..worker_limit {
            let receiver = Arc::clone(&job_receiver);
            let sender = message_sender.clone();
            let token = cancellation.clone();
            let memory = Arc::clone(&memory);
            let conversion = options.conversion.clone();
            let default_edit = options.edit.clone();
            scope.spawn(move || {
                run_worker(
                    receiver,
                    sender,
                    token,
                    memory,
                    options.target,
                    conversion,
                    default_edit,
                )
            });
        }
        drop(message_sender);
        let mut finished = 0usize;
        while finished < planned_count {
            match message_receiver.recv() {
                Ok(WorkerMessage::Progress { index, event }) => {
                    progress(index + 1, inputs.len(), &event);
                }
                Ok(WorkerMessage::Finished { index, result }) => {
                    finished += 1;
                    let Some((safe_input, source_output_root, output)) =
                        planned_by_index[index].as_ref()
                    else {
                        continue;
                    };
                    let item = match *result {
                        Ok(report) => BatchItemReport {
                            source: safe_input.source.clone(),
                            output: Some(output.clone()),
                            relative_output: relative_output(
                                output,
                                source_output_root,
                                safe_input,
                                options.target,
                                options.preserve_tree,
                            ),
                            success: true,
                            report: Some(report),
                            error: None,
                        },
                        Err(error) => {
                            if stop_on_error {
                                aborted = true;
                                cancellation.cancel();
                            }
                            BatchItemReport {
                                source: safe_input.source.clone(),
                                output: None,
                                relative_output: None,
                                success: false,
                                report: None,
                                error: Some(if cancellation.is_cancelled() {
                                    format!("batch cancelled: {error}")
                                } else {
                                    error
                                }),
                            }
                        }
                    };
                    item_reports[index] = Some(item);
                }
                Err(_) => {
                    aborted = true;
                    break;
                }
            }
        }
    });

    let mut batch = BatchReport {
        aborted: aborted || cancellation.is_cancelled(),
        ..BatchReport::default()
    };
    for (index, input) in inputs.iter().enumerate() {
        let item = item_reports[index]
            .take()
            .unwrap_or_else(|| BatchItemReport {
                source: input.source.clone(),
                output: None,
                relative_output: None,
                success: false,
                report: None,
                error: Some("batch cancelled before this item started".to_owned()),
            });
        if item.success {
            batch.succeeded += 1;
        } else {
            batch.failed += 1;
        }
        batch.items.push(item);
    }
    if options.batch_mode == BatchMode::Strict && batch.failed > 0 {
        batch.aborted = true;
        rollback_strict_batch(&mut batch);
    }
    Ok(batch)
}

fn rollback_strict_batch(batch: &mut BatchReport) {
    cleanup_outputs(batch);
    for item in &mut batch.items {
        if item.success {
            item.success = false;
            item.error = Some("strict batch aborted; successful output rolled back".to_owned());
        }
        item.output = None;
        item.relative_output = None;
        item.report = None;
    }
    batch.succeeded = 0;
    batch.failed = batch.items.len();
}

fn relative_output(
    output: &Path,
    output_dir: &Path,
    input: &BatchInput,
    target: Target,
    preserve_tree: bool,
) -> Option<PathBuf> {
    output
        .strip_prefix(output_dir)
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            let relative = input
                .relative_path
                .clone()
                .or_else(|| input.source.file_name().map(PathBuf::from))?;
            let relative = if preserve_tree {
                relative
            } else {
                relative.file_name().map(PathBuf::from)?
            };
            Some(relative.with_extension(target.extension()))
        })
}

pub fn archive_report(report: &BatchReport, output: &Path) -> Result<(), BatchError> {
    let mut entries = Vec::new();
    for item in &report.items {
        if !item.success {
            continue;
        }
        let Some(path) = &item.output else { continue };
        let bytes = fs::read(path)?;
        let archive_path = item
            .relative_output
            .clone()
            .or_else(|| path.file_name().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("output.bin"));
        entries.push((archive_path, bytes));
    }
    folio_archive::write_archive(
        output,
        &entries,
        &serde_json::to_value(report)
            .unwrap_or_else(|_| serde_json::json!({"serialization":"failed"})),
    )?;
    Ok(())
}

fn cleanup_outputs(report: &BatchReport) {
    for item in &report.items {
        if let Some(path) = &item.output {
            let _ = fs::remove_file(path);
        }
    }
}

fn output_path(
    input: &BatchInput,
    output_dir: &Path,
    target: Target,
    preserve_tree: bool,
) -> PathBuf {
    let relative = input
        .relative_path
        .clone()
        .or_else(|| input.source.file_name().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("book"));
    let relative = if preserve_tree {
        relative
    } else {
        relative.file_name().map(PathBuf::from).unwrap_or(relative)
    };
    output_dir.join(relative).with_extension(target.extension())
}

fn resolve_collision(
    planned: PathBuf,
    used: &mut BTreeSet<PathBuf>,
    policy: CollisionPolicy,
) -> Result<PathBuf, BatchError> {
    if !used.contains(&planned) && !planned.exists() {
        used.insert(planned.clone());
        return Ok(planned);
    }
    if policy == CollisionPolicy::Error {
        return Err(BatchError::Collision(planned));
    }
    let stem = planned
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("book");
    let extension = planned
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("bin");
    let parent = planned.parent().unwrap_or_else(|| Path::new("."));
    for index in 1u32.. {
        let candidate = parent.join(format!("{stem}-{index}.{extension}"));
        if !used.contains(&candidate) && !candidate.exists() {
            used.insert(candidate.clone());
            return Ok(candidate);
        }
    }
    Err(BatchError::Collision(planned))
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-batch/src/lib.rs"]
mod tests;
