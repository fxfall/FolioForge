//! Core-owned lifecycle and FFI-safe projections for the format-blind Reader.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
};

use base64::Engine as _;
use folio_model::{Book, Diagnostic, DocumentId, ResourceId};
use folio_reader::{
    FixedPageRenderModel, FixedSpreadRenderModel, ReaderContentMode, ReaderDirection, ReaderError,
    ReaderErrorCode, ReaderLocation, ReaderNavigationSnapshot, ReaderNavigationTarget, ReaderRect,
    ReaderSession, ReaderSize, ReaderSpreadMode, ReaderViewport,
};
use serde::{Deserialize, Serialize};

use crate::{CancellationToken, CoreError, FormatRegistry};

const MAX_READER_TRANSPORT_BYTES: usize = 64 * 1024 * 1024;

static READER_SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);
static READER_SESSIONS: OnceLock<Mutex<HashMap<String, Arc<ReaderSessionEntry>>>> = OnceLock::new();

struct ReaderSessionEntry {
    session: Mutex<ReaderSession>,
    title: Option<String>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderViewportRequest {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub content_mode: ReaderContentMode,
    #[serde(default)]
    pub pan_x: f32,
    #[serde(default)]
    pub pan_y: f32,
}

impl ReaderViewportRequest {
    fn build(&self) -> Result<ReaderViewport, ReaderError> {
        ReaderViewport::new(self.width, self.height, self.scale, self.content_mode)?
            .with_pan(self.pan_x, self.pan_y)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderOpenRequest {
    pub input: PathBuf,
    pub viewport: ReaderViewportRequest,
    #[serde(default)]
    pub direction: Option<ReaderDirection>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderOpenComicRequest {
    pub comic_session_id: String,
    pub viewport: ReaderViewportRequest,
    #[serde(default)]
    pub direction: Option<ReaderDirection>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderSessionSummary {
    pub session_id: String,
    pub title: Option<String>,
    pub layout_mode: folio_model::LayoutMode,
    pub direction: ReaderDirection,
    pub spread_mode: ReaderSpreadMode,
    pub current_location: ReaderLocation,
    /// Position in the immutable IR document vector, distinct from the
    /// direction-aware Reader navigation index.
    pub current_document_index: usize,
    pub current_index: usize,
    pub page_count: usize,
    pub viewport: ReaderViewportRequest,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderResourceDto {
    pub resource_id: ResourceId,
    pub media_type: String,
    pub data_base64: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderImagePlacementDto {
    pub resource: ReaderResourceDto,
    pub destination: ReaderRect,
    pub clipping_rect: ReaderRect,
    pub clips_to_viewport: bool,
    pub alt_text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderPageModelDto {
    pub location: ReaderLocation,
    pub page_index: usize,
    pub document_index: usize,
    pub page_count: usize,
    pub page_size: ReaderSize,
    pub viewport_size: ReaderSize,
    pub page_side: Option<folio_model::PageSide>,
    pub spread_hint: folio_model::SpreadHint,
    pub placements: Vec<ReaderImagePlacementDto>,
    pub diagnostics: Vec<folio_reader::ReaderDiagnostic>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderSpreadPageDto {
    pub location: ReaderLocation,
    pub page_index: usize,
    pub page_side: folio_model::PageSide,
    pub spread_hint: folio_model::SpreadHint,
    pub page_size: ReaderSize,
    pub viewport_rect: ReaderRect,
    pub placements: Vec<ReaderImagePlacementDto>,
    pub diagnostics: Vec<folio_reader::ReaderDiagnostic>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderSpreadModelDto {
    pub current_location: ReaderLocation,
    pub page_count: usize,
    pub direction: ReaderDirection,
    pub viewport_size: ReaderSize,
    pub pages: Vec<ReaderSpreadPageDto>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderNavigationResult {
    pub location: ReaderLocation,
    pub moved: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderGoToRequest {
    pub session_id: String,
    pub location: ReaderLocation,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderGoToDocumentIndexRequest {
    pub session_id: String,
    pub document_index: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderGoToPageRequest {
    pub session_id: String,
    pub document_id: DocumentId,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderGoToNavigationTargetRequest {
    pub session_id: String,
    pub target: ReaderNavigationTarget,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderSetViewportRequest {
    pub session_id: String,
    pub viewport: ReaderViewportRequest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderSetDirectionRequest {
    pub session_id: String,
    pub direction: ReaderDirection,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderSetSpreadModeRequest {
    pub session_id: String,
    pub spread_mode: ReaderSpreadMode,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderResourceRequest {
    pub session_id: String,
    pub resource_id: ResourceId,
}

pub fn reader_open(request: &ReaderOpenRequest) -> Result<ReaderSessionSummary, CoreError> {
    reader_open_with_cancellation(request, &CancellationToken::new())
}

/// Create a Reader session from the exact IR projection already owned by an
/// open Comic Core session. This avoids a second importer pass and keeps
/// format-specific page/resource lookup out of the GUI.
pub fn reader_open_comic(
    request: &ReaderOpenComicRequest,
) -> Result<ReaderSessionSummary, CoreError> {
    let comic = super::comic_session(&request.comic_session_id)?;
    reader_open_ir(
        Arc::new(comic.projection.book.clone()),
        request.viewport.clone(),
        request.direction,
        comic.projection.diagnostics.clone(),
    )
}

pub fn reader_open_with_cancellation(
    request: &ReaderOpenRequest,
    cancellation: &CancellationToken,
) -> Result<ReaderSessionSummary, CoreError> {
    cancellation.check()?;
    crate::ensure_input(&request.input)?;

    let (book, diagnostics) = match FormatRegistry.import_path(&request.input) {
        Ok(imported) => {
            cancellation.check()?;
            (Arc::new(imported.book), imported.diagnostics)
        }
        Err(_) if request.input.is_dir() || is_comic_archive(&request.input) => {
            comic_book(&request.input, cancellation)?
        }
        Err(error) => return Err(error),
    };
    cancellation.check()?;
    open_book_session(
        book,
        request.viewport.build()?,
        request.direction,
        diagnostics,
    )
}

/// Open a book already owned by another Core workflow. This lets a Comic GUI
/// create a transient Reader session from its existing IR without re-reading
/// the selected source path.
pub fn reader_open_ir(
    book: Arc<Book>,
    viewport: ReaderViewportRequest,
    direction: Option<ReaderDirection>,
    diagnostics: Vec<Diagnostic>,
) -> Result<ReaderSessionSummary, CoreError> {
    open_book_session(book, viewport.build()?, direction, diagnostics)
}

fn open_book_session(
    book: Arc<Book>,
    viewport: ReaderViewport,
    direction: Option<ReaderDirection>,
    diagnostics: Vec<Diagnostic>,
) -> Result<ReaderSessionSummary, CoreError> {
    let validation = folio_validator::validate_book(&book);
    if !validation.is_valid() {
        return Err(CoreError::ValidationFailed(validation.diagnostics));
    }
    let title = book.metadata.title.clone();
    let session = if let Some(direction) = direction {
        ReaderSession::open_with_direction(book, viewport, direction)?
    } else {
        ReaderSession::open(book, viewport)?
    };
    let sequence = READER_SESSION_COUNTER
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| CoreError::ReaderSessionIdExhausted)?;
    let session_id = format!("reader-session-{sequence:016x}");
    let entry = Arc::new(ReaderSessionEntry {
        session: Mutex::new(session),
        title,
        diagnostics,
    });
    reader_sessions()
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?
        .insert(session_id.clone(), Arc::clone(&entry));
    reader_summary_for(&session_id, &entry)
}

pub(super) fn comic_book(
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<(Arc<Book>, Vec<Diagnostic>), CoreError> {
    cancellation.check()?;
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(folio_comic::ComicImportError::UnsafePath(
            "the selected source root is a symbolic link".to_owned(),
        )
        .into());
    }
    let stable_key = path.canonicalize()?.to_string_lossy().into_owned();
    let importer =
        folio_comic::ComicSourceImporter::new(folio_comic::ComicImportLimits::default())?;
    let imported = importer
        .import_path_with_cancel(path, &stable_key, || cancellation.is_cancelled())
        .map_err(|error| match error {
            folio_comic::ComicImportError::Cancelled => CoreError::Cancelled,
            other => CoreError::ComicImport(other),
        })?;
    cancellation.check()?;
    let projection = imported.to_semantic_ir()?;
    Ok((Arc::new(projection.book), projection.diagnostics))
}

fn is_comic_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cbz") || extension.eq_ignore_ascii_case("zip")
        })
}

pub fn reader_close(session_id: &str) -> Result<bool, CoreError> {
    Ok(reader_sessions()
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?
        .remove(session_id)
        .is_some())
}

pub fn reader_summary(session_id: &str) -> Result<ReaderSessionSummary, CoreError> {
    let entry = reader_session(session_id)?;
    reader_summary_for(session_id, &entry)
}

pub fn reader_current_page(session_id: &str) -> Result<ReaderPageModelDto, CoreError> {
    reader_current_page_with_cancellation(session_id, &CancellationToken::new())
}

pub fn reader_current_page_with_cancellation(
    session_id: &str,
    cancellation: &CancellationToken,
) -> Result<ReaderPageModelDto, CoreError> {
    cancellation.check()?;
    let entry = reader_session(session_id)?;
    let session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    let model = session
        .fixed_page_with_cancellation(session.current_location(), || cancellation.is_cancelled())?;
    let document_index = session.current_document_index();
    cancellation.check()?;
    page_model_dto(model, document_index)
}

pub fn reader_current_spread(session_id: &str) -> Result<ReaderSpreadModelDto, CoreError> {
    let entry = reader_session(session_id)?;
    let session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    let model = session.fixed_spread()?;
    spread_model_dto(model)
}

pub fn reader_resource(request: &ReaderResourceRequest) -> Result<ReaderResourceDto, CoreError> {
    let entry = reader_session(&request.session_id)?;
    let session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    let handle = session.resource_handle(request.resource_id)?;
    let resource = session.resolve_resource(handle)?;
    resource_dto(request.resource_id, resource.media_type(), resource.bytes())
}

pub fn reader_next(session_id: &str) -> Result<ReaderNavigationResult, CoreError> {
    move_page(session_id, true)
}

pub fn reader_previous(session_id: &str) -> Result<ReaderNavigationResult, CoreError> {
    move_page(session_id, false)
}

pub fn reader_first(session_id: &str) -> Result<ReaderNavigationResult, CoreError> {
    move_to_boundary(session_id, true)
}

pub fn reader_last(session_id: &str) -> Result<ReaderNavigationResult, CoreError> {
    move_to_boundary(session_id, false)
}

fn move_to_boundary(session_id: &str, first: bool) -> Result<ReaderNavigationResult, CoreError> {
    let entry = reader_session(session_id)?;
    let mut session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    let previous = session.current_location();
    let location = if first {
        session.first()
    } else {
        session.last()
    };
    Ok(ReaderNavigationResult {
        location,
        moved: location != previous,
    })
}

pub fn reader_navigation(session_id: &str) -> Result<ReaderNavigationSnapshot, CoreError> {
    let entry = reader_session(session_id)?;
    let session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    Ok(session.navigation())
}

fn move_page(session_id: &str, forward: bool) -> Result<ReaderNavigationResult, CoreError> {
    let entry = reader_session(session_id)?;
    let mut session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    let moved = if forward {
        session.next().is_some()
    } else {
        session.previous().is_some()
    };
    Ok(ReaderNavigationResult {
        location: session.current_location(),
        moved,
    })
}

pub fn reader_go_to(request: &ReaderGoToRequest) -> Result<ReaderLocation, CoreError> {
    let entry = reader_session(&request.session_id)?;
    let mut session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    Ok(session.go_to(request.location)?)
}

pub fn reader_go_to_document_index(
    request: &ReaderGoToDocumentIndexRequest,
) -> Result<ReaderLocation, CoreError> {
    let entry = reader_session(&request.session_id)?;
    let mut session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    Ok(session.go_to_document_index(request.document_index)?)
}

pub fn reader_go_to_page(request: &ReaderGoToPageRequest) -> Result<ReaderLocation, CoreError> {
    let entry = reader_session(&request.session_id)?;
    let mut session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    Ok(session.go_to_page(request.document_id)?)
}

pub fn reader_go_to_navigation_target(
    request: &ReaderGoToNavigationTargetRequest,
) -> Result<ReaderLocation, CoreError> {
    let entry = reader_session(&request.session_id)?;
    let mut session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    Ok(session.go_to_navigation_target(&request.target)?)
}

pub fn reader_set_viewport(
    request: &ReaderSetViewportRequest,
) -> Result<ReaderSessionSummary, CoreError> {
    let entry = reader_session(&request.session_id)?;
    let mut session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    session.set_viewport(request.viewport.build()?);
    drop(session);
    reader_summary_for(&request.session_id, &entry)
}

pub fn reader_set_direction(
    request: &ReaderSetDirectionRequest,
) -> Result<ReaderSessionSummary, CoreError> {
    let entry = reader_session(&request.session_id)?;
    let mut session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    session.set_direction(request.direction);
    drop(session);
    reader_summary_for(&request.session_id, &entry)
}

pub fn reader_set_spread_mode(
    request: &ReaderSetSpreadModeRequest,
) -> Result<ReaderSessionSummary, CoreError> {
    let entry = reader_session(&request.session_id)?;
    let mut session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    session.set_spread_mode(request.spread_mode);
    drop(session);
    reader_summary_for(&request.session_id, &entry)
}

pub(super) fn page_model_dto(
    model: FixedPageRenderModel,
    document_index: usize,
) -> Result<ReaderPageModelDto, CoreError> {
    let placements = model
        .placements
        .into_iter()
        .map(|placement| {
            let resource_id = placement.resource.handle().resource_id();
            let resource = resource_dto(
                resource_id,
                placement.resource.media_type(),
                placement.resource.bytes(),
            )?;
            Ok(ReaderImagePlacementDto {
                resource,
                destination: placement.destination,
                clipping_rect: placement.clipping_rect,
                clips_to_viewport: placement.clips_to_viewport,
                alt_text: placement.alt_text,
            })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    Ok(ReaderPageModelDto {
        location: model.location,
        page_index: model.page_index,
        document_index,
        page_count: model.page_count,
        page_size: model.page_size,
        viewport_size: model.viewport_size,
        page_side: model.page_side,
        spread_hint: model.spread_hint,
        placements,
        diagnostics: model.diagnostics,
    })
}

fn spread_model_dto(model: FixedSpreadRenderModel) -> Result<ReaderSpreadModelDto, CoreError> {
    let mut pages = Vec::with_capacity(model.pages.len());
    for page in model.pages {
        let mut placements = Vec::with_capacity(page.placements.len());
        for placement in page.placements {
            let resource_id = placement.resource.handle().resource_id();
            let resource = resource_dto(
                resource_id,
                placement.resource.media_type(),
                placement.resource.bytes(),
            )?;
            placements.push(ReaderImagePlacementDto {
                resource,
                destination: placement.destination,
                clipping_rect: placement.clipping_rect,
                clips_to_viewport: placement.clips_to_viewport,
                alt_text: placement.alt_text,
            });
        }
        pages.push(ReaderSpreadPageDto {
            location: page.location,
            page_index: page.page_index,
            page_side: page.page_side,
            spread_hint: page.spread_hint,
            page_size: page.page_size,
            viewport_rect: page.viewport_rect,
            placements,
            diagnostics: page.diagnostics,
        });
    }
    Ok(ReaderSpreadModelDto {
        current_location: model.current_location,
        page_count: model.page_count,
        direction: model.direction,
        viewport_size: model.viewport_size,
        pages,
    })
}

fn resource_dto(
    resource_id: ResourceId,
    media_type: &str,
    bytes: &[u8],
) -> Result<ReaderResourceDto, CoreError> {
    if bytes.len() > MAX_READER_TRANSPORT_BYTES {
        return Err(CoreError::Reader(ReaderError {
            code: ReaderErrorCode::ResourceTooLarge,
            resource_id: Some(resource_id),
        }));
    }
    Ok(ReaderResourceDto {
        resource_id,
        media_type: media_type.to_owned(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

fn reader_summary_for(
    session_id: &str,
    entry: &ReaderSessionEntry,
) -> Result<ReaderSessionSummary, CoreError> {
    let session = entry
        .session
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?;
    let viewport = session.viewport();
    Ok(ReaderSessionSummary {
        session_id: session_id.to_owned(),
        title: entry.title.clone(),
        layout_mode: session.layout_mode(),
        direction: session.direction(),
        spread_mode: session.spread_mode(),
        current_location: session.current_location(),
        current_document_index: session.current_document_index(),
        current_index: session.current_index(),
        page_count: session.page_count(),
        viewport: ReaderViewportRequest {
            width: viewport.width(),
            height: viewport.height(),
            scale: viewport.scale(),
            content_mode: viewport.content_mode(),
            pan_x: viewport.pan_x(),
            pan_y: viewport.pan_y(),
        },
        diagnostics: entry.diagnostics.clone(),
    })
}

fn reader_session(session_id: &str) -> Result<Arc<ReaderSessionEntry>, CoreError> {
    reader_sessions()
        .lock()
        .map_err(|_| CoreError::ReaderSessionRegistryUnavailable)?
        .get(session_id)
        .cloned()
        .ok_or_else(|| CoreError::ReaderSessionNotFound(session_id.to_owned()))
}

fn reader_sessions() -> &'static Mutex<HashMap<String, Arc<ReaderSessionEntry>>> {
    READER_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}
