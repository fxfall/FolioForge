use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use base64::Engine as _;
use folio_batch::{BatchInput, BatchMode, BatchOptions, CollisionPolicy};
use folio_core::{
    self, CancellationToken, ComicConversionRequest, ComicImageRequest, ConversionOptions,
    DegradationMode, FormatDescriptor, ReaderOpenComicRequest, ReaderOpenRequest,
    ReaderSetDirectionRequest, ReaderSetSpreadModeRequest, ReaderSetViewportRequest,
    ReaderViewportRequest, Target,
};
use folio_reader::{ReaderContentMode, ReaderDirection, ReaderSpreadMode};
use folio_text::{ParagraphMode, TextEncoding, TextImportMode};
use rfd::FileDialog;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

type ReaderTask = Box<dyn FnOnce() + Send + 'static>;

#[derive(Clone, Copy, Eq, PartialEq)]
enum RowStatus {
    Ready,
    Checking,
    Approved,
    Blocked,
    Converting,
    Completed,
    Failed,
    Cancelled,
}

struct QueueRecord {
    input: BatchInput,
    status: RowStatus,
    preflight_revision: Option<u64>,
    approved_revision: Option<u64>,
    detail: String,
}

struct AppState {
    queue: Vec<QueueRecord>,
    selected_index: i32,
    selected_sources: HashSet<PathBuf>,
    selected_comic_index: i32,
    input_extensions: Vec<String>,
    output_descriptors: Vec<FormatDescriptor>,
    targets: Vec<Target>,
    target: Target,
    mode: DegradationMode,
    batch_mode: BatchMode,
    options: ConversionOptions,
    settings_revision: u64,
    output_directory: Option<PathBuf>,
    active_operation: Option<CancellationToken>,
    operation_id: u64,
    reader_operation_id: u64,
    reader_session_id: Option<String>,
    reader_is_comic: bool,
    reader_viewport: ReaderViewportRequest,
    reader_drag_origin: (f32, f32),
    reader_pan_last_dispatch: Instant,
    reader_content_mode: ReaderContentMode,
    reader_direction: ReaderDirection,
    reader_spread_mode: ReaderSpreadMode,
    comic_session_id: Option<String>,
    comic_source_path: Option<PathBuf>,
    comic_pages: Vec<folio_core::ComicPageDto>,
    comic_thumbnail_generation: u64,
    comic_thumbnail_cancellation: Option<CancellationToken>,
    comic_thumbnail_cache: HashMap<String, Arc<DecodedImageData>>,
    comic_thumbnail_cache_order: VecDeque<String>,
    comic_thumbnail_cache_bytes: usize,
    comic_target_ids: Vec<String>,
    comic_target_names: Vec<String>,
    comic_target_extensions: Vec<String>,
    comic_target_index: usize,
}

fn model<T: Clone + 'static>(values: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(values))
}

fn shared(value: impl Into<String>) -> SharedString {
    SharedString::from(value.into())
}

fn system_translation() -> &'static str {
    match sys_locale::get_locale().as_deref() {
        Some(locale) if locale.to_ascii_lowercase().starts_with("zh") => "zh-Hans",
        _ => "en",
    }
}

fn output_format_model(state: &AppState) -> Vec<SharedString> {
    state
        .output_descriptors
        .iter()
        .map(|descriptor| {
            let extension = descriptor
                .extensions
                .first()
                .map(String::as_str)
                .unwrap_or("");
            if extension.is_empty() {
                shared(descriptor.canonical_name.clone())
            } else {
                shared(format!("{} (.{extension})", descriptor.canonical_name))
            }
        })
        .collect()
}

fn refresh_localized_models(ui: &MainWindow) {
    ui.set_language_options(model(vec![
        ui.get_l10n_language_system(),
        ui.get_l10n_language_english(),
        ui.get_l10n_language_chinese(),
    ]));
    ui.set_mode_options(model(vec![
        ui.get_l10n_mode_strict(),
        ui.get_l10n_mode_compatible(),
        ui.get_l10n_mode_readable(),
    ]));
    ui.set_compression_options(model(vec![
        ui.get_l10n_compression_none(),
        ui.get_l10n_compression_palmdoc(),
    ]));
    ui.set_batch_mode_options(model(vec![
        ui.get_l10n_batch_best_effort(),
        ui.get_l10n_batch_strict(),
    ]));
    ui.set_text_mode_options(model(vec![
        ui.get_l10n_text_auto(),
        ui.get_l10n_text_novel(),
        ui.get_l10n_text_markdown(),
        ui.get_l10n_text_plain(),
    ]));
    ui.set_paragraph_mode_options(model(vec![
        ui.get_l10n_paragraph_auto(),
        ui.get_l10n_paragraph_blank_line(),
        ui.get_l10n_paragraph_every_line(),
        ui.get_l10n_paragraph_indented(),
        ui.get_l10n_paragraph_hard_wrap(),
    ]));
    ui.set_text_encoding_options(model(vec![
        ui.get_l10n_text_encoding_auto(),
        shared(TextEncoding::Utf8.label()),
        shared(TextEncoding::Utf16Le.label()),
        shared(TextEncoding::Utf16Be.label()),
        shared(TextEncoding::Gb18030.label()),
        shared(TextEncoding::Big5.label()),
        shared(TextEncoding::ShiftJis.label()),
        shared(TextEncoding::Windows1252.label()),
    ]));
    ui.set_reader_content_options(model(vec![
        ui.get_l10n_reader_fit(),
        ui.get_l10n_reader_fill(),
        ui.get_l10n_reader_actual_size(),
    ]));
    ui.set_reader_direction_options(model(vec![
        ui.get_l10n_reader_ltr(),
        ui.get_l10n_reader_rtl(),
    ]));
    ui.set_reader_spread_options(model(vec![
        ui.get_l10n_reader_single(),
        ui.get_l10n_reader_two_page(),
    ]));
}

fn row_status_label(ui: &MainWindow, status: RowStatus) -> SharedString {
    match status {
        RowStatus::Ready => ui.get_l10n_ready(),
        RowStatus::Checking => ui.get_l10n_checking(),
        RowStatus::Approved => ui.get_l10n_completed(),
        RowStatus::Blocked => ui.get_l10n_blocked(),
        RowStatus::Converting => ui.get_l10n_converting(),
        RowStatus::Completed => ui.get_l10n_completed(),
        RowStatus::Failed => ui.get_l10n_failed(),
        RowStatus::Cancelled => ui.get_l10n_cancelled(),
    }
}

fn progress_stage_label(ui: &MainWindow, stage: folio_core::ProgressStage) -> SharedString {
    match stage {
        folio_core::ProgressStage::Opening => ui.get_l10n_progress_opening(),
        folio_core::ProgressStage::Parsing => ui.get_l10n_progress_parsing(),
        folio_core::ProgressStage::Normalizing => ui.get_l10n_progress_normalizing(),
        folio_core::ProgressStage::ResolvingStyles => ui.get_l10n_progress_resolving_styles(),
        folio_core::ProgressStage::ProcessingResources => {
            ui.get_l10n_progress_processing_resources()
        }
        folio_core::ProgressStage::Lowering => ui.get_l10n_progress_lowering(),
        folio_core::ProgressStage::BuildingIndexes => ui.get_l10n_progress_building_indexes(),
        folio_core::ProgressStage::Encoding => ui.get_l10n_progress_encoding(),
        folio_core::ProgressStage::Writing => ui.get_l10n_progress_writing(),
        folio_core::ProgressStage::Validating => ui.get_l10n_progress_validating(),
        folio_core::ProgressStage::Finished => ui.get_l10n_progress_finished(),
    }
}

fn refresh_queue(ui: &MainWindow, state: &Arc<Mutex<AppState>>) {
    let Ok(state) = state.lock() else { return };
    let rows = state
        .queue
        .iter()
        .map(|item| {
            let name = item
                .input
                .source
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_else(|| item.input.source.to_str().unwrap_or(""));
            QueueRow {
                name: shared(name.to_owned()),
                state: row_status_label(ui, item.status),
                selected: state.selected_sources.contains(&item.input.source),
            }
        })
        .collect();
    ui.set_queue_rows(model(rows));
    ui.set_selected_index(state.selected_index);
    ui.set_selected_can_convert(selected_sources_can_convert(&state));
    ui.set_all_can_convert(all_sources_can_convert(&state));
    ui.set_selected_page_index(state.selected_comic_index);
    ui.set_output_directory(shared(
        state
            .output_directory
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
    ));
    let source_names = state
        .input_extensions
        .iter()
        .map(|extension| format!(".{extension}"))
        .collect::<Vec<_>>()
        .join(" · ");
    ui.set_source_summary(shared(source_names));
}

fn selected_queue_indices(state: &AppState) -> Vec<usize> {
    if state.selected_sources.is_empty() {
        return (state.selected_index >= 0)
            .then_some(state.selected_index as usize)
            .into_iter()
            .collect();
    }
    state
        .queue
        .iter()
        .enumerate()
        .filter_map(|(index, record)| {
            state
                .selected_sources
                .contains(&record.input.source)
                .then_some(index)
        })
        .collect()
}

fn indices_can_convert(state: &AppState, indices: &[usize]) -> bool {
    if indices.is_empty()
        || indices.iter().any(|index| {
            state
                .queue
                .get(*index)
                .is_none_or(|record| record.preflight_revision != Some(state.settings_revision))
        })
    {
        return false;
    }
    let approved = indices
        .iter()
        .filter_map(|index| state.queue.get(*index))
        .filter(|record| record.approved_revision == Some(state.settings_revision))
        .count();
    approved > 0 && (state.batch_mode != BatchMode::Strict || approved == indices.len())
}

