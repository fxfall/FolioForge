//! Minimal C ABI surface for Swift and other native clients.
//!
//! Ownership rule: every returned `FolioResult*` and JSON string is allocated
//! by Rust and released only by the matching `folio_result_free` or
//! `folio_string_free` function.  Swift must not call `free` on Rust memory.

#![deny(unsafe_op_in_unsafe_fn)]

use std::{
    ffi::{CStr, CString},
    os::raw::{c_char, c_void},
    path::PathBuf,
    ptr,
};

use base64::Engine as _;
use folio_core::{
    CancellationToken, ConversionRequest, CoreError, DegradationMode, DegradationOptions,
    ProgressEvent, Target,
};

const VERSION: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();

#[repr(C)]
pub struct FolioResult {
    pub code: i32,
    pub json: *mut c_char,
}

pub struct FolioCancellation {
    token: CancellationToken,
}

/// A progress callback receives a borrowed JSON string. The callback must
/// copy it before returning; FolioForge releases the temporary string after
/// the callback returns.
pub type FolioProgressCallback = unsafe extern "C" fn(*const c_char, *mut c_void);

#[no_mangle]
pub extern "C" fn folio_version() -> *const c_char {
    VERSION.as_ptr().cast()
}

#[no_mangle]
pub extern "C" fn folio_capabilities() -> *mut c_char {
    let registry = folio_core::FormatRegistry;
    let input_formats = registry
        .formats()
        .into_iter()
        .filter(|descriptor| descriptor.input_supported)
        .map(|descriptor| {
            serde_json::json!({
                "format": descriptor.id,
                "extensions": descriptor.extensions,
                "support": descriptor.adapter_capabilities,
            })
        })
        .collect::<Vec<_>>();
    allocate_json(&serde_json::json!({
        "core_version": folio_core::CORE_VERSION,
        "abi_version": folio_core::ABI_VERSION,
        "formats": registry.formats(),
        "targets": registry.output_formats(),
        "input_formats": input_formats,
        "degradation_modes": ["Strict", "Compatible", "Readable"],
        "capability_profiles": folio_core::capabilities(),
        "progress_callback": true,
        "cancellation": true,
        "offline": true,
        "network": false,
    }))
}

#[no_mangle]
///
/// # Safety
/// `request_json` must be a valid, NUL-terminated UTF-8 C string for the
/// duration of the call, or null to receive an error result.
pub unsafe extern "C" fn folio_convert(request_json: *const c_char) -> *mut FolioResult {
    // SAFETY: this wrapper passes the caller's pointer through unchanged and
    // uses a private token when the caller does not request cancellation.
    unsafe { folio_convert_with_token(request_json, ptr::null(), None, ptr::null_mut()) }
}

#[no_mangle]
/// Convert with a cancellation token owned by the caller.
///
/// # Safety
/// `request_json` must be a valid, NUL-terminated UTF-8 C string for the
/// duration of the call. `cancellation` must be null or a valid, not-yet-freed
/// pointer returned by `folio_cancellation_new`.
pub unsafe extern "C" fn folio_convert_with_cancellation(
    request_json: *const c_char,
    cancellation: *const FolioCancellation,
) -> *mut FolioResult {
    // SAFETY: the documented preconditions are forwarded to the shared
    // implementation, which only borrows both values during this call.
    unsafe { folio_convert_with_token(request_json, cancellation, None, ptr::null_mut()) }
}

#[no_mangle]
/// Convert with cancellation and a JSON progress callback.
///
/// # Safety
/// `request_json` must be a valid, NUL-terminated UTF-8 C string for the
/// duration of the call. `cancellation` must be null or a valid, not-yet-freed
/// pointer returned by `folio_cancellation_new`. If `callback` is non-null it
/// must remain valid for the duration of the call, and `user_data` is passed
/// through unchanged to every callback invocation. The callback receives a
/// borrowed JSON string and must copy it before returning.
pub unsafe extern "C" fn folio_convert_with_progress(
    request_json: *const c_char,
    cancellation: *const FolioCancellation,
    callback: Option<FolioProgressCallback>,
    user_data: *mut c_void,
) -> *mut FolioResult {
    // SAFETY: the documented pointer and callback lifetime requirements are
    // forwarded to the shared implementation.
    unsafe { folio_convert_with_token(request_json, cancellation, callback, user_data) }
}

#[derive(serde::Deserialize)]
struct AnalyzeRequest {
    input: PathBuf,
    target: Target,
    #[serde(default)]
    mode: DegradationMode,
    #[serde(default)]
    degradation: DegradationOptions,
    #[serde(default)]
    edit: folio_edit::BookEditPlan,
    #[serde(default)]
    text: folio_text::TextImportOptions,
}

#[derive(serde::Deserialize)]
struct PreviewRequest {
    input: PathBuf,
    target: Target,
    #[serde(default)]
    mode: DegradationMode,
    #[serde(default)]
    degradation: DegradationOptions,
    #[serde(default)]
    edit: folio_edit::BookEditPlan,
    #[serde(default)]
    settings: folio_core::PreviewSettings,
    #[serde(default)]
    text: folio_text::TextImportOptions,
}

#[derive(serde::Deserialize)]
struct BatchConversionRequest {
    inputs: Vec<folio_batch::BatchInput>,
    output_dir: PathBuf,
    options: folio_batch::BatchOptions,
}

