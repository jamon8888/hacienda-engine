//! Handlers for `POST /v1/pii/scan`, `POST /v1/pii/redact`, `GET /v1/pii/config`,
//! and `POST /v1/pii/reveal`.

use axum::{extract::State, http::request::Parts, Json};
use hacienda_core::SpanText;
use std::time::Instant;

use crate::{
    dto::{
        EntityDto, MintTokenRequest, MintTokenResponse, PiiConfigResponse, RedactTextRequest,
        RedactTextResponse, RevealTokenRequest, RevealTokenResponse, ScanTextRequest,
        ScanTextResponse,
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
            vertical_id: None,
            vertical_labels: Vec::new(),
        }),
        Some(pii) => {
            // The explicit field allowlist is here. Do not add:
            //   - pii.redaction.key_id (secret; identifies which key de-pseudonymises the corpus)
            //   - any field from KeyResolver or PseudonymKey
            //   - model_dir / lora_adapter_dir (filesystem paths, host topology)
            // vertical_id/vertical_labels ARE exposed, deliberately — see the rustdoc
            // on `PiiConfigResponse::vertical_id`.
            Json(PiiConfigResponse {
                enabled: true,
                regex_first: pii.regex_first,
                model_threshold_default: pii.model_threshold_default,
                merge_overlap_threshold: pii.merge_overlap_threshold,
                redaction_mode: format!("{:?}", pii.redaction.mode),
                model_enabled: pii.model.enabled,
                audit_enabled: pii.audit.enabled,
                vertical_id: pii.vertical.as_ref().map(|v| v.id.clone()),
                vertical_labels: pii
                    .vertical
                    .as_ref()
                    .map(|v| v.labels.clone())
                    .unwrap_or_default(),
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

/// `POST /v1/pii/token`
///
/// Mints the pseudonym token for a caller-supplied `(category, value)` pair, without
/// revealing anything. Requires `documents:process` only — not `pii:reveal` — because
/// computing the token for a value the caller already supplies discloses nothing new
/// (`facade.mint_pseudonym_token_with_auth`'s own doc comment, P3b §2). This is what
/// makes a GDPR right of access or erasure practicable against redacted storage: compute
/// the token for a named value, then search the corpus for *that token*.
#[utoipa::path(
    post,
    path = "/v1/pii/token",
    tag = "pii",
    operation_id = "mintToken",
    security(("bearerAuth" = [])),
    request_body = MintTokenRequest,
    responses(
        (status = 200, description = "The pseudonym token for the requested value", body = MintTokenResponse),
        (status = 400, description = "Unsupported category name, or no key resolvable for the caller's tenant"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn mint_token(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(body): SafeJson<MintTokenRequest>,
) -> Result<Json<MintTokenResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let token = state
        .facade
        .mint_pseudonym_token_with_auth(caller, &body.category, &body.value)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(MintTokenResponse { token }))
}

#[cfg(test)]
mod tests {
    use crate::{
        routes::build_router,
        state::{ApiLimits, ApiState},
    };
    use axum::{body::Body, http::Request, http::StatusCode};
    use hacienda_core::{
        auth::{
            authn::{InMemoryTokenStore, Token},
            AuthState, Capability, CapabilitySet,
        },
        jobs::InMemoryJobStore,
        pii::{PipelineConfig, VerticalConfig},
        HaciendaConfig, HaciendaFacade,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Body-size limit passed to `axum::body::to_bytes` when reading a test response.
    const MAX_CONFIG_RESPONSE_BYTES: usize = 64 * 1024;

    fn app(config: HaciendaConfig) -> axum::Router {
        let mut store = InMemoryTokenStore::new();
        let caps = CapabilitySet::new([Capability::DocumentsProcess]);
        store.insert("t-secret", Token::new("t-1", "user", caps));

        let facade = Arc::new(HaciendaFacade::new(config).unwrap());
        let jobs = InMemoryJobStore::new().into_arc();
        let auth = AuthState::new(Arc::new(store));
        build_router(ApiState::new(facade, jobs, auth, ApiLimits::default()))
    }

    async fn get_pii_config(app: &axum::Router) -> serde_json::Value {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/pii/config")
                    .header("authorization", "Bearer t-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), MAX_CONFIG_RESPONSE_BYTES)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn should_report_no_active_vertical_when_none_is_configured() {
        let config = HaciendaConfig::default().with_pii(PipelineConfig::default());
        let body = get_pii_config(&app(config)).await;
        assert!(matches!(body.get("vertical_id"), Some(value) if value.is_null()));
        assert_eq!(body["vertical_labels"], serde_json::json!([]));
    }

    /// The untested branch at the top of `pii_config`: `config.pii` is `None` entirely
    /// (PII disabled), not merely a `PipelineConfig` with no vertical configured.
    #[tokio::test]
    async fn should_report_no_vertical_when_pii_is_disabled() {
        let config = HaciendaConfig::default();
        assert!(
            config.pii.is_none(),
            "fixture must leave pii disabled for this test to be meaningful"
        );
        let body = get_pii_config(&app(config)).await;
        assert!(matches!(body.get("vertical_id"), Some(value) if value.is_null()));
        assert_eq!(body["vertical_labels"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn should_report_the_active_vertical_id_and_labels() {
        let pii = PipelineConfig {
            vertical: Some(VerticalConfig {
                id: "finance".to_string(),
                labels: vec!["swift_code".to_string(), "account_number".to_string()],
            }),
            ..Default::default()
        };
        let config = HaciendaConfig::default().with_pii(pii);
        let body = get_pii_config(&app(config)).await;
        assert_eq!(body["vertical_id"], "finance");
        assert_eq!(
            body["vertical_labels"],
            serde_json::json!(["swift_code", "account_number"])
        );
    }
}
