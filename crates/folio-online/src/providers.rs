use std::{collections::BTreeMap, sync::Arc, time::Duration};

use serde::Deserialize;

use super::{
    detect_media_type, AssetProvider, CoverCandidate, CoverProvider, DownloadRequest,
    DownloadedAsset, FontCandidate, FontProvider, Metadata, MetadataCandidate, MetadataProvider,
    OnlineError, ProviderRequest, MAX_COVER_BYTES, MAX_FONT_BYTES,
};

const OPEN_LIBRARY_RESPONSE_LIMIT: usize = 4 * 1024 * 1024;
const GOOGLE_FONTS_RESPONSE_LIMIT: usize = 2 * 1024 * 1024;
const OPEN_LIBRARY_FIELDS: &str =
    "key,title,author_name,publisher,first_publish_year,subject,isbn,cover_i,language";
const GOOGLE_FONTS_LICENSE_GUIDE: &str = "https://developers.google.com/fonts/faq";

/// Small, mockable HTTP surface. Implementations must honor the byte limit and
/// must not follow redirects; provider code still checks status and signatures.
pub trait HttpTransport: Send + Sync {
    fn get(&self, url: &str, max_response_bytes: usize) -> Result<HttpResponse, OnlineError>;
}

pub struct HttpResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// Production HTTPS transport with a fixed timeout, no redirects, and bounded
/// response bodies. Error text deliberately excludes URLs and response data.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(15)))
            .max_redirects(0)
            .user_agent("FolioForge/0.1")
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl HttpTransport for UreqTransport {
    fn get(&self, url: &str, max_response_bytes: usize) -> Result<HttpResponse, OnlineError> {
        if !url.starts_with("https://") || max_response_bytes == 0 {
            return Err(OnlineError::InvalidRequest(
                "online transport requires HTTPS and a nonzero response limit".to_owned(),
            ));
        }
        let mut response = self
            .agent
            .get(url)
            .call()
            .map_err(|_| OnlineError::Unavailable("HTTPS request failed".to_owned()))?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .body_mut()
            .with_config()
            .limit(max_response_bytes as u64)
            .read_to_vec()
            .map_err(|_| {
                OnlineError::Unavailable(
                    "response could not be read or exceeded its size limit".to_owned(),
                )
            })?;
        if body.len() > max_response_bytes {
            return Err(OnlineError::Unavailable(
                "response exceeded its size limit".to_owned(),
            ));
        }
        Ok(HttpResponse {
            status,
            content_type,
            body,
        })
    }
}

#[derive(Clone)]
pub struct OpenLibraryProvider {
    transport: Arc<dyn HttpTransport>,
}

impl Default for OpenLibraryProvider {
    fn default() -> Self {
        Self::with_transport(Arc::new(UreqTransport::default()))
    }
}

impl OpenLibraryProvider {
    pub fn with_transport(transport: Arc<dyn HttpTransport>) -> Self {
        Self { transport }
    }
}

impl MetadataProvider for OpenLibraryProvider {
    fn name(&self) -> &str {
        "openlibrary"
    }

    fn search(&self, request: &ProviderRequest) -> Result<Vec<MetadataCandidate>, OnlineError> {
        let docs = fetch_openlibrary_docs(self.transport.as_ref(), request)?;
        let mut candidates = Vec::new();
        for (index, doc) in docs.into_iter().take(request.result_limit()).enumerate() {
            let metadata = openlibrary_metadata(&doc);
            if metadata.title.is_none() && metadata.author_names().is_empty() {
                continue;
            }
            let key = valid_openlibrary_key(doc.key.as_deref().unwrap_or_default());
            let fallback = metadata
                .identifier_values()
                .first()
                .cloned()
                .unwrap_or_else(|| format!("result-{index}"));
            candidates.push(MetadataCandidate {
                candidate_id: format!("openlibrary:{}", key.unwrap_or(&fallback)),
                provider: self.name().to_owned(),
                confidence: estimate_confidence(request, &metadata),
                metadata,
                source_url: key.map(|key| format!("https://openlibrary.org{key}")),
            });
        }
        Ok(candidates)
    }
}

#[derive(Clone)]
pub struct OpenLibraryCoverProvider {
    transport: Arc<dyn HttpTransport>,
}

