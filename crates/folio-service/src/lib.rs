//! Local HTTP adapter for offline conversion and opt-in online resources.
//!
//! The implementation intentionally keeps HTTP concerns here.  Uploaded
//! bytes become files in an isolated work directory and every conversion is
//! delegated to folio-batch/folio-core.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use folio_batch::{BatchMode, BatchOptions, BatchReport, CollisionPolicy};
use folio_core::{DegradationMode, DegradationOptions, Target};
use folio_edit::{BookEditPlan, CoverEdit, CoverFit, StyleEdit};
use folio_online::{
    CacheStore, DownloadRequest, GoogleFontsProvider, HttpTransport, MemoryCache,
    MetadataCandidate, MetadataMergeAction, MetadataMergePlan, OnlineBoundary, OnlineError,
    OnlineSettings, OpenLibraryCoverProvider, OpenLibraryProvider, ProviderRequest, UreqTransport,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

const DEFAULT_MAX_BODY: usize = 512 * 1024 * 1024;
const DEFAULT_MAX_FILES: usize = 256;
const DEFAULT_MAX_JOB_SECONDS: u64 = 1800;
const MAX_ONLINE_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_ACTIVE_JOBS: usize = 16;
const MAX_RETAINED_JOBS: usize = 128;
const JOB_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
static JOB_COUNTER: AtomicU64 = AtomicU64::new(1);
const LOGO_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/folioforge-logo.png"
));

#[derive(Clone, Debug)]
pub struct ServiceConfig {
    pub bind: String,
    pub work_dir: PathBuf,
    pub max_body_bytes: usize,
    pub max_files: usize,
    pub max_job_seconds: u64,
    /// Online lookups are opt-in. The core conversion path remains offline.
    pub online_enabled: bool,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:8080".to_owned(),
            work_dir: PathBuf::from("/work"),
            max_body_bytes: DEFAULT_MAX_BODY,
            max_files: DEFAULT_MAX_FILES,
            max_job_seconds: DEFAULT_MAX_JOB_SECONDS,
            online_enabled: false,
        }
    }
}

impl ServiceConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(value) = std::env::var("FOLIOFORGE_BIND") {
            config.bind = value;
        }
        if let Ok(value) = std::env::var("FOLIOFORGE_WORK_DIR") {
            config.work_dir = PathBuf::from(value);
        }
        if let Ok(value) = std::env::var("FOLIOFORGE_MAX_BODY_BYTES") {
            if let Ok(value) = value.parse() {
                config.max_body_bytes = value;
            }
        }
        if let Ok(value) = std::env::var("FOLIOFORGE_MAX_FILES") {
            if let Ok(value) = value.parse() {
                config.max_files = value;
            }
        }
        if let Ok(value) = std::env::var("FOLIOFORGE_MAX_JOB_SECONDS") {
            if let Ok(value) = value.parse() {
                config.max_job_seconds = value;
            }
        }
        config.online_enabled = std::env::var("FOLIOFORGE_ONLINE_ENABLED")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes"
                )
            })
            .unwrap_or(false);
        config
    }
}

#[derive(Clone, Debug, Serialize)]
struct JobView {
    id: String,
    status: String,
    progress: f32,
    message: String,
    report: Option<BatchReport>,
    error: Option<String>,
    download: Option<String>,
}

#[derive(Clone)]
struct JobState {
    view: JobView,
    output_path: Option<PathBuf>,
    cancellation: folio_core::CancellationToken,
    worker_done: bool,
    finished_at: Option<SystemTime>,
}

#[derive(Clone)]
struct AppState {
    config: ServiceConfig,
    jobs: Arc<Mutex<HashMap<String, JobState>>>,
    active_jobs: Arc<AtomicUsize>,
    online: Arc<OnlineBoundary<MemoryCache>>,
    metadata_provider: OpenLibraryProvider,
    cover_provider: OpenLibraryCoverProvider,
    font_provider: Option<GoogleFontsProvider>,
}

impl AppState {
    fn new(config: ServiceConfig, google_fonts_api_key: Option<String>) -> Self {
        Self::with_transport(
            config,
            google_fonts_api_key,
            Arc::new(UreqTransport::default()),
        )
    }

    fn with_transport(
        config: ServiceConfig,
        google_fonts_api_key: Option<String>,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        let font_provider = google_fonts_api_key
            .filter(|key| !key.trim().is_empty())
            .map(|key| GoogleFontsProvider::with_transport(key, transport.clone()));
        Self {
            online: Arc::new(OnlineBoundary::new(
                OnlineSettings {
                    enabled: config.online_enabled,
                },
                MemoryCache::default(),
            )),
            metadata_provider: OpenLibraryProvider::with_transport(transport.clone()),
            cover_provider: OpenLibraryCoverProvider::with_transport(transport),
            font_provider,
            config,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            active_jobs: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("service I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("service JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("service request is invalid: {0}")]
    BadRequest(String),
    #[error("too many conversion jobs are active; retry after a job finishes")]
    TooManyJobs,
    #[error(transparent)]
    Online(#[from] OnlineError),
}

impl ServiceError {
    const fn status(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "400 Bad Request",
            Self::TooManyJobs => "429 Too Many Requests",
            Self::Online(OnlineError::Disabled) => "503 Service Unavailable",
            Self::Online(OnlineError::InvalidRequest(_)) => "400 Bad Request",
            Self::Online(OnlineError::Unavailable(_)) => "502 Bad Gateway",
            Self::Online(OnlineError::InvalidAsset) => "422 Unprocessable Content",
            Self::Online(OnlineError::ConfirmationRequired) => "428 Precondition Required",
            Self::Io(_) | Self::Json(_) => "500 Internal Server Error",
        }
    }
}

struct ActiveJobPermit(Arc<AtomicUsize>);

impl ActiveJobPermit {
    fn acquire(active_jobs: Arc<AtomicUsize>) -> Result<Self, ServiceError> {
        active_jobs
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_ACTIVE_JOBS).then_some(active + 1)
            })
            .map(|_| Self(active_jobs))
            .map_err(|_| ServiceError::TooManyJobs)
    }
}

