//! Optional, source-scoped legal research over public European legal sources.
//!
//! Private Hacienda documents are deliberately outside this client: only the user's
//! search query is sent to the configured web-search provider. Source restrictions are
//! expressed as `site:` queries and every returned URL is checked against the selected
//! source before it is exposed to a caller.

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::{str::FromStr, time::Duration};
use thiserror::Error;
use url::Url;

const PARALLEL_SEARCH_URL: &str = "https://api.parallel.ai/v1/search";
const PARALLEL_API_BASE: &str = "https://api.parallel.ai/v1";
const MAX_RESULTS: u8 = 20;
const MAX_FETCH_BYTES: usize = 256 * 1024;
const MAX_PROVIDER_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_RESEARCH_QUERY_CHARS: usize = 2_000;
const MAX_PROVIDER_ID_CHARS: usize = 128;
const DEFAULT_TENANT_SCOPE: &str = "default";
const TENANT_METADATA_KEY: &str = "legal_tenant";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Official legal source supported by this add-on.
pub enum LegalSource {
    /// European Union law and case law on EUR-Lex.
    Eurlex,
    /// French legislation and case law on Légifrance.
    Legifrance,
    /// German federal legislation on Gesetze im Internet.
    GesetzeImInternet,
    /// Dutch legislation and case law on Rechtspraak.
    Rechtspraak,
    /// Spanish legislation and case law on BOE.
    Boe,
    /// Italian legislation on Normattiva.
    Normattiva,
}

impl LegalSource {
    /// All six supported official-source identifiers.
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
/// Source-scoped legal web-search request.
pub struct LegalSearchRequest {
    /// Search question or keywords, limited to 2,000 Unicode characters.
    pub query: String,
    /// Official source used for provider filtering and local URL validation.
    pub source: LegalSource,
    /// Desired result count, clamped to the supported range 1–20; defaults to 10.
    #[serde(default = "default_max_results")]
    pub max_results: u8,
}

const fn default_max_results() -> u8 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One result from the selected official source.
pub struct LegalSearchResult {
    /// Source selected for this search.
    pub source: LegalSource,
    /// Provider result title, or a generic title if omitted upstream.
    pub title: String,
    /// HTTPS URL validated against the selected source host.
    pub url: String,
    /// Provider publication date when available.
    pub published: Option<String>,
    /// At most three provider excerpts, each bounded to 2,000 characters.
    pub excerpts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Result page returned by the Parallel Search API.
pub struct LegalSearchResponse {
    /// Parallel search request identifier.
    pub provider_search_id: String,
    /// Source used to constrain and validate results.
    pub source: LegalSource,
    /// Matching results; an empty vector is a valid no-match response.
    pub results: Vec<LegalSearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to fetch text from a previously found official URL.
pub struct LegalFetchRequest {
    /// Expected official source; redirects are disabled and the URL must match it.
    pub source: LegalSource,
    /// HTTPS URL on the selected source host.
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Bounded textual content fetched from an official legal source.
pub struct LegalFetchResponse {
    /// Source used to validate the requested URL.
    pub source: LegalSource,
    /// Normalized final URL for the fetch (redirects are not followed).
    pub url: String,
    /// Response MIME type when the server provided one.
    pub content_type: Option<String>,
    /// UTF-8 HTML, XHTML, JSON, XML, or plain text, limited to 256 KiB.
    pub content: String,
}

/// Source-restricted Parallel Task API deep-research request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Request for a source-restricted Parallel Task API deep-research run.
pub struct LegalDeepResearchRequest {
    /// Official source used to constrain the task and validate citations.
    pub source: LegalSource,
    /// Research question, limited to 2,000 Unicode characters.
    pub query: String,
}

#[derive(Debug, Clone, Serialize)]
/// Identifier and current state for a submitted deep-research run.
pub struct LegalTaskRunResponse {
    /// Opaque Parallel task-run identifier.
    pub run_id: String,
    /// Official source attached to the run.
    pub source: LegalSource,
    /// Parallel run state such as `queued`, `running`, or `completed`.
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
/// Result of polling a deep-research run.
pub struct LegalTaskResultResponse {
    /// Opaque Parallel task-run identifier.
    pub run_id: String,
    /// Official source attached to the run.
    pub source: LegalSource,
    /// Parallel run state; non-completed runs may have no output yet.
    pub status: String,
    /// Original Parallel output object, including content and research basis.
    pub output: Option<serde_json::Value>,
}

/// Request to monitor one official legal source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Request to create a recurring monitor on one official legal source.
pub struct LegalMonitorRequest {
    /// Official source used to filter monitor events and citations.
    pub source: LegalSource,
    /// Research question, limited to 2,000 Unicode characters.
    pub query: String,
    /// Provider cadence, from 1 hour to 30 days (for example `1d`).
    pub frequency: String,
    /// Optional Parallel processor (`lite`, `base`, or `core`).
    pub processor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
/// Identifier and state for a created or cancelled monitor.
pub struct LegalMonitorResponse {
    /// Official source attached to the monitor.
    pub source: LegalSource,
    /// Opaque Parallel monitor identifier.
    pub monitor_id: String,
    /// Provider monitor state.
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
/// One bounded page of monitor events.
pub struct LegalMonitorEventsResponse {
    /// Opaque Parallel monitor identifier.
    pub monitor_id: String,
    /// Original provider event values, after source/citation validation.
    pub events: serde_json::Value,
    /// Provider cursor for retrieving the next page, when one exists.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Error)]
/// Safe, fixed-message errors from validation or the external provider.
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
    #[error("upstream legal provider returned HTTP status {status}")]
    UpstreamStatus { status: u16 },
    #[error("provider returned an invalid response")]
    InvalidResponse,
    #[error("official source returned a non-text or non-UTF-8 document")]
    UnsupportedDocumentFormat,
    #[error("provider resource identifier is invalid")]
    InvalidProviderId,
    #[error("legal research resource was not found")]
    ResourceNotFound,
}

#[derive(Clone)]
/// Optional Parallel-backed legal research client.
pub struct LegalResearchClient {
    http: reqwest::Client,
    api_key: String,
}

impl LegalResearchClient {
    /// Construct the paid provider client from server-side environment variables.
    /// This client never accepts credentials from an API/MCP request payload; construction
    /// fails unless the add-on and a server-side Parallel key are configured.
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