impl Default for OpenLibraryCoverProvider {
    fn default() -> Self {
        Self::with_transport(Arc::new(UreqTransport::default()))
    }
}

impl OpenLibraryCoverProvider {
    pub fn with_transport(transport: Arc<dyn HttpTransport>) -> Self {
        Self { transport }
    }
}

impl CoverProvider for OpenLibraryCoverProvider {
    fn name(&self) -> &str {
        "openlibrary_cover"
    }

    fn search(&self, request: &ProviderRequest) -> Result<Vec<CoverCandidate>, OnlineError> {
        let docs = fetch_openlibrary_docs(self.transport.as_ref(), request)?;
        Ok(docs
            .into_iter()
            .filter_map(|doc| {
                let cover_id = doc.cover_i?;
                let url = openlibrary_cover_url(cover_id);
                let key = valid_openlibrary_key(doc.key.as_deref().unwrap_or_default());
                Some(CoverCandidate {
                    candidate_id: format!("openlibrary-cover:{cover_id}"),
                    provider: CoverProvider::name(self).to_owned(),
                    title: clean_text(doc.title.as_deref(), 512).unwrap_or_default(),
                    authors: clean_list(doc.author_name, 16, 256),
                    preview_url: url.clone(),
                    download_url: url,
                    cache_key: format!("cover:{cover_id}"),
                    source_url: key.map(|key| format!("https://openlibrary.org{key}")),
                    license: None,
                    license_url: None,
                    requires_license_confirmation: true,
                })
            })
            .take(request.result_limit())
            .collect())
    }
}

impl AssetProvider for OpenLibraryCoverProvider {
    fn name(&self) -> &str {
        "openlibrary_cover"
    }

    fn requires_license_confirmation(&self) -> bool {
        true
    }

    fn download(&self, request: &DownloadRequest) -> Result<DownloadedAsset, OnlineError> {
        if !request.license_acknowledged {
            return Err(OnlineError::ConfirmationRequired);
        }
        let cover_id = parse_openlibrary_cover_url(&request.url)?;
        let canonical_url = openlibrary_cover_url(cover_id);
        if request.provider != AssetProvider::name(self) || request.url != canonical_url {
            return Err(OnlineError::InvalidRequest(
                "cover URL is not a canonical Open Library cover URL".to_owned(),
            ));
        }
        let response = self.transport.get(&canonical_url, MAX_COVER_BYTES)?;
        if response.status != 200 {
            return Err(OnlineError::Unavailable(
                "cover provider returned an unsuccessful response".to_owned(),
            ));
        }
        let media_type = detected_media_type(&response)?;
        if !media_type.starts_with("image/") {
            return Err(OnlineError::InvalidAsset);
        }
        Ok(DownloadedAsset {
            cache_key: request.cache_key.clone(),
            media_type,
            bytes: response.body,
            source_url: Some(canonical_url),
            license: Some("Per-image license not supplied; user rights review required".to_owned()),
        })
    }
}

#[derive(Clone)]
pub struct GoogleFontsProvider {
    api_key: String,
    transport: Arc<dyn HttpTransport>,
}

impl GoogleFontsProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_transport(api_key, Arc::new(UreqTransport::default()))
    }

    pub fn with_transport(api_key: impl Into<String>, transport: Arc<dyn HttpTransport>) -> Self {
        Self {
            api_key: api_key.into(),
            transport,
        }
    }
}

impl FontProvider for GoogleFontsProvider {
    fn name(&self) -> &str {
        "google_fonts"
    }

