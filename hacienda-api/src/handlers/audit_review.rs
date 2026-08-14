//! Handlers for audit, review, compliance, and glossary endpoints (Phase 10).

use axum::{extract::State, http::request::Parts, Json};
use std::time::Instant;

use crate::{
    dto::{
        AssignReviewItemRequest, AssignReviewItemResponse, AuditResponse, AuditVerifyResponse,
        ComplianceChecklistResponse, ComplianceDpiaResponse, ComplianceModelCardResponse,
        ComplianceReportResponse, DoraIncidentRequest, DoraReportResponse, GlossaryResponse,
        ReviewDecideRequest, ReviewDecideResponse, ReviewItemResponse, ReviewResponse,
        ReviewStatsResponse,
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

/// Coarse `valid: bool` verification, superseded at `/v1/audit/verify` by
/// `handlers::audit::audit_verify` (broken-chain-as-200, names the offending
/// entry/seal — see `hacienda-api/src/routes.rs`'s comment on the `/v1/audit*`
/// routes). Kept, unrouted, as a smaller reference implementation.
#[allow(dead_code)]
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

    let tenant = caller.tenant_ctx().tenant;
    let items = match queue {
        Some(q) => q.list(&tenant, None).await.unwrap_or_default(),
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

    let tenant = caller.tenant_ctx().tenant;
    let item = queue
        .decide(
            &tenant,
            &id,
            body.decision.into(),
            &body.reviewer,
            &body.comment,
        )
        .await
        .map_err(ApiError::from)?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    Ok(Json(ReviewDecideResponse {
        item: item.into(),
        audit_chain_tip,
    }))
}

/// `GET /v1/review/{id}`
///
/// Requires `audit:read` — same tier as `GET /v1/review` (`review_queue_read_with_auth`),
/// not `review:decide`. Reading one item is not more sensitive than reading the list it
/// comes from.
#[utoipa::path(
    get,
    path = "/v1/review/{id}",
    tag = "review",
    operation_id = "getReviewItem",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "Review item id")),
    responses(
        (status = 200, description = "The review item", body = ReviewItemResponse),
        (status = 400, description = "Review queue not configured, or no item with this id"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks audit:read")
    )
)]
pub async fn get_review_item(
    State(state): State<ApiState>,
    parts: Parts,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<ReviewItemResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let queue = state
        .facade
        .review_queue_read_with_auth(caller)
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::invalid_request("review queue not configured"))?;

    let tenant = caller.tenant_ctx().tenant;
    let item = queue
        .get(&tenant, &id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::invalid_request("no review item with this id"))?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    Ok(Json(ReviewItemResponse {
        item: item.into(),
        audit_chain_tip,
    }))
}

/// `POST /v1/review/{id}/assign`
///
/// Requires `review:decide` — a write, same tier as `decide` above, not the `audit:read`
/// tier `GET /v1/review/{id}` uses.
#[utoipa::path(
    post,
    path = "/v1/review/{id}/assign",
    tag = "review",
    operation_id = "assignReviewItem",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "Review item id")),
    request_body = AssignReviewItemRequest,
    responses(
        (status = 200, description = "The assigned review item", body = AssignReviewItemResponse),
        (status = 400, description = "Review queue not configured, or no item with this id"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks review:decide")
    )
)]
pub async fn assign_review_item(
    State(state): State<ApiState>,
    parts: Parts,
    axum::extract::Path(id): axum::extract::Path<String>,
    SafeJson(body): SafeJson<AssignReviewItemRequest>,
) -> Result<Json<AssignReviewItemResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let queue = state
        .facade
        .review_queue_with_auth(caller)
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::invalid_request("review queue not configured"))?;

    let tenant = caller.tenant_ctx().tenant;
    let item = queue
        .assign(&tenant, &id, &body.reviewer)
        .await
        .map_err(ApiError::from)?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    Ok(Json(AssignReviewItemResponse {
        item: item.into(),
        audit_chain_tip,
    }))
}