impl Drop for ActiveJobPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct JobWorkspaceGuard {
    path: PathBuf,
    keep: bool,
}

impl JobWorkspaceGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, keep: false }
    }

    fn preserve(&mut self) {
        self.keep = true;
    }
}

impl Drop for JobWorkspaceGuard {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

struct JobCompletionGuard {
    jobs: Arc<Mutex<HashMap<String, JobState>>>,
    id: String,
    finished: bool,
}

impl JobCompletionGuard {
    fn new(jobs: Arc<Mutex<HashMap<String, JobState>>>, id: String) -> Self {
        Self {
            jobs,
            id,
            finished: false,
        }
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        mark_job_worker_done(&self.jobs, &self.id);
        self.finished = true;
    }
}

impl Drop for JobCompletionGuard {
    fn drop(&mut self) {
        self.finish();
    }
}

pub fn run(config: ServiceConfig) -> Result<(), ServiceError> {
    fs::create_dir_all(&config.work_dir)?;
    prune_orphan_job_directories(&config.work_dir, config.max_job_seconds)?;
    let listener = TcpListener::bind(&config.bind)?;
    let fonts_api_key = std::env::var("FOLIOFORGE_GOOGLE_FONTS_API_KEY").ok();
    let state = AppState::new(config, fonts_api_key);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = state.clone();
                std::thread::spawn(move || {
                    let _ = handle_connection(stream, &state);
                });
            }
            Err(error) => eprintln!("folio-service accept error: {error}"),
        }
    }
    Ok(())
}

pub fn healthcheck(address: &str) -> Result<(), ServiceError> {
    let mut stream = TcpStream::connect(address)?;
    stream.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    if response.starts_with("HTTP/1.1 200") && response.contains("\"status\":\"ok\"") {
        Ok(())
    } else {
        Err(ServiceError::BadRequest(
            "health endpoint did not return ok".to_owned(),
        ))
    }
}

fn handle_connection(mut stream: TcpStream, state: &AppState) -> Result<(), ServiceError> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let request = read_request(&mut stream, state.config.max_body_bytes)?;
    let response = route(&request, state);
    write_response(&mut stream, response)?;
    Ok(())
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

#[derive(Debug)]
struct HttpResponse {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
    extra_headers: Vec<(&'static str, String)>,
}

fn read_request(stream: &mut TcpStream, max_body: usize) -> Result<HttpRequest, ServiceError> {
    let mut head = Vec::new();
    let mut one = [0u8; 1];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        if head.len() >= 128 * 1024 {
            return Err(ServiceError::BadRequest(
                "request headers are too large".to_owned(),
            ));
        }
        stream.read_exact(&mut one)?;
        head.push(one[0]);
    }
    let marker = head
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| ServiceError::BadRequest("request headers are incomplete".to_owned()))?;
    let header_text = String::from_utf8_lossy(&head[..marker]);
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| ServiceError::BadRequest("request line is missing".to_owned()))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| ServiceError::BadRequest("HTTP method is missing".to_owned()))?
        .to_owned();
    let path = request_parts
        .next()
        .ok_or_else(|| ServiceError::BadRequest("HTTP path is missing".to_owned()))?
        .split('?')
        .next()
        .unwrap_or("/")
        .to_owned();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length = headers
        .get("content-length")
        .map(|value| {
            value.parse::<usize>().map_err(|_| {
                ServiceError::BadRequest("content-length is not a valid integer".to_owned())
            })
        })
        .transpose()?
        .unwrap_or(0);
    if length > max_body {
        return Err(ServiceError::BadRequest(
            "request body exceeds configured limit".to_owned(),
        ));
    }
    let already_read = head.len().saturating_sub(marker + 4);
    let mut body = head[marker + 4..].to_vec();
    if already_read < length {
        body.resize(length, 0);
        stream.read_exact(&mut body[already_read..])?;
    } else {
        body.truncate(length);
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn route(request: &HttpRequest, state: &AppState) -> HttpResponse {
    if let Err(error) = prune_terminal_jobs(state) {
        return json_error(error.status(), error.to_string());
    }
    let result = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => Ok(HttpResponse {
            status: "200 OK",
            content_type: "text/html; charset=utf-8",
            body: INDEX_HTML_PHASE3.as_bytes().to_vec(),
            extra_headers: Vec::new(),
        }),
        ("GET", "/health") => json_response(&serde_json::json!({
            "status": "ok",
            "service": "folio-service",
            "offline_core": true
        })),
        ("GET", "/capabilities") => capabilities(),
        ("GET", "/online/status") => online_status(state),
        ("GET", "/logo.png") => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/png",
            body: LOGO_BYTES.to_vec(),
            extra_headers: vec![("cache-control", "public, max-age=3600".to_owned())],
        }),
        ("POST", "/preflight") => preflight(request, state),
        ("POST", "/preview") => preview(request, state),
        ("POST", "/convert") => start_job(request, state),
        ("POST", "/online/metadata/search") => online_metadata_search(request, state),
        ("POST", "/online/covers/search") => online_cover_search(request, state),
        ("POST", "/online/fonts/search") => online_font_search(request, state),
        ("POST", "/online/metadata/merge-plan") => online_metadata_merge(request),
        ("POST", "/online/download") => online_download(request, state),
        _ if matches!(request.method.as_str(), "GET" | "POST")
            && request.path.starts_with("/jobs/") =>
        {
            job_route(request, state)
        }
        _ => Ok(HttpResponse {
            status: "404 Not Found",
            content_type: "text/plain; charset=utf-8",
            body: b"not found".to_vec(),
            extra_headers: Vec::new(),
        }),
    };
    match result {
        Ok(response) => response,
        Err(error) => json_error(error.status(), error.to_string()),
    }
}