    fn search(&self, request: &ProviderRequest) -> Result<Vec<FontCandidate>, OnlineError> {
        request.validate()?;
        if self.api_key.trim().is_empty() {
            return Err(OnlineError::InvalidRequest(
                "Google Fonts API key is not configured".to_owned(),
            ));
        }
        if self.api_key.len() > 512 || self.api_key.chars().any(char::is_control) {
            return Err(OnlineError::InvalidRequest(
                "Google Fonts API key is invalid".to_owned(),
            ));
        }
        let family = first_nonempty(request.query.as_str(), request.title.as_deref())
            .ok_or_else(|| OnlineError::InvalidRequest("font family is empty".to_owned()))?;
        let url = format!(
            "https://www.googleapis.com/webfonts/v1/webfonts?key={}&family={}&sort=alpha",
            encode_component(&self.api_key),
            encode_component(family)
        );
        let response = self.transport.get(&url, GOOGLE_FONTS_RESPONSE_LIMIT)?;
        if response.status != 200 {
            return Err(OnlineError::Unavailable(
                "Google Fonts returned an unsuccessful response".to_owned(),
            ));
        }
        let payload: GoogleFontsResponse =
            serde_json::from_slice(&response.body).map_err(|_| {
                OnlineError::Unavailable("Google Fonts response was invalid".to_owned())
            })?;
        let mut candidates = Vec::new();
        for item in payload.items.into_iter().filter(|item| {
            item.family
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(family))
        }) {
            let family_name = clean_text(item.family.as_deref(), 128).unwrap_or_default();
            let source_url = format!(
                "https://fonts.google.com/specimen/{}",
                encode_component(&family_name)
            );
            let version = clean_text(item.version.as_deref(), 32);
            let category = clean_text(item.category.as_deref(), 32);
            for variant in item.variants.iter().take(request.result_limit()) {
                let Some(variant) = clean_text(Some(variant), 32) else {
                    continue;
                };
                let Some(raw_url) = item.files.get(&variant) else {
                    continue;
                };
                let Some(download_url) = normalize_google_font_url(raw_url) else {
                    continue;
                };
                candidates.push(FontCandidate {
                    candidate_id: format!(
                        "google-font:{}:{}",
                        encode_component(&family_name),
                        encode_component(&variant)
                    ),
                    provider: FontProvider::name(self).to_owned(),
                    family: family_name.clone(),
                    variant: variant.clone(),
                    category: category.clone(),
                    subsets: clean_list(item.subsets.clone(), 64, 64),
                    version: version.clone(),
                    download_url,
                    cache_key: format!(
                        "font:{}:{}:{}",
                        family_name,
                        variant,
                        version.as_deref().unwrap_or_default()
                    ),
                    license: Some("Catalog font; exact family license requires review".to_owned()),
                    license_url: Some(GOOGLE_FONTS_LICENSE_GUIDE.to_owned()),
                    requires_license_confirmation: true,
                    source_url: Some(source_url.clone()),
                });
                if candidates.len() >= request.result_limit() {
                    return Ok(candidates);
                }
            }
        }
        Ok(candidates)
    }
}

impl AssetProvider for GoogleFontsProvider {
    fn name(&self) -> &str {
        "google_fonts"
    }

    fn requires_license_confirmation(&self) -> bool {
        true
    }

    fn download(&self, request: &DownloadRequest) -> Result<DownloadedAsset, OnlineError> {
        if !request.license_acknowledged {
            return Err(OnlineError::ConfirmationRequired);
        }
        let canonical_url = normalize_google_font_url(&request.url).ok_or_else(|| {
            OnlineError::InvalidRequest(
                "font URL is outside the Google Fonts asset host".to_owned(),
            )
        })?;
        if request.provider != AssetProvider::name(self) || request.url != canonical_url {
            return Err(OnlineError::InvalidRequest(
                "font URL is not a canonical HTTPS asset URL".to_owned(),
            ));
        }
        let response = self.transport.get(&canonical_url, MAX_FONT_BYTES)?;
        if response.status != 200 {
            return Err(OnlineError::Unavailable(
                "Google Fonts asset returned an unsuccessful response".to_owned(),
            ));
        }
        let media_type = detected_media_type(&response)?;
        if !media_type.starts_with("font/") {
            return Err(OnlineError::InvalidAsset);
        }
        Ok(DownloadedAsset {
            cache_key: request.cache_key.clone(),
            media_type,
            bytes: response.body,
            source_url: Some(canonical_url),
            license: Some("Exact family license requires user review".to_owned()),
        })
    }
}

