//! Paid external legal-source research routes.
//!
//! Private document retrieval continues through `/v1/rag/*`; these endpoints send only
//! the caller's query to Parallel and discard results outside the selected official host.

use axum::{
    extract::{Path, Query},
    http::request::Parts,
    Json,
};
use hacienda_legal::{
    LegalDeepResearchRequest, LegalFetchRequest, LegalMonitorRequest, LegalResearchClient,
    LegalSearchRequest, LegalSource,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use utoipa::ToSchema;

use crate::error::ApiError;

fn tenant_scope(parts: &Parts) -> String {
    crate::handlers::extract_auth_context(parts)
        .map(|context| context.tenant.as_str().to_owned())
        .unwrap_or_else(|| {
            hacienda_core::tenancy::TenantId::default_tenant()
                .as_str()
                .to_owned()
        })
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
/// Legal search request for one supported European official source.
pub struct LegalSearchRequestDto {
    /// Research terms or a legal question.
    pub query: String,
    /// One official-source provider (eurlex, legifrance, gesetze, rechtspraak, boe, normattiva).
    pub source: String,
    /// Maximum results to return; defaults to 10 and is clamped to 1–20.
    #[serde(default = "default_results")]
    pub max_results: u8,
}

const fn default_results() -> u8 {
    10
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
/// Request to fetch bounded text from an official legal-source URL.
pub struct LegalFetchRequestDto {
    /// One source name: `eurlex`, `legifrance`, `gesetze`, `rechtspraak`, `boe`, or `normattiva`.
    pub source: String,
    /// HTTPS URL on the selected provider's official domain.
    pub url: String,
}

/// Safe provenance for a result returned by the paid provider.
#[derive(Debug, Serialize, ToSchema)]
/// Search result after source and URL validation.
pub struct LegalSearchResultDto {
    /// Official legal source for this result.
    pub source: LegalSourceDto,
    /// Title returned by Parallel.
    pub title: String,
    /// Validated HTTPS URL on the selected official host.
    pub url: String,
    /// Publication date if supplied by the provider.
    pub published: Option<String>,
    /// Bounded source excerpts returned by Parallel.
    pub excerpts: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
/// Supported legal source identifiers returned by the API.
pub enum LegalSourceDto {
    /// European Union law on EUR-Lex.
    Eurlex,
    /// French law on Légifrance.
    Legifrance,
    /// German federal law on Gesetze im Internet.
    GesetzeImInternet,
    /// Dutch law on Rechtspraak.
    Rechtspraak,
    /// Spanish law on BOE.
    Boe,
    /// Italian law on Normattiva.
    Normattiva,
}

impl From<LegalSource> for LegalSourceDto {
    fn from(source: LegalSource) -> Self {
        match source {
            LegalSource::Eurlex => Self::Eurlex,
            LegalSource::Legifrance => Self::Legifrance,
            LegalSource::GesetzeImInternet => Self::GesetzeImInternet,
            LegalSource::Rechtspraak => Self::Rechtspraak,
            LegalSource::Boe => Self::Boe,
            LegalSource::Normattiva => Self::Normattiva,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
/// Search response with provider provenance and validated results.
pub struct LegalSearchResponseDto {
    /// Provider request identifier, safe for tracing and billing reconciliation.
    pub provider_search_id: String,
    /// Source used to constrain the provider request.
    pub source: LegalSourceDto,
    /// Validated results. Empty means the provider found no matching legal pages.
    pub results: Vec<LegalSearchResultDto>,
}

#[derive(Debug, Serialize, ToSchema)]
/// Fetch response with bounded UTF-8 legal text.
pub struct LegalFetchResponseDto {
    /// Source used to validate the requested host.
    pub source: LegalSourceDto,
    /// URL that was fetched; redirects are not followed.
    pub url: String,
    /// MIME type when returned by the server.
    pub content_type: Option<String>,
    /// Bounded UTF-8 textual content.
    pub content: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
/// Request for an asynchronous, source-restricted legal research brief.
pub struct LegalDeepResearchRequestDto {
    /// One supported official legal source.
    pub source: String,
    /// Research question, limited to 2,000 Unicode characters.
    pub query: String,
}

#[derive(Debug, Serialize, ToSchema)]
/// Run identifier, selected source and provider status.
pub struct LegalDeepResearchStatusDto {
    /// Opaque provider task-run identifier.
    pub run_id: String,
    /// Official source selected for this run.
    pub source: LegalSourceDto,
    /// Provider run state.
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
/// Deep-research result; `output` is absent while the provider is still running.
pub struct LegalDeepResearchResultDto {
    /// Opaque provider task-run identifier.
    pub run_id: String,
    /// Official source selected for this run.
    pub source: LegalSourceDto,
    /// Provider run state.
    pub status: String,
    /// Provider output, including citation and research basis fields.
    pub output: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
/// Request to monitor one official source for legal changes.
pub struct LegalMonitorRequestDto {
    /// One supported official legal source.
    pub source: String,
    /// Question to monitor, limited to 2,000 Unicode characters.
    pub query: String,
    /// Monitoring cadence, from 1 hour to 30 days; defaults to `1d`.
    #[serde(default = "default_monitor_frequency")]
    pub frequency: String,
    /// Optional processor: `lite`, `base`, or `core`.
    pub processor: Option<String>,
}
fn default_monitor_frequency() -> String {
    "1d".to_owned()
}

#[derive(Debug, Serialize, ToSchema)]
/// Identifier, source, and state of a provider monitor.
pub struct LegalMonitorResponseDto {
    /// Opaque provider monitor identifier.
    pub monitor_id: String,
    /// Official source watched by the monitor.
    pub source: LegalSourceDto,
    /// Provider monitor state.
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
/// One page of monitor events and the cursor for the next page.
pub struct LegalMonitorEventsResponseDto {
    /// Opaque provider monitor identifier.
    pub monitor_id: String,
    /// Events retain the provider's citation and basis fields.
    pub events: serde_json::Value,
    /// Opaque provider cursor to request the next page.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
/// Query parameters for monitor event pagination.
pub struct LegalMonitorEventsQueryDto {
    /// Cursor returned by the preceding page.
    pub cursor: Option<String>,
    /// Include scheduled runs that detected no material changes.
    #[param(default = false)]
    pub include_completions: Option<bool>,
}

fn provider_error(error: hacienda_legal::LegalError) -> ApiError {
    use hacienda_legal::LegalError as E;
    match error {
        E::InvalidQuery => ApiError::invalid_request(
            "Invalid `query`; provide 1–2,000 characters of legal research terms.",
        ),
        E::UnsupportedSource => ApiError::invalid_request(
            "Invalid `source`; choose eurlex, legifrance, gesetze, rechtspraak, boe, or normattiva.",
        ),
        E::UnsafeUrl => ApiError::invalid_request(
            "Invalid `url`; use HTTPS on the selected source's official host.",
        ),
        E::UnsupportedDocumentFormat => ApiError::invalid_request(
            "The official URL must return UTF-8 HTML, XHTML, JSON, XML, or plain text; PDF and binary formats are not supported by this fetch endpoint.",
        ),
        E::ResourceNotFound => ApiError::not_found(),
        error @ (E::Disabled | E::ProviderNotConfigured) => {
            tracing::warn!(error = %error, "legal research provider is not enabled or configured");
            ApiError::internal()
        }
        error @ (E::Upstream | E::InvalidResponse) => {
            tracing::warn!(error = %error, "legal research provider request failed");
            ApiError::internal()
        }
        E::UpstreamStatus { status } => {
            tracing::warn!(provider_status = status, "legal research provider request failed");
            ApiError::internal()
        }
        E::InvalidProviderId => ApiError::invalid_request(
            "Invalid provider resource ID; use the run_id or monitor_id returned when creating it.",
        ),
    }
}

fn status_dto(s: hacienda_legal::LegalTaskRunResponse) -> LegalDeepResearchStatusDto {
    LegalDeepResearchStatusDto {
        run_id: s.run_id,
        source: s.source.into(),
        status: s.status,
    }
}
fn monitor_dto(m: hacienda_legal::LegalMonitorResponse) -> LegalMonitorResponseDto {
    LegalMonitorResponseDto {
        monitor_id: m.monitor_id,
        source: m.source.into(),
        status: m.status,
    }
}

/// Start an asynchronous legal deep-research brief. Requires `legal:research`.
#[utoipa::path(post, path = "/v1/legal/deep-research", request_body = LegalDeepResearchRequestDto,
    responses((status = 202, body = LegalDeepResearchStatusDto, description = "Research run accepted"), (status = 400, description = "Invalid request"), (status = 403, description = "Requires legal:research")),
    security(("bearerAuth" = [])), description = "Requires legal:research", tag = "legal")]
pub async fn create_deep_research(
    parts: Parts,
    Json(request): Json<LegalDeepResearchRequestDto>,
) -> Result<(axum::http::StatusCode, Json<LegalDeepResearchStatusDto>), ApiError> {
    let source = LegalSource::from_str(&request.source).map_err(provider_error)?;
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let status = client
        .create_deep_research_for_tenant(
            LegalDeepResearchRequest {
                source,
                query: request.query,
            },
            &tenant_scope(&parts),
        )
        .await
        .map_err(provider_error)?;
    let dto = status_dto(status);
    tracing::info!(source = source.tool_prefix(), run_id = %dto.run_id, "legal deep research started");
    Ok((axum::http::StatusCode::ACCEPTED, Json(dto)))
}

/// Read deep-research task status. Requires `legal:research`.
#[utoipa::path(get, path = "/v1/legal/deep-research/{run_id}", params(("run_id" = String, Path, description = "Provider task run ID")), responses((status = 200, body = LegalDeepResearchStatusDto), (status = 403, description = "Requires legal:research")), security(("bearerAuth" = [])), description = "Requires legal:research", tag = "legal")]
pub async fn deep_research_status(
    parts: Parts,
    Path(run_id): Path<String>,
) -> Result<Json<LegalDeepResearchStatusDto>, ApiError> {
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let status = client
        .deep_research_status_for_tenant(&run_id, &tenant_scope(&parts))
        .await
        .map_err(provider_error)?;
    Ok(Json(status_dto(status)))
}

/// Read a completed brief and its citation basis. Requires `legal:research`.
#[utoipa::path(get, path = "/v1/legal/deep-research/{run_id}/result", params(("run_id" = String, Path)), responses((status = 200, body = LegalDeepResearchResultDto), (status = 202, body = LegalDeepResearchResultDto, description = "Research is still running; retry with the same run_id"), (status = 403, description = "Requires legal:research")), security(("bearerAuth" = [])), description = "Requires legal:research", tag = "legal")]
pub async fn deep_research_result(
    parts: Parts,
    Path(run_id): Path<String>,
) -> Result<(axum::http::StatusCode, Json<LegalDeepResearchResultDto>), ApiError> {
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let result = client
        .deep_research_result_for_tenant(&run_id, &tenant_scope(&parts))
        .await
        .map_err(provider_error)?;
    let pending = result.status != "completed";
    let dto = LegalDeepResearchResultDto {
        run_id: result.run_id,
        source: result.source.into(),
        status: result.status,
        output: result.output,
    };
    let status = if pending {
        axum::http::StatusCode::ACCEPTED
    } else {
        axum::http::StatusCode::OK
    };
    Ok((status, Json(dto)))
}

/// Create a recurring legal-source monitor. Requires `legal:research`.
#[utoipa::path(post, path = "/v1/legal/monitors", request_body = LegalMonitorRequestDto, responses((status = 201, body = LegalMonitorResponseDto), (status = 400, description = "Invalid request"), (status = 403, description = "Requires legal:research")), security(("bearerAuth" = [])), description = "Requires legal:research", tag = "legal")]
pub async fn create_monitor(
    parts: Parts,
    Json(request): Json<LegalMonitorRequestDto>,
) -> Result<(axum::http::StatusCode, Json<LegalMonitorResponseDto>), ApiError> {
    let source = LegalSource::from_str(&request.source).map_err(provider_error)?;
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let monitor = client
        .create_monitor_for_tenant(
            LegalMonitorRequest {
                source,
                query: request.query,
                frequency: request.frequency,
                processor: request.processor,
            },
            &tenant_scope(&parts),
        )
        .await
        .map_err(provider_error)?;
    let dto = monitor_dto(monitor);
    tracing::info!(source = source.tool_prefix(), monitor_id = %dto.monitor_id, "legal monitor created");
    Ok((axum::http::StatusCode::CREATED, Json(dto)))
}

/// List events and citation basis for a monitor. Requires `legal:research`.
#[utoipa::path(get, path = "/v1/legal/monitors/{monitor_id}/events", params(("monitor_id" = String, Path), LegalMonitorEventsQueryDto), responses((status = 200, body = LegalMonitorEventsResponseDto), (status = 403, description = "Requires legal:research")), security(("bearerAuth" = [])), description = "Requires legal:research", tag = "legal")]
pub async fn monitor_events(
    parts: Parts,
    Path(monitor_id): Path<String>,
    Query(query): Query<LegalMonitorEventsQueryDto>,
) -> Result<Json<LegalMonitorEventsResponseDto>, ApiError> {
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let events = client
        .monitor_events_for_tenant(
            &monitor_id,
            &tenant_scope(&parts),
            query.cursor.as_deref(),
            query.include_completions.unwrap_or(false),
        )
        .await
        .map_err(provider_error)?;
    Ok(Json(LegalMonitorEventsResponseDto {
        monitor_id: events.monitor_id,
        events: events.events,
        next_cursor: events.next_cursor,
    }))
}

/// Cancel a legal monitor. Requires `legal:research`.
#[utoipa::path(delete, path = "/v1/legal/monitors/{monitor_id}", params(("monitor_id" = String, Path)), responses((status = 200, body = LegalMonitorResponseDto), (status = 403, description = "Requires legal:research")), security(("bearerAuth" = [])), description = "Requires legal:research", tag = "legal")]
pub async fn cancel_monitor(
    parts: Parts,
    Path(monitor_id): Path<String>,
) -> Result<Json<LegalMonitorResponseDto>, ApiError> {
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let monitor = client
        .cancel_monitor_for_tenant(&monitor_id, &tenant_scope(&parts))
        .await
        .map_err(provider_error)?;
    Ok(Json(monitor_dto(monitor)))
}

/// Search one jurisdiction's official legal source. Requires `legal:research`.
#[utoipa::path(
    post,
    path = "/v1/legal/search",
    request_body = LegalSearchRequestDto,
    responses(
        (status = 200, body = LegalSearchResponseDto, description = "Official-source results with provider provenance"),
        (status = 400, description = "Invalid source or query"),
        (status = 403, description = "Requires legal:research")
    ),
    security(("bearerAuth" = [])),
    description = "Requires legal:research",
    tag = "legal"
)]
pub async fn search(
    Json(request): Json<LegalSearchRequestDto>,
) -> Result<Json<LegalSearchResponseDto>, ApiError> {
    let source = LegalSource::from_str(&request.source).map_err(provider_error)?;
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let result = client
        .search(LegalSearchRequest {
            query: request.query,
            source,
            max_results: request.max_results,
        })
        .await
        .map_err(provider_error)?;
    tracing::info!(source = source.tool_prefix(), provider_search_id = %result.provider_search_id, result_count = result.results.len(), "legal search completed");
    Ok(Json(LegalSearchResponseDto {
        provider_search_id: result.provider_search_id,
        source: result.source.into(),
        results: result
            .results
            .into_iter()
            .map(|r| LegalSearchResultDto {
                source: r.source.into(),
                title: r.title,
                url: r.url,
                published: r.published,
                excerpts: r.excerpts,
            })
            .collect(),
    }))
}

/// Fetch one official-source URL after enforcing its host allowlist. Requires `legal:research`.
#[utoipa::path(
    post,
    path = "/v1/legal/fetch",
    request_body = LegalFetchRequestDto,
    responses(
        (status = 200, body = LegalFetchResponseDto, description = "Bounded content from an official legal source"),
        (status = 400, description = "URL is not on the selected source"),
        (status = 403, description = "Requires legal:research")
    ),
    security(("bearerAuth" = [])),
    description = "Requires legal:research",
    tag = "legal"
)]
pub async fn fetch(
    Json(request): Json<LegalFetchRequestDto>,
) -> Result<Json<LegalFetchResponseDto>, ApiError> {
    let source = LegalSource::from_str(&request.source).map_err(provider_error)?;
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let result = client
        .fetch(LegalFetchRequest {
            source,
            url: request.url,
        })
        .await
        .map_err(provider_error)?;
    Ok(Json(LegalFetchResponseDto {
        source: result.source.into(),
        url: result.url,
        content_type: result.content_type,
        content: result.content,
    }))
}
