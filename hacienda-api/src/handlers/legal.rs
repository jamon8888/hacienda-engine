//! Paid external legal-source research routes.
//!
//! Private document retrieval continues through `/v1/rag/*`; these endpoints send only
//! the caller's query to Parallel and discard results outside the selected official host.

use axum::{extract::Path, Json};
use hacienda_legal::{
    LegalDeepResearchRequest, LegalFetchRequest, LegalMonitorRequest, LegalResearchClient,
    LegalSearchRequest, LegalSource,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::ApiError;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LegalSearchRequestDto {
    /// Research terms or a legal question.
    pub query: String,
    /// One official-source provider (eurlex, legifrance, gesetze, rechtspraak, boe, normattiva).
    pub source: String,
    #[serde(default = "default_results")]
    pub max_results: u8,
}

const fn default_results() -> u8 {
    10
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LegalFetchRequestDto {
    pub source: String,
    /// HTTPS URL on the selected provider's official domain.
    pub url: String,
}

/// Safe provenance for a result returned by the paid provider.
#[derive(Debug, Serialize, ToSchema)]
pub struct LegalSearchResultDto {
    pub source: LegalSourceDto,
    pub title: String,
    pub url: String,
    pub published: Option<String>,
    pub excerpts: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LegalSourceDto {
    Eurlex,
    Legifrance,
    GesetzeImInternet,
    Rechtspraak,
    Boe,
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
pub struct LegalSearchResponseDto {
    /// Provider request identifier, safe for tracing and billing reconciliation.
    pub provider_search_id: String,
    pub source: LegalSourceDto,
    pub results: Vec<LegalSearchResultDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LegalFetchResponseDto {
    pub source: LegalSourceDto,
    pub url: String,
    pub content_type: Option<String>,
    pub content: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LegalDeepResearchRequestDto {
    pub source: String,
    pub query: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LegalDeepResearchStatusDto {
    pub run_id: String,
    pub source: LegalSourceDto,
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LegalDeepResearchResultDto {
    pub run_id: String,
    pub source: LegalSourceDto,
    pub status: String,
    /// Provider output, including citation and research basis fields.
    pub output: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LegalMonitorRequestDto {
    pub source: String,
    pub query: String,
    #[serde(default = "default_monitor_frequency")]
    pub frequency: String,
    pub processor: Option<String>,
}
fn default_monitor_frequency() -> String {
    "1d".to_owned()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LegalMonitorResponseDto {
    pub monitor_id: String,
    pub source: LegalSourceDto,
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LegalMonitorEventsResponseDto {
    pub monitor_id: String,
    /// Events retain the provider's citation and basis fields.
    pub events: serde_json::Value,
}

fn provider_error(error: hacienda_legal::LegalError) -> ApiError {
    use hacienda_legal::LegalError as E;
    match error {
        E::InvalidQuery | E::UnsupportedSource | E::UnsafeUrl => {
            ApiError::invalid_request("Invalid legal research request.")
        }
        E::Disabled | E::ProviderNotConfigured => {
            tracing::warn!("legal research provider is not enabled or configured");
            ApiError::not_found()
        }
        E::Upstream | E::InvalidResponse => {
            tracing::warn!("legal research provider request failed");
            ApiError::internal()
        }
        E::InvalidProviderId => ApiError::invalid_request("Invalid legal research request."),
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
    Json(request): Json<LegalDeepResearchRequestDto>,
) -> Result<(axum::http::StatusCode, Json<LegalDeepResearchStatusDto>), ApiError> {
    let source = LegalSource::from_str(&request.source).map_err(provider_error)?;
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let status = client
        .create_deep_research(LegalDeepResearchRequest {
            source,
            query: request.query,
        })
        .await
        .map_err(provider_error)?;
    let dto = status_dto(status);
    tracing::info!(source = source.tool_prefix(), run_id = %dto.run_id, "legal deep research started");
    Ok((axum::http::StatusCode::ACCEPTED, Json(dto)))
}

/// Read deep-research task status. Requires `legal:research`.
#[utoipa::path(get, path = "/v1/legal/deep-research/{run_id}", params(("run_id" = String, Path, description = "Provider task run ID")), responses((status = 200, body = LegalDeepResearchStatusDto), (status = 403, description = "Requires legal:research")), security(("bearerAuth" = [])), description = "Requires legal:research", tag = "legal")]
pub async fn deep_research_status(
    Path(run_id): Path<String>,
) -> Result<Json<LegalDeepResearchStatusDto>, ApiError> {
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let status = client
        .deep_research_status(&run_id)
        .await
        .map_err(provider_error)?;
    Ok(Json(status_dto(status)))
}

/// Read a completed brief and its citation basis. Requires `legal:research`.
#[utoipa::path(get, path = "/v1/legal/deep-research/{run_id}/result", params(("run_id" = String, Path)), responses((status = 200, body = LegalDeepResearchResultDto), (status = 403, description = "Requires legal:research")), security(("bearerAuth" = [])), description = "Requires legal:research", tag = "legal")]
pub async fn deep_research_result(
    Path(run_id): Path<String>,
) -> Result<Json<LegalDeepResearchResultDto>, ApiError> {
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let result = client
        .deep_research_result(&run_id)
        .await
        .map_err(provider_error)?;
    Ok(Json(LegalDeepResearchResultDto {
        run_id: result.run_id,
        source: result.source.into(),
        status: result.status,
        output: result.output,
    }))
}

/// Create a recurring legal-source monitor. Requires `legal:research`.
#[utoipa::path(post, path = "/v1/legal/monitors", request_body = LegalMonitorRequestDto, responses((status = 201, body = LegalMonitorResponseDto), (status = 400, description = "Invalid request"), (status = 403, description = "Requires legal:research")), security(("bearerAuth" = [])), description = "Requires legal:research", tag = "legal")]
pub async fn create_monitor(
    Json(request): Json<LegalMonitorRequestDto>,
) -> Result<(axum::http::StatusCode, Json<LegalMonitorResponseDto>), ApiError> {
    let source = LegalSource::from_str(&request.source).map_err(provider_error)?;
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let monitor = client
        .create_monitor(LegalMonitorRequest {
            source,
            query: request.query,
            frequency: request.frequency,
            processor: request.processor,
        })
        .await
        .map_err(provider_error)?;
    let dto = monitor_dto(monitor);
    tracing::info!(source = source.tool_prefix(), monitor_id = %dto.monitor_id, "legal monitor created");
    Ok((axum::http::StatusCode::CREATED, Json(dto)))
}

/// List events and citation basis for a monitor. Requires `legal:research`.
#[utoipa::path(get, path = "/v1/legal/monitors/{monitor_id}/events", params(("monitor_id" = String, Path)), responses((status = 200, body = LegalMonitorEventsResponseDto), (status = 403, description = "Requires legal:research")), security(("bearerAuth" = [])), description = "Requires legal:research", tag = "legal")]
pub async fn monitor_events(
    Path(monitor_id): Path<String>,
) -> Result<Json<LegalMonitorEventsResponseDto>, ApiError> {
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let events = client
        .monitor_events(&monitor_id)
        .await
        .map_err(provider_error)?;
    Ok(Json(LegalMonitorEventsResponseDto {
        monitor_id: events.monitor_id,
        events: events.events,
    }))
}

/// Cancel a legal monitor. Requires `legal:research`.
#[utoipa::path(delete, path = "/v1/legal/monitors/{monitor_id}", params(("monitor_id" = String, Path)), responses((status = 200, body = LegalMonitorResponseDto), (status = 403, description = "Requires legal:research")), security(("bearerAuth" = [])), description = "Requires legal:research", tag = "legal")]
pub async fn cancel_monitor(
    Path(monitor_id): Path<String>,
) -> Result<Json<LegalMonitorResponseDto>, ApiError> {
    let client = LegalResearchClient::from_env().map_err(provider_error)?;
    let monitor = client
        .cancel_monitor(&monitor_id)
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