fn fetch_openlibrary_docs(
    transport: &dyn HttpTransport,
    request: &ProviderRequest,
) -> Result<Vec<OpenLibraryDoc>, OnlineError> {
    request.validate()?;
    let mut parameters = vec![
        ("fields", OPEN_LIBRARY_FIELDS.to_owned()),
        ("limit", request.result_limit().to_string()),
    ];
    if let Some(isbn) = request
        .isbn
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parameters.push(("isbn", isbn.trim().to_owned()));
    } else if let Some(identifier) = request
        .identifier
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parameters.push(("q", identifier.trim().to_owned()));
    } else {
        if let Some(title) = request
            .title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            parameters.push(("title", title.trim().to_owned()));
        }
        if let Some(author) = request
            .author
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            parameters.push(("author", author.trim().to_owned()));
        }
        if request
            .title
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
            && request
                .author
                .as_deref()
                .map(str::trim)
                .is_none_or(str::is_empty)
        {
            parameters.push(("q", request.query.trim().to_owned()));
        }
    }
    let query = parameters
        .iter()
        .map(|(key, value)| format!("{key}={}", encode_component(value)))
        .collect::<Vec<_>>()
        .join("&");
    let url = format!("https://openlibrary.org/search.json?{query}");
    let response = transport.get(&url, OPEN_LIBRARY_RESPONSE_LIMIT)?;
    if response.status != 200 {
        return Err(OnlineError::Unavailable(
            "Open Library returned an unsuccessful response".to_owned(),
        ));
    }
    let payload: OpenLibraryResponse = serde_json::from_slice(&response.body)
        .map_err(|_| OnlineError::Unavailable("Open Library response was invalid".to_owned()))?;
    Ok(payload.docs)
}

fn openlibrary_metadata(doc: &OpenLibraryDoc) -> Metadata {
    let authors = clean_list(doc.author_name.clone(), 32, 256);
    let identifiers = clean_list(doc.isbn.clone(), 32, 64);
    Metadata {
        title: clean_text(doc.title.as_deref(), 512),
        language: doc
            .language
            .first()
            .and_then(|value| clean_text(Some(value), 64)),
        authors,
        publisher: doc
            .publisher
            .first()
            .and_then(|value| clean_text(Some(value), 256)),
        date: doc.first_publish_year.map(|year| year.to_string()),
        identifiers: identifiers.clone(),
        identifier: identifiers.first().cloned(),
        subjects: clean_list(doc.subject.clone(), 64, 256),
        ..Metadata::default()
    }
}

fn estimate_confidence(request: &ProviderRequest, metadata: &Metadata) -> f32 {
    let Some(actual_title) = metadata.title.as_deref() else {
        return 0.4;
    };
    let expected_title = request
        .title
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&request.query);
    let normalize = |value: &str| {
        value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let expected = normalize(expected_title);
    let actual = normalize(actual_title);
    let mut score: f32 = if expected == actual {
        0.92
    } else if !expected.is_empty() && (actual.contains(&expected) || expected.contains(&actual)) {
        0.76
    } else {
        0.52
    };
    if let Some(author) = request.author.as_deref() {
        if metadata
            .author_names()
            .iter()
            .any(|candidate| normalize(candidate) == normalize(author))
        {
            score += 0.06;
        }
    }
    // This is a local heuristic, not a score supplied or verified by the catalog.
    score.min(0.98)
}

fn valid_openlibrary_key(value: &str) -> Option<&str> {
    let rest = value
        .strip_prefix("/works/OL")
        .map(|rest| rest.strip_suffix('W'))
        .or_else(|| {
            value
                .strip_prefix("/books/OL")
                .map(|rest| rest.strip_suffix('M'))
        })??;
    (!rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())).then_some(value)
}

fn openlibrary_cover_url(cover_id: u32) -> String {
    format!("https://covers.openlibrary.org/b/id/{cover_id}-L.jpg?default=false")
}

fn parse_openlibrary_cover_url(value: &str) -> Result<u32, OnlineError> {
    let id = value
        .strip_prefix("https://covers.openlibrary.org/b/id/")
        .and_then(|value| value.strip_suffix("-L.jpg?default=false"))
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| {
            OnlineError::InvalidRequest("cover URL is outside the Open Library host".to_owned())
        })?;
    id.parse()
        .map_err(|_| OnlineError::InvalidRequest("cover ID is invalid".to_owned()))
}

fn normalize_google_font_url(value: &str) -> Option<String> {
    let without_scheme = value
        .strip_prefix("https://fonts.gstatic.com/")
        .or_else(|| value.strip_prefix("http://fonts.gstatic.com/"))?;
    if without_scheme.is_empty()
        || without_scheme.contains(['?', '#', '\\', '@'])
        || without_scheme.chars().any(char::is_control)
        || !without_scheme.is_ascii()
        || without_scheme.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
        })
        || without_scheme
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || !matches!(
            without_scheme
                .rsplit_once('.')
                .map(|(_, extension)| extension.to_ascii_lowercase())
                .as_deref(),
            Some("woff2" | "woff" | "ttf" | "otf" | "ttc")
        )
    {
        return None;
    }
    Some(format!("https://fonts.gstatic.com/{without_scheme}"))
}