/// `GET /v1/review/stats`
///
/// Requires `audit:read`, same tier as `GET /v1/review` and `GET /v1/review/{id}` — an
/// aggregate read, no snippet text.
#[utoipa::path(
    get,
    path = "/v1/review/stats",
    tag = "review",
    operation_id = "getReviewStats",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Counts per review status", body = ReviewStatsResponse),
        (status = 400, description = "Review queue not configured"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks audit:read")
    )
)]
pub async fn get_review_stats(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<ReviewStatsResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let queue = state
        .facade
        .review_queue_read_with_auth(caller)
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::invalid_request("review queue not configured"))?;

    let tenant = caller.tenant_ctx().tenant;
    let stats = queue.stats(&tenant).await.map_err(ApiError::from)?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    Ok(Json(ReviewStatsResponse {
        total: stats.total,
        pending: stats.pending,
        in_review: stats.in_review,
        approved: stats.approved,
        rejected: stats.rejected,
        modified: stats.modified,
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

/// `GET /v1/compliance/model-card`
#[utoipa::path(
    get,
    path = "/v1/compliance/model-card",
    tag = "compliance",
    operation_id = "getComplianceModelCard",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "AI Act Art. 11 model card", body = ComplianceModelCardResponse),
        (status = 400, description = "Compliance reporting not configured")
    )
)]
pub async fn get_compliance_model_card(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<ComplianceModelCardResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let card = state
        .facade
        .model_card_with_auth(caller)
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::invalid_request("compliance not configured"))?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    let report = serde_json::to_value(card).map_err(|e| {
        tracing::error!(error = %e, "failed to serialise model card");
        ApiError::internal()
    })?;

    Ok(Json(ComplianceModelCardResponse {
        report,
        audit_chain_tip,
    }))
}

/// `GET /v1/compliance/checklist`
#[utoipa::path(
    get,
    path = "/v1/compliance/checklist",
    tag = "compliance",
    operation_id = "getComplianceChecklist",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Compliance control checklist", body = ComplianceChecklistResponse),
        (status = 400, description = "Compliance reporting not configured")
    )
)]
pub async fn get_compliance_checklist(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<ComplianceChecklistResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let checklist = state
        .facade
        .compliance_checklist_with_auth(caller)
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::invalid_request("compliance not configured"))?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    let report = serde_json::to_value(checklist).map_err(|e| {
        tracing::error!(error = %e, "failed to serialise compliance checklist");
        ApiError::internal()
    })?;

    Ok(Json(ComplianceChecklistResponse {
        report,
        audit_chain_tip,
    }))
}

/// `POST /v1/compliance/dora`
///
/// A `POST`, not `GET` — see `DoraIncidentRequest`'s own doc comment and
/// `superpowers/specs/2026-08-14-P4-P5-missing-endpoints.md` §2. Unlike `model-card` and
/// `checklist` above, a DORA report was previously unreachable through this API at all:
/// `compliance_report_with_auth` never supplies an incident, so `GET /v1/compliance/report`
/// can never include one.
#[utoipa::path(
    post,
    path = "/v1/compliance/dora",
    tag = "compliance",
    operation_id = "postComplianceDora",
    security(("bearerAuth" = [])),
    request_body = DoraIncidentRequest,
    responses(
        (status = 200, description = "DORA Art. 11 incident report", body = DoraReportResponse),
        (status = 400, description = "Compliance reporting not configured")
    )
)]
pub async fn post_compliance_dora(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(body): SafeJson<DoraIncidentRequest>,
) -> Result<Json<DoraReportResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let incident = body.into();
    let report = state
        .facade
        .dora_report_with_auth(caller, &incident)
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::invalid_request("compliance not configured"))?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    let report = serde_json::to_value(report).map_err(|e| {
        tracing::error!(error = %e, "failed to serialise DORA report");
        ApiError::internal()
    })?;

    Ok(Json(DoraReportResponse {
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