    /// Construct a client with an explicit key, useful for applications and tests.
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

    /// Search one source; provider allowlisting and local host checks both apply.
    pub async fn search(
        &self,
        request: LegalSearchRequest,
    ) -> Result<LegalSearchResponse, LegalError> {
        let query = request.query.trim();
        validate_query(query)?;
        let max_results = request.max_results.clamp(1, MAX_RESULTS);
        let source = request.source;
        let body = serde_json::json!({
            "objective": format!("Find authoritative legal instruments, judgments, or official guidance on: {query}. Search only the official source {host}.", host = source.host()),
            "search_queries": [format!("site:{} {}", source.host(), query)],
            "mode": "fast",
            "advanced_settings": {
                "max_results": max_results,
                "source_policy": { "include_domains": [source.host()] }
            }
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
            return Err(LegalError::UpstreamStatus {
                status: response.status().as_u16(),
            });
        }
        let raw: serde_json::Value =
            read_bounded(response, MAX_FETCH_BYTES)
                .await
                .and_then(|bytes| {
                    serde_json::from_slice(&bytes).map_err(|_| LegalError::InvalidResponse)
                })?;
        let search_id = required_string(&raw, "search_id")?.to_owned();
        let mut results = Vec::new();
        let items = raw
            .get("results")
            .and_then(serde_json::Value::as_array)
            .ok_or(LegalError::InvalidResponse)?;
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
        self.create_deep_research_for_tenant(request, DEFAULT_TENANT_SCOPE)
            .await
    }

    /// Create a research run bound to the authenticated tenant scope.
    pub async fn create_deep_research_for_tenant(
        &self,
        request: LegalDeepResearchRequest,
        tenant_scope: &str,
    ) -> Result<LegalTaskRunResponse, LegalError> {
        let query = request.query.trim();
        validate_query(query)?;
        // TaskRunInput's documented source_policy is a root-level field. Text schema
        // makes the result a cited markdown report and basis is retained in the DTO.
        let body = serde_json::json!({
            "input": query,
            "processor": "pro",
            "source_policy": { "include_domains": [request.source.host()] },
            "metadata": {
                "legal_src": request.source.tool_prefix(),
                (TENANT_METADATA_KEY): tenant_marker(tenant_scope)
            },
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
        self.deep_research_status_for_tenant(run_id, DEFAULT_TENANT_SCOPE)
            .await
    }

    /// Read status only when the provider run belongs to the authenticated tenant.
    pub async fn deep_research_status_for_tenant(
        &self,
        run_id: &str,
        tenant_scope: &str,
    ) -> Result<LegalTaskRunResponse, LegalError> {
        validate_provider_id(run_id, "trun_")?;
        let raw = self
            .parallel_get(format!("{PARALLEL_API_BASE}/tasks/runs/{run_id}"))
            .await?;
        ensure_tenant(&raw, tenant_scope)?;
        let source = source_from_provider(&raw).ok_or(LegalError::InvalidResponse)?;
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
        self.deep_research_result_for_tenant(run_id, DEFAULT_TENANT_SCOPE)
            .await
    }

    /// Read a completed result only for the tenant that created the run.
    pub async fn deep_research_result_for_tenant(
        &self,
        run_id: &str,
        tenant_scope: &str,
    ) -> Result<LegalTaskResultResponse, LegalError> {
        validate_provider_id(run_id, "trun_")?;
        let response = self
            .authenticated(self.http.get(format!(
                "{PARALLEL_API_BASE}/tasks/runs/{run_id}/result?timeout=20"
            )))
            .send()
            .await
            .map_err(|_| LegalError::Upstream)?;
        if response.status() == reqwest::StatusCode::REQUEST_TIMEOUT {
            let status = self
                .deep_research_status_for_tenant(run_id, tenant_scope)
                .await?;
            return Ok(LegalTaskResultResponse {
                run_id: status.run_id,
                source: status.source,
                status: status.status,
                output: None,
            });
        }
        let raw = read_parallel_json(response).await?;
        let run = raw.get("run").ok_or(LegalError::InvalidResponse)?;
        ensure_tenant(run, tenant_scope)?;
        let source = source_from_provider(run).ok_or(LegalError::InvalidResponse)?;
        let output = raw.get("output").filter(|value| !value.is_null()).cloned();
        let status = required_string(run, "status")?;
        if status == "completed" && output.is_none() {
            return Err(LegalError::InvalidResponse);
        }
        validate_output_citations(output.as_ref(), source)?;
        Ok(LegalTaskResultResponse {
            run_id: required_string(run, "run_id")?.to_owned(),
            source,
            status: status.to_owned(),
            output,
        })
    }

    /// Start an `event_stream` monitor restricted to the selected official source.
    pub async fn create_monitor(
        &self,
        request: LegalMonitorRequest,
    ) -> Result<LegalMonitorResponse, LegalError> {
        self.create_monitor_for_tenant(request, DEFAULT_TENANT_SCOPE)
            .await
    }

    /// Create a monitor bound to the authenticated tenant scope.
    pub async fn create_monitor_for_tenant(
        &self,
        request: LegalMonitorRequest,
        tenant_scope: &str,
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
            "metadata": {
                "legal_src": request.source.tool_prefix(),
                (TENANT_METADATA_KEY): tenant_marker(tenant_scope)
            },
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
        self.monitor_events_for_tenant(monitor_id, DEFAULT_TENANT_SCOPE, None, false)
            .await
    }

    /// Read one bounded monitor event page only for the creating tenant.
    pub async fn monitor_events_for_tenant(
        &self,
        monitor_id: &str,
        tenant_scope: &str,
        cursor: Option<&str>,
        include_completions: bool,
    ) -> Result<LegalMonitorEventsResponse, LegalError> {
        validate_provider_id(monitor_id, "monitor_")?;
        if cursor.is_some_and(|cursor| {
            cursor.len() > MAX_PROVIDER_RESPONSE_BYTES || cursor.chars().any(char::is_control)
        }) {
            return Err(LegalError::InvalidQuery);
        }
        let monitor = self
            .parallel_get(format!("{PARALLEL_API_BASE}/monitors/{monitor_id}"))
            .await?;
        ensure_tenant(&monitor, tenant_scope)?;
        let source = source_from_provider(&monitor).ok_or(LegalError::InvalidResponse)?;
        let mut url = Url::parse(&format!("{PARALLEL_API_BASE}/monitors/{monitor_id}/events"))
            .map_err(|_| LegalError::InvalidResponse)?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", "50");
            if let Some(cursor) = cursor {
                query.append_pair("cursor", cursor);
            }
            if include_completions {
                query.append_pair("include_completions", "true");
            }
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
        let next_cursor = match raw.get("next_cursor") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(
                value
                    .as_str()
                    .ok_or(LegalError::InvalidResponse)?
                    .to_owned(),
            ),
        };
        validate_monitor_event_citations(&events, source)?;
        Ok(LegalMonitorEventsResponse {
            monitor_id: monitor_id.to_owned(),
            events,
            next_cursor,
        })
    }

    /// Permanently cancel a monitor using Parallel's documented cancel operation.
    pub async fn cancel_monitor(
        &self,
        monitor_id: &str,
    ) -> Result<LegalMonitorResponse, LegalError> {
        self.cancel_monitor_for_tenant(monitor_id, DEFAULT_TENANT_SCOPE)
            .await
    }

    /// Cancel a monitor only when it belongs to the authenticated tenant.
    pub async fn cancel_monitor_for_tenant(
        &self,
        monitor_id: &str,
        tenant_scope: &str,
    ) -> Result<LegalMonitorResponse, LegalError> {
        validate_provider_id(monitor_id, "monitor_")?;
        let existing = self
            .parallel_get(format!("{PARALLEL_API_BASE}/monitors/{monitor_id}"))
            .await?;
        ensure_tenant(&existing, tenant_scope)?;
        let source = source_from_provider(&existing).ok_or(LegalError::InvalidResponse)?;
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
            source,
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

    /// Fetch bounded UTF-8 text from an HTTPS URL on the selected source.
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
            return Err(LegalError::UpstreamStatus {
                status: response.status().as_u16(),
            });
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        if content_type
            .as_deref()
            .is_some_and(|value| !is_supported_text_mime(value))
        {
            return Err(LegalError::UnsupportedDocumentFormat);
        }
        let bytes = read_bounded(response, MAX_FETCH_BYTES).await?;
        if bytes.starts_with(b"%PDF-") || bytes.contains(&0) {
            return Err(LegalError::UnsupportedDocumentFormat);
        }
        let content =
            String::from_utf8(bytes).map_err(|_| LegalError::UnsupportedDocumentFormat)?;
        Ok(LegalFetchResponse {
            source: request.source,
            url: url.to_string(),
            content_type,
            content,
        })
    }
}

fn is_supported_text_mime(value: &str) -> bool {
    matches!(
        value
            .split(';')
            .next()
            .unwrap_or(value)
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "text/html"
            | "application/xhtml+xml"
            | "application/json"
            | "application/ld+json"
            | "application/xml"
            | "text/xml"
            | "text/plain"
    )
}

/// Return whether a URL uses HTTPS on the selected source's host or a subdomain.
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
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(LegalError::ResourceNotFound);
        }
        return Err(LegalError::UpstreamStatus {
            status: response.status().as_u16(),
        });
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

fn tenant_marker(tenant_scope: &str) -> String {
    blake3::hash(tenant_scope.as_bytes()).to_hex().to_string()
}

fn ensure_tenant(value: &serde_json::Value, tenant_scope: &str) -> Result<(), LegalError> {
    let owner = value
        .get("metadata")
        .and_then(|metadata| metadata.get(TENANT_METADATA_KEY))
        .and_then(serde_json::Value::as_str);
    if owner.is_some_and(|owner| owner == tenant_marker(tenant_scope)) {
        Ok(())
    } else {
        Err(LegalError::ResourceNotFound)
    }
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
    if let Some(output) = output {
        if let Some(content) = output.get("content") {
            validate_content_urls(content, source)?;
        }
    }
    if let Some(basis) = output
        .and_then(|output| output.get("basis"))
        .and_then(serde_json::Value::as_array)
    {
        validate_basis_citations(basis, source)?;
    }
    Ok(())
}

fn validate_content_urls(value: &serde_json::Value, source: LegalSource) -> Result<(), LegalError> {
    match value {
        serde_json::Value::String(text) => {
            for token in text.split_whitespace() {
                let candidate = token.trim_matches(|ch: char| {
                    matches!(
                        ch,
                        '(' | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '<'
                            | '>'
                            | '"'
                            | '\''
                            | ','
                            | '.'
                            | ';'
                            | ':'
                    )
                });
                if (candidate.starts_with("https://") || candidate.starts_with("http://"))
                    && !url_belongs_to_source(source, candidate)
                {
                    return Err(LegalError::InvalidResponse);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                validate_content_urls(value, source)?;
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                validate_content_urls(value, source)?;
            }
        }
        _ => {}
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
        if let Some(content) = event.get("output").and_then(|output| output.get("content")) {
            validate_content_urls(content, source)?;
        }
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