#[no_mangle]
/// Discover supported files under a directory using shared input and batch
/// traversal/path-safety rules. Returns JSON-encoded `BatchInput` values.
///
/// # Safety
/// `path` must be a valid, NUL-terminated UTF-8 C string for the duration of
/// the call, or null to receive an error result.
pub unsafe extern "C" fn folio_batch_list_directory(path: *const c_char) -> *mut FolioResult {
    allocate_result(unsafe { read_path(path) }.and_then(|path| {
        folio_batch::list_directory_inputs(&path)
            .map_err(|error| error.to_string())
            .and_then(|items| serde_json::to_value(items).map_err(|error| error.to_string()))
    }))
}

#[no_mangle]
/// Convert explicit per-book inputs through the shared Rust batch engine.
/// The request accepts per-book edit/output-root overrides while retaining
/// one batch mode, target, degradation policy, and collision policy.
///
/// # Safety
/// `request_json` must be a valid, NUL-terminated UTF-8 C string for the
/// duration of the call. `cancellation` must be null or a valid, not-yet-freed
/// pointer returned by `folio_cancellation_new`. If `callback` is non-null it
/// must remain valid for the duration of the call; `user_data` is passed
/// unchanged to every callback invocation.
pub unsafe extern "C" fn folio_batch_convert_with_progress(
    request_json: *const c_char,
    cancellation: *const FolioCancellation,
    callback: Option<FolioProgressCallback>,
    user_data: *mut c_void,
) -> *mut FolioResult {
    let result: Result<serde_json::Value, String> = match unsafe { read_utf8(request_json) }
        .and_then(|value| {
            serde_json::from_str::<BatchConversionRequest>(value).map_err(|error| error.to_string())
        }) {
        Ok(request) => {
            let fallback = CancellationToken::new();
            let token = unsafe { cancellation.as_ref() }
                .map(|value| &value.token)
                .unwrap_or(&fallback);
            folio_batch::convert_items_with_cancellation(
                &request.inputs,
                &request.output_dir,
                &request.options,
                token,
                |current, total, event| {
                    emit_batch_progress(callback, user_data, current, total, event);
                },
            )
            .map_err(|error| error.to_string())
            .and_then(|report| serde_json::to_value(report).map_err(|error| error.to_string()))
        }
        Err(error) => Err(error),
    };
    allocate_result(result)
}

#[derive(serde::Deserialize)]
struct OnlineMetadataSearchRequest {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    request: folio_online::ProviderRequest,
}

#[derive(serde::Deserialize)]
struct OnlineMetadataMergeRequest {
    current: folio_model::Metadata,
    candidate: folio_online::MetadataCandidate,
    fields: std::collections::BTreeMap<String, folio_online::MetadataMergeAction>,
    #[serde(default)]
    confirmed: bool,
}

#[no_mangle]
/// Analyze an input and return the same compatibility plan that conversion
/// will use.  The request is JSON so new planner options can be added without
/// changing the C ABI shape.
///
/// # Safety
/// `request_json` must be a valid, NUL-terminated UTF-8 C string for the
/// duration of the call, or null to receive an error result.
pub unsafe extern "C" fn folio_analyze(request_json: *const c_char) -> *mut FolioResult {
    let result = unsafe { read_utf8(request_json) }
        .and_then(|value| {
            serde_json::from_str::<AnalyzeRequest>(value).map_err(|error| error.to_string())
        })
        .and_then(|request| {
            let report = folio_core::preflight_with_text_options(
                &request.input,
                request.target,
                request.mode,
                &request.degradation,
                &request.edit,
                &request.text,
            )
            .map_err(|error| error.to_string())?;
            serde_json::to_value(serde_json::json!({
                "source_format": report.detected_format,
                "target_format": request.target.format().name(),
                "input_report": report.input_report,
                "semantic_report": report.semantic_report,
                "plan": report.plan,
            }))
            .map_err(|error| error.to_string())
        });
    allocate_result(result)
}

#[no_mangle]
/// Build a compatibility preview from a source file and an optional
/// non-destructive edit plan.
///
/// # Safety
/// `request_json` must be a valid, NUL-terminated UTF-8 C string for the
/// duration of the call, or null to receive an error result.
pub unsafe extern "C" fn folio_preview(request_json: *const c_char) -> *mut FolioResult {
    let result = unsafe { read_utf8(request_json) }
        .and_then(|value| {
            serde_json::from_str::<PreviewRequest>(value).map_err(|error| error.to_string())
        })
        .and_then(|request| {
            folio_core::preview_with_settings_and_text_options(
                &request.input,
                request.target,
                request.mode,
                &request.degradation,
                &request.edit,
                &request.settings,
                &request.text,
            )
            .map_err(|error| error.to_string())
            .and_then(|preview| serde_json::to_value(preview).map_err(|error| error.to_string()))
        });
    allocate_result(result)
}

