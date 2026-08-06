//! Handlers for audit, review, compliance, and glossary endpoints (Phase 10).

use axum::{extract::State, http::request::Parts, Json};
use std::time::Instant;

use crate::{
    dto::{
        AuditResponse, AuditVerifyResponse, ComplianceDpiaResponse, ComplianceReportResponse,
        GlossaryResponse, ReviewDecideRequest, ReviewDecideResponse, ReviewResponse,
    },
    error::ApiError,
    extract::Json as SafeJson,
    handlers::{caller_from_arc, extract_auth_context},
    state::ApiState,
};

/// `GET /v1/audit`
#[utoipa::path(
    get,
    path = "/v1/audit",
    tag = "audit",
    operation_id = "getAudit",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Audit chain entries", body = AuditResponse))
)]
pub async fn get_audit(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<AuditResponse>, ApiError> {
    let _start = Instant::now();

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let entries = state
        .facade
        .audit_entries_with_auth(caller)
        .await
        .map_err(ApiError::from)?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    Ok(Json(AuditResponse {
        entries: entries.into_iter().map(Into::into).collect(),
        audit_chain_tip,
    }))
}

/// `GET /v1/audit/verify`
#[utoipa::path(
    get,
    path = "/v1/audit/verify",
    tag = "audit",
    operation_id = "verifyAudit",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Whether the audit chain verifies", body = AuditVerifyResponse))
)]
pub async fn verify_audit(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<AuditVerifyResponse>, ApiError> {
    let _start = Instant::now();

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    state
        .facade
        .verify_audit_with_auth(caller)
        .await
        .map_err(ApiError::from)?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    Ok(Json(AuditVerifyResponse {
        valid: true,
        audit_chain_tip,
    }))
}

/// `GET /v1/review`
#[utoipa::path(
    get,
    path = "/v1/review",
    tag = "review",
    operation_id = "getReview",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Pending review queue items", body = ReviewResponse))
)]
pub async fn get_review(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<ReviewResponse>, ApiError> {
    let _start = Instant::now();

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let queue = state
        .facade
        .review_queue_read_with_auth(caller)
        .map_err(ApiError::from)?;

    let items = match queue {
        Some(q) => q.list(None).await.unwrap_or_default(),
        None => Vec::new(),
    };

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    Ok(Json(ReviewResponse {
        items: items.into_iter().map(Into::into).collect(),
        audit_chain_tip,
    }))
}

/// `POST /v1/review/{id}/decide`
#[utoipa::path(
    post,
    path = "/v1/review/{id}/decide",
    tag = "review",
    operation_id = "decideReview",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "Review item id")),
    request_body = ReviewDecideRequest,
    responses(
        (status = 200, description = "The decided review item", body = ReviewDecideResponse),
        (status = 400, description = "Invalid decision or review queue not configured"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks review:decide")
    )
)]
pub async fn decide_review(
    State(state): State<ApiState>,
    parts: Parts,
    axum::extract::Path(id): axum::extract::Path<String>,
    SafeJson(body): SafeJson<ReviewDecideRequest>,
) -> Result<Json<ReviewDecideResponse>, ApiError> {
    let _start = Instant::now();

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let queue = state
        .facade
        .review_queue_with_auth(caller)
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::invalid_request("review queue not configured"))?;

    let item = queue
        .decide(&id, body.decision.into(), &body.reviewer, &body.comment)
        .await
        .map_err(ApiError::from)?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    Ok(Json(ReviewDecideResponse {
        item: item.into(),
        audit_chain_tip,
    }))
}

/// `GET /v1/compliance/dpia`
#[utoipa::path(
    get,
    path = "/v1/compliance/dpia",
    tag = "compliance",
    operation_id = "getComplianceDpia",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "DPIA report", body = ComplianceDpiaResponse),
        (status = 400, description = "Compliance reporting not configured")
    )
)]
pub async fn get_compliance_dpia(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<ComplianceDpiaResponse>, ApiError> {
    let _start = Instant::now();

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let report = state
        .facade
        .compliance_report_with_auth(caller)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::invalid_request("compliance not configured"))?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    let report = serde_json::to_value(report.dpia).map_err(|e| {
        tracing::error!(error = %e, "failed to serialise DPIA report");
        ApiError::internal()
    })?;

    Ok(Json(ComplianceDpiaResponse {
        report,
        audit_chain_tip,
    }))
}

/// `GET /v1/compliance/report`
#[utoipa::path(
    get,
    path = "/v1/compliance/report",
    tag = "compliance",
    operation_id = "getComplianceReport",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Full compliance report (model card, DORA, checklist)", body = ComplianceReportResponse),
        (status = 400, description = "Compliance reporting not configured")
    )
)]
pub async fn get_compliance_report(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<ComplianceReportResponse>, ApiError> {
    let _start = Instant::now();

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let report = state
        .facade
        .compliance_report_with_auth(caller)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::invalid_request("compliance not configured"))?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    let report = serde_json::to_value(report).map_err(|e| {
        tracing::error!(error = %e, "failed to serialise compliance report");
        ApiError::internal()
    })?;

    Ok(Json(ComplianceReportResponse {
        report,
        audit_chain_tip,
    }))
}

/// `GET /v1/glossary`
#[utoipa::path(
    get,
    path = "/v1/glossary",
    tag = "glossary",
    operation_id = "getGlossary",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Entity glossary snapshot", body = GlossaryResponse))
)]
pub async fn get_glossary(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<GlossaryResponse>, ApiError> {
    let _start = Instant::now();

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let entries = state
        .facade
        .glossary_snapshot_with_auth(caller)
        .await
        .map_err(ApiError::from)?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    let entries = entries
        .into_iter()
        .map(|e| {
            serde_json::to_value(e).map_err(|err| {
                tracing::error!(error = %err, "failed to serialise glossary entry");
                ApiError::internal()
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(GlossaryResponse {
        entries,
        audit_chain_tip,
    }))
}