fn detected_media_type(response: &HttpResponse) -> Result<String, OnlineError> {
    let detected = detect_media_type(&response.body).ok_or(OnlineError::InvalidAsset)?;
    if let Some(declared) = response
        .content_type
        .as_deref()
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| {
            !value.is_empty() && !value.eq_ignore_ascii_case("application/octet-stream")
        })
    {
        let declared_lower = declared.to_ascii_lowercase();
        let declared = if declared_lower == "image/jpg" {
            "image/jpeg"
        } else if declared_lower == "application/font-woff" {
            "font/woff"
        } else if declared_lower == "application/x-font-ttf" {
            "font/ttf"
        } else {
            declared_lower.as_str()
        };
        if declared != detected {
            return Err(OnlineError::InvalidAsset);
        }
    }
    Ok(detected.to_owned())
}

fn clean_text(value: Option<&str>, max_bytes: usize) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        None
    } else {
        Some(value.to_owned())
    }
}

fn clean_list(values: Vec<String>, max_items: usize, max_bytes: usize) -> Vec<String> {
    values
        .into_iter()
        .filter_map(|value| clean_text(Some(&value), max_bytes))
        .take(max_items)
        .collect()
}

fn first_nonempty<'a>(first: &'a str, second: Option<&'a str>) -> Option<&'a str> {
    if !first.trim().is_empty() {
        Some(first.trim())
    } else {
        second.map(str::trim).filter(|value| !value.is_empty())
    }
}