#[no_mangle]
/// Render imported, edited, or Core-projected generic IR through Reader.
/// `target` is required only when `mode` is `target`; compatibility planning
/// and semantic preparation remain Core responsibilities.
///
/// # Safety
/// `request_json` must be a valid, NUL-terminated UTF-8 C string for the
/// duration of the call, or null to receive an error result.
pub unsafe extern "C" fn folio_reader_preview(request_json: *const c_char) -> *mut FolioResult {
    let result = read_comic_request::<folio_core::CoreReaderPreviewRequest>(request_json)
        .and_then(|request| folio_core::reader_preview(&request).map_err(reader_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Open a format-neutral Reader session from an importable book or Comic
/// image source. The viewport and optional direction are validated by Core.
///
/// # Safety
/// `request_json` must be valid NUL-terminated UTF-8 JSON for this call. A
/// non-null cancellation pointer must be live until the call returns.
pub unsafe extern "C" fn folio_reader_open(
    request_json: *const c_char,
    cancellation: *const FolioCancellation,
) -> *mut FolioResult {
    let result =
        read_comic_request::<folio_core::ReaderOpenRequest>(request_json).and_then(|request| {
            let local_cancellation = CancellationToken::new();
            // SAFETY: the public ABI contract requires a live pointer when it
            // is non-null; the call only borrows its token synchronously.
            let token = unsafe { cancellation.as_ref() }
                .map(|handle| &handle.token)
                .unwrap_or(&local_cancellation);
            folio_core::reader_open_with_cancellation(&request, token).map_err(reader_core_error)
        });
    allocate_comic_result(result)
}

#[no_mangle]
/// Open a Reader view over the IR already projected by an existing Comic
/// Core session; this does not re-import or resolve comic pages in Swift.
///
/// # Safety
/// `request_json` must be valid NUL-terminated UTF-8 JSON for this call.
pub unsafe extern "C" fn folio_reader_open_comic(request_json: *const c_char) -> *mut FolioResult {
    let result = read_comic_request::<folio_core::ReaderOpenComicRequest>(request_json)
        .and_then(|request| folio_core::reader_open_comic(&request).map_err(reader_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Close a Reader session. In-flight calls retain their Core-owned entry until
/// completion; later calls with this ID receive the stable `session_closed`
/// error.
///
/// # Safety
/// `session_id` must be valid NUL-terminated UTF-8 for the duration of the call.
pub unsafe extern "C" fn folio_reader_close(session_id: *const c_char) -> *mut FolioResult {
    let result = unsafe { read_utf8(session_id) }
        .map_err(|error| ComicFfiError::new("invalid_request", error))
        .and_then(|session_id| folio_core::reader_close(session_id).map_err(reader_core_error))
        .map(|closed| serde_json::json!({ "closed": closed }));
    allocate_comic_result(result)
}

#[no_mangle]
/// Return the current Reader session state without exposing internal Rust
/// references.
///
/// # Safety
/// `session_id` must be valid NUL-terminated UTF-8 for the duration of the call.
pub unsafe extern "C" fn folio_reader_summary(session_id: *const c_char) -> *mut FolioResult {
    let result = unsafe { read_utf8(session_id) }
        .map_err(|error| ComicFfiError::new("invalid_request", error))
        .and_then(|session_id| folio_core::reader_summary(session_id).map_err(reader_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Return the current fixed-page model with its encoded image bytes and
/// Reader-owned geometry.
///
/// # Safety
/// `session_id` must be valid NUL-terminated UTF-8. A non-null cancellation
/// pointer must be live until this call returns.
pub unsafe extern "C" fn folio_reader_current_page(
    session_id: *const c_char,
    cancellation: *const FolioCancellation,
) -> *mut FolioResult {
    let result = unsafe { read_utf8(session_id) }
        .map_err(|error| ComicFfiError::new("invalid_request", error))
        .and_then(|session_id| {
            let local_cancellation = CancellationToken::new();
            // SAFETY: the public ABI contract requires a live pointer when it
            // is non-null; the call only borrows its token synchronously.
            let token = unsafe { cancellation.as_ref() }
                .map(|handle| &handle.token)
                .unwrap_or(&local_cancellation);
            folio_core::reader_current_page_with_cancellation(session_id, token)
                .map_err(reader_core_error)
        });
    allocate_comic_result(result)
}

#[no_mangle]
/// Return the current generic fixed-layout one/two-page render model.
///
/// # Safety
/// `session_id` must be valid NUL-terminated UTF-8 for the duration of the call.
pub unsafe extern "C" fn folio_reader_current_spread(
    session_id: *const c_char,
) -> *mut FolioResult {
    let result = unsafe { read_utf8(session_id) }
        .map_err(|error| ComicFfiError::new("invalid_request", error))
        .and_then(|session_id| {
            folio_core::reader_current_spread(session_id).map_err(reader_core_error)
        });
    allocate_comic_result(result)
}

#[no_mangle]
/// Return one encoded Reader resource as base64 JSON data.
///
/// # Safety
/// `request_json` must be valid NUL-terminated UTF-8 JSON for this call.
pub unsafe extern "C" fn folio_reader_resource(request_json: *const c_char) -> *mut FolioResult {
    let result = read_comic_request::<folio_core::ReaderResourceRequest>(request_json)
        .and_then(|request| folio_core::reader_resource(&request).map_err(reader_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Advance according to the Reader's effective LTR/RTL reading order.
///
/// # Safety
/// `session_id` must be valid NUL-terminated UTF-8 for the duration of the call.
pub unsafe extern "C" fn folio_reader_next(session_id: *const c_char) -> *mut FolioResult {
    let result = unsafe { read_utf8(session_id) }
        .map_err(|error| ComicFfiError::new("invalid_request", error))
        .and_then(|session_id| folio_core::reader_next(session_id).map_err(reader_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Move backward according to the Reader's effective LTR/RTL reading order.
///
/// # Safety
/// `session_id` must be valid NUL-terminated UTF-8 for the duration of the call.
pub unsafe extern "C" fn folio_reader_previous(session_id: *const c_char) -> *mut FolioResult {
    let result = unsafe { read_utf8(session_id) }
        .map_err(|error| ComicFfiError::new("invalid_request", error))
        .and_then(|session_id| folio_core::reader_previous(session_id).map_err(reader_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Move to the first Reader document in effective reading order.
///
/// # Safety
/// `session_id` must be valid NUL-terminated UTF-8 for the duration of the call.
pub unsafe extern "C" fn folio_reader_first(session_id: *const c_char) -> *mut FolioResult {
    let result = unsafe { read_utf8(session_id) }
        .map_err(|error| ComicFfiError::new("invalid_request", error))
        .and_then(|session_id| folio_core::reader_first(session_id).map_err(reader_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Move to the last Reader document in effective reading order.
///
/// # Safety
/// `session_id` must be valid NUL-terminated UTF-8 for the duration of the call.
pub unsafe extern "C" fn folio_reader_last(session_id: *const c_char) -> *mut FolioResult {
    let result = unsafe { read_utf8(session_id) }
        .map_err(|error| ComicFfiError::new("invalid_request", error))
        .and_then(|session_id| folio_core::reader_last(session_id).map_err(reader_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Return the generic Reader navigation tree from the session's immutable IR.
///
/// # Safety
/// `session_id` must be valid NUL-terminated UTF-8 for the duration of the call.
pub unsafe extern "C" fn folio_reader_navigation(session_id: *const c_char) -> *mut FolioResult {
    let result = unsafe { read_utf8(session_id) }
        .map_err(|error| ComicFfiError::new("invalid_request", error))
        .and_then(|session_id| {
            folio_core::reader_navigation(session_id).map_err(reader_core_error)
        });
    allocate_comic_result(result)
}

#[no_mangle]
/// Navigate to a validated generic Reader location.
///
/// # Safety
/// `request_json` must be valid NUL-terminated UTF-8 JSON for this call.
pub unsafe extern "C" fn folio_reader_go_to(request_json: *const c_char) -> *mut FolioResult {
    let result = read_comic_request::<folio_core::ReaderGoToRequest>(request_json)
        .and_then(|request| folio_core::reader_go_to(&request).map_err(reader_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Navigate by a zero-based position in the immutable IR document vector.
/// This selection coordinate is independent of effective LTR/RTL traversal.
///
/// # Safety
/// `request_json` must be valid NUL-terminated UTF-8 JSON for this call.
pub unsafe extern "C" fn folio_reader_go_to_document_index(
    request_json: *const c_char,
) -> *mut FolioResult {
    let result = read_comic_request::<folio_core::ReaderGoToDocumentIndexRequest>(request_json)
        .and_then(|request| {
            folio_core::reader_go_to_document_index(&request).map_err(reader_core_error)
        });
    allocate_comic_result(result)
}

#[no_mangle]
/// Navigate to a page using its immutable IR `DocumentId`.
///
/// # Safety
/// `request_json` must be valid NUL-terminated UTF-8 JSON for this call.
pub unsafe extern "C" fn folio_reader_go_to_page(request_json: *const c_char) -> *mut FolioResult {
    let result = read_comic_request::<folio_core::ReaderGoToPageRequest>(request_json)
        .and_then(|request| folio_core::reader_go_to_page(&request).map_err(reader_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Navigate to a validated entry in the generic IR navigation tree.
///
/// # Safety
/// `request_json` must be valid NUL-terminated UTF-8 JSON for this call.
pub unsafe extern "C" fn folio_reader_go_to_navigation_target(
    request_json: *const c_char,
) -> *mut FolioResult {
    let result = read_comic_request::<folio_core::ReaderGoToNavigationTargetRequest>(request_json)
        .and_then(|request| {
            folio_core::reader_go_to_navigation_target(&request).map_err(reader_core_error)
        });
    allocate_comic_result(result)
}

#[no_mangle]
/// Update only transient Reader viewport state.
///
/// # Safety
/// `request_json` must be valid NUL-terminated UTF-8 JSON for this call.
pub unsafe extern "C" fn folio_reader_set_viewport(
    request_json: *const c_char,
) -> *mut FolioResult {
    let result = read_comic_request::<folio_core::ReaderSetViewportRequest>(request_json)
        .and_then(|request| folio_core::reader_set_viewport(&request).map_err(reader_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Set the generic reading direction while preserving the current document.
///
/// # Safety
/// `request_json` must be valid NUL-terminated UTF-8 JSON for this call.
pub unsafe extern "C" fn folio_reader_set_direction(
    request_json: *const c_char,
) -> *mut FolioResult {
    let result = read_comic_request::<folio_core::ReaderSetDirectionRequest>(request_json)
        .and_then(|request| folio_core::reader_set_direction(&request).map_err(reader_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Set the transient single-page or synthetic-spread display mode.
///
/// # Safety
/// `request_json` must be valid NUL-terminated UTF-8 JSON for this call.
pub unsafe extern "C" fn folio_reader_set_spread_mode(
    request_json: *const c_char,
) -> *mut FolioResult {
    let result = read_comic_request::<folio_core::ReaderSetSpreadModeRequest>(request_json)
        .and_then(|request| {
            folio_core::reader_set_spread_mode(&request).map_err(reader_core_error)
        });
    allocate_comic_result(result)
}

#[no_mangle]
/// Open a local comic folder or ZIP/CBZ and create a Core-owned session.
///
/// # Safety
/// `path` must be a valid, NUL-terminated UTF-8 C string for the duration of
/// the call, or null to receive a structured error result.
pub unsafe extern "C" fn folio_comic_open(path: *const c_char) -> *mut FolioResult {
    let result = unsafe { read_path(path) }
        .map_err(|error| ComicFfiError::new("invalid_path", error))
        .and_then(|path| folio_core::comic_open(path).map_err(comic_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Open a local comic source with cooperative cancellation during ingestion.
///
/// # Safety
/// `path` must be a valid UTF-8 C string. A non-null cancellation pointer
/// must be live for the duration of this call.
pub unsafe extern "C" fn folio_comic_open_with_cancellation(
    path: *const c_char,
    cancellation: *const FolioCancellation,
) -> *mut FolioResult {
    let result = unsafe { read_path(path) }
        .map_err(|error| ComicFfiError::new("invalid_path", error))
        .and_then(|path| {
            let local_cancellation = CancellationToken::new();
            // SAFETY: the ABI contract requires a live pointer when non-null.
            let token = unsafe { cancellation.as_ref() }
                .map(|handle| &handle.token)
                .unwrap_or(&local_cancellation);
            folio_core::comic_open_with_cancellation(path, token).map_err(comic_core_error)
        });
    allocate_comic_result(result)
}

#[no_mangle]
/// Close a Comic session. In-flight operations keep their own Core reference.
///
/// # Safety
/// `session_id` must be a valid, NUL-terminated UTF-8 C string, or null to
/// receive a structured error result.
pub unsafe extern "C" fn folio_comic_close(session_id: *const c_char) -> *mut FolioResult {
    let result = unsafe { read_utf8(session_id) }
        .map_err(|error| ComicFfiError::new("invalid_request", error))
        .and_then(|session_id| folio_core::comic_close(session_id).map_err(comic_core_error))
        .map(|closed| serde_json::json!({ "closed": closed }));
    allocate_comic_result(result)
}

#[no_mangle]
/// Return current metadata and warnings for a Comic session.
///
/// # Safety
/// `session_id` must be a valid, NUL-terminated UTF-8 C string, or null to
/// receive a structured error result.
pub unsafe extern "C" fn folio_comic_summary(session_id: *const c_char) -> *mut FolioResult {
    let result = unsafe { read_utf8(session_id) }
        .map_err(|error| ComicFfiError::new("invalid_request", error))
        .and_then(|session_id| folio_core::comic_summary(session_id).map_err(comic_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Enumerate source pages in Core-provided reading order using stable page IDs.
///
/// # Safety
/// `session_id` must be a valid, NUL-terminated UTF-8 C string, or null to
/// receive a structured error result.
pub unsafe extern "C" fn folio_comic_pages(session_id: *const c_char) -> *mut FolioResult {
    let result = unsafe { read_utf8(session_id) }
        .map_err(|error| ComicFfiError::new("invalid_request", error))
        .and_then(|session_id| folio_core::comic_pages(session_id).map_err(comic_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
/// Return Core-owned metadata for one stable Comic page ID.
///
/// # Safety
/// `request_json` must be a valid, NUL-terminated UTF-8 JSON string for the
/// duration of this call, or null to receive a structured error result.
pub unsafe extern "C" fn folio_comic_page_info(request_json: *const c_char) -> *mut FolioResult {
    let result = read_comic_request::<folio_core::ComicPageRequest>(request_json)
        .and_then(|request| folio_core::comic_page_info(&request).map_err(comic_core_error));
    allocate_comic_result(result)
}

#[no_mangle]
pub extern "C" fn folio_comic_conversion_options() -> *mut FolioResult {
    allocate_comic_result(Ok(folio_core::comic_conversion_options()))
}

#[no_mangle]
/// Return a Core-rendered, bounded PNG thumbnail as base64 JSON data.
///
/// # Safety
/// `request_json` must be a valid, NUL-terminated UTF-8 JSON string. A
/// non-null `cancellation` must be a live pointer returned by
/// `folio_cancellation_new` for the duration of this call.
pub unsafe extern "C" fn folio_comic_thumbnail(
    request_json: *const c_char,
    cancellation: *const FolioCancellation,
) -> *mut FolioResult {
    let result =
        read_comic_request::<folio_core::ComicImageRequest>(request_json).and_then(|request| {
            let local_cancellation = CancellationToken::new();
            // SAFETY: the public ABI contract requires a live cancellation
            // pointer when it is non-null; only an immutable reference is used.
            let token = unsafe { cancellation.as_ref() }
                .map(|handle| &handle.token)
                .unwrap_or(&local_cancellation);
            folio_core::comic_thumbnail(&request, token)
                .map(comic_image_response)
                .map_err(comic_core_error)
        });
    allocate_comic_result(result)
}

#[no_mangle]
/// Return a Core-rendered source preview as base64 PNG JSON data.
///
/// # Safety
/// `request_json` must be a valid, NUL-terminated UTF-8 JSON string. A
/// non-null `cancellation` must be a live pointer returned by
/// `folio_cancellation_new` for the duration of this call.
pub unsafe extern "C" fn folio_comic_preview(
    request_json: *const c_char,
    cancellation: *const FolioCancellation,
) -> *mut FolioResult {
    let result =
        read_comic_request::<folio_core::ComicImageRequest>(request_json).and_then(|request| {
            let local_cancellation = CancellationToken::new();
            // SAFETY: the public ABI contract requires a live cancellation
            // pointer when it is non-null; only an immutable reference is used.
            let token = unsafe { cancellation.as_ref() }
                .map(|handle| &handle.token)
                .unwrap_or(&local_cancellation);
            folio_core::comic_preview(&request, token)
                .map(comic_image_response)
                .map_err(comic_core_error)
        });
    allocate_comic_result(result)
}

#[no_mangle]
/// Convert a Comic session to the Core-supported target with progress and
/// cancellation. The callback receives borrowed JSON and must copy it.
///
/// # Safety
/// `request_json` must be valid JSON. A non-null `cancellation` must be a
/// live pointer returned by `folio_cancellation_new`. A non-null callback and
/// `user_data` must remain valid for the duration of this synchronous call.
pub unsafe extern "C" fn folio_comic_convert_with_progress(
    request_json: *const c_char,
    cancellation: *const FolioCancellation,
    callback: Option<FolioProgressCallback>,
    user_data: *mut c_void,
) -> *mut FolioResult {
    let result = read_comic_request::<folio_core::ComicConversionRequest>(request_json).and_then(
        |request| {
            let local_cancellation = CancellationToken::new();
            // SAFETY: the public ABI contract requires a live cancellation
            // pointer when it is non-null; only an immutable reference is used.
            let token = unsafe { cancellation.as_ref() }
                .map(|handle| &handle.token)
                .unwrap_or(&local_cancellation);
            folio_core::comic_convert_with_progress(&request, token, |event| {
                emit_progress(callback, user_data, event)
            })
            .map_err(comic_core_error)
        },
    );
    allocate_comic_result(result)
}

#[no_mangle]
/// Search Open Library through the optional online-resource boundary. The
/// caller must explicitly opt in by setting `enabled` on this request.
///
/// # Safety
/// `request_json` must be a valid, NUL-terminated UTF-8 C string for the
/// duration of the call, or null to receive an error result.
pub unsafe extern "C" fn folio_online_metadata_search(
    request_json: *const c_char,
) -> *mut FolioResult {
    let result = unsafe { read_utf8(request_json) }
        .and_then(|value| {
            serde_json::from_str::<OnlineMetadataSearchRequest>(value)
                .map_err(|error| error.to_string())
        })
        .and_then(|request| {
            let boundary = folio_online::OnlineBoundary::new(
                folio_online::OnlineSettings {
                    enabled: request.enabled,
                },
                folio_online::MemoryCache::default(),
            );
            boundary
                .search(
                    &folio_online::OpenLibraryProvider::default(),
                    &request.request,
                )
                .map_err(|error| error.to_string())
                .and_then(|candidates| {
                    serde_json::to_value(candidates).map_err(|error| error.to_string())
                })
        });
    allocate_result(result)
}

#[no_mangle]
/// Build a metadata-only BookEditPlan fragment from an online candidate.
/// The plan is not applied to the input. Confirmation is required before this
/// function returns any edits.
///
/// # Safety
/// `request_json` must be a valid, NUL-terminated UTF-8 C string for the
/// duration of the call, or null to receive an error result.
pub unsafe extern "C" fn folio_online_metadata_merge_plan(
    request_json: *const c_char,
) -> *mut FolioResult {
    let result = unsafe { read_utf8(request_json) }
        .and_then(|value| {
            serde_json::from_str::<OnlineMetadataMergeRequest>(value)
                .map_err(|error| error.to_string())
        })
        .and_then(|request| {
            let merge = folio_online::MetadataMergePlan {
                candidate: request.candidate,
                fields: request.fields,
                requires_confirmation: true,
            };
            merge
                .build_edit_plan(&request.current, request.confirmed)
                .map(|plan| plan.metadata)
                .map_err(|error| error.to_string())
                .and_then(|metadata| {
                    serde_json::to_value(metadata).map_err(|error| error.to_string())
                })
        });
    allocate_result(result)
}

unsafe fn folio_convert_with_token(
    request_json: *const c_char,
    cancellation: *const FolioCancellation,
    callback: Option<FolioProgressCallback>,
    user_data: *mut c_void,
) -> *mut FolioResult {
    let result: Result<serde_json::Value, String> = match unsafe { read_utf8(request_json) }
        .and_then(|value| {
            serde_json::from_str::<ConversionRequest>(value).map_err(|error| error.to_string())
        }) {
        Ok(request) => {
            let fallback = CancellationToken::new();
            let token = unsafe { cancellation.as_ref() }
                .map(|value| &value.token)
                .unwrap_or(&fallback);
            folio_core::convert_with_progress(&request, token, |event| {
                emit_progress(callback, user_data, event);
            })
            .map_err(|error| error.to_string())
            .and_then(|report| serde_json::to_value(report).map_err(|error| error.to_string()))
        }
        Err(error) => Err(error),
    };
    allocate_result(result)
}

#[no_mangle]
///
/// # Safety
/// `path` must be a valid, NUL-terminated UTF-8 C string for the duration of
/// the call, or null to receive an error result.
pub unsafe extern "C" fn folio_inspect(path: *const c_char) -> *mut FolioResult {
    allocate_result(unsafe { read_path(path) }.and_then(|path| {
        folio_core::inspect(path)
            .map_err(|error| error.to_string())
            .and_then(|report| serde_json::to_value(report).map_err(|error| error.to_string()))
    }))
}

#[no_mangle]
///
/// # Safety
/// `path` must be a valid, NUL-terminated UTF-8 C string for the duration of
/// the call, or null to receive an error result.
pub unsafe extern "C" fn folio_validate(path: *const c_char) -> *mut FolioResult {
    allocate_result(unsafe { read_path(path) }.and_then(|path| {
        folio_core::validate(path)
            .map_err(|error| error.to_string())
            .and_then(|report| serde_json::to_value(report).map_err(|error| error.to_string()))
    }))
}

#[no_mangle]
pub extern "C" fn folio_cancellation_new() -> *mut FolioCancellation {
    Box::into_raw(Box::new(FolioCancellation {
        token: CancellationToken::new(),
    }))
}

#[no_mangle]
///
/// # Safety
/// `cancellation` must be null or a valid, not-yet-freed pointer returned by
/// `folio_cancellation_new`.
pub unsafe extern "C" fn folio_cancellation_cancel(cancellation: *mut FolioCancellation) {
    if let Some(cancellation) = unsafe { cancellation.as_ref() } {
        cancellation.token.cancel();
    }
}

#[no_mangle]
///
/// # Safety
/// `cancellation` must be null or the unique pointer returned by
/// `folio_cancellation_new`.
pub unsafe extern "C" fn folio_cancellation_free(cancellation: *mut FolioCancellation) {
    if !cancellation.is_null() {
        // SAFETY: callers may pass only the pointer returned by
        // `folio_cancellation_new`, at most once.
        // SAFETY: pointer provenance and uniqueness are documented above.
        drop(unsafe { Box::from_raw(cancellation) });
    }
}

#[no_mangle]
///
/// # Safety
/// `result` must be null or the unique pointer returned by a FolioForge
/// operation and may be released at most once.
pub unsafe extern "C" fn folio_result_free(result: *mut FolioResult) {
    if result.is_null() {
        return;
    }
    // SAFETY: `result` must be the unique pointer returned by one of the
    // FolioForge result functions and is consumed exactly once here.
    let result = unsafe { Box::from_raw(result) };
    if !result.json.is_null() {
        // SAFETY: `json` was allocated by `CString::into_raw` in this crate.
        drop(unsafe { CString::from_raw(result.json) });
    }
}

#[no_mangle]
///
/// # Safety
/// `value` must be null or a string pointer returned by FolioForge and may be
/// released at most once.
pub unsafe extern "C" fn folio_string_free(value: *mut c_char) {
    if !value.is_null() {
        // SAFETY: `value` must originate from a FolioForge function returning
        // an owned CString pointer.
        drop(unsafe { CString::from_raw(value) });
    }
}

fn allocate_result(result: Result<serde_json::Value, String>) -> *mut FolioResult {
    match result {
        Ok(value) => Box::into_raw(Box::new(FolioResult {
            code: 0,
            json: allocate_json(&value),
        })),
        Err(error) => Box::into_raw(Box::new(FolioResult {
            code: 1,
            json: allocate_json(&serde_json::json!({"error": error})),
        })),
    }
}

struct ComicFfiError {
    code: &'static str,
    message: String,
}

impl ComicFfiError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

fn comic_core_error(error: CoreError) -> ComicFfiError {
    let code = match &error {
        CoreError::ComicSessionNotFound(_) => "session_not_found",
        CoreError::InvalidComicRequest(_) => "invalid_request",
        CoreError::UnsupportedComicSource(_) => "unsupported_source",
        CoreError::UnsupportedTarget(_) => "unsupported_target",
        CoreError::ComicImport(_) => "source_import_failed",
        CoreError::ComicRender(_) => "preview_failed",
        CoreError::ComicOutput(_) => "output_failed",
        CoreError::ValidationFailed(_) => "validation_failed",
        CoreError::Cancelled => "cancelled",
        CoreError::ComicSessionRegistryUnavailable => "session_unavailable",
        CoreError::Io(_) => "io_error",
        _ => "core_error",
    };
    ComicFfiError::new(code, error.to_string())
}

fn reader_core_error(error: CoreError) -> ComicFfiError {
    let code = match &error {
        CoreError::Reader(reader_error) => match reader_error.code {
            folio_reader::ReaderErrorCode::InvalidLocation => "invalid_location",
            folio_reader::ReaderErrorCode::InvalidNavigationTarget => "invalid_navigation_target",
            folio_reader::ReaderErrorCode::MissingResource => "missing_resource",
            folio_reader::ReaderErrorCode::UnsupportedLayout => "unsupported_layout",
            folio_reader::ReaderErrorCode::InvalidPageGeometry => "invalid_page_geometry",
            folio_reader::ReaderErrorCode::DecodeFailed => "decode_failed",
            folio_reader::ReaderErrorCode::SessionClosed => "session_closed",
            folio_reader::ReaderErrorCode::ResourceUnavailable => "resource_unavailable",
            folio_reader::ReaderErrorCode::ResourceTooLarge => "resource_too_large",
            folio_reader::ReaderErrorCode::ResourceHandleSessionMismatch => {
                "resource_handle_session_mismatch"
            }
            folio_reader::ReaderErrorCode::InvalidViewport => "invalid_viewport",
            folio_reader::ReaderErrorCode::InvalidPreviewRequest => "invalid_preview_request",
            folio_reader::ReaderErrorCode::EmptyBook => "empty_book",
            folio_reader::ReaderErrorCode::Cancelled => "cancelled",
        },
        CoreError::ReaderSessionNotFound(_) => "session_closed",
        CoreError::ReaderSessionRegistryUnavailable => "session_unavailable",
        CoreError::ReaderSessionIdExhausted => "session_id_exhausted",
        CoreError::MissingInput(_) => "missing_input",
        CoreError::UnsupportedInput(_) => "unsupported_input",
        CoreError::Format(_) | CoreError::Epub(_) => "import_failed",
        CoreError::ComicImport(_) => "source_import_failed",
        CoreError::ValidationFailed(_) => "validation_failed",
        CoreError::Cancelled => "cancelled",
        CoreError::Io(_) => "io_error",
        _ => "reader_error",
    };
    ComicFfiError::new(code, error.to_string())
}

fn read_comic_request<T: serde::de::DeserializeOwned>(
    request_json: *const c_char,
) -> Result<T, ComicFfiError> {
    // SAFETY: every caller forwards the JSON-pointer precondition documented
    // by its exported ABI function.
    let json = unsafe { read_utf8(request_json) }
        .map_err(|error| ComicFfiError::new("invalid_request", error))?;
    serde_json::from_str(json)
        .map_err(|error| ComicFfiError::new("invalid_request", error.to_string()))
}

#[derive(serde::Serialize)]
struct ComicImageFfiDto {
    data_base64: String,
    width: u32,
    height: u32,
    source_width: u32,
    source_height: u32,
    mime_type: String,
    cache_identity: String,
}

fn comic_image_response(image: folio_core::ComicImageDto) -> ComicImageFfiDto {
    ComicImageFfiDto {
        data_base64: base64::engine::general_purpose::STANDARD.encode(image.bytes),
        width: image.width,
        height: image.height,
        source_width: image.source_width,
        source_height: image.source_height,
        mime_type: image.mime_type,
        cache_identity: image.cache_identity,
    }
}

fn allocate_comic_result<T: serde::Serialize>(
    result: Result<T, ComicFfiError>,
) -> *mut FolioResult {
    match result {
        Ok(value) => match serde_json::to_value(value) {
            Ok(value) => allocate_comic_value_result(Ok(value)),
            Err(error) => allocate_comic_value_result(Err(ComicFfiError::new(
                "serialization_failed",
                error.to_string(),
            ))),
        },
        Err(error) => allocate_comic_value_result(Err(error)),
    }
}

fn allocate_comic_value_result(
    result: Result<serde_json::Value, ComicFfiError>,
) -> *mut FolioResult {
    match result {
        Ok(value) => Box::into_raw(Box::new(FolioResult {
            code: 0,
            json: allocate_json(&value),
        })),
        Err(error) => Box::into_raw(Box::new(FolioResult {
            code: 1,
            json: allocate_json(&serde_json::json!({
                "error_code": error.code,
                "error": error.message,
            })),
        })),
    }
}

fn allocate_json(value: &serde_json::Value) -> *mut c_char {
    let text = serde_json::to_string(value)
        .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_owned());
    match CString::new(text) {
        Ok(value) => value.into_raw(),
        Err(_) => match CString::new("{\"error\":\"NUL in JSON\"}") {
            Ok(value) => value.into_raw(),
            Err(_) => ptr::null_mut(),
        },
    }
}

fn emit_progress(
    callback: Option<FolioProgressCallback>,
    user_data: *mut c_void,
    event: ProgressEvent,
) {
    let Some(callback) = callback else {
        return;
    };
    let Ok(json) = serde_json::to_string(&event) else {
        return;
    };
    let Ok(json) = CString::new(json) else {
        return;
    };
    // SAFETY: the caller guarantees that the callback is valid for the
    // duration of the conversion and that `user_data` has the callback's
    // documented provenance. `json` remains alive for this invocation.
    unsafe { callback(json.as_ptr(), user_data) };
}

fn emit_batch_progress(
    callback: Option<FolioProgressCallback>,
    user_data: *mut c_void,
    current_item: usize,
    total_items: usize,
    event: &ProgressEvent,
) {
    let Some(callback) = callback else {
        return;
    };
    let payload = serde_json::json!({
        "current_item": current_item,
        "total_items": total_items,
        "event": event,
    });
    let Ok(json) = serde_json::to_string(&payload) else {
        return;
    };
    let Ok(json) = CString::new(json) else {
        return;
    };
    // SAFETY: the batch conversion caller guarantees callback validity and
    // user-data provenance for the duration of this synchronous FFI call.
    unsafe { callback(json.as_ptr(), user_data) };
}

unsafe fn read_utf8<'a>(value: *const c_char) -> Result<&'a str, String> {
    if value.is_null() {
        return Err("null C string".to_owned());
    }
    // SAFETY: the caller promises a valid NUL-terminated C string for the
    // duration of the ABI call.
    // SAFETY: the public ABI contract requires a valid NUL-terminated string.
    unsafe { CStr::from_ptr(value) }
        .to_str()
        .map_err(|error| error.to_string())
}

unsafe fn read_path(value: *const c_char) -> Result<PathBuf, String> {
    // SAFETY: the caller of this helper has the same pointer precondition as
    // the public ABI function that supplied the value.
    Ok(PathBuf::from(unsafe { read_utf8(value)? }))
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-ffi/src/lib.rs"]
mod tests;