fn prune_terminal_jobs(state: &AppState) -> Result<(), ServiceError> {
    let now = SystemTime::now();
    let mut removed_ids = Vec::new();
    {
        let mut jobs = state
            .jobs
            .lock()
            .map_err(|_| ServiceError::BadRequest("job state lock poisoned".to_owned()))?;
        let mut finished = jobs
            .iter()
            .filter(|(_, job)| job.worker_done)
            .map(|(id, job)| (id.clone(), job.finished_at.unwrap_or(now)))
            .collect::<Vec<_>>();
        finished.sort_by_key(|(_, finished_at)| *finished_at);

        for (id, finished_at) in &finished {
            if now
                .duration_since(*finished_at)
                .is_ok_and(|age| age >= JOB_RETENTION)
            {
                removed_ids.push(id.clone());
            }
        }
        let retained = finished
            .iter()
            .filter(|(id, _)| !removed_ids.contains(id))
            .collect::<Vec<_>>();
        let excess = retained.len().saturating_sub(MAX_RETAINED_JOBS);
        removed_ids.extend(retained.iter().take(excess).map(|(id, _)| id.to_string()));
        removed_ids.sort();
        removed_ids.dedup();
        for id in &removed_ids {
            jobs.remove(id);
        }
    }

    let jobs_root = state.config.work_dir.join("jobs");
    for id in removed_ids {
        if !is_generated_job_id(&id) {
            continue;
        }
        let path = jobs_root.join(id);
        if let Err(error) = fs::remove_dir_all(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("folio-service could not prune a finished job workspace: {error}");
            }
        }
    }
    Ok(())
}

fn prune_orphan_job_directories(
    work_dir: &Path,
    max_job_seconds: u64,
) -> Result<(), std::io::Error> {
    let jobs_root = work_dir.join("jobs");
    fs::create_dir_all(&jobs_root)?;
    let now = SystemTime::now();
    let stale_after = JOB_RETENTION.saturating_add(Duration::from_secs(max_job_seconds.max(1)));
    for entry in fs::read_dir(&jobs_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        if !is_generated_job_id(&id) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
            continue;
        };
        if now
            .duration_since(modified)
            .is_ok_and(|age| age >= stale_after)
        {
            if let Err(error) = fs::remove_dir_all(entry.path()) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("folio-service could not prune an orphan job workspace: {error}");
                }
            }
        }
    }
    Ok(())
}

