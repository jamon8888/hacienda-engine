//! Optional, source-scoped legal research over public European legal sources.
//!
//! Private Hacienda documents are deliberately outside this client: only the user's
//! search query is sent to the configured web-search provider. Source restrictions are
//! expressed as `site:` queries and every returned URL is checked against the selected
//! source before it is exposed to a caller.

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use thiserror::Error;
use url::Url;

const PARALLEL_SEARCH_URL: &str = "https://api.parallel.ai/v1/search";
const PARALLEL_API_BASE: &str = "https://api.parallel.ai/v1";
const MAX_RESULTS: u8 = 20;
const MAX_FETCH_BYTES: usize = 256 * 1024;
const MAX_PROVIDER_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_RESEARCH_QUERY_CHARS: usize = 2_000;
const MAX_PROVIDER_ID_CHARS: usize = 128;

static TASK_SOURCES: OnceLock<Mutex<HashMap<String, LegalSource>>> = OnceLock::new();
static MONITOR_SOURCES: OnceLock<Mutex<HashMap<String, LegalSource>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalSource {
    Eurlex,
    Legifrance,
    GesetzeImInternet,
    Rechtspraak,
    Boe,
    Normattiva,
}

impl LegalSource {
    pub const ALL: [Self; 6] = [
        Self::Eurlex,
        Self::Legifrance,
        Self::GesetzeImInternet,
        Self::Rechtspraak,
        Self::Boe,
        Self::Normattiva,
    ];

    pub fn host(self) -> &'static str {
        match self {
            Self::Eurlex => "eur-lex.europa.eu",
            Self::Legifrance => "legifrance.gouv.fr",
            Self::GesetzeImInternet => "gesetze-im-internet.de",
            Self::Rechtspraak => "rechtspraak.nl",
            Self::Boe => "boe.es",
            Self::Normattiva => "normattiva.it",
        }
    }

    pub fn tool_prefix(self) -> &'static str {
        match self {
            Self::Eurlex => "eurlex",
            Self::Legifrance => "legifrance",
            Self::GesetzeImInternet => "gesetze",
            Self::Rechtspraak => "rechtspraak",
            Self::Boe => "boe",
            Self::Normattiva => "normattiva",
        }
    }
}