fn selected_sources_can_convert(state: &AppState) -> bool {
    indices_can_convert(state, &selected_queue_indices(state))
}

fn all_sources_can_convert(state: &AppState) -> bool {
    indices_can_convert(state, &(0..state.queue.len()).collect::<Vec<_>>())
}

fn diagnostic_detail(report: &folio_core::PreflightReport) -> String {
    let mut lines = vec![format!(
        "format={} · compatibility={:?} · blocked={}",
        report.detected_format, report.plan.quality, report.plan.blocked
    )];
    for diagnostic in &report.plan.diagnostics {
        lines.push(format!(
            "[{:?}] {}: {}",
            diagnostic.severity, diagnostic.code, diagnostic.message
        ));
    }
    for item in &report.plan.items {
        lines.push(format!(
            "[{:?}] {} → {}: {}",
            item.quality, item.source_representation, item.selected_fallback, item.reason
        ));
    }
    lines.join("\n")
}

fn reader_viewport(state: &AppState) -> ReaderViewportRequest {
    state.reader_viewport.clone()
}

#[derive(Clone)]
struct DecodedImageData {
    rgba: Arc<[u8]>,
    width: u32,
    height: u32,
}

struct ReaderDisplayPageData {
    source: DecodedImageData,
    clip_x: f32,
    clip_y: f32,
    clip_width: f32,
    clip_height: f32,
    offset_x: f32,
    offset_y: f32,
    image_width: f32,
    image_height: f32,
}

fn set_reader_images(ui: &MainWindow, pages: Vec<ReaderDisplayPageData>) {
    let pages = pages
        .into_iter()
        .map(|page| ReaderDisplayPage {
            source: image_to_slint(&page.source),
            clip_x: page.clip_x,
            clip_y: page.clip_y,
            clip_width: page.clip_width,
            clip_height: page.clip_height,
            offset_x: page.offset_x,
            offset_y: page.offset_y,
            image_width: page.image_width,
            image_height: page.image_height,
        })
        .collect();
    ui.set_reader_pages(model(pages));
    ui.set_preview_image(slint::Image::default());
}

fn decode_image(bytes: &[u8]) -> Result<DecodedImageData, String> {
    let image = image::load_from_memory(bytes).map_err(|error| error.to_string())?;
    let rgba = image.to_rgba8();
    Ok(DecodedImageData {
        rgba: Arc::from(rgba.into_raw()),
        width: image.width(),
        height: image.height(),
    })
}

fn image_to_slint(image: &DecodedImageData) -> slint::Image {
    let pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        &image.rgba,
        image.width,
        image.height,
    );
    slint::Image::from_rgba8(pixels)
}

fn placement_to_slint(
    resource: &folio_core::ReaderResourceDto,
    destination: folio_reader::ReaderRect,
    clipping: folio_reader::ReaderRect,
    viewport: folio_reader::ReaderSize,
) -> Result<ReaderDisplayPageData, String> {
    if !viewport.width.is_finite()
        || !viewport.height.is_finite()
        || viewport.width <= 0.0
        || viewport.height <= 0.0
        || !clipping.width.is_finite()
        || !clipping.height.is_finite()
        || clipping.width <= 0.0
        || clipping.height <= 0.0
    {
        return Err("Reader returned invalid placement geometry".to_owned());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&resource.data_base64)
        .map_err(|error| error.to_string())?;
    let source = decode_image(&bytes)?;
    Ok(ReaderDisplayPageData {
        source,
        clip_x: clipping.x / viewport.width,
        clip_y: clipping.y / viewport.height,
        clip_width: clipping.width / viewport.width,
        clip_height: clipping.height / viewport.height,
        offset_x: (destination.x - clipping.x) / clipping.width,
        offset_y: (destination.y - clipping.y) / clipping.height,
        image_width: destination.width / clipping.width,
        image_height: destination.height / clipping.height,
    })
}

fn render_reader(
    reader_session_id: &str,
    cancellation: &CancellationToken,
) -> Result<(Vec<ReaderDisplayPageData>, folio_core::ReaderSessionSummary), String> {
    cancellation.check().map_err(|error| error.to_string())?;
    let summary =
        folio_core::reader_summary(reader_session_id).map_err(|error| error.to_string())?;
    let mut images = Vec::new();
    if summary.spread_mode == ReaderSpreadMode::Synthetic {
        let spread = folio_core::reader_current_spread(reader_session_id)
            .map_err(|error| error.to_string())?;
        for page in spread.pages {
            for placement in page.placements {
                cancellation.check().map_err(|error| error.to_string())?;
                images.push(placement_to_slint(
                    &placement.resource,
                    placement.destination,
                    placement.clipping_rect,
                    spread.viewport_size,
                )?);
            }
        }
    } else {
        let page =
            folio_core::reader_current_page_with_cancellation(reader_session_id, cancellation)
                .map_err(|error| error.to_string())?;
        for placement in page.placements {
            cancellation.check().map_err(|error| error.to_string())?;
            images.push(placement_to_slint(
                &placement.resource,
                placement.destination,
                placement.clipping_rect,
                page.viewport_size,
            )?);
        }
    }
    Ok((images, summary))
}

fn close_active_reader(active_reader: &Arc<Mutex<Option<String>>>) {
    let old = active_reader.lock().ok().and_then(|mut value| value.take());
    if let Some(id) = old {
        let _ = folio_core::reader_close(&id);
    }
}

fn update_reader_view(
    ui: &MainWindow,
    state_arc: &Arc<Mutex<AppState>>,
    generation: u64,
    images: Vec<ReaderDisplayPageData>,
    summary: folio_core::ReaderSessionSummary,
    owner_is_comic: bool,
) {
    let Ok(mut state) = state_arc.lock() else {
        return;
    };
    if state.reader_operation_id != generation {
        return;
    }
    state.reader_session_id = Some(summary.session_id.clone());
    state.reader_is_comic = owner_is_comic;
    state.reader_viewport = summary.viewport.clone();
    state.reader_content_mode = summary.viewport.content_mode;
    set_reader_images(ui, images);
    ui.set_reader_current_page(summary.current_index as i32);
    ui.set_reader_page_count(summary.page_count as i32);
    ui.set_reader_zoom_percent((summary.viewport.scale * 100.0).round() as i32);
    ui.set_status_text(ui.get_l10n_ready());
    ui.set_diagnostics_text(SharedString::default());
    ui.set_detail_text(SharedString::default());
    let thumbnail_center = if owner_is_comic {
        let selected = state
            .comic_pages
            .iter()
            .position(|page| page.display_index == summary.current_document_index + 1);
        state.selected_comic_index = selected.map(|index| index as i32).unwrap_or(-1);
        ui.set_selected_page_index(state.selected_comic_index);
        Some(state.selected_comic_index)
    } else {
        None
    };
    drop(state);
    if let Some(center) = thumbnail_center {
        enqueue_comic_thumbnails(ui, state_arc, center);
    }
}

fn show_reader_failure(ui: &MainWindow, error: String) {
    ui.set_reader_pages(model(Vec::<ReaderDisplayPage>::new()));
    ui.set_reader_page_count(0);
    ui.set_diagnostics_text(ui.get_l10n_reader_unavailable());
    ui.set_detail_text(shared(error));
}

fn enqueue_reader_open(
    ui: &MainWindow,
    state: &Arc<Mutex<AppState>>,
    reader_sender: &mpsc::Sender<ReaderTask>,
    active_reader: &Arc<Mutex<Option<String>>>,
    input: PathBuf,
) {
    let (generation, viewport, direction) = {
        let Ok(mut state) = state.lock() else { return };
        state.reader_operation_id = state.reader_operation_id.wrapping_add(1);
        state.reader_session_id = None;
        state.reader_is_comic = false;
        (
            state.reader_operation_id,
            reader_viewport(&state),
            Some(state.reader_direction),
        )
    };
    ui.set_reader_pages(model(Vec::<ReaderDisplayPage>::new()));
    ui.set_reader_page_count(0);
    ui.set_status_text(ui.get_l10n_checking());
    let weak = ui.as_weak();
    let state = Arc::clone(state);
    let active_reader = Arc::clone(active_reader);
    let request = ReaderOpenRequest {
        input,
        viewport,
        direction,
    };
    let task: ReaderTask = Box::new(move || {
        close_active_reader(&active_reader);
        let cancellation = CancellationToken::new();
        let result = folio_core::reader_open_with_cancellation(&request, &cancellation)
            .map_err(|error| error.to_string())
            .and_then(|summary| {
                let _ = active_reader
                    .lock()
                    .map(|mut active| *active = Some(summary.session_id.clone()));
                render_reader(&summary.session_id, &cancellation)
                    .map(|(images, state)| (summary.session_id, images, state))
            });
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            match result {
                Ok((session_id, images, mut summary)) => {
                    summary.session_id = session_id;
                    update_reader_view(&ui, &state, generation, images, summary, false);
                }
                Err(error) => show_reader_failure(&ui, error),
            }
        });
    });
    let _ = reader_sender.send(task);
}

