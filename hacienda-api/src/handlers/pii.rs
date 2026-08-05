//! Handlers for `POST /v1/pii/scan`, `POST /v1/pii/redact`, `GET /v1/pii/config`,
//! and `POST /v1/pii/reveal`.

use axum::{extract::State, http::request::Parts, Json};
use hacienda_core::SpanText;
use std::time::Instant;

use crate::{
    dto::{
        EntityDto, PiiConfigResponse, RedactTextRequest, RedactTextResponse, RevealTokenRequest,
        RevealTokenResponse, ScanTextRequest, ScanTextResponse,
    },
    error::ApiError,
    extract::Json as SafeJson,
    handlers::{caller_from_arc, extract_auth_context},
    state::ApiState,
};

/// `POST /v1/pii/scan`
///
/// With `include_text: false` (the default), requires `documents:process`.
/// With `include_text: true`, requires `documents:process` AND `pii:reveal` — that
/// second requirement is enforced by `facade.scan_text_with_auth` itself, which also
/// writes the attributed `Reveal` audit entry. We do not re-check it here; we
/// delegate to the facade and map `HaciendaError::Authz` to 403.
#[utoipa::path(
    post,
    path = "/v1/pii/scan",
    tag = "pii",
    operation_id = "scanText",
    security(("bearerAuth" = [])),
    request_body = ScanTextRequest,
    responses(
        (status = 200, description = "Detected PII entities", body = ScanTextResponse),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks documents:process, or lacks pii:reveal when include_text=true")
    )
)]
pub async fn scan_text(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(body): SafeJson<ScanTextRequest>,
) -> Result<Json<ScanTextResponse>, ApiError> {
    let start = Instant::now();

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let span_text = if body.include_text {
        SpanText::Include
    } else {
        SpanText::Omit
    };

    let result = state
        .facade
        .scan_text_with_auth(caller, &body.text, span_text)
        .await
        .map_err(ApiError::from)?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    Ok(Json(ScanTextResponse {
        entities: result.entities.into_iter().map(EntityDto::from).collect(),
        document_count: 1,
        processing_time_ms: start.elapsed().as_millis() as u64,
        audit_chain_tip,
    }))
}

/// `POST /v1/pii/redact`
#[utoipa::path(
    post,
    path = "/v1/pii/redact",
    tag = "pii",
    operation_id = "redactText",
    security(("bearerAuth" = [])),
    request_body = RedactTextRequest,
    responses(
        (status = 200, description = "Redacted text", body = RedactTextResponse),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn redact_text(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(body): SafeJson<RedactTextRequest>,
) -> Result<Json<RedactTextResponse>, ApiError> {
    let start = Instant::now();

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let result = state
        .facade
        .redact_text_with_auth(caller, &body.text)
        .await
        .map_err(ApiError::from)?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    Ok(Json(RedactTextResponse {
        redacted_text: result.redacted_text,
        entity_count: result.entities.len(),
        processing_time_ms: start.elapsed().as_millis() as u64,
        audit_chain_tip,
    }))
}

/// `GET /v1/pii/config`
///
/// Returns the effective detection configuration using an explicit allowlist of
/// fields. Key material, key IDs, and resolver configuration are deliberately absent.
#[utoipa::path(
    get,
    path = "/v1/pii/config",
    tag = "pii",
    operation_id = "getPiiConfig",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Effective PII detection configuration", body = PiiConfigResponse))
)]
pub async fn pii_config(State(state): State<ApiState>) -> Json<PiiConfigResponse> {
    let config = state.facade.config();
    match &config.pii {
        None => Json(PiiConfigResponse {
            enabled: false,
            regex_first: false,
            model_threshold_default: 0.0,
            merge_overlap_threshold: 0.0,
            redaction_mode: "none".to_string(),
            model_enabled: false,
            audit_enabled: false,
        }),
        Some(pii) => {
            // The explicit field allowlist is here. Do not add:
            //   - pii.redaction.key_id (secret; identifies which key de-pseudonymises the corpus)
            //   - any field from KeyResolver or PseudonymKey
            //   - model_dir / lora_adapter_dir (filesystem paths, host topology)
            Json(PiiConfigResponse {
                enabled: true,
                regex_first: pii.regex_first,
                model_threshold_default: pii.model_threshold_default,
                merge_overlap_threshold: pii.merge_overlap_threshold,
                redaction_mode: format!("{:?}", pii.redaction.mode),
                model_enabled: pii.model.enabled,
                audit_enabled: pii.audit.enabled,
            })
        }
    }
}

/// `POST /v1/pii/reveal`
///
/// Requires `documents:process` AND `pii:reveal`. The second requirement is
/// enforced by `facade.reveal_token_with_auth` itself, which also writes the
/// attributed `Reveal` audit entry. We delegate to the facade and map
/// `HaciendaError::Authz` to 403.
#[utoipa::path(
    post,
    path = "/v1/pii/reveal",
    tag = "pii",
    operation_id = "revealToken",
    security(("bearerAuth" = [])),
    request_body = RevealTokenRequest,
    responses(
        (status = 200, description = "The plaintext behind the pseudonym token", body = RevealTokenResponse),
        (status = 400, description = "Malformed, unreadable, or unknown-key token"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks pii:reveal")
    )
)]
pub async fn reveal_token(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(body): SafeJson<RevealTokenRequest>,
) -> Result<Json<RevealTokenResponse>, ApiError> {
    let _start = Instant::now();

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let result = state
        .facade
        .reveal_token_with_auth(caller, &body.token)
        .await
        .map_err(ApiError::from)?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    Ok(Json(RevealTokenResponse {
        plaintext: result,
        audit_chain_tip,
    }))
}