fn is_generated_job_id(value: &str) -> bool {
    let Some((timestamp, counter)) = value.split_once('-') else {
        return false;
    };
    !timestamp.is_empty()
        && !counter.is_empty()
        && timestamp
            .chars()
            .all(|character| character.is_ascii_hexdigit())
        && counter
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn online_status(state: &AppState) -> Result<HttpResponse, ServiceError> {
    json_response(&serde_json::json!({
        "enabled": state.online.settings.enabled,
        "core_offline": true,
        "user_action_required": true,
        "providers": {
            "openlibrary_metadata": true,
            "openlibrary_covers": true,
            "google_fonts": state.font_provider.is_some()
        }
    }))
}

fn online_metadata_search(
    request: &HttpRequest,
    state: &AppState,
) -> Result<HttpResponse, ServiceError> {
    let provider_request: ProviderRequest = parse_online_json(request)?;
    let candidates = state
        .online
        .search(&state.metadata_provider, &provider_request)?;
    json_response(&candidates)
}

fn online_cover_search(
    request: &HttpRequest,
    state: &AppState,
) -> Result<HttpResponse, ServiceError> {
    let provider_request: ProviderRequest = parse_online_json(request)?;
    let candidates = state
        .online
        .search_covers(&state.cover_provider, &provider_request)?;
    json_response(&candidates)
}

fn online_font_search(
    request: &HttpRequest,
    state: &AppState,
) -> Result<HttpResponse, ServiceError> {
    let provider_request: ProviderRequest = parse_online_json(request)?;
    let provider = state.font_provider.as_ref().ok_or_else(|| {
        ServiceError::Online(OnlineError::InvalidRequest(
            "Google Fonts is unavailable because no API key is configured".to_owned(),
        ))
    })?;
    let candidates = state.online.search_fonts(provider, &provider_request)?;
    json_response(&candidates)
}

#[derive(Deserialize)]
struct MetadataMergeRequest {
    current: folio_model::Metadata,
    candidate: MetadataCandidate,
    fields: BTreeMap<String, MetadataMergeAction>,
    #[serde(default)]
    confirmed: bool,
}

fn online_metadata_merge(request: &HttpRequest) -> Result<HttpResponse, ServiceError> {
    let request: MetadataMergeRequest = parse_online_json(request)?;
    let plan = MetadataMergePlan {
        candidate: request.candidate,
        fields: request.fields,
        requires_confirmation: true,
    }
    .build_edit_plan(&request.current, request.confirmed)?;
    json_response(&plan)
}

fn online_download(request: &HttpRequest, state: &AppState) -> Result<HttpResponse, ServiceError> {
    let download: DownloadRequest = parse_online_json(request)?;
    let asset = match download.provider.as_str() {
        "openlibrary_cover" => state.online.download(&state.cover_provider, &download)?,
        "google_fonts" => {
            let provider = state.font_provider.as_ref().ok_or_else(|| {
                ServiceError::Online(OnlineError::InvalidRequest(
                    "Google Fonts is unavailable because no API key is configured".to_owned(),
                ))
            })?;
            state.online.download(provider, &download)?
        }
        _ => {
            return Err(ServiceError::Online(OnlineError::InvalidRequest(
                "unknown asset provider".to_owned(),
            )))
        }
    };
    let content_type = match asset.media_type.as_str() {
        "image/png" => "image/png",
        "image/jpeg" => "image/jpeg",
        "image/gif" => "image/gif",
        "font/woff2" => "font/woff2",
        "font/woff" => "font/woff",
        "font/otf" => "font/otf",
        "font/ttf" => "font/ttf",
        "font/collection" => "font/collection",
        _ => {
            return Err(ServiceError::Online(OnlineError::InvalidAsset));
        }
    };
    Ok(HttpResponse {
        status: "200 OK",
        content_type,
        body: asset.bytes,
        extra_headers: vec![
            ("cache-control", "no-store".to_owned()),
            ("x-content-type-options", "nosniff".to_owned()),
            (
                "x-folioforge-asset-id",
                encode_header_component(&asset.cache_key),
            ),
        ],
    })
}

fn encode_header_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
            encoded.push(char::from(b"0123456789ABCDEF"[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

fn parse_online_json<T: DeserializeOwned>(request: &HttpRequest) -> Result<T, ServiceError> {
    if request.body.len() > MAX_ONLINE_REQUEST_BYTES {
        return Err(ServiceError::BadRequest(
            "online request JSON exceeds the 1 MiB limit".to_owned(),
        ));
    }
    serde_json::from_slice(&request.body).map_err(|error| {
        ServiceError::BadRequest(format!("online request JSON is invalid: {error}"))
    })
}

fn capabilities() -> Result<HttpResponse, ServiceError> {
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
    json_response(&serde_json::json!({
        "core_version": folio_core::CORE_VERSION,
        "formats": registry.formats(),
        "input_formats": input_formats,
        "targets": registry.output_formats(),
        "degradation_modes": ["Strict", "Compatible", "Readable"],
        "capability_profiles": folio_core::capabilities(),
        "offline": true,
        "network": false,
    }))
}

fn preflight(request: &HttpRequest, state: &AppState) -> Result<HttpResponse, ServiceError> {
    let form = parse_multipart(request)?;
    validate_file_count(&form, state.config.max_files)?;
    let target = parse_target(form.field("target").unwrap_or("kfx"))?;
    let mode = parse_mode(form.field("mode").unwrap_or("compatible"))?;
    let mut edit = edit_plan_from_form(&form)?;
    apply_cover_upload(&form, &mut edit)?;
    apply_font_upload(&form, &mut edit)?;
    apply_online_assets(&form, state, &mut edit)?;
    let mut items = Vec::new();
    let mut exact = 0usize;
    let mut high = 0usize;
    let mut compatible = 0usize;
    let mut reduced = 0usize;
    let mut failed = 0usize;
    for file in form.files.iter().filter(|file| file.field == "files[]") {
        let path = state.config.work_dir.join("preflight").join(unique_id());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &file.bytes)?;
        let analysis =
            folio_core::preflight(&path, target, mode, &DegradationOptions::default(), &edit);
        let _ = fs::remove_file(&path);
        match analysis {
            Ok(report) => {
                match report.plan.quality {
                    folio_core::CompatibilityQuality::Exact => exact += 1,
                    folio_core::CompatibilityQuality::High => high += 1,
                    folio_core::CompatibilityQuality::Compatible => compatible += 1,
                    folio_core::CompatibilityQuality::Reduced
                    | folio_core::CompatibilityQuality::SevereLoss => reduced += 1,
                }
                items.push(serde_json::json!({
                    "name": file.name,
                    "detected_format": report.detected_format,
                    "error": null,
                    "input_report": report.input_report,
                    "source_metadata": report.source_metadata,
                    "semantic_report": {
                        "valid": report.semantic_report.valid,
                        "document_count": report.semantic_report.document_count,
                        "resource_count": report.semantic_report.resource_count,
                        "feature_count": report.semantic_report.feature_count,
                        "metadata": report.semantic_metadata,
                        "warnings": report.semantic_report.warnings,
                        "input_loss": report.semantic_report.input_loss,
                    },
                    "quality": format!("{:?}", report.plan.quality),
                    "blocked": report.plan.blocked,
                    "degradation": report.plan.report(),
                }));
            }
            Err(error) => {
                failed += 1;
                items.push(serde_json::json!({
                    "name": file.name,
                    "detected_format": null,
                    "error": error.to_string(),
                    "input_report": null,
                    "source_metadata": null,
                    "semantic_report": null,
                    "quality": null,
                    "blocked": null,
                    "degradation": null,
                }));
            }
        }
    }
    json_response(&serde_json::json!({
        "target": target.format().name(),
        "mode": mode,
        "summary": {
            "total": items.len(),
            "exact": exact,
            "high": high,
            "compatible": compatible,
            "reduced": reduced,
            "failed": failed
        },
        "files": items
    }))
}

fn preview(request: &HttpRequest, state: &AppState) -> Result<HttpResponse, ServiceError> {
    let form = parse_multipart(request)?;
    validate_file_count(&form, state.config.max_files)?;
    let target_value = form.field("target").unwrap_or("epub").to_owned();
    let mode_value = form.field("mode").unwrap_or("compatible").to_owned();
    let preview_settings = parse_preview_settings(&form)?;
    let mut edit = edit_plan_from_form(&form)?;
    apply_cover_upload(&form, &mut edit)?;
    apply_font_upload(&form, &mut edit)?;
    apply_online_assets(&form, state, &mut edit)?;
    let preview_files = form
        .files
        .iter()
        .filter(|file| file.field == "files[]")
        .collect::<Vec<_>>();
    if preview_files.len() != 1 {
        return Err(ServiceError::BadRequest(
            "exactly one input file is required for preview".to_owned(),
        ));
    }
    let file = preview_files[0];
    if !is_supported_upload(&file.name) {
        return Err(ServiceError::BadRequest(format!(
            "unsupported uploaded file: {}",
            file.name
        )));
    }
    let path = state.config.work_dir.join("preview").join(unique_id());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, &file.bytes)?;
    let result = (|| {
        let target = parse_target(&target_value)?;
        let mode = parse_mode(&mode_value)?;
        let bundle = folio_core::preview_with_settings(
            &path,
            target,
            mode,
            &DegradationOptions::default(),
            &edit,
            &preview_settings,
        )
        .map_err(|error| ServiceError::BadRequest(error.to_string()))?;
        json_response(&bundle)
    })();
    let _ = fs::remove_file(&path);
    result
}

fn start_job(request: &HttpRequest, state: &AppState) -> Result<HttpResponse, ServiceError> {
    let form = parse_multipart(request)?;
    validate_file_count(&form, state.config.max_files)?;
    let input_files = form
        .files
        .iter()
        .filter(|file| file.field == "files[]")
        .collect::<Vec<_>>();
    if input_files.is_empty() {
        return Err(ServiceError::BadRequest(
            "at least one file is required".to_owned(),
        ));
    }
    let job_permit = ActiveJobPermit::acquire(Arc::clone(&state.active_jobs))?;
    let mut edit = edit_plan_from_form(&form)?;
    apply_cover_upload(&form, &mut edit)?;
    apply_font_upload(&form, &mut edit)?;
    apply_online_assets(&form, state, &mut edit)?;
    let target = parse_target(form.field("target").unwrap_or("kfx"))?;
    let mode = parse_mode(form.field("mode").unwrap_or("compatible"))?;
    let batch_mode = match form.field("batch_mode").unwrap_or("best_effort") {
        "strict" => BatchMode::Strict,
        _ => BatchMode::BestEffort,
    };
    let job_id = unique_id();
    let job_root = state.config.work_dir.join("jobs").join(&job_id);
    let input_root = job_root.join("input");
    let output_root = job_root.join("result");
    let mut uploaded_paths = BTreeSet::new();
    let mut upload_plan = Vec::with_capacity(input_files.len());
    for file in input_files {
        if !is_supported_upload(&file.name) {
            return Err(ServiceError::BadRequest(format!(
                "unsupported uploaded file: {}",
                file.name
            )));
        }
        let relative = safe_relative_path(&file.name)?;
        if !uploaded_paths.insert(relative.clone()) {
            return Err(ServiceError::BadRequest(format!(
                "duplicate uploaded path: {}",
                file.name
            )));
        }
        upload_plan.push((file, relative));
    }
    let mut workspace_guard = JobWorkspaceGuard::new(job_root.clone());
    fs::create_dir_all(&input_root)?;
    for (file, relative) in upload_plan {
        let path = input_root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, &file.bytes)?;
    }
    let cancellation = folio_core::CancellationToken::new();
    let initial = JobView {
        id: job_id.clone(),
        status: "queued".to_owned(),
        progress: 0.0,
        message: "files stored in isolated job workspace".to_owned(),
        report: None,
        error: None,
        download: None,
    };
    {
        let mut jobs = state
            .jobs
            .lock()
            .map_err(|_| ServiceError::BadRequest("job state lock poisoned".to_owned()))?;
        jobs.insert(
            job_id.clone(),
            JobState {
                view: initial.clone(),
                output_path: None,
                cancellation: cancellation.clone(),
                worker_done: false,
                finished_at: None,
            },
        );
    }
    workspace_guard.preserve();
    let jobs = Arc::clone(&state.jobs);
    let thread_job_id = job_id.clone();
    let max_job_duration = Duration::from_secs(state.config.max_job_seconds.max(1));
    let (done_sender, done_receiver) = std::sync::mpsc::sync_channel(1);
    let watchdog_jobs = Arc::clone(&jobs);
    let watchdog_id = thread_job_id.clone();
    let watchdog_cancel = cancellation.clone();
    std::thread::spawn(move || match done_receiver.recv_timeout(max_job_duration) {
        Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            watchdog_cancel.cancel();
            update_job(&watchdog_jobs, &watchdog_id, |view| {
                view.status = "timed_out".to_owned();
                view.message = "job exceeded the configured time limit".to_owned();
                view.error = Some("job timed out and cancellation was requested".to_owned());
            });
        }
    });
    std::thread::spawn(move || {
        let _job_permit = job_permit;
        let mut completion = JobCompletionGuard::new(Arc::clone(&jobs), thread_job_id.clone());
        let options = BatchOptions {
            target,
            conversion: folio_core::ConversionOptions {
                degradation_mode: mode,
                ..folio_core::ConversionOptions::default()
            },
            batch_mode,
            collision_policy: CollisionPolicy::Rename,
            preserve_tree: true,
            max_concurrent_jobs: 0,
            per_job_parallelism: 1,
            memory_budget_bytes: None,
            fail_fast: false,
            edit,
        };
        update_job(&jobs, &thread_job_id, |view| {
            view.status = "running".to_owned();
            view.message = "converting through folio-core".to_owned();
        });
        let result = folio_batch::convert_directory_with_cancellation(
            &input_root,
            &output_root,
            &options,
            &cancellation,
            |current, total, event| {
                update_job(&jobs, &thread_job_id, |view| {
                    view.progress = event
                        .fraction
                        .unwrap_or_else(|| current as f32 / total.max(1) as f32);
                    view.message = event.message.clone();
                });
            },
        );
        match result {
            Ok(report) => {
                let successes = report.items.iter().filter(|item| item.success).count();
                let output = if successes == 1 {
                    report.items.iter().find_map(|item| item.output.clone())
                } else if successes > 1 {
                    let archive = job_root.join("FolioForge-output.zip");
                    if folio_batch::archive_report(&report, &archive).is_ok() {
                        Some(archive)
                    } else {
                        None
                    }
                } else {
                    None
                };
                update_job_with_output(&jobs, &thread_job_id, report, output);
            }
            Err(error) => update_job(&jobs, &thread_job_id, |view| {
                if view.status != "timed_out" {
                    view.status = "failed".to_owned();
                    view.error = Some(error.to_string());
                }
            }),
        }
        completion.finish();
        let _ = done_sender.send(());
    });
    json_response(&serde_json::json!({
        "job_id": job_id,
        "status": initial.status
    }))
}