fn enqueue_reader_refresh(
    ui: &MainWindow,
    state: &Arc<Mutex<AppState>>,
    reader_sender: &mpsc::Sender<ReaderTask>,
    operation: ReaderOperation,
) {
    let (
        generation,
        session_id,
        viewport,
        direction,
        spread_mode,
        is_comic,
        comic_id,
        page_id,
        document_index,
    ) = {
        let Ok(mut state) = state.lock() else { return };
        let Some(session_id) = state.reader_session_id.clone() else {
            return;
        };
        state.reader_operation_id = state.reader_operation_id.wrapping_add(1);
        let (page_id, document_index) = if matches!(operation, ReaderOperation::SelectComicPage(_))
        {
            let index = match operation {
                ReaderOperation::SelectComicPage(index) => Some(index),
                _ => None,
            };
            let page = index.and_then(|index| state.comic_pages.get(index));
            (
                page.map(|page| page.page_id.clone()),
                page.map(|page| page.display_index.saturating_sub(1)),
            )
        } else {
            (None, None)
        };
        (
            state.reader_operation_id,
            session_id,
            reader_viewport(&state),
            state.reader_direction,
            state.reader_spread_mode,
            state.reader_is_comic,
            state.comic_session_id.clone(),
            page_id,
            document_index,
        )
    };
    let weak = ui.as_weak();
    let state = Arc::clone(state);
    let task: ReaderTask = Box::new(move || {
        if matches!(
            operation,
            ReaderOperation::SetOptions
                | ReaderOperation::Resize
                | ReaderOperation::SelectComicPage(_)
        ) && state
            .lock()
            .is_ok_and(|state| state.reader_operation_id != generation)
        {
            return;
        }
        let result = (|| -> Result<_, String> {
            if let Some(document_index) = document_index {
                folio_core::reader_go_to_document_index(
                    &folio_core::ReaderGoToDocumentIndexRequest {
                        session_id: session_id.clone(),
                        document_index,
                    },
                )
                .map_err(|error| error.to_string())?;
            } else {
                match operation {
                    ReaderOperation::Previous => {
                        let _ = folio_core::reader_previous(&session_id)
                            .map_err(|error| error.to_string())?;
                    }
                    ReaderOperation::Next => {
                        let _ = folio_core::reader_next(&session_id)
                            .map_err(|error| error.to_string())?;
                    }
                    ReaderOperation::SetOptions
                    | ReaderOperation::Resize
                    | ReaderOperation::SelectComicPage(_) => {}
                }
            }
            folio_core::reader_set_viewport(&ReaderSetViewportRequest {
                session_id: session_id.clone(),
                viewport,
            })
            .map_err(|error| error.to_string())?;
            folio_core::reader_set_direction(&ReaderSetDirectionRequest {
                session_id: session_id.clone(),
                direction,
            })
            .map_err(|error| error.to_string())?;
            folio_core::reader_set_spread_mode(&ReaderSetSpreadModeRequest {
                session_id: session_id.clone(),
                spread_mode,
            })
            .map_err(|error| error.to_string())?;
            let cancellation = CancellationToken::new();
            match render_reader(&session_id, &cancellation) {
                Ok((images, summary)) => Ok((images, summary, None)),
                Err(reader_error) if is_comic => {
                    let Some(comic_id) = comic_id else {
                        return Err(reader_error);
                    };
                    let summary = folio_core::reader_summary(&session_id)
                        .map_err(|error| error.to_string())?;
                    let target_page = document_index
                        .and_then(|index| index.checked_add(1))
                        .or_else(|| Some(summary.current_document_index + 1));
                    let page = folio_core::comic_pages(&comic_id)
                        .map_err(|error| error.to_string())?
                        .into_iter()
                        .find(|page| Some(page.display_index) == target_page)
                        .or_else(|| {
                            page_id.as_ref().and_then(|wanted| {
                                folio_core::comic_pages(&comic_id)
                                    .ok()?
                                    .into_iter()
                                    .find(|page| &page.page_id == wanted)
                            })
                        })
                        .ok_or_else(|| reader_error.clone())?;
                    let image = folio_core::comic_preview(
                        &ComicImageRequest {
                            session_id: comic_id,
                            page_id: page.page_id,
                            max_width: 1600,
                            max_height: 2400,
                            scale: 1.0,
                        },
                        &cancellation,
                    )
                    .map_err(|error| error.to_string())?;
                    let image = decode_image(&image.bytes)?;
                    let single = ReaderDisplayPageData {
                        source: image,
                        clip_x: 0.0,
                        clip_y: 0.0,
                        clip_width: 1.0,
                        clip_height: 1.0,
                        offset_x: 0.0,
                        offset_y: 0.0,
                        image_width: 1.0,
                        image_height: 1.0,
                    };
                    Ok((vec![single], summary, Some(reader_error)))
                }
                Err(error) => Err(error),
            }
        })();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            match result {
                Ok((images, summary, fallback)) => {
                    update_reader_view(&ui, &state, generation, images, summary, is_comic);
                    if let Some(reason) = fallback {
                        ui.set_diagnostics_text(ui.get_l10n_reader_unavailable());
                        ui.set_detail_text(shared(reason));
                    }
                }
                Err(error) => show_reader_failure(&ui, error),
            }
        });
    });
    let _ = reader_sender.send(task);
}

#[derive(Clone, Copy)]
enum ReaderOperation {
    Previous,
    Next,
    SetOptions,
    Resize,
    SelectComicPage(usize),
}

fn add_file_records(state: &Arc<Mutex<AppState>>, paths: Vec<PathBuf>) {
    let Ok(mut state) = state.lock() else { return };
    let mut existing = state
        .queue
        .iter()
        .map(|item| item.input.source.clone())
        .collect::<HashSet<_>>();
    for path in paths {
        if path.is_dir() || !existing.insert(path.clone()) {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !state
            .input_extensions
            .iter()
            .any(|supported| supported == &ext)
        {
            continue;
        }
        let parent = path.parent().map(Path::to_path_buf);
        let output_root = if state.output_directory.is_none() {
            parent
        } else {
            None
        };
        let source = path.clone();
        state.queue.push(QueueRecord {
            input: BatchInput {
                source: path,
                relative_path: None,
                output_root,
                edit: None,
            },
            status: RowStatus::Ready,
            preflight_revision: None,
            approved_revision: None,
            detail: String::new(),
        });
        if state.selected_index < 0 {
            state.selected_index = (state.queue.len() - 1) as i32;
            state.selected_sources.insert(source);
        }
    }
    if state.selected_index < 0 && !state.queue.is_empty() {
        state.selected_index = 0;
    }
}

fn add_directory_records(state: &Arc<Mutex<AppState>>, root: PathBuf) -> Result<usize, String> {
    let mut inputs =
        folio_batch::list_directory_inputs(&root).map_err(|error| error.to_string())?;
    let Ok(mut state) = state.lock() else {
        return Err("application state is unavailable".to_owned());
    };
    let output_root = state
        .output_directory
        .clone()
        .unwrap_or_else(|| root.join("FolioForge-output"));
    let mut existing = state
        .queue
        .iter()
        .map(|item| item.input.source.clone())
        .collect::<HashSet<_>>();
    let mut count = 0;
    for input in &mut inputs {
        if !existing.insert(input.source.clone()) {
            continue;
        }
        input.output_root = Some(output_root.clone());
        state.queue.push(QueueRecord {
            input: input.clone(),
            status: RowStatus::Ready,
            preflight_revision: None,
            approved_revision: None,
            detail: String::new(),
        });
        if state.selected_index < 0 {
            state.selected_index = (state.queue.len() - 1) as i32;
            state.selected_sources.insert(input.source.clone());
        }
        count += 1;
    }
    if state.selected_index < 0 && !state.queue.is_empty() {
        state.selected_index = 0;
    }
    Ok(count)
}

fn comic_page_rows(
    pages: &[folio_core::ComicPageDto],
    selected_index: i32,
    thumbnails: &HashMap<String, Arc<DecodedImageData>>,
) -> Vec<ComicPageRow> {
    pages
        .iter()
        .enumerate()
        .map(|(index, page)| {
            let dimensions = match (page.width, page.height) {
                (Some(width), Some(height)) => format!(" · {width}×{height}"),
                _ => String::new(),
            };
            let thumbnail = thumbnails.get(&page.page_id);
            ComicPageRow {
                name: shared(page.name.clone()),
                detail: shared(format!(
                    "{} · {}{}",
                    page.display_index, page.source_format, dimensions
                )),
                thumbnail: thumbnail
                    .map(|image| image_to_slint(image))
                    .unwrap_or_default(),
                has_thumbnail: thumbnail.is_some(),
                selected: index as i32 == selected_index,
            }
        })
        .collect()
}

fn enqueue_comic_thumbnails(ui: &MainWindow, state: &Arc<Mutex<AppState>>, center_index: i32) {
    const THUMBNAIL_CACHE_LIMIT: usize = 32 * 1024 * 1024;
    const THUMBNAIL_WINDOW_BEFORE: usize = 8;
    const THUMBNAIL_WINDOW_AFTER: usize = 16;

    let (generation, session_id, selected_page, missing, cancellation) = {
        let Ok(mut state) = state.lock() else { return };
        let Some(session_id) = state.comic_session_id.clone() else {
            return;
        };
        if let Some(previous) = state.comic_thumbnail_cancellation.take() {
            previous.cancel();
        }
        state.comic_thumbnail_generation = state.comic_thumbnail_generation.wrapping_add(1);
        let generation = state.comic_thumbnail_generation;
        let center = if center_index < 0 {
            0
        } else {
            (center_index as usize).min(state.comic_pages.len().saturating_sub(1))
        };
        let selected_page = state.comic_pages.get(center).cloned();
        let start = center.saturating_sub(THUMBNAIL_WINDOW_BEFORE);
        let end = (center + THUMBNAIL_WINDOW_AFTER).min(state.comic_pages.len());
        let missing = state.comic_pages[start..end]
            .iter()
            .filter(|page| !state.comic_thumbnail_cache.contains_key(&page.page_id))
            .cloned()
            .collect::<Vec<_>>();
        let cancellation = CancellationToken::new();
        state.comic_thumbnail_cancellation = Some(cancellation.clone());
        (generation, session_id, selected_page, missing, cancellation)
    };

    let weak = ui.as_weak();
    let state = Arc::clone(state);
    thread::spawn(move || {
        let selected_info = selected_page.and_then(|page| {
            folio_core::comic_page_info(&folio_core::ComicPageRequest {
                session_id: session_id.clone(),
                page_id: page.page_id,
            })
            .ok()
        });
        let mut decoded = Vec::with_capacity(missing.len());
        for page in missing {
            if cancellation.is_cancelled() {
                break;
            }
            let result = folio_core::comic_thumbnail(
                &ComicImageRequest {
                    session_id: session_id.clone(),
                    page_id: page.page_id.clone(),
                    max_width: 180,
                    max_height: 240,
                    scale: 1.0,
                },
                &cancellation,
            );
            if let Ok(image) = result {
                if let Ok(image) = decode_image(&image.bytes) {
                    decoded.push((page.page_id, image));
                }
            }
        }
        let cancelled = cancellation.is_cancelled();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Ok(mut state) = state.lock() else { return };
            if state.comic_thumbnail_generation != generation
                || state.comic_session_id.as_deref() != Some(session_id.as_str())
            {
                return;
            }
            if let Some(info) = selected_info {
                if let Some(page) = state
                    .comic_pages
                    .iter_mut()
                    .find(|page| page.page_id == info.page_id)
                {
                    *page = info;
                }
            }
            for (page_id, image) in decoded {
                if state.comic_thumbnail_cache.contains_key(&page_id) {
                    continue;
                }
                state.comic_thumbnail_cache_bytes = state
                    .comic_thumbnail_cache_bytes
                    .saturating_add(image.rgba.len());
                state
                    .comic_thumbnail_cache
                    .insert(page_id.clone(), Arc::new(image));
                state.comic_thumbnail_cache_order.push_back(page_id);
            }
            while state.comic_thumbnail_cache_bytes > THUMBNAIL_CACHE_LIMIT {
                let Some(oldest) = state.comic_thumbnail_cache_order.pop_front() else {
                    break;
                };
                if let Some(image) = state.comic_thumbnail_cache.remove(&oldest) {
                    state.comic_thumbnail_cache_bytes = state
                        .comic_thumbnail_cache_bytes
                        .saturating_sub(image.rgba.len());
                }
            }
            if !cancelled {
                let rows = comic_page_rows(
                    &state.comic_pages,
                    state.selected_comic_index,
                    &state.comic_thumbnail_cache,
                );
                ui.set_comic_page_rows(model(rows));
            }
            state.comic_thumbnail_cancellation = None;
        });
    });
}

