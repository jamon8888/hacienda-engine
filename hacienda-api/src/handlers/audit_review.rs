//! Handlers for audit, review, compliance, and glossary endpoints (Phase 10).

use axum::{extract::State, http::request::Parts, Json};
use std::time::Instant;

use crate::{
    dto::{
        AuditResponse, AuditVerifyResponse, ReviewDecideRequest, ReviewDecideResponse,
        ReviewResponse,
    },
    error::ApiError,
    extract::Json as SafeJson,
    handlers::{caller_from_arc, extract_auth_context},
    state::ApiState,
};

/// `GET /v1/audit`
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

    let decision = match body.decision.as_str() {
        "approve" => hacienda_core::review::types::ReviewDecision::Approve,
        "reject" => hacienda_core::review::types::ReviewDecision::Reject,
        "modify" => hacienda_core::review::types::ReviewDecision::Modify,
        _ => return Err(ApiError::invalid_request("invalid decision")),
    };

    let item = queue
        .decide(&id, decision, &body.reviewer, &body.comment)
        .await
        .map_err(ApiError::from)?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    Ok(Json(ReviewDecideResponse {
        item: item.into(),
        audit_chain_tip,
    }))
}

/// `GET /v1/compliance/dpia`
pub async fn get_compliance_dpia(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<serde_json::Value>, ApiError> {
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

    Ok(Json(serde_json::json!({
        "report": report.dpia,
        "audit_chain_tip": audit_chain_tip,
    })))
}

/// `GET /v1/compliance/report`
pub async fn get_compliance_report(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<serde_json::Value>, ApiError> {
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

    Ok(Json(serde_json::json!({
        "report": report,
        "audit_chain_tip": audit_chain_tip,
    })))
}

/// `GET /v1/glossary`
pub async fn get_glossary(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _start = Instant::now();

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let entries = state
        .facade
        .glossary_snapshot_with_auth(caller)
        .await
        .map_err(ApiError::from)?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    Ok(Json(serde_json::json!({
        "entries": entries,
        "audit_chain_tip": audit_chain_tip,
    })))
}