fn edit_plan_from_form(form: &MultipartForm) -> Result<BookEditPlan, ServiceError> {
    let mut edit = match form.field("edit") {
        Some(value) if !value.trim().is_empty() => serde_json::from_str(value)?,
        _ => BookEditPlan::default(),
    };
    if let Some(title) = form.field("title").filter(|value| !value.trim().is_empty()) {
        edit.metadata.title = Some(title.to_owned());
    }
    if let Some(css) = form.field("css").filter(|value| !value.trim().is_empty()) {
        edit.styles.push(StyleEdit {
            css: Some(css.to_owned()),
            ..StyleEdit::default()
        });
    }
    Ok(edit)
}

fn parse_preview_settings(
    form: &MultipartForm,
) -> Result<folio_core::PreviewSettings, ServiceError> {
    let device = match form.field("preview_device").unwrap_or("e_reader") {
        "e_reader" => folio_core::PreviewDevice::EReader,
        "phone" => folio_core::PreviewDevice::Phone,
        "tablet" => folio_core::PreviewDevice::Tablet,
        value => {
            return Err(ServiceError::BadRequest(format!(
                "unknown preview device {value}"
            )))
        }
    };
    let orientation = match form.field("preview_orientation").unwrap_or("portrait") {
        "portrait" => folio_core::PreviewOrientation::Portrait,
        "landscape" => folio_core::PreviewOrientation::Landscape,
        value => {
            return Err(ServiceError::BadRequest(format!(
                "unknown preview orientation {value}"
            )))
        }
    };
    let font_size_percent = form
        .field("preview_font_size_percent")
        .unwrap_or("100")
        .parse::<u8>()
        .map_err(|_| {
            ServiceError::BadRequest(
                "preview font size must be an integer from 60 to 200".to_owned(),
            )
        })?;
    if !(60..=200).contains(&font_size_percent) {
        return Err(ServiceError::BadRequest(
            "preview font size must be from 60 to 200".to_owned(),
        ));
    }
    Ok(folio_core::PreviewSettings {
        device,
        orientation,
        font_size_percent,
    })
}