fn initialize_window(
    ui: &MainWindow,
    state: &Arc<Mutex<AppState>>,
    active_reader: &Arc<Mutex<Option<String>>>,
    active_comic: &Arc<Mutex<Option<String>>>,
) {
    refresh_localized_models(ui);
    let registry = folio_core::FormatRegistry;
    let formats = registry.formats();
    let mut input_extensions = formats
        .iter()
        .filter(|descriptor| descriptor.input_supported)
        .flat_map(|descriptor| descriptor.extensions.clone())
        .map(|extension| extension.to_ascii_lowercase())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    input_extensions.sort();
    let output_descriptors = registry.output_formats();
    let output_targets = output_descriptors
        .iter()
        .filter_map(|descriptor| Target::parse(&descriptor.id))
        .collect::<Vec<_>>();
    let comic_options = folio_core::comic_conversion_options();
    if let Ok(mut state) = state.lock() {
        state.input_extensions = input_extensions;
        state.output_descriptors = output_descriptors;
        state.targets = output_targets;
        state.target = state
            .targets
            .iter()
            .copied()
            .find(|target| *target == Target::EPUB)
            .or_else(|| state.targets.first().copied())
            .unwrap_or(Target::EPUB);
        state.comic_target_ids = comic_options
            .targets
            .iter()
            .map(|target| target.id.clone())
            .collect();
        state.comic_target_names = comic_options
            .targets
            .iter()
            .map(|target| target.display_name.clone())
            .collect();
        state.comic_target_extensions = comic_options
            .targets
            .iter()
            .map(|target| target.extension.clone())
            .collect();
        state.comic_target_index = 0;
    }
    if let Ok(state) = state.lock() {
        let target_index = state
            .targets
            .iter()
            .position(|target| *target == state.target)
            .unwrap_or(0);
        ui.set_target_options(model(output_format_model(&state)));
        ui.set_target_index(target_index as i32);
        ui.set_comic_target_options(model(
            state
                .comic_target_names
                .iter()
                .map(|name| shared(name.clone()))
                .collect(),
        ));
        ui.set_comic_target_index(0);
    }
    ui.set_language_index(0);
    ui.set_mode_index(1);
    ui.set_compression_index(0);
    ui.set_batch_mode_index(0);
    ui.set_text_mode_index(0);
    ui.set_paragraph_mode_index(0);
    ui.set_text_encoding_index(0);
    ui.set_reader_content_index(0);
    ui.set_reader_direction_index(0);
    ui.set_reader_spread_index(0);
    ui.set_status_text(ui.get_l10n_ready());
    ui.set_diagnostics_text(SharedString::default());
    ui.set_detail_text(SharedString::default());
    let _ = active_reader.lock().map(|mut active| active.take());
    let _ = active_comic.lock().map(|mut active| active.take());
}

fn selected_target_index(state: &AppState, index: i32) -> Option<Target> {
    if index < 0 {
        return None;
    }
    state.targets.get(index as usize).copied()
}

fn text_import_mode_from_index(index: i32) -> TextImportMode {
    match index {
        1 => TextImportMode::Novel,
        2 => TextImportMode::Markdown,
        3 => TextImportMode::Plain,
        _ => TextImportMode::Auto,
    }
}

fn paragraph_mode_from_index(index: i32) -> ParagraphMode {
    match index {
        1 => ParagraphMode::BlankLine,
        2 => ParagraphMode::EveryLine,
        3 => ParagraphMode::Indented,
        4 => ParagraphMode::HardWrap,
        _ => ParagraphMode::Auto,
    }
}

fn text_encoding_from_index(index: i32) -> Option<TextEncoding> {
    match index {
        1 => Some(TextEncoding::Utf8),
        2 => Some(TextEncoding::Utf16Le),
        3 => Some(TextEncoding::Utf16Be),
        4 => Some(TextEncoding::Gb18030),
        5 => Some(TextEncoding::Big5),
        6 => Some(TextEncoding::ShiftJis),
        7 => Some(TextEncoding::Windows1252),
        _ => None,
    }
}

