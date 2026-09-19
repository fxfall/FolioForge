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

use folio_core::{
    CancellationToken, ConversionRequest, DegradationMode, DegradationOptions, ProgressEvent,
    Target,
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

#[cfg(test)]
mod tests {
    use super::*;
    use folio_core::{ConversionRequest, ProgressEvent, ProgressStage};
    use std::io::{Seek, Write};
    use std::path::Path;
    use zip::{write::SimpleFileOptions, ZipWriter};

    #[test]
    fn capabilities_are_owned_json_and_advertise_the_stable_surface() {
        let pointer = folio_capabilities();
        assert!(!pointer.is_null());
        let value: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(pointer).to_str().unwrap()).unwrap() };
        assert_eq!(value["offline"], true);
        assert_eq!(value["network"], false);
        assert_eq!(value["progress_callback"], true);
        assert_eq!(value["cancellation"], true);
        let docx = value["input_formats"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["format"] == "DOCX")
            .expect("DOCX input capability");
        assert!(docx["extensions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|extension| extension == "docx"));
        assert_eq!(docx["support"]["import"], true);
        assert_eq!(docx["support"]["export"], false);
        let format_ids = value["formats"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let target_ids = value["targets"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(format_ids.contains("EPUB"));
        assert!(format_ids.contains("KF7KF8Combo"));
        assert!(target_ids.contains("KFX"));
        assert!(target_ids.iter().all(|id| format_ids.contains(id)));
        unsafe { folio_string_free(pointer) };
    }

    #[test]
    fn native_online_search_is_disabled_without_explicit_opt_in() {
        let request = CString::new(r#"{"request":{"query":"Test title"}}"#).unwrap();
        let result = unsafe { folio_online_metadata_search(request.as_ptr()) };
        assert_eq!(unsafe { (*result).code }, 1);
        let json = unsafe { CStr::from_ptr((*result).json).to_str().unwrap() };
        assert!(json.contains("disabled"));
        unsafe { folio_result_free(result) };
    }

    #[test]
    fn native_online_merge_requires_confirmation_and_returns_edit_data() {
        let request_value = serde_json::json!({
            "current": {"title":"Source title", "authors":["Existing Author"]},
            "candidate": {
                "candidate_id":"openlibrary:/works/OL1W",
                "provider":"openlibrary",
                "confidence":0.9,
                "metadata":{"title":"Catalog title", "authors":["Catalog Author"]}
            },
            "fields":{"title":"replace", "authors":"append"},
            "confirmed":false
        });
        let request = CString::new(request_value.to_string()).unwrap();
        let result = unsafe { folio_online_metadata_merge_plan(request.as_ptr()) };
        assert_eq!(unsafe { (*result).code }, 1);
        unsafe { folio_result_free(result) };

        let mut request_value = request_value;
        request_value["confirmed"] = serde_json::Value::Bool(true);
        let request = CString::new(request_value.to_string()).unwrap();
        let result = unsafe { folio_online_metadata_merge_plan(request.as_ptr()) };
        assert_eq!(unsafe { (*result).code }, 0);
        let edit: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr((*result).json).to_str().unwrap()).unwrap()
        };
        assert_eq!(edit["title"], "Catalog title");
        assert_eq!(edit["authors"][0], "Existing Author");
        assert_eq!(edit["authors"][1], "Catalog Author");
        unsafe { folio_result_free(result) };
    }

    #[test]
    fn invalid_request_returns_owned_error_result() {
        let request = CString::new("{}").unwrap();
        let result = unsafe { folio_convert(request.as_ptr()) };
        assert!(!result.is_null());
        assert_eq!(unsafe { (*result).code }, 1);
        let json = unsafe { CStr::from_ptr((*result).json).to_str().unwrap() };
        assert!(json.contains("error"));
        unsafe { folio_result_free(result) };
    }

    #[test]
    fn analyze_returns_the_compatibility_plan() {
        let root =
            std::env::temp_dir().join(format!("folioforge-ffi-analyze-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let input = root.join("input.epub");
        let file = std::fs::File::create(&input).unwrap();
        let mut zip = ZipWriter::new(file);
        add_epub_entry(&mut zip, "mimetype", "application/epub+zip");
        add_epub_entry(
            &mut zip,
            "META-INF/container.xml",
            r#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#,
        );
        add_epub_entry(
            &mut zip,
            "OEBPS/content.opf",
            r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Analyze Smoke</dc:title><dc:language>en</dc:language></metadata><manifest><item id="c" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c"/></spine></package>"#,
        );
        add_epub_entry(
            &mut zip,
            "OEBPS/chapter.xhtml",
            r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>Analyze</h1></body></html>"#,
        );
        zip.finish().unwrap();

        let request = serde_json::json!({
            "input": input,
            "target": "KF7",
            "mode": "Compatible",
            "degradation": {}
        });
        let request = CString::new(request.to_string()).unwrap();
        let result = unsafe { folio_analyze(request.as_ptr()) };
        assert_eq!(unsafe { (*result).code }, 0);
        let value: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr((*result).json).to_str().unwrap()).unwrap()
        };
        assert_eq!(value["source_format"], "EPUB");
        assert_eq!(value["target_format"], "KF7");
        assert_eq!(value["plan"]["mode"], "Compatible");
        unsafe { folio_result_free(result) };
        std::fs::remove_dir_all(root).unwrap();
    }

    unsafe extern "C" fn count_progress(event: *const c_char, user_data: *mut c_void) {
        if event.is_null() || user_data.is_null() {
            return;
        }
        // SAFETY: the test supplies a valid pointer to a live usize for the
        // duration of this callback.
        let count = unsafe { &mut *user_data.cast::<usize>() };
        // SAFETY: `event` points to the callback-borrowed NUL-terminated JSON
        // string created by `emit_progress`.
        let json = unsafe { CStr::from_ptr(event) };
        if json
            .to_bytes()
            .windows(7)
            .any(|window| window == b"Opening")
        {
            *count += 1;
        }
    }

    #[test]
    fn progress_callback_receives_borrowed_json() {
        let mut count = 0usize;
        emit_progress(
            Some(count_progress),
            (&mut count as *mut usize).cast(),
            ProgressEvent {
                stage: ProgressStage::Opening,
                current: 0,
                total: None,
                fraction: None,
                message: "opening".to_owned(),
            },
        );
        assert_eq!(count, 1);
    }

    unsafe extern "C" fn collect_progress(event: *const c_char, user_data: *mut c_void) {
        if event.is_null() || user_data.is_null() {
            return;
        }
        // SAFETY: the test supplies a valid pointer to a live String vector
        // for the duration of the conversion callback.
        let events = unsafe { &mut *user_data.cast::<Vec<String>>() };
        // SAFETY: `event` is the borrowed, NUL-terminated JSON string from
        // the FFI callback contract.
        let value = unsafe { CStr::from_ptr(event) };
        if let Ok(value) = value.to_str() {
            events.push(value.to_owned());
        }
    }

    fn add_epub_entry<W: Write + Seek>(zip: &mut ZipWriter<W>, name: &str, value: &str) {
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file(name, options).unwrap();
        zip.write_all(value.as_bytes()).unwrap();
    }

    fn unique_test_root(prefix: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()))
    }

    fn write_epub_fixture(path: &Path, title: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        add_epub_entry(&mut zip, "mimetype", "application/epub+zip");
        add_epub_entry(
            &mut zip,
            "META-INF/container.xml",
            r#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#,
        );
        add_epub_entry(
            &mut zip,
            "OEBPS/content.opf",
            &format!(
                r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>{title}</dc:title><dc:language>en</dc:language></metadata><manifest><item id="c" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c"/></spine></package>"#
            ),
        );
        add_epub_entry(
            &mut zip,
            "OEBPS/chapter.xhtml",
            r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>Fixture</h1><p>Batch bridge.</p></body></html>"#,
        );
        zip.finish().unwrap();
    }

    fn batch_request_value(
        inputs: Vec<folio_batch::BatchInput>,
        output_dir: &Path,
        batch_mode: folio_batch::BatchMode,
    ) -> serde_json::Value {
        let options = folio_batch::BatchOptions {
            target: folio_core::Target::EPUB,
            batch_mode,
            ..folio_batch::BatchOptions::default()
        };
        serde_json::json!({
            "inputs": inputs,
            "output_dir": output_dir,
            "options": options,
        })
    }

    #[test]
    fn ffi_conversion_matches_direct_core_and_emits_progress() {
        let root = std::env::temp_dir().join(format!("folioforge-ffi-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let input = root.join("input.epub");
        let output = root.join("ffi.kfx");
        let direct_output = root.join("direct.kfx");
        let file = std::fs::File::create(&input).unwrap();
        let mut zip = ZipWriter::new(file);
        add_epub_entry(&mut zip, "mimetype", "application/epub+zip");
        add_epub_entry(
            &mut zip,
            "META-INF/container.xml",
            r#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#,
        );
        add_epub_entry(
            &mut zip,
            "OEBPS/content.opf",
            r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>FFI Smoke</dc:title><dc:creator>FolioForge</dc:creator><dc:language>en</dc:language></metadata><manifest><item id="c" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c"/></spine></package>"#,
        );
        add_epub_entry(
            &mut zip,
            "OEBPS/chapter.xhtml",
            r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>FFI</h1><p>Smoke.</p></body></html>"#,
        );
        zip.finish().unwrap();

        let request = ConversionRequest {
            input: input.clone(),
            output: output.clone(),
            target: folio_core::Target::KFX,
            options: folio_core::ConversionOptions::default(),
            edit: Default::default(),
        };
        let request_json = serde_json::to_string(&request).unwrap();
        let request_c = CString::new(request_json).unwrap();
        let cancellation = folio_cancellation_new();
        let mut events: Vec<String> = Vec::new();
        let result = unsafe {
            folio_convert_with_progress(
                request_c.as_ptr(),
                cancellation,
                Some(collect_progress),
                (&mut events as *mut Vec<String>).cast(),
            )
        };
        assert!(!result.is_null());
        assert_eq!(unsafe { (*result).code }, 0);
        let ffi_report: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr((*result).json).to_str().unwrap()).unwrap()
        };
        unsafe {
            folio_result_free(result);
            folio_cancellation_free(cancellation);
        }

        let direct_request = ConversionRequest {
            output: direct_output.clone(),
            ..request
        };
        folio_core::convert(&direct_request).unwrap();
        assert_eq!(
            std::fs::read(&output).unwrap(),
            std::fs::read(&direct_output).unwrap()
        );
        assert_eq!(ffi_report["output_path"], output.to_string_lossy().as_ref());
        assert!(events
            .iter()
            .any(|event| event.contains(r#""stage":"Finished""#)));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_ffi_uses_core_batch_naming_edits_and_progress() {
        let root = unique_test_root("folioforge-ffi-batch");
        let first = root.join("inputs/first/book.epub");
        let second = root.join("inputs/second/book.epub");
        let output_dir = root.join("output");
        write_epub_fixture(&first, "Original One");
        write_epub_fixture(&second, "Original Two");

        let first_edit = folio_edit::BookEditPlan {
            metadata: folio_edit::MetadataEdit {
                title: Some("Edited One".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        let second_edit = folio_edit::BookEditPlan {
            metadata: folio_edit::MetadataEdit {
                title: Some("Edited Two".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        let inputs = vec![
            folio_batch::BatchInput {
                source: first,
                relative_path: Some("book.epub".into()),
                output_root: Some(output_dir.clone()),
                edit: Some(first_edit),
            },
            folio_batch::BatchInput {
                source: second,
                relative_path: Some("book.epub".into()),
                output_root: Some(output_dir.clone()),
                edit: Some(second_edit),
            },
        ];
        let request_value =
            batch_request_value(inputs, &output_dir, folio_batch::BatchMode::BestEffort);
        let request = CString::new(request_value.to_string()).unwrap();
        let cancellation = folio_cancellation_new();
        let mut events: Vec<String> = Vec::new();
        let result = unsafe {
            folio_batch_convert_with_progress(
                request.as_ptr(),
                cancellation,
                Some(collect_progress),
                (&mut events as *mut Vec<String>).cast(),
            )
        };
        assert!(!result.is_null());
        assert_eq!(unsafe { (*result).code }, 0);
        let report: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr((*result).json).to_str().unwrap()).unwrap()
        };
        unsafe {
            folio_result_free(result);
            folio_cancellation_free(cancellation);
        }

        assert_eq!(report["succeeded"], 2);
        assert_eq!(report["failed"], 0);
        assert_eq!(report["items"][0]["relative_output"], "book.epub");
        assert_eq!(report["items"][1]["relative_output"], "book-1.epub");
        assert!(events
            .iter()
            .any(|event| event.contains("\"current_item\":1")));
        assert!(events
            .iter()
            .any(|event| event.contains("\"current_item\":2")));

        let first_output = output_dir.join("book.epub");
        let second_output = output_dir.join("book-1.epub");
        assert_eq!(
            folio_core::FormatRegistry
                .import_path(&first_output)
                .unwrap()
                .book
                .metadata
                .title
                .as_deref(),
            Some("Edited One")
        );
        assert_eq!(
            folio_core::FormatRegistry
                .import_path(&second_output)
                .unwrap()
                .book
                .metadata
                .title
                .as_deref(),
            Some("Edited Two")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn strict_batch_ffi_rolls_back_outputs_when_a_later_item_fails() {
        let root = unique_test_root("folioforge-ffi-strict-batch");
        let first = root.join("inputs/first.epub");
        let broken = root.join("inputs/broken.epub");
        let output_dir = root.join("output");
        write_epub_fixture(&first, "Valid");
        std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
        std::fs::write(&broken, b"not an EPUB").unwrap();
        let inputs = [first, broken]
            .into_iter()
            .map(|source| folio_batch::BatchInput {
                relative_path: source.file_name().map(PathBuf::from),
                source,
                output_root: Some(output_dir.clone()),
                edit: None,
            })
            .collect();
        let request_value =
            batch_request_value(inputs, &output_dir, folio_batch::BatchMode::Strict);
        let request = CString::new(request_value.to_string()).unwrap();
        let cancellation = folio_cancellation_new();
        let result = unsafe {
            folio_batch_convert_with_progress(request.as_ptr(), cancellation, None, ptr::null_mut())
        };
        assert!(!result.is_null());
        assert_eq!(unsafe { (*result).code }, 0);
        let report: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr((*result).json).to_str().unwrap()).unwrap()
        };
        unsafe {
            folio_result_free(result);
            folio_cancellation_free(cancellation);
        }
        assert_eq!(report["aborted"], true);
        assert_eq!(report["succeeded"], 0);
        assert_eq!(report["failed"], 2);
        assert!(std::fs::read_dir(&output_dir).unwrap().next().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