fn apply_cover_upload(form: &MultipartForm, edit: &mut BookEditPlan) -> Result<(), ServiceError> {
    let Some(file) = form.files.iter().find(|file| file.field == "cover") else {
        return Ok(());
    };
    if file.bytes.is_empty() || file.bytes.len() > 32 * 1024 * 1024 {
        return Err(ServiceError::BadRequest(
            "replacement cover must be non-empty and at most 32 MiB".to_owned(),
        ));
    }
    let media_type = detect_cover_media_type(&file.name, &file.bytes).ok_or_else(|| {
        ServiceError::BadRequest(
            "replacement cover must be a PNG, JPEG, GIF, or SVG image".to_owned(),
        )
    })?;
    let fit = parse_cover_fit(form)?;
    edit.cover = CoverEdit::Replace {
        file_name: file.name.clone(),
        media_type: media_type.to_owned(),
        bytes: file.bytes.clone(),
        fit,
    };
    Ok(())
}

fn apply_font_upload(form: &MultipartForm, edit: &mut BookEditPlan) -> Result<(), ServiceError> {
    let Some(file) = form.files.iter().find(|file| file.field == "font") else {
        return Ok(());
    };
    if file.bytes.is_empty() || file.bytes.len() > 32 * 1024 * 1024 {
        return Err(ServiceError::BadRequest(
            "replacement font must be non-empty and at most 32 MiB".to_owned(),
        ));
    }
    let media_type = detect_font_media_type(&file.name, &file.bytes).ok_or_else(|| {
        ServiceError::BadRequest(
            "replacement font must be a valid TTF, OTF, WOFF, WOFF2, or TTC asset".to_owned(),
        )
    })?;
    edit.fonts.replacement = Some(folio_edit::FontReplacement {
        file_name: file.name.clone(),
        media_type: media_type.to_owned(),
        bytes: file.bytes.clone(),
        family: edit.fonts.preferred_family.clone(),
    });
    Ok(())
}

fn apply_online_assets(
    form: &MultipartForm,
    state: &AppState,
    edit: &mut BookEditPlan,
) -> Result<(), ServiceError> {
    if let Some(asset_id) = form.field("cover_asset_id") {
        if form.files.iter().any(|file| file.field == "cover") {
            return Err(ServiceError::BadRequest(
                "provide either a local cover file or an online cover asset ID, not both"
                    .to_owned(),
            ));
        }
        let asset = cached_online_asset(state, asset_id, "openlibrary_cover")?;
        let extension = match asset.media_type.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            _ => {
                return Err(ServiceError::BadRequest(
                    "cached online asset is not a supported cover image".to_owned(),
                ));
            }
        };
        edit.cover = CoverEdit::Replace {
            file_name: format!("online-cover.{extension}"),
            media_type: asset.media_type,
            bytes: asset.bytes,
            fit: parse_cover_fit(form)?,
        };
    }
    if let Some(asset_id) = form.field("font_asset_id") {
        if form.files.iter().any(|file| file.field == "font") {
            return Err(ServiceError::BadRequest(
                "provide either a local font file or an online font asset ID, not both".to_owned(),
            ));
        }
        let asset = cached_online_asset(state, asset_id, "google_fonts")?;
        let extension = match asset.media_type.as_str() {
            "font/woff2" => "woff2",
            "font/woff" => "woff",
            "font/otf" => "otf",
            "font/ttf" => "ttf",
            "font/collection" => "ttc",
            _ => {
                return Err(ServiceError::BadRequest(
                    "cached online asset is not a supported font".to_owned(),
                ));
            }
        };
        edit.fonts.replacement = Some(folio_edit::FontReplacement {
            file_name: format!("online-font.{extension}"),
            media_type: asset.media_type,
            bytes: asset.bytes,
            family: edit.fonts.preferred_family.clone(),
        });
    }
    Ok(())
}

fn cached_online_asset(
    state: &AppState,
    asset_id: &str,
    expected_provider: &str,
) -> Result<folio_online::DownloadedAsset, ServiceError> {
    if asset_id.is_empty()
        || asset_id.len() > 512
        || asset_id.chars().any(char::is_control)
        || !asset_id.starts_with(&format!("{expected_provider}:"))
    {
        return Err(ServiceError::BadRequest(
            "online asset ID is invalid for this resource type".to_owned(),
        ));
    }
    state.online.cache.get(asset_id).ok_or_else(|| {
        ServiceError::BadRequest(
            "online asset is no longer cached; download it again before preview or conversion"
                .to_owned(),
        )
    })
}