impl FromStr for LegalSource {
    type Err = LegalError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "eurlex" | "eur-lex" | "eu" => Ok(Self::Eurlex),
            "legifrance" | "fr" => Ok(Self::Legifrance),
            "gesetze" | "gesetze_im_internet" | "de" => Ok(Self::GesetzeImInternet),
            "rechtspraak" | "nl" => Ok(Self::Rechtspraak),
            "boe" | "es" => Ok(Self::Boe),
            "normattiva" | "it" => Ok(Self::Normattiva),
            _ => Err(LegalError::UnsupportedSource),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegalSearchRequest {
    pub query: String,
    pub source: LegalSource,
    #[serde(default = "default_max_results")]
    pub max_results: u8,
}

const fn default_max_results() -> u8 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegalSearchResult {
    pub source: LegalSource,
    pub title: String,
    pub url: String,
    pub published: Option<String>,
    pub excerpts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegalSearchResponse {
    pub provider_search_id: String,
    pub source: LegalSource,
    pub results: Vec<LegalSearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegalFetchRequest {
    pub source: LegalSource,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegalFetchResponse {
    pub source: LegalSource,
    pub url: String,
    pub content_type: Option<String>,
    pub content: String,
}

/// Request for a source-restricted Parallel Task API deep research run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegalDeepResearchRequest {
    pub source: LegalSource,
    pub query: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LegalTaskRunResponse {
    pub run_id: String,
    pub source: LegalSource,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LegalTaskResultResponse {
    pub run_id: String,
    pub source: LegalSource,
    pub status: String,
    /// Original Parallel output object, including content and research basis.
    pub output: Option<serde_json::Value>,
}

/// Request to monitor a query on one official legal source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegalMonitorRequest {
    pub source: LegalSource,
    pub query: String,
    pub frequency: String,
    pub processor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LegalMonitorResponse {
    pub source: LegalSource,
    pub monitor_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LegalMonitorEventsResponse {
    pub monitor_id: String,
    pub events: serde_json::Value,
}

#[derive(Debug, Error)]
pub enum LegalError {
    #[error("legal research add-on is disabled")]
    Disabled,
    #[error("legal research provider is not configured")]
    ProviderNotConfigured,
    #[error("unsupported legal source")]
    UnsupportedSource,
    #[error("query must contain between 1 and 2000 characters")]
    InvalidQuery,
    #[error("URL is outside the selected official source")]
    UnsafeUrl,
    #[error("upstream legal source request failed")]
    Upstream,
    #[error("provider returned an invalid response")]
    InvalidResponse,
    #[error("provider resource identifier is invalid")]
    InvalidProviderId,
}

#[derive(Clone)]
pub struct LegalResearchClient {
    http: reqwest::Client,
    api_key: String,
}

impl LegalResearchClient {
    /// Construct the paid provider client from server-side environment variables.
    /// This client never accepts credentials from an API/MCP request payload.
    pub fn from_env() -> Result<Self, LegalError> {
        if std::env::var("HACIENDA_LEGAL_RESEARCH_ENABLED")
            .ok()
            .as_deref()
            != Some("true")
        {
            return Err(LegalError::Disabled);
        }
        let api_key = std::env::var("HACIENDA_PARALLEL_API_KEY")
            .or_else(|_| std::env::var("PARALLEL_API_KEY"))
            .map_err(|_| LegalError::ProviderNotConfigured)?;
        Self::new(api_key)
    }

    pub fn new(api_key: impl Into<String>) -> Result<Self, LegalError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(LegalError::ProviderNotConfigured);
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(25))
            // A source URL is checked before the request and redirects are never followed:
            // otherwise an allowlisted URL could redirect the fetch to an arbitrary host.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("hacienda-legal/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| LegalError::ProviderNotConfigured)?;
        Ok(Self { http, api_key })
    }

    pub async fn search(
        &self,
        request: LegalSearchRequest,
    ) -> Result<LegalSearchResponse, LegalError> {
        let query = request.query.trim();
        if query.is_empty() || query.len() > 2_000 {
            return Err(LegalError::InvalidQuery);
        }
        let max_results = request.max_results.clamp(1, MAX_RESULTS);
        let source = request.source;
        let body = serde_json::json!({
            "objective": format!("Find authoritative legal instruments, judgments, or official guidance on: {query}. Search only the official source {host}.", host = source.host()),
            "search_queries": [format!("site:{} {}", source.host(), query)],
            "mode": "fast",
            "advanced_settings": { "max_results": max_results }
        });
        let response = self
            .http
            .post(PARALLEL_SEARCH_URL)
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header("x-api-key", &self.api_key)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|_| LegalError::Upstream)?;
        if !response.status().is_success() {
            return Err(LegalError::Upstream);
        }
        let raw: serde_json::Value =
            read_bounded(response, MAX_FETCH_BYTES)
                .await
                .and_then(|bytes| {
                    serde_json::from_slice(&bytes).map_err(|_| LegalError::InvalidResponse)
                })?;
        let search_id = raw
            .get("search_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unavailable")
            .to_owned();
        let mut results = Vec::new();
        if let Some(items) = raw.get("results").and_then(serde_json::Value::as_array) {
            for item in items.iter().take(usize::from(max_results)) {
                let Some(url) = item.get("url").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                if !url_belongs_to_source(source, url) {
                    continue;
                }
                let excerpts = item
                    .get("excerpts")
                    .and_then(serde_json::Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .take(3)
                            .map(|text| truncate_chars(text, 2_000))
                            .collect()
                    })
                    .unwrap_or_default();
                results.push(LegalSearchResult {
                    source,
                    title: item
                        .get("title")
                        .and_then(serde_json::Value::as_str)
                        .map(|title| truncate_chars(title, 500))
                        .unwrap_or_else(|| "Official legal source".to_owned()),
                    url: truncate_chars(url, 4_096),
                    published: item
                        .get("publish_date")
                        .and_then(serde_json::Value::as_str)
                        .map(|date| truncate_chars(date, 80)),
                    excerpts,
                });
            }
        }
        Ok(LegalSearchResponse {
            provider_search_id: search_id,
            source,
            results,
        })
    }

    /// Create a Parallel Task API deep research run restricted to this source domain.
    pub async fn create_deep_research(
        &self,
        request: LegalDeepResearchRequest,
    ) -> Result<LegalTaskRunResponse, LegalError> {
        let query = request.query.trim();
        validate_query(query)?;
        // TaskRunInput's documented source_policy is a root-level field. Text schema
        // makes the result a cited markdown report and basis is retained in the DTO.
        let body = serde_json::json!({
            "input": query,
            "processor": "pro",
            "source_policy": { "include_domains": [request.source.host()] },
            "metadata": { "legal_src": request.source.tool_prefix() },
            "task_spec": {
                "output_schema": {
                    "type": "text",
                    "description": format!("Research the legal question using only {}. State uncertainty explicitly and include citations for factual claims.", request.source.host())
                }
            }
        });
        let raw = self
            .parallel_post(format!("{PARALLEL_API_BASE}/tasks/runs"), &body)
            .await?;
        let run_id = required_string(&raw, "run_id")?;
        validate_provider_id(run_id, "trun_")?;
        remember_source(&TASK_SOURCES, run_id, request.source)?;
        Ok(LegalTaskRunResponse {
            run_id: run_id.to_owned(),
            source: request.source,
            status: required_string(&raw, "status")?.to_owned(),
        })
    }

    /// Retrieve lightweight status for a previously created Task API run.
    pub async fn deep_research_status(
        &self,
        run_id: &str,
    ) -> Result<LegalTaskRunResponse, LegalError> {
        validate_provider_id(run_id, "trun_")?;
        let raw = self
            .parallel_get(format!("{PARALLEL_API_BASE}/tasks/runs/{run_id}"))
            .await?;
        let source = lookup_source(&TASK_SOURCES, run_id)
            .or_else(|_| source_from_provider(&raw).ok_or(LegalError::InvalidResponse))?;
        remember_source(&TASK_SOURCES, run_id, source)?;
        Ok(LegalTaskRunResponse {
            run_id: required_string(&raw, "run_id")?.to_owned(),
            source,
            status: required_string(&raw, "status")?.to_owned(),
        })
    }

    /// Retrieve a completed Task API run result, preserving provider content and basis.
    pub async fn deep_research_result(
        &self,
        run_id: &str,
    ) -> Result<LegalTaskResultResponse, LegalError> {
        validate_provider_id(run_id, "trun_")?;
        let raw = self
            .parallel_get(format!(
                "{PARALLEL_API_BASE}/tasks/runs/{run_id}/result?timeout=20"
            ))
            .await?;
        let run = raw.get("run").ok_or(LegalError::InvalidResponse)?;
        let source = lookup_source(&TASK_SOURCES, run_id)
            .or_else(|_| source_from_provider(run).ok_or(LegalError::InvalidResponse))?;
        remember_source(&TASK_SOURCES, run_id, source)?;
        let output = raw.get("output").filter(|value| !value.is_null()).cloned();
        validate_output_citations(output.as_ref(), source)?;
        Ok(LegalTaskResultResponse {
            run_id: required_string(run, "run_id")?.to_owned(),
            source,
            status: required_string(run, "status")?.to_owned(),
            output,
        })
    }

    /// Start an `event_stream` monitor restricted to the selected official source.
    pub async fn create_monitor(
        &self,
        request: LegalMonitorRequest,
    ) -> Result<LegalMonitorResponse, LegalError> {
        let query = request.query.trim();
        validate_query(query)?;
        validate_frequency(&request.frequency)?;
        let processor = request.processor.as_deref().unwrap_or("lite");
        if !matches!(processor, "lite" | "base" | "core") {
            return Err(LegalError::InvalidQuery);
        }
        let body = serde_json::json!({
            "type": "event_stream",
            "frequency": request.frequency,
            "processor": processor,
            "metadata": { "legal_src": request.source.tool_prefix() },
            "settings": {
                "query": query,
                "advanced_settings": {
                    "source_policy": { "include_domains": [request.source.host()] }
                }
            }
        });
        let raw = self
            .parallel_post(format!("{PARALLEL_API_BASE}/monitors"), &body)
            .await?;
        let monitor_id = required_string(&raw, "monitor_id")?;
        validate_provider_id(monitor_id, "monitor_")?;
        remember_source(&MONITOR_SOURCES, monitor_id, request.source)?;
        Ok(LegalMonitorResponse {
            monitor_id: monitor_id.to_owned(),
            source: request.source,
            status: required_string(&raw, "status")?.to_owned(),
        })
    }

    /// List a bounded page of events from one monitor.
    pub async fn monitor_events(
        &self,
        monitor_id: &str,
    ) -> Result<LegalMonitorEventsResponse, LegalError> {
        validate_provider_id(monitor_id, "monitor_")?;
        let source = if let Ok(source) = lookup_source(&MONITOR_SOURCES, monitor_id) {
            source
        } else {
            let monitor = self
                .parallel_get(format!("{PARALLEL_API_BASE}/monitors/{monitor_id}"))
                .await?;
            let source = source_from_provider(&monitor).ok_or(LegalError::InvalidResponse)?;
            remember_source(&MONITOR_SOURCES, monitor_id, source)?;
            source
        };
        let mut url = Url::parse(&format!("{PARALLEL_API_BASE}/monitors/{monitor_id}/events"))
            .map_err(|_| LegalError::InvalidResponse)?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", "50");
        }
        let response = self
            .authenticated(self.http.get(url))
            .send()
            .await
            .map_err(|_| LegalError::Upstream)?;
        let raw = read_parallel_json(response).await?;
        let events = raw
            .get("events")
            .cloned()
            .ok_or(LegalError::InvalidResponse)?;
        validate_monitor_event_citations(&events, source)?;
        Ok(LegalMonitorEventsResponse {
            monitor_id: monitor_id.to_owned(),
            events,
        })
    }

    /// Permanently cancel a monitor using Parallel's documented cancel operation.
    pub async fn cancel_monitor(
        &self,
        monitor_id: &str,
    ) -> Result<LegalMonitorResponse, LegalError> {
        validate_provider_id(monitor_id, "monitor_")?;
        let response = self
            .authenticated(
                self.http
                    .post(format!("{PARALLEL_API_BASE}/monitors/{monitor_id}/cancel")),
            )
            .send()
            .await
            .map_err(|_| LegalError::Upstream)?;
        let raw = read_parallel_json(response).await?;
        Ok(LegalMonitorResponse {
            monitor_id: required_string(&raw, "monitor_id")?.to_owned(),
            source: lookup_source(&MONITOR_SOURCES, monitor_id)
                .or_else(|_| source_from_provider(&raw).ok_or(LegalError::InvalidResponse))?,
            status: required_string(&raw, "status")?.to_owned(),
        })
    }

    async fn parallel_post(
        &self,
        endpoint: String,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, LegalError> {
        let response = self
            .authenticated(self.http.post(endpoint).json(body))
            .send()
            .await
            .map_err(|_| LegalError::Upstream)?;
        read_parallel_json(response).await
    }

    async fn parallel_get(&self, endpoint: String) -> Result<serde_json::Value, LegalError> {
        let response = self
            .authenticated(self.http.get(endpoint))
            .send()
            .await
            .map_err(|_| LegalError::Upstream)?;
        read_parallel_json(response).await
    }

    fn authenticated(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header("x-api-key", &self.api_key)
    }

    pub async fn fetch(
        &self,
        request: LegalFetchRequest,
    ) -> Result<LegalFetchResponse, LegalError> {
        let url = Url::parse(&request.url).map_err(|_| LegalError::UnsafeUrl)?;
        if !url_belongs_to_source(request.source, &request.url) {
            return Err(LegalError::UnsafeUrl);
        }
        let response = self
            .http
            .get(url.clone())
            .header(
                ACCEPT,
                "text/html, application/xhtml+xml, application/json, text/plain",
            )
            .send()
            .await
            .map_err(|_| LegalError::Upstream)?;
        if !response.status().is_success() {
            return Err(LegalError::Upstream);
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = read_bounded(response, MAX_FETCH_BYTES).await?;
        let content = String::from_utf8_lossy(&bytes).into_owned();
        Ok(LegalFetchResponse {
            source: request.source,
            url: url.to_string(),
            content_type,
            content,
        })
    }
}

pub fn url_belongs_to_source(source: LegalSource, candidate: &str) -> bool {
    if candidate.len() > 4_096 {
        return false;
    }
    Url::parse(candidate)
        .ok()
        .filter(|url| {
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.port_or_known_default() == Some(443)
        })
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| host_matches(source, &host))
}

fn host_matches(source: LegalSource, host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == source.host() || host.ends_with(&format!(".{}", source.host()))
}

async fn read_bounded(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, LegalError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(LegalError::InvalidResponse);
    }
    let mut bytes =
        Vec::with_capacity(response.content_length().unwrap_or(0).min(limit as u64) as usize);
    while let Some(chunk) = response.chunk().await.map_err(|_| LegalError::Upstream)? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(LegalError::InvalidResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn validate_query(query: &str) -> Result<(), LegalError> {
    if query.is_empty() || query.chars().count() > MAX_RESEARCH_QUERY_CHARS {
        Err(LegalError::InvalidQuery)
    } else {
        Ok(())
    }
}

fn validate_provider_id(id: &str, required_prefix: &str) -> Result<(), LegalError> {
    if id.len() <= MAX_PROVIDER_ID_CHARS
        && id.starts_with(required_prefix)
        && id[required_prefix.len()..].bytes().all(is_safe_id_byte)
        && id.len() > required_prefix.len()
    {
        Ok(())
    } else {
        Err(LegalError::InvalidProviderId)
    }
}

fn is_safe_id_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn validate_frequency(frequency: &str) -> Result<(), LegalError> {
    let split = frequency
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(frequency.len());
    let (amount, unit) = frequency.split_at(split);
    let amount: u32 = amount.parse().map_err(|_| LegalError::InvalidQuery)?;
    let hours = match unit {
        "h" => amount,
        "d" => amount.saturating_mul(24),
        "w" => amount.saturating_mul(24 * 7),
        _ => return Err(LegalError::InvalidQuery),
    };
    if (1..=30 * 24).contains(&hours) {
        Ok(())
    } else {
        Err(LegalError::InvalidQuery)
    }
}

async fn read_parallel_json(response: reqwest::Response) -> Result<serde_json::Value, LegalError> {
    if !response.status().is_success() {
        return Err(LegalError::Upstream);
    }
    let bytes = read_bounded(response, MAX_PROVIDER_RESPONSE_BYTES).await?;
    serde_json::from_slice(&bytes).map_err(|_| LegalError::InvalidResponse)
}

fn required_string<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str, LegalError> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or(LegalError::InvalidResponse)
}

fn remember_source(
    sources: &OnceLock<Mutex<HashMap<String, LegalSource>>>,
    id: &str,
    source: LegalSource,
) -> Result<(), LegalError> {
    sources
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| LegalError::ProviderNotConfigured)?
        .insert(id.to_owned(), source);
    Ok(())
}

fn lookup_source(
    sources: &OnceLock<Mutex<HashMap<String, LegalSource>>>,
    id: &str,
) -> Result<LegalSource, LegalError> {
    sources
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| LegalError::ProviderNotConfigured)?
        .get(id)
        .copied()
        .ok_or(LegalError::InvalidProviderId)
}