fn install_callbacks(
    ui: &MainWindow,
    state: Arc<Mutex<AppState>>,
    reader_sender: mpsc::Sender<ReaderTask>,
    active_reader: Arc<Mutex<Option<String>>>,
    active_comic: Arc<Mutex<Option<String>>>,
) {
    let weak = ui.as_weak();
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_choose_files(move || {
            let Some(ui) = weak.upgrade() else { return };
            let extensions = state
                .lock()
                .map(|state| state.input_extensions.clone())
                .unwrap_or_default();
            let paths = FileDialog::new()
                .set_title(ui.get_l10n_open_books().to_string())
                .add_filter(ui.get_l10n_supported_books().to_string(), &extensions)
                .pick_files();
            if let Some(paths) = paths {
                add_file_records(&state, paths);
                refresh_queue(&ui, &state);
            }
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_choose_folder(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(folder) = FileDialog::new()
                .set_title(ui.get_l10n_select_folder().to_string())
                .pick_folder()
            else {
                return;
            };
            match add_directory_records(&state, folder) {
                Ok(_) => {
                    refresh_queue(&ui, &state);
                    ui.set_status_text(ui.get_l10n_completed());
                }
                Err(error) => {
                    ui.set_status_text(ui.get_l10n_error());
                    ui.set_detail_text(shared(error));
                }
            }
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_choose_output(move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Some(folder) = FileDialog::new()
                .set_title(ui.get_l10n_select_folder().to_string())
                .pick_folder()
            {
                if let Ok(mut state) = state.lock() {
                    state.output_directory = Some(folder);
                    for record in &mut state.queue {
                        record.input.output_root = None;
                    }
                    invalidate_preflight(&mut state);
                }
                refresh_queue(&ui, &state);
            }
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let active_reader = Arc::clone(&active_reader);
        let active_comic = Arc::clone(&active_comic);
        let reader_sender = reader_sender.clone();
        ui.on_choose_comic(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(path) = FileDialog::new()
                .set_title(ui.get_l10n_open_comic().to_string())
                .pick_file()
            else {
                return;
            };
            open_comic_source(
                &ui,
                &state,
                &reader_sender,
                &active_reader,
                &active_comic,
                path,
            );
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let active_reader = Arc::clone(&active_reader);
        let active_comic = Arc::clone(&active_comic);
        let reader_sender = reader_sender.clone();
        ui.on_choose_comic_folder(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(path) = FileDialog::new()
                .set_title(ui.get_l10n_open_comic().to_string())
                .pick_folder()
            else {
                return;
            };
            open_comic_source(
                &ui,
                &state,
                &reader_sender,
                &active_reader,
                &active_comic,
                path,
            );
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_remove_selected(move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                let index = state.selected_index;
                if index >= 0 && (index as usize) < state.queue.len() {
                    let removed = state.queue.remove(index as usize);
                    state.selected_sources.remove(&removed.input.source);
                    state.selected_index = if state.queue.is_empty() {
                        -1
                    } else {
                        (index as usize).min(state.queue.len() - 1) as i32
                    };
                }
            }
            refresh_queue(&ui, &state);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_queue_item_selected(move |index, selected| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                let Some(record) = (index >= 0)
                    .then(|| state.queue.get(index as usize))
                    .flatten()
                else {
                    return;
                };
                let path = record.input.source.clone();
                if selected {
                    state.selected_sources.insert(path);
                } else {
                    state.selected_sources.remove(&path);
                }
            }
            refresh_queue(&ui, &state);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let reader_sender = reader_sender.clone();
        let active_reader = Arc::clone(&active_reader);
        ui.on_select_book(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            let path = {
                let Ok(mut state) = state.lock() else { return };
                if index < 0 || index as usize >= state.queue.len() {
                    return;
                }
                state.selected_index = index;
                let path = state.queue[index as usize].input.source.clone();
                if state.selected_sources.is_empty() {
                    state.selected_sources.insert(path.clone());
                }
                Some(path)
            };
            refresh_queue(&ui, &state);
            if let Some(path) = path {
                enqueue_reader_open(&ui, &state, &reader_sender, &active_reader, path);
            }
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let active_reader = Arc::clone(&active_reader);
        let active_comic = Arc::clone(&active_comic);
        let reader_sender = reader_sender.clone();
        ui.on_select_comic_page(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                state.selected_comic_index = index;
            }
            ui.set_selected_page_index(index);
            enqueue_comic_thumbnails(&ui, &state, index);
            enqueue_reader_refresh(
                &ui,
                &state,
                &reader_sender,
                ReaderOperation::SelectComicPage(index.max(0) as usize),
            );
            let _ = (&active_reader, &active_comic);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_set_workspace(move |workspace| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_active_workspace(workspace);
            if workspace == 1 {
                ui.set_reader_caption(shared(""));
            }
            let _ = &state;
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_target_selected(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                if let Some(target) = selected_target_index(&state, index) {
                    state.target = target;
                    invalidate_preflight(&mut state);
                    if let Some(descriptor_index) = state
                        .targets
                        .iter()
                        .position(|candidate| *candidate == target)
                    {
                        ui.set_target_index(descriptor_index as i32);
                    }
                }
            }
            refresh_queue(&ui, &state);
        });
    }
    {
        let weak = weak.clone();
        ui.on_language_selected(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            let locale = match index {
                1 => "en",
                2 => "zh-Hans",
                _ => system_translation(),
            };
            let _ = slint::select_bundled_translation(locale);
            ui.set_language_index(index.clamp(0, 2));
            refresh_localized_models(&ui);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_mode_selected(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                state.mode = match index {
                    0 => DegradationMode::Strict,
                    2 => DegradationMode::Readable,
                    _ => DegradationMode::Compatible,
                };
                state.options.degradation_mode = state.mode;
                invalidate_preflight(&mut state);
            }
            refresh_queue(&ui, &state);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_compression_selected(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                state.options.compression = if index == 1 {
                    folio_core::CompressionOption::PalmDoc
                } else {
                    folio_core::CompressionOption::None
                };
            }
            ui.set_compression_index(index.clamp(0, 1));
        });
    }
    {
        let state = Arc::clone(&state);
        ui.on_deterministic_selected(move |selected| {
            if let Ok(mut state) = state.lock() {
                state.options.deterministic = selected;
            }
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_batch_mode_selected(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                state.batch_mode = if index == 1 {
                    BatchMode::Strict
                } else {
                    BatchMode::BestEffort
                };
            }
            ui.set_batch_mode_index(index.clamp(0, 1));
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_text_mode_selected(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                state.options.text.mode = text_import_mode_from_index(index);
                invalidate_preflight(&mut state);
            }
            ui.set_text_mode_index(index.clamp(0, 3));
            refresh_queue(&ui, &state);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_paragraph_mode_selected(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                state.options.text.paragraph_mode = paragraph_mode_from_index(index);
                invalidate_preflight(&mut state);
            }
            ui.set_paragraph_mode_index(index.clamp(0, 4));
            refresh_queue(&ui, &state);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_text_encoding_selected(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                state.options.text.encoding_override = text_encoding_from_index(index);
                invalidate_preflight(&mut state);
            }
            ui.set_text_encoding_index(index.clamp(0, 7));
            refresh_queue(&ui, &state);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_text_title_changed(move |value| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                let value = value.to_string();
                state.options.text.title_override = (!value.trim().is_empty()).then_some(value);
                invalidate_preflight(&mut state);
            }
            refresh_queue(&ui, &state);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_text_author_changed(move |value| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                let value = value.to_string();
                state.options.text.author_override = (!value.trim().is_empty()).then_some(value);
                invalidate_preflight(&mut state);
            }
            refresh_queue(&ui, &state);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_degradation_option_selected(move |index, selected| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                match index {
                    0 => state.options.degradation.linearize_complex_tables = selected,
                    1 => state.options.degradation.prefer_rasterization = selected,
                    2 => state.options.degradation.strip_embedded_fonts = selected,
                    _ => return,
                }
                invalidate_preflight(&mut state);
            }
            refresh_queue(&ui, &state);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let reader_sender = reader_sender.clone();
        ui.on_reader_content_selected(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                state.reader_content_mode = match index {
                    1 => ReaderContentMode::Fill,
                    2 => ReaderContentMode::ActualSize,
                    _ => ReaderContentMode::Fit,
                };
                state.reader_viewport.content_mode = state.reader_content_mode;
                state.reader_viewport.scale = 1.0;
                state.reader_viewport.pan_x = 0.0;
                state.reader_viewport.pan_y = 0.0;
            }
            ui.set_reader_content_index(index.clamp(0, 2));
            ui.set_reader_zoom_percent(100);
            enqueue_reader_refresh(&ui, &state, &reader_sender, ReaderOperation::SetOptions);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let reader_sender = reader_sender.clone();
        ui.on_reader_direction_selected(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                state.reader_direction = if index == 1 {
                    ReaderDirection::RightToLeft
                } else {
                    ReaderDirection::LeftToRight
                };
            }
            ui.set_reader_direction_index(index.clamp(0, 1));
            enqueue_reader_refresh(&ui, &state, &reader_sender, ReaderOperation::SetOptions);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let reader_sender = reader_sender.clone();
        ui.on_reader_spread_selected(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                state.reader_spread_mode = if index == 1 {
                    ReaderSpreadMode::Synthetic
                } else {
                    ReaderSpreadMode::SinglePage
                };
            }
            ui.set_reader_spread_index(index.clamp(0, 1));
            enqueue_reader_refresh(&ui, &state, &reader_sender, ReaderOperation::SetOptions);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let reader_sender = reader_sender.clone();
        ui.on_reader_zoom_delta(move |delta| {
            let Some(ui) = weak.upgrade() else { return };
            if !delta.is_finite() {
                return;
            }
            let zoom_percent = if let Ok(mut state) = state.lock() {
                state.reader_viewport.scale =
                    (state.reader_viewport.scale + delta).clamp(0.25, 8.0);
                (state.reader_viewport.scale * 100.0).round() as i32
            } else {
                return;
            };
            ui.set_reader_zoom_percent(zoom_percent);
            enqueue_reader_refresh(&ui, &state, &reader_sender, ReaderOperation::SetOptions);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let reader_sender = reader_sender.clone();
        ui.on_reader_viewport_changed(move |width, height| {
            if width <= 1.0 || height <= 1.0 {
                return;
            }
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                state.reader_viewport.width = width;
                state.reader_viewport.height = height;
            }
            enqueue_reader_refresh(&ui, &state, &reader_sender, ReaderOperation::Resize);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let reader_sender = reader_sender.clone();
        ui.on_reader_previous(move || {
            let Some(ui) = weak.upgrade() else { return };
            enqueue_reader_refresh(&ui, &state, &reader_sender, ReaderOperation::Previous);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let reader_sender = reader_sender.clone();
        ui.on_reader_next(move || {
            let Some(ui) = weak.upgrade() else { return };
            enqueue_reader_refresh(&ui, &state, &reader_sender, ReaderOperation::Next);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let reader_sender = reader_sender.clone();
        ui.on_comic_previous(move || {
            let Some(ui) = weak.upgrade() else { return };
            enqueue_reader_refresh(&ui, &state, &reader_sender, ReaderOperation::Previous);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let reader_sender = reader_sender.clone();
        ui.on_comic_next(move || {
            let Some(ui) = weak.upgrade() else { return };
            enqueue_reader_refresh(&ui, &state, &reader_sender, ReaderOperation::Next);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_comic_target_selected(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                state.comic_target_index = index.max(0) as usize;
            }
            ui.set_comic_target_index(index.max(0));
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_reader_pan_start(move || {
            let Some(ui) = weak.upgrade() else { return };
            let _ = ui;
            if let Ok(mut state) = state.lock() {
                state.reader_drag_origin =
                    (state.reader_viewport.pan_x, state.reader_viewport.pan_y);
                state.reader_pan_last_dispatch = Instant::now() - Duration::from_secs(1);
            }
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let reader_sender = reader_sender.clone();
        ui.on_reader_pan(move |delta_x, delta_y| {
            let Some(ui) = weak.upgrade() else { return };
            if !delta_x.is_finite() || !delta_y.is_finite() {
                return;
            }
            let should_dispatch = if let Ok(mut state) = state.lock() {
                state.reader_viewport.pan_x =
                    (state.reader_drag_origin.0 + delta_x).clamp(-10_000.0, 10_000.0);
                state.reader_viewport.pan_y =
                    (state.reader_drag_origin.1 + delta_y).clamp(-10_000.0, 10_000.0);
                let now = Instant::now();
                if now.duration_since(state.reader_pan_last_dispatch) >= Duration::from_millis(40) {
                    state.reader_pan_last_dispatch = now;
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if should_dispatch {
                enqueue_reader_refresh(&ui, &state, &reader_sender, ReaderOperation::SetOptions);
            }
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let reader_sender = reader_sender.clone();
        ui.on_reader_pan_nudge(move |delta_x, delta_y| {
            let Some(ui) = weak.upgrade() else { return };
            if !delta_x.is_finite() || !delta_y.is_finite() {
                return;
            }
            if let Ok(mut state) = state.lock() {
                state.reader_viewport.pan_x =
                    (state.reader_viewport.pan_x + delta_x).clamp(-10_000.0, 10_000.0);
                state.reader_viewport.pan_y =
                    (state.reader_viewport.pan_y + delta_y).clamp(-10_000.0, 10_000.0);
            }
            enqueue_reader_refresh(&ui, &state, &reader_sender, ReaderOperation::SetOptions);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        let reader_sender = reader_sender.clone();
        ui.on_reader_pan_ended(move || {
            let Some(ui) = weak.upgrade() else { return };
            enqueue_reader_refresh(&ui, &state, &reader_sender, ReaderOperation::SetOptions);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_comic_convert(move || {
            let Some(ui) = weak.upgrade() else { return };
            convert_comic(&ui, &state);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_preflight_selected(move || {
            let Some(ui) = weak.upgrade() else { return };
            start_preflight(&ui, &state, false);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_preflight_all(move || {
            let Some(ui) = weak.upgrade() else { return };
            start_preflight(&ui, &state, true);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_convert_selected(move || {
            let Some(ui) = weak.upgrade() else { return };
            start_conversion(&ui, &state, false);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_convert_all_approved(move || {
            let Some(ui) = weak.upgrade() else { return };
            start_conversion(&ui, &state, true);
        });
    }
    {
        let state = Arc::clone(&state);
        let weak = weak.clone();
        ui.on_cancel_operation(move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(state) = state.lock() {
                if let Some(token) = &state.active_operation {
                    token.cancel();
                }
                ui.set_status_text(ui.get_l10n_cancelled());
            }
        });
    }
}

fn invalidate_preflight(state: &mut AppState) {
    state.settings_revision = state.settings_revision.wrapping_add(1);
    for record in &mut state.queue {
        record.preflight_revision = None;
        record.approved_revision = None;
        record.status = RowStatus::Ready;
    }
}

fn start_preflight(ui: &MainWindow, state: &Arc<Mutex<AppState>>, all: bool) {
    let (operation_id, revision, target, mode, degradation, jobs, cancellation) = {
        let Ok(mut state) = state.lock() else { return };
        if state.active_operation.is_some() || state.queue.is_empty() {
            return;
        }
        state.operation_id = state.operation_id.wrapping_add(1);
        let operation_id = state.operation_id;
        let indexes = if all {
            (0..state.queue.len()).collect::<Vec<_>>()
        } else {
            selected_queue_indices(&state)
        };
        if indexes.is_empty() {
            return;
        }
        let cancellation = CancellationToken::new();
        state.active_operation = Some(cancellation.clone());
        for index in &indexes {
            state.queue[*index].status = RowStatus::Checking;
        }
        (
            operation_id,
            state.settings_revision,
            state.target,
            state.mode,
            state.options.degradation.clone(),
            indexes
                .into_iter()
                .map(|index| {
                    (
                        index,
                        state.queue[index].input.source.clone(),
                        state.queue[index].input.edit.clone().unwrap_or_default(),
                    )
                })
                .collect::<Vec<_>>(),
            cancellation,
        )
    };
    ui.set_busy(true);
    ui.set_progress(0.0);
    ui.set_status_text(ui.get_l10n_checking());
    ui.set_diagnostics_text(SharedString::default());
    let weak = ui.as_weak();
    let state = Arc::clone(state);
    thread::spawn(move || {
        let mut results = Vec::with_capacity(jobs.len());
        let mut cancelled = false;
        for (index, path, edit) in jobs {
            if cancellation.is_cancelled() {
                cancelled = true;
                break;
            }
            let result = folio_core::preflight(&path, target, mode, &degradation, &edit)
                .map_err(|error| error.to_string());
            results.push((index, path, result));
        }
        cancelled |= cancellation.is_cancelled();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut detail = Vec::new();
            let mut blocked = 0usize;
            let completed_indexes = results
                .iter()
                .map(|(index, _, _)| *index)
                .collect::<HashSet<_>>();
            if let Ok(mut state) = state.lock() {
                if state.operation_id != operation_id {
                    return;
                }
                for (index, path, result) in results {
                    if let Some(record) = state.queue.get_mut(index) {
                        match result {
                            Ok(report) => {
                                let is_blocked = report.plan.blocked;
                                record.preflight_revision = Some(revision);
                                record.approved_revision = (!is_blocked).then_some(revision);
                                record.status = if is_blocked {
                                    RowStatus::Blocked
                                } else {
                                    RowStatus::Approved
                                };
                                if is_blocked {
                                    blocked += 1;
                                }
                                record.detail = diagnostic_detail(&report);
                                detail.push(format!("{}\n{}", path.display(), record.detail));
                            }
                            Err(error) => {
                                record.preflight_revision = Some(revision);
                                record.approved_revision = None;
                                record.status = RowStatus::Failed;
                                record.detail = error.clone();
                                detail.push(format!("{}\n{error}", path.display()));
                            }
                        }
                    }
                }
                if cancelled {
                    for (index, record) in state.queue.iter_mut().enumerate() {
                        if record.status == RowStatus::Checking
                            && !completed_indexes.contains(&index)
                        {
                            record.status = RowStatus::Cancelled;
                            record.preflight_revision = None;
                            record.approved_revision = None;
                        }
                    }
                }
                state.active_operation = None;
            }
            ui.set_busy(false);
            ui.set_progress(0.0);
            ui.set_status_text(if cancelled {
                ui.get_l10n_cancelled()
            } else if blocked == 0 {
                ui.get_l10n_completed()
            } else {
                ui.get_l10n_blocked()
            });
            ui.set_diagnostics_text(if cancelled {
                ui.get_l10n_cancelled()
            } else if blocked == 0 {
                ui.get_l10n_preflight_complete()
            } else {
                ui.get_l10n_preflight_blocked()
            });
            ui.set_detail_text(shared(detail.join("\n\n")));
            refresh_queue(&ui, &state);
        });
    });
}

fn start_conversion(ui: &MainWindow, state: &Arc<Mutex<AppState>>, all: bool) {
    let (operation_id, token, options, target, batch_mode, output_directory, inputs) =
        {
            let Ok(mut state) = state.lock() else { return };
            if state.active_operation.is_some() {
                return;
            }
            let selected = if all {
                (0..state.queue.len()).collect::<Vec<_>>()
            } else {
                selected_queue_indices(&state)
            };
            let preflighted = selected.iter().all(|index| {
                state.queue.get(*index).is_some_and(|record| {
                    record.preflight_revision == Some(state.settings_revision)
                })
            });
            if !preflighted {
                drop(state);
                ui.set_status_text(ui.get_l10n_blocked());
                ui.set_diagnostics_text(ui.get_l10n_preflight_required());
                return;
            }
            if state.batch_mode == BatchMode::Strict
                && selected.iter().any(|index| {
                    state.queue.get(*index).is_none_or(|record| {
                        record.approved_revision != Some(state.settings_revision)
                    })
                })
            {
                drop(state);
                ui.set_status_text(ui.get_l10n_blocked());
                ui.set_diagnostics_text(ui.get_l10n_strict_batch_required());
                return;
            }
            let eligible = selected
                .into_iter()
                .filter(|index| {
                    state.queue.get(*index).is_some_and(|record| {
                        record.approved_revision == Some(state.settings_revision)
                    })
                })
                .collect::<Vec<_>>();
            if eligible.is_empty() {
                drop(state);
                ui.set_status_text(ui.get_l10n_blocked());
                ui.set_diagnostics_text(ui.get_l10n_preflight_required());
                return;
            }
            state.operation_id = state.operation_id.wrapping_add(1);
            let operation_id = state.operation_id;
            let token = CancellationToken::new();
            state.active_operation = Some(token.clone());
            for index in &eligible {
                state.queue[*index].status = RowStatus::Converting;
            }
            let mut inputs = Vec::with_capacity(eligible.len());
            for index in eligible {
                let mut input = state.queue[index].input.clone();
                if let Some(output) = &state.output_directory {
                    input.output_root = Some(output.clone());
                }
                input.edit.get_or_insert_with(Default::default);
                inputs.push((index, input));
            }
            (
                operation_id,
                token,
                state.options.clone(),
                state.target,
                state.batch_mode,
                state.output_directory.clone(),
                inputs,
            )
        };
    let indexes = inputs.iter().map(|(index, _)| *index).collect::<Vec<_>>();
    let batch_inputs = inputs
        .into_iter()
        .map(|(_, input)| input)
        .collect::<Vec<_>>();
    let fallback_output = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let output_directory = output_directory.unwrap_or(fallback_output);
    ui.set_busy(true);
    ui.set_progress(0.0);
    ui.set_status_text(ui.get_l10n_converting());
    ui.set_diagnostics_text(SharedString::default());
    let weak = ui.as_weak();
    let state = Arc::clone(state);
    let progress_token = token.clone();
    thread::spawn(move || {
        let options = BatchOptions {
            target,
            conversion: options,
            batch_mode,
            collision_policy: CollisionPolicy::Rename,
            preserve_tree: true,
            max_concurrent_jobs: 0,
            per_job_parallelism: 1,
            memory_budget_bytes: None,
            fail_fast: false,
            edit: folio_core::BookEditPlan::default(),
        };
        let result = folio_batch::convert_items_with_cancellation(
            &batch_inputs,
            &output_directory,
            &options,
            &token,
            |current, total, event| {
                let fraction = event.fraction.unwrap_or(0.0).clamp(0.0, 1.0);
                let progress = (((current.saturating_sub(1)) as f32 + fraction)
                    / total.max(1) as f32)
                    .clamp(0.0, 1.0);
                let event_current = event.current.min(i32::MAX as u64) as i32;
                let event_total = event.total.unwrap_or_default().min(i32::MAX as u64) as i32;
                let stage = event.stage;
                let weak = weak.clone();
                let state = Arc::clone(&state);
                let indexes = indexes.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    let Ok(mut state) = state.lock() else { return };
                    if state.operation_id != operation_id {
                        return;
                    }
                    ui.set_progress(progress);
                    ui.set_progress_current(event_current);
                    ui.set_progress_total(event_total);
                    if let Some(index) = indexes.get(current.saturating_sub(1)) {
                        if let Some(record) = state.queue.get_mut(*index) {
                            record.status = RowStatus::Converting;
                        }
                    }
                    ui.set_status_text(progress_stage_label(&ui, stage));
                });
            },
        );
        let _ = progress_token;
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut details = Vec::new();
            let mut succeeded = 0usize;
            let mut failed = 0usize;
            if let Ok(mut state) = state.lock() {
                if state.operation_id != operation_id {
                    return;
                }
                match result {
                    Ok(report) => {
                        succeeded = report.succeeded;
                        failed = report.failed;
                        for (batch_item, queue_index) in report.items.iter().zip(indexes.iter()) {
                            if let Some(record) = state.queue.get_mut(*queue_index) {
                                record.status = if batch_item.success {
                                    RowStatus::Completed
                                } else if batch_item.error.as_deref().is_some_and(|error| {
                                    error.to_ascii_lowercase().contains("cancel")
                                }) {
                                    RowStatus::Cancelled
                                } else {
                                    RowStatus::Failed
                                };
                                record.detail = batch_item
                                    .output
                                    .as_ref()
                                    .map(|path| path.display().to_string())
                                    .or_else(|| batch_item.error.clone())
                                    .unwrap_or_default();
                                details.push(format!(
                                    "{}\n{}",
                                    batch_item.source.display(),
                                    record.detail
                                ));
                            }
                        }
                    }
                    Err(error) => {
                        failed = indexes.len();
                        for index in &indexes {
                            if let Some(record) = state.queue.get_mut(*index) {
                                record.status =
                                    if error.to_string().to_ascii_lowercase().contains("cancel") {
                                        RowStatus::Cancelled
                                    } else {
                                        RowStatus::Failed
                                    };
                                record.detail = error.to_string();
                            }
                        }
                        details.push(error.to_string());
                    }
                }
                state.active_operation = None;
            }
            ui.set_busy(false);
            ui.set_progress(if failed == 0 { 1.0 } else { 0.0 });
            ui.set_status_text(if failed == 0 {
                ui.get_l10n_completed()
            } else if progress_token.is_cancelled() {
                ui.get_l10n_cancelled()
            } else {
                ui.get_l10n_failed()
            });
            ui.set_conversion_succeeded(succeeded.min(i32::MAX as usize) as i32);
            ui.set_conversion_failed(failed.min(i32::MAX as usize) as i32);
            ui.set_diagnostics_text(ui.get_l10n_conversion_count());
            ui.set_detail_text(shared(details.join("\n\n")));
            refresh_queue(&ui, &state);
        });
    });
}

fn open_comic_source(
    ui: &MainWindow,
    state: &Arc<Mutex<AppState>>,
    reader_sender: &mpsc::Sender<ReaderTask>,
    active_reader: &Arc<Mutex<Option<String>>>,
    active_comic: &Arc<Mutex<Option<String>>>,
    path: PathBuf,
) {
    let reader_sender_ui = reader_sender.clone();
    let active_reader_ui = Arc::clone(active_reader);
    let active_comic_ui = Arc::clone(active_comic);
    let (generation, viewport, direction) = {
        let Ok(mut state) = state.lock() else { return };
        state.reader_operation_id = state.reader_operation_id.wrapping_add(1);
        state.reader_session_id = None;
        state.reader_is_comic = true;
        state.comic_pages.clear();
        state.comic_source_path = Some(path.clone());
        (
            state.reader_operation_id,
            reader_viewport(&state),
            Some(state.reader_direction),
        )
    };
    ui.set_busy(true);
    ui.set_progress(0.0);
    ui.set_status_text(ui.get_l10n_checking());
    ui.set_active_workspace(2);
    ui.set_reader_pages(model(Vec::<ReaderDisplayPage>::new()));
    ui.set_reader_page_count(0);
    if let Ok(mut state) = state.lock() {
        if let Some(cancellation) = state.comic_thumbnail_cancellation.take() {
            cancellation.cancel();
        }
        state.comic_thumbnail_generation = state.comic_thumbnail_generation.wrapping_add(1);
        state.comic_session_id = None;
        state.comic_pages.clear();
        state.comic_thumbnail_cache.clear();
        state.comic_thumbnail_cache_order.clear();
        state.comic_thumbnail_cache_bytes = 0;
    }
    let weak = ui.as_weak();
    let state = Arc::clone(state);
    let active_reader_worker = Arc::clone(active_reader);
    let active_comic_worker = Arc::clone(active_comic);
    let input_path = path.clone();
    let task: ReaderTask = Box::new(move || {
        close_active_reader(&active_reader_worker);
        let old_comic = active_comic_worker
            .lock()
            .ok()
            .and_then(|mut value| value.take());
        if let Some(old) = old_comic {
            let _ = folio_core::comic_close(&old);
        }
        let cancellation = CancellationToken::new();
        let result = (|| {
            let summary = folio_core::comic_open_with_cancellation(&input_path, &cancellation)
                .map_err(|error| error.to_string())?;
            let pages =
                folio_core::comic_pages(&summary.session_id).map_err(|error| error.to_string())?;
            let targets = folio_core::comic_conversion_options().targets;
            let reader = folio_core::reader_open_comic(&ReaderOpenComicRequest {
                comic_session_id: summary.session_id.clone(),
                viewport,
                direction,
            })
            .map_err(|error| error.to_string());
            Ok::<_, String>((summary, pages, targets, reader))
        })();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_busy(false);
            match result {
                Ok((summary, pages, targets, reader_result)) => {
                    let target_names = targets
                        .iter()
                        .map(|target| shared(target.display_name.clone()))
                        .collect::<Vec<_>>();
                    let target_ids = targets
                        .iter()
                        .map(|target| target.id.clone())
                        .collect::<Vec<_>>();
                    let target_extensions = targets
                        .iter()
                        .map(|target| target.extension.clone())
                        .collect::<Vec<_>>();
                    if let Ok(mut state) = state.lock() {
                        if state.reader_operation_id != generation {
                            return;
                        }
                        state.comic_session_id = Some(summary.session_id.clone());
                        state.comic_pages = pages.clone();
                        state.selected_comic_index = if pages.is_empty() { -1 } else { 0 };
                        state.comic_thumbnail_cache.clear();
                        state.comic_thumbnail_cache_order.clear();
                        state.comic_thumbnail_cache_bytes = 0;
                        state.comic_target_names = targets
                            .iter()
                            .map(|target| target.display_name.clone())
                            .collect();
                        state.comic_target_ids = target_ids;
                        state.comic_target_extensions = target_extensions;
                        state.comic_target_index = 0;
                    }
                    let rows = comic_page_rows(&pages, 0, &HashMap::new());
                    ui.set_comic_page_rows(model(rows));
                    ui.set_selected_page_index(if pages.is_empty() { -1 } else { 0 });
                    ui.set_comic_target_options(model(target_names));
                    ui.set_comic_target_index(0);
                    ui.set_comic_title(shared(summary.title.clone()));
                    ui.set_comic_source_type(shared(summary.source_type.clone()));
                    ui.set_comic_page_count(summary.page_count as i32);
                    ui.set_comic_authors(shared(summary.authors.join(", ")));
                    ui.set_comic_reading_direction(shared(
                        summary.reading_direction.clone().unwrap_or_default(),
                    ));
                    ui.set_comic_output(shared(
                        folio_core::comic_conversion_options().notes.join(" · "),
                    ));
                    if !pages.is_empty() {
                        enqueue_comic_thumbnails(&ui, &state, 0);
                    }
                    let target_id = summary.session_id.clone();
                    let _ = active_comic_ui
                        .lock()
                        .map(|mut active| *active = Some(target_id.clone()));
                    match reader_result {
                        Ok(reader) => {
                            let reader_id = reader.session_id.clone();
                            let _ = active_reader_ui
                                .lock()
                                .map(|mut active| *active = Some(reader_id.clone()));
                            let weak = ui.as_weak();
                            let reader_session_id = reader.session_id;
                            let generation = generation;
                            let state_for_render = Arc::clone(&state);
                            let task: ReaderTask = Box::new(move || {
                                let result =
                                    render_reader(&reader_session_id, &CancellationToken::new());
                                let _ = slint::invoke_from_event_loop(move || {
                                    let Some(ui) = weak.upgrade() else { return };
                                    match result {
                                        Ok((images, summary)) => update_reader_view(
                                            &ui,
                                            &state_for_render,
                                            generation,
                                            images,
                                            summary,
                                            true,
                                        ),
                                        Err(error) => show_reader_failure(&ui, error),
                                    }
                                });
                            });
                            let _ = state
                                .lock()
                                .map(|mut current| current.reader_session_id = Some(reader_id));
                            // The worker owns all Reader decoding so it never blocks Slint's event loop.
                            let _ = reader_sender_ui.send(task);
                        }
                        Err(error) => {
                            ui.set_diagnostics_text(ui.get_l10n_reader_unavailable());
                            ui.set_detail_text(shared(error));
                        }
                    }
                    ui.set_status_text(ui.get_l10n_completed());
                    ui.set_busy(false);
                    let _ = active_comic_worker;
                }
                Err(error) => {
                    ui.set_busy(false);
                    ui.set_status_text(ui.get_l10n_failed());
                    ui.set_diagnostics_text(ui.get_l10n_error());
                    ui.set_detail_text(shared(error));
                }
            }
        });
    });
    let _ = reader_sender.send(task);
}

fn convert_comic(ui: &MainWindow, state: &Arc<Mutex<AppState>>) {
    let (session_id, target_id, extension) = {
        let Ok(state) = state.lock() else { return };
        let Some(session) = state.comic_session_id.clone() else {
            ui.set_status_text(ui.get_l10n_select_source());
            return;
        };
        let index = state.comic_target_index;
        let Some(target) = state.comic_target_ids.get(index) else {
            return;
        };
        let Some(extension) = state.comic_target_extensions.get(index) else {
            return;
        };
        (session, target.clone(), extension.clone())
    };
    let mut dialog = FileDialog::new().set_title(ui.get_l10n_save_comic().to_string());
    if let Some(source) = state
        .lock()
        .ok()
        .and_then(|state| state.comic_source_path.clone())
    {
        dialog = dialog.set_directory(source.parent().unwrap_or_else(|| Path::new(".")));
        dialog = dialog.set_file_name(
            source
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("comic")
                .to_owned()
                + "."
                + &extension,
        );
    }
    let Some(output) = dialog
        .add_filter(
            ui.get_l10n_comic_output().to_string(),
            &[extension.as_str()],
        )
        .save_file()
    else {
        return;
    };
    let Some(core_target) = folio_core::comic_conversion_options()
        .targets
        .iter()
        .find(|target| target.id == target_id)
        .and_then(|target| match target.id.as_str() {
            "CBZ" => Some(folio_core::ComicOutputTarget::Cbz),
            _ => None,
        })
    else {
        ui.set_status_text(ui.get_l10n_error());
        ui.set_detail_text(shared(format!(
            "Unsupported Core comic target: {target_id}"
        )));
        return;
    };
    let (operation_id, token) = {
        let Ok(mut state) = state.lock() else { return };
        if state.active_operation.is_some() {
            return;
        }
        state.operation_id = state.operation_id.wrapping_add(1);
        let token = CancellationToken::new();
        state.active_operation = Some(token.clone());
        (state.operation_id, token)
    };
    let request = ComicConversionRequest {
        session_id,
        target: core_target,
        output,
    };
    ui.set_busy(true);
    ui.set_progress(0.0);
    ui.set_progress_current(0);
    ui.set_progress_total(0);
    ui.set_status_text(ui.get_l10n_converting());
    ui.set_diagnostics_text(SharedString::default());
    let weak = ui.as_weak();
    let state = Arc::clone(state);
    let progress_token = token.clone();
    thread::spawn(move || {
        let progress_weak = weak.clone();
        let progress_state = Arc::clone(&state);
        let result = folio_core::comic_convert_with_progress(&request, &token, move |event| {
            let fraction = event
                .fraction
                .unwrap_or_else(|| {
                    event.total.map_or(0.0, |total| {
                        if total == 0 {
                            0.0
                        } else {
                            event.current as f32 / total as f32
                        }
                    })
                })
                .clamp(0.0, 1.0);
            let current = event.current.min(i32::MAX as u64) as i32;
            let total = event.total.unwrap_or_default().min(i32::MAX as u64) as i32;
            let stage = event.stage;
            let weak = progress_weak.clone();
            let state = Arc::clone(&progress_state);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else { return };
                let Ok(state) = state.lock() else { return };
                if state.operation_id != operation_id {
                    return;
                }
                ui.set_progress(fraction);
                ui.set_progress_current(current);
                ui.set_progress_total(total);
                ui.set_status_text(progress_stage_label(&ui, stage));
            });
        })
        .map_err(|error| error.to_string());
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(mut state) = state.lock() {
                if state.operation_id != operation_id {
                    return;
                }
                state.active_operation = None;
            }
            ui.set_busy(false);
            match result {
                Ok(report) => {
                    ui.set_progress(1.0);
                    ui.set_status_text(ui.get_l10n_completed());
                    ui.set_diagnostics_text(shared(report.target));
                    let warnings = report
                        .warnings
                        .iter()
                        .map(|warning| {
                            format!(
                                "{} [{}] {}",
                                warning.code, warning.severity, warning.message
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    ui.set_detail_text(shared(format!(
                        "{} · {} bytes · {} ms · {} pages\n{}",
                        report.output_path.display(),
                        report.output_size,
                        report.duration_ms,
                        report.page_count,
                        warnings
                    )));
                }
                Err(error) => {
                    ui.set_status_text(if progress_token.is_cancelled() {
                        ui.get_l10n_cancelled()
                    } else {
                        ui.get_l10n_failed()
                    });
                    ui.set_diagnostics_text(ui.get_l10n_error());
                    ui.set_detail_text(shared(error));
                }
            }
        });
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let system_locale = system_translation();
    let _ = slint::select_bundled_translation(system_locale);
    let ui = MainWindow::new()?;
    let (reader_sender, reader_receiver) = mpsc::channel::<ReaderTask>();
    thread::Builder::new()
        .name("folioforge-reader-adapter".to_owned())
        .spawn(move || {
            while let Ok(task) = reader_receiver.recv() {
                task();
            }
        })?;
    let state = Arc::new(Mutex::new(AppState {
        queue: Vec::new(),
        selected_index: -1,
        selected_sources: HashSet::new(),
        selected_comic_index: -1,
        input_extensions: Vec::new(),
        output_descriptors: Vec::new(),
        targets: Vec::new(),
        target: Target::EPUB,
        mode: DegradationMode::Compatible,
        batch_mode: BatchMode::BestEffort,
        options: ConversionOptions::default(),
        settings_revision: 0,
        output_directory: None,
        active_operation: None,
        operation_id: 0,
        reader_operation_id: 0,
        reader_session_id: None,
        reader_is_comic: false,
        reader_viewport: ReaderViewportRequest {
            width: 800.0,
            height: 600.0,
            scale: 1.0,
            content_mode: ReaderContentMode::Fit,
            pan_x: 0.0,
            pan_y: 0.0,
        },
        reader_drag_origin: (0.0, 0.0),
        reader_pan_last_dispatch: Instant::now() - Duration::from_secs(1),
        reader_content_mode: ReaderContentMode::Fit,
        reader_direction: ReaderDirection::LeftToRight,
        reader_spread_mode: ReaderSpreadMode::SinglePage,
        comic_session_id: None,
        comic_source_path: None,
        comic_pages: Vec::new(),
        comic_thumbnail_generation: 0,
        comic_thumbnail_cancellation: None,
        comic_thumbnail_cache: HashMap::new(),
        comic_thumbnail_cache_order: VecDeque::new(),
        comic_thumbnail_cache_bytes: 0,
        comic_target_ids: Vec::new(),
        comic_target_names: Vec::new(),
        comic_target_extensions: Vec::new(),
        comic_target_index: 0,
    }));
    let active_reader = Arc::new(Mutex::new(None));
    let active_comic = Arc::new(Mutex::new(None));
    initialize_window(&ui, &state, &active_reader, &active_comic);
    install_callbacks(
        &ui,
        Arc::clone(&state),
        reader_sender,
        active_reader,
        active_comic,
    );
    ui.run()?;
    Ok(())
}