fn parse_cover_fit(form: &MultipartForm) -> Result<CoverFit, ServiceError> {
    match form
        .field("cover_fit")
        .unwrap_or("Fit")
        .to_ascii_lowercase()
        .as_str()
    {
        "fill" => Ok(CoverFit::Fill),
        "fit" => Ok(CoverFit::Fit),
        _ => Err(ServiceError::BadRequest(
            "cover_fit must be Fit or Fill".to_owned(),
        )),
    }
}

fn detect_cover_media_type(file_name: &str, bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if extension == "svg" {
        let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)])
            .trim_start_matches('\u{feff}')
            .trim()
            .to_ascii_lowercase();
        if prefix.starts_with("<svg") || (prefix.starts_with("<?xml") && prefix.contains("<svg")) {
            return Some("image/svg+xml");
        }
    }
    None
}

fn detect_font_media_type(file_name: &str, bytes: &[u8]) -> Option<&'static str> {
    let detected = if bytes.starts_with(b"wOF2") {
        "font/woff2"
    } else if bytes.starts_with(b"wOFF") {
        "font/woff"
    } else if bytes.starts_with(b"OTTO") {
        "font/otf"
    } else if bytes.starts_with(b"ttcf") {
        "font/collection"
    } else if bytes.starts_with(&[0, 1, 0, 0]) || bytes.starts_with(b"true") {
        "font/ttf"
    } else {
        return None;
    };
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase();
    let extension_matches = matches!(
        (extension.as_str(), detected),
        ("woff2", "font/woff2")
            | ("woff", "font/woff")
            | ("otf", "font/otf")
            | ("ttf", "font/ttf")
            | ("ttc", "font/collection")
    );
    extension_matches.then_some(detected)
}

fn job_route(request: &HttpRequest, state: &AppState) -> Result<HttpResponse, ServiceError> {
    let remainder = request
        .path
        .strip_prefix("/jobs/")
        .ok_or_else(|| ServiceError::BadRequest("invalid job path".to_owned()))?;
    let mut segments = remainder.split('/');
    let id = segments
        .next()
        .filter(|value| {
            !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .ok_or_else(|| ServiceError::BadRequest("invalid job id".to_owned()))?;
    let state_value = state
        .jobs
        .lock()
        .map_err(|_| ServiceError::BadRequest("job state lock poisoned".to_owned()))?
        .get(id)
        .cloned()
        .ok_or_else(|| ServiceError::BadRequest("job not found".to_owned()))?;
    match segments.next() {
        None => json_response(&state_value.view),
        Some("cancel") if request.method == "POST" => {
            if state_value.view.status.starts_with("completed")
                || matches!(state_value.view.status.as_str(), "failed" | "timed_out")
            {
                return Err(ServiceError::BadRequest(
                    "job is already finished".to_owned(),
                ));
            }
            state_value.cancellation.cancel();
            update_job(&state.jobs, id, |view| {
                view.status = "cancelling".to_owned();
                view.message = "cancellation requested".to_owned();
            });
            json_response(&serde_json::json!({"id": id, "status": "cancelling"}))
        }
        Some("events") => {
            let payload = serde_json::to_string(&state_value.view)?;
            Ok(HttpResponse {
                status: "200 OK",
                content_type: "text/event-stream; charset=utf-8",
                body: format!("event: progress\ndata: {payload}\n\n").into_bytes(),
                extra_headers: vec![("Cache-Control", "no-cache".to_owned())],
            })
        }
        Some("download") => {
            let (path, body) = read_job_download(state, id)?;
            let filename = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("output.bin");
            Ok(HttpResponse {
                status: "200 OK",
                content_type: "application/octet-stream",
                body,
                extra_headers: vec![(
                    "Content-Disposition",
                    format!("attachment; filename=\"{}\"", safe_header(filename)),
                )],
            })
        }
        Some(_) => Err(ServiceError::BadRequest("unknown job endpoint".to_owned())),
    }
}

fn read_job_download(state: &AppState, id: &str) -> Result<(PathBuf, Vec<u8>), ServiceError> {
    // Hold the map lock until the output has been copied into the response
    // buffer. Retention pruning uses the same lock, so it cannot remove this
    // job workspace while the download is reading it.
    let jobs = state
        .jobs
        .lock()
        .map_err(|_| ServiceError::BadRequest("job state lock poisoned".to_owned()))?;
    let job = jobs
        .get(id)
        .ok_or_else(|| ServiceError::BadRequest("job not found".to_owned()))?;
    let path = job
        .output_path
        .clone()
        .ok_or_else(|| ServiceError::BadRequest("job output is not ready".to_owned()))?;
    let body = fs::read(&path)?;
    Ok((path, body))
}

fn update_job(
    jobs: &Arc<Mutex<HashMap<String, JobState>>>,
    id: &str,
    update: impl FnOnce(&mut JobView),
) {
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(id) {
            update(&mut job.view);
        }
    }
}

fn mark_job_worker_done(jobs: &Arc<Mutex<HashMap<String, JobState>>>, id: &str) {
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(id) {
            if job.worker_done {
                return;
            }
            if !is_terminal_job_status(&job.view.status) {
                job.view.status = "failed".to_owned();
                job.view.message = "conversion worker stopped unexpectedly".to_owned();
                job.view.error =
                    Some("conversion worker stopped before returning a report".to_owned());
            }
            job.worker_done = true;
            job.finished_at = Some(SystemTime::now());
        }
    }
}

fn is_terminal_job_status(status: &str) -> bool {
    status.starts_with("completed") || matches!(status, "failed" | "timed_out" | "cancelled")
}