fn source_from_provider(value: &serde_json::Value) -> Option<LegalSource> {
    value
        .get("metadata")
        .and_then(|metadata| metadata.get("legal_src"))
        .and_then(serde_json::Value::as_str)
        .and_then(|source| LegalSource::from_str(source).ok())
        .or_else(|| {
            let include_domains = value
                .get("source_policy")
                .and_then(|policy| policy.get("include_domains"))
                .or_else(|| {
                    value
                        .get("settings")
                        .and_then(|settings| settings.get("advanced_settings"))
                        .and_then(|advanced| advanced.get("source_policy"))
                        .and_then(|policy| policy.get("include_domains"))
                })?
                .as_array()?;
            include_domains
                .iter()
                .filter_map(serde_json::Value::as_str)
                .find_map(|domain| {
                    LegalSource::ALL
                        .into_iter()
                        .find(|source| domain.eq_ignore_ascii_case(source.host()))
                })
        })
}

fn validate_output_citations(
    output: Option<&serde_json::Value>,
    source: LegalSource,
) -> Result<(), LegalError> {
    if let Some(basis) = output
        .and_then(|output| output.get("basis"))
        .and_then(serde_json::Value::as_array)
    {
        validate_basis_citations(basis, source)?;
    }
    Ok(())
}

fn validate_basis_citations(
    basis: &[serde_json::Value],
    source: LegalSource,
) -> Result<(), LegalError> {
    for item in basis {
        if let Some(citations) = item.get("citations").and_then(serde_json::Value::as_array) {
            for citation in citations {
                if citation
                    .get("url")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|url| !url_belongs_to_source(source, url))
                {
                    return Err(LegalError::InvalidResponse);
                }
            }
        }
    }
    Ok(())
}

fn validate_monitor_event_citations(
    events: &serde_json::Value,
    source: LegalSource,
) -> Result<(), LegalError> {
    let Some(events) = events.as_array() else {
        return Err(LegalError::InvalidResponse);
    };
    for event in events {
        if let Some(basis) = event
            .get("output")
            .and_then(|output| output.get("basis"))
            .and_then(serde_json::Value::as_array)
        {
            validate_basis_citations(basis, source)?;
        }
    }
    Ok(())
}