fn encode_component(value: &str) -> String {
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

#[derive(Deserialize)]
struct OpenLibraryResponse {
    #[serde(default)]
    docs: Vec<OpenLibraryDoc>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct OpenLibraryDoc {
    key: Option<String>,
    title: Option<String>,
    author_name: Vec<String>,
    publisher: Vec<String>,
    first_publish_year: Option<i32>,
    subject: Vec<String>,
    isbn: Vec<String>,
    cover_i: Option<u32>,
    language: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GoogleFontsResponse {
    items: Vec<GoogleFontItem>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GoogleFontItem {
    family: Option<String>,
    variants: Vec<String>,
    subsets: Vec<String>,
    version: Option<String>,
    files: BTreeMap<String, String>,
    category: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use super::*;
    use crate::{MemoryCache, OnlineBoundary, OnlineSettings};

    #[derive(Default)]
    struct StubTransport {
        responses: Mutex<VecDeque<HttpResponse>>,
        requests: Mutex<Vec<String>>,
    }

    impl StubTransport {
        fn with_responses(responses: Vec<HttpResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl HttpTransport for StubTransport {
        fn get(&self, url: &str, max_response_bytes: usize) -> Result<HttpResponse, OnlineError> {
            self.requests.lock().unwrap().push(url.to_owned());
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| OnlineError::Unavailable("no test response".to_owned()))?;
            if response.body.len() > max_response_bytes {
                return Err(OnlineError::Unavailable(
                    "oversized test response".to_owned(),
                ));
            }
            Ok(response)
        }
    }

    fn json_response(body: &'static [u8]) -> HttpResponse {
        HttpResponse {
            status: 200,
            content_type: Some("application/json".to_owned()),
            body: body.to_vec(),
        }
    }

    fn woff2_fixture() -> Vec<u8> {
        let mut bytes = vec![0; 51];
        let length = bytes.len() as u32;
        bytes[..4].copy_from_slice(b"wOF2");
        bytes[4..8].copy_from_slice(&[0, 1, 0, 0]);
        bytes[8..12].copy_from_slice(&length.to_be_bytes());
        bytes[12..14].copy_from_slice(&1u16.to_be_bytes());
        bytes[16..20].copy_from_slice(&32u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&1u32.to_be_bytes());
        bytes[49] = 1;
        bytes[50] = 1;
        bytes
    }

    #[test]
    fn openlibrary_maps_candidates_and_encodes_search_fields() {
        let transport = Arc::new(StubTransport::with_responses(vec![json_response(
            br#"{"docs":[{"key":"/works/OL123W","title":"Pride and Prejudice","author_name":["Jane Austen"],"publisher":["T. Egerton"],"first_publish_year":1813,"subject":["Fiction"],"isbn":["9780141439518"],"cover_i":42,"language":["eng"]}]}"#,
        )]));
        let provider = OpenLibraryProvider::with_transport(transport.clone());
        let candidates = provider
            .search(&ProviderRequest {
                query: "Pride and Prejudice".to_owned(),
                title: Some("Pride and Prejudice".to_owned()),
                author: Some("Jane Austen".to_owned()),
                ..ProviderRequest::default()
            })
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].provider, "openlibrary");
        assert_eq!(candidates[0].confidence, 0.98);
        assert_eq!(
            candidates[0].metadata.title.as_deref(),
            Some("Pride and Prejudice")
        );
        assert_eq!(candidates[0].metadata.author_names(), ["Jane Austen"]);
        assert_eq!(
            candidates[0].source_url.as_deref(),
            Some("https://openlibrary.org/works/OL123W")
        );
        let url = &transport.requests()[0];
        assert!(url.starts_with("https://openlibrary.org/search.json?"));
        assert!(url.contains("title=Pride%20and%20Prejudice"));
        assert!(url.contains("author=Jane%20Austen"));
    }

    #[test]
    fn cover_provider_rejects_untrusted_hosts_before_network_access() {
        let transport = Arc::new(StubTransport::with_responses(Vec::new()));
        let provider = OpenLibraryCoverProvider::with_transport(transport.clone());
        let error = provider
            .download(&DownloadRequest {
                provider: AssetProvider::name(&provider).to_owned(),
                url: "https://attacker.example/cover.jpg".to_owned(),
                cache_key: "cover:1".to_owned(),
                license_acknowledged: true,
            })
            .unwrap_err();
        assert!(matches!(error, OnlineError::InvalidRequest(_)));
        assert!(transport.requests().is_empty());
    }

    #[test]
    fn google_font_assets_are_https_pinned_and_require_license_confirmation() {
        let transport = Arc::new(StubTransport::with_responses(vec![
            json_response(
                br#"{"items":[{"family":"Roboto","variants":["regular"],"subsets":["latin"],"version":"v30","files":{"regular":"http://fonts.gstatic.com/s/roboto/v30/a.woff2"},"category":"sans-serif"}]}"#,
            ),
            HttpResponse {
                status: 200,
                content_type: Some("font/woff2".to_owned()),
                body: woff2_fixture(),
            },
        ]));
        let provider = GoogleFontsProvider::with_transport("private-key", transport.clone());
        let candidates = provider
            .search(&ProviderRequest {
                query: "Roboto".to_owned(),
                ..ProviderRequest::default()
            })
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].requires_license_confirmation);
        assert_eq!(
            candidates[0].download_url,
            "https://fonts.gstatic.com/s/roboto/v30/a.woff2"
        );
        assert!(transport.requests()[0].contains("key=private-key"));

        let boundary =
            OnlineBoundary::new(OnlineSettings { enabled: true }, MemoryCache::default());
        let request = DownloadRequest {
            provider: FontProvider::name(&provider).to_owned(),
            url: candidates[0].download_url.clone(),
            cache_key: candidates[0].cache_key.clone(),
            license_acknowledged: false,
        };
        assert_eq!(
            boundary.download(&provider, &request).unwrap_err(),
            OnlineError::ConfirmationRequired
        );
        assert_eq!(transport.requests().len(), 1);

        let asset = boundary
            .download(
                &provider,
                &DownloadRequest {
                    license_acknowledged: true,
                    ..request
                },
            )
            .unwrap();
        assert_eq!(asset.media_type, "font/woff2");
        assert_eq!(asset.bytes, woff2_fixture());
    }

    #[test]
    fn google_font_url_parser_rejects_host_confusion_and_path_traversal() {
        assert!(
            normalize_google_font_url("https://fonts.gstatic.com.evil.test/font.woff2").is_none()
        );
        assert!(normalize_google_font_url("https://fonts.gstatic.com/../private").is_none());
        assert!(normalize_google_font_url("https://user@fonts.gstatic.com/font.woff2").is_none());
    }
}