fn update_job_with_output(
    jobs: &Arc<Mutex<HashMap<String, JobState>>>,
    id: &str,
    report: BatchReport,
    output: Option<PathBuf>,
) {
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(id) {
            if job.view.status != "timed_out" {
                job.view.status = if report.aborted {
                    "failed"
                } else if report.failed > 0 {
                    "completed_with_errors"
                } else {
                    "completed"
                }
                .to_owned();
                job.view.progress = 1.0;
                job.view.message = "conversion finished".to_owned();
            }
            job.view.report = Some(report);
            job.view.download = output.as_ref().map(|_| format!("/jobs/{id}/download"));
            job.output_path = output;
        }
    }
}

fn write_response(stream: &mut TcpStream, response: HttpResponse) -> Result<(), ServiceError> {
    write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        response.content_type,
        response.body.len()
    )?;
    for (key, value) in response.extra_headers {
        write!(stream, "{key}: {value}\r\n")?;
    }
    write!(stream, "\r\n")?;
    stream.write_all(&response.body)?;
    Ok(())
}

fn json_response(value: &impl Serialize) -> Result<HttpResponse, ServiceError> {
    Ok(HttpResponse {
        status: "200 OK",
        content_type: "application/json; charset=utf-8",
        body: serde_json::to_vec(value)?,
        extra_headers: Vec::new(),
    })
}

fn json_error(status: &'static str, message: String) -> HttpResponse {
    HttpResponse {
        status,
        content_type: "application/json; charset=utf-8",
        body: serde_json::to_vec(&serde_json::json!({"error": message}))
            .unwrap_or_else(|_| b"{\"error\":\"serialization failed\"}".to_vec()),
        extra_headers: Vec::new(),
    }
}

#[derive(Debug)]
struct MultipartFile {
    field: String,
    name: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Default)]
struct MultipartForm {
    fields: HashMap<String, String>,
    files: Vec<MultipartFile>,
}

impl MultipartForm {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }
}

fn parse_multipart(request: &HttpRequest) -> Result<MultipartForm, ServiceError> {
    let content_type = request
        .headers
        .get("content-type")
        .ok_or_else(|| ServiceError::BadRequest("multipart content type is required".to_owned()))?;
    let boundary = content_type
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("boundary="))
        .map(|value| value.trim_matches('"'))
        .ok_or_else(|| ServiceError::BadRequest("multipart boundary is missing".to_owned()))?;
    let delimiter = format!("--{boundary}").into_bytes();
    let mut form = MultipartForm::default();
    for part in split_bytes(&request.body, &delimiter).into_iter().skip(1) {
        let part = part.strip_prefix(b"\r\n").unwrap_or(part);
        let part = part.strip_suffix(b"\r\n").unwrap_or(part);
        let part = part.strip_suffix(b"--").unwrap_or(part);
        let Some((headers, content)) = split_once_bytes(part, b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(headers);
        let disposition = headers
            .lines()
            .find(|line| {
                line.to_ascii_lowercase()
                    .starts_with("content-disposition:")
            })
            .unwrap_or("");
        let name = disposition
            .split(';')
            .find_map(|value| value.trim().strip_prefix("name="))
            .map(|value| value.trim_matches('"'))
            .unwrap_or("");
        let filename = disposition
            .split(';')
            .find_map(|value| value.trim().strip_prefix("filename="))
            .map(|value| value.trim_matches('"'));
        if let Some(filename) = filename.filter(|value| !value.is_empty()) {
            form.files.push(MultipartFile {
                field: name.to_owned(),
                name: filename.to_owned(),
                bytes: content.to_vec(),
            });
        } else if !name.is_empty() {
            form.fields.insert(
                name.to_owned(),
                String::from_utf8_lossy(content).trim().to_owned(),
            );
        }
    }
    Ok(form)
}

fn validate_file_count(form: &MultipartForm, maximum: usize) -> Result<(), ServiceError> {
    let count = form
        .files
        .iter()
        .filter(|file| file.field == "files[]")
        .count();
    if count > maximum {
        return Err(ServiceError::BadRequest(format!(
            "request contains {count} input files; the configured limit is {maximum}"
        )));
    }
    Ok(())
}

fn split_bytes<'a>(value: &'a [u8], delimiter: &[u8]) -> Vec<&'a [u8]> {
    if delimiter.is_empty() {
        return vec![value];
    }
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut cursor = 0usize;
    while let Some(relative) = value[cursor..]
        .windows(delimiter.len())
        .position(|window| window == delimiter)
    {
        let position = cursor + relative;
        parts.push(&value[start..position]);
        cursor = position + delimiter.len();
        start = cursor;
    }
    parts.push(&value[start..]);
    parts
}

fn split_once_bytes<'a>(value: &'a [u8], delimiter: &[u8]) -> Option<(&'a [u8], &'a [u8])> {
    let position = value
        .windows(delimiter.len())
        .position(|window| window == delimiter)?;
    Some((&value[..position], &value[position + delimiter.len()..]))
}

fn safe_relative_path(value: &str) -> Result<PathBuf, ServiceError> {
    folio_input::normalize_relative_path(Path::new(value))
        .map_err(|error| ServiceError::BadRequest(error.to_string()))
}

fn safe_header(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn is_supported_upload(value: &str) -> bool {
    folio_input::is_supported_book_path(Path::new(value))
}

fn unique_id() -> String {
    let counter = JOB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{now:x}-{counter:x}")
}

fn parse_target(value: &str) -> Result<Target, ServiceError> {
    Target::parse(value).ok_or_else(|| ServiceError::BadRequest(format!("unknown target {value}")))
}

fn parse_mode(value: &str) -> Result<DegradationMode, ServiceError> {
    match value.to_ascii_lowercase().as_str() {
        "strict" => Ok(DegradationMode::Strict),
        "compatible" => Ok(DegradationMode::Compatible),
        "readable" => Ok(DegradationMode::Readable),
        _ => Err(ServiceError::BadRequest(format!(
            "unknown compatibility mode {value}"
        ))),
    }
}

const INDEX_HTML_PHASE3: &str = include_str!("../web/index.html");

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-service/src/lib.rs"]
mod tests;
