//! Handlers for API key management (Phase 11 Task 3).
//!
//! All three routes require `auth:manage` — see the `ROUTE_TABLE` entries in
//! `routes.rs`. `issue_key` additionally never logs or re-emits the raw key: it
//! appears exactly once, in `IssueKeyResponse::raw_key`.

use std::str::FromStr;

use axum::{
    extract::{Path, State},
    http::{request::Parts, StatusCode},
    Json,
};
use hacienda_core::auth::{Caller, Capability};
use uuid::Uuid;

use crate::{
    dto::{AuthConfigResponse, IssueKeyRequest, IssueKeyResponse, WhoamiResponse},
    error::ApiError,
    extract::Json as SafeJson,
    handlers::{caller_from_arc, extract_auth_context},
    state::ApiState,
};

/// Enforce `auth:manage` for a caller, converting the core `AuthzError` into the API
/// error envelope.
///
/// `get_auth_config` has no facade method to enforce this for it (there is nothing to
/// call — the data comes straight off `HaciendaFacade::config()`), so it checks here
/// directly rather than relying solely on the route table's declared requirement. This
/// mirrors every other handler in this crate: the route table decides who *may* reach
/// the handler, but the handler independently re-asserts the capability it actually
/// needs, so the two cannot silently drift apart (see
/// `guarded_handler_observes_the_caller_not_trusted` in `routes.rs`).
fn require_auth_manage(caller: Caller<'_>) -> Result<(), ApiError> {
    caller
        .require(Capability::AuthManage)
        .map_err(hacienda_core::error::HaciendaError::from)
        .map_err(ApiError::from)
}

/// `POST /v1/auth/keys` — issue a new API key.
///
/// Requires `auth:manage`, enforced inside `HaciendaFacade::issue_key_with_auth`. The
/// raw key is returned once in the response body and is never logged or echoed by any
/// other endpoint.
#[utoipa::path(
    post,
    path = "/v1/auth/keys",
    tag = "auth",
    operation_id = "issueKey",
    security(("bearerAuth" = [])),
    request_body = IssueKeyRequest,
    responses(
        (status = 200, description = "The issued key; raw_key is shown exactly once", body = IssueKeyResponse),
        (status = 400, description = "One or more requested capabilities are not recognised"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks auth:manage")
    )
)]
pub async fn issue_key(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(body): SafeJson<IssueKeyRequest>,
) -> Result<Json<IssueKeyResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    // Parse capability names before calling the facade so a typo never results in a
    // key issued with a silently-narrower grant than the caller asked for.
    let capabilities: Vec<Capability> = body
        .capabilities
        .iter()
        .map(|s| Capability::from_str(s))
        .collect::<Result<_, _>>()
        .map_err(|_| {
            ApiError::invalid_request("one or more requested capabilities are not recognised")
        })?;
    // Canonical wire-form strings for the response, derived from the parsed
    // capabilities rather than round-tripped through storage: `Capability`'s stored
    // JSON representation is `#[serde(rename_all = "snake_case")]` on the variant name
    // (e.g. `"documents_process"`), which is a different string than the colon-form
    // this API accepts and returns everywhere else (`"documents:process"`, via
    // `Capability`'s `Display` impl).
    let capability_strings: Vec<String> = capabilities.iter().map(ToString::to_string).collect();

    // `tenant: None` — HTTP callers are always `Caller::Principal` (resolved by the auth
    // middleware from a bearer token), so this always resolves to the caller's own
    // tenant inside `issue_key_with_auth`. There is no `tenant` field on `IssueKeyRequest`
    // and there must not be one: an admin cannot mint a key for a different tenant than
    // its own over this endpoint, by construction, not just by convention.
    let (pair, key) = state
        .facade
        .issue_key_with_auth(caller, &body.owner, None, capabilities)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(IssueKeyResponse {
        id: key.id,
        raw_key: pair.raw_key,
        owner: key.owner,
        tenant: key.tenant.as_str().to_owned(),
        capabilities: capability_strings,
        created_at: key.created_at.to_rfc3339(),
    }))
}

/// `DELETE /v1/auth/keys/{id}` — revoke an API key.
///
/// Requires `auth:manage`, enforced inside `HaciendaFacade::revoke_key_with_auth`.
/// Revoking an id that does not exist (or was already revoked) is not an error: both
/// the in-memory and Postgres `ApiKeyStore` implementations treat revocation as
/// idempotent, so this always answers 204 rather than leaking whether an id exists —
/// the same membership-oracle concern `GET /v1/jobs/{id}` documents for job ids.
#[utoipa::path(
    delete,
    path = "/v1/auth/keys/{id}",
    tag = "auth",
    operation_id = "revokeKey",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "Key id")),
    responses(
        (status = 204, description = "Revoked (idempotent — also returned for an unknown id)"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks auth:manage")
    )
)]
pub async fn revoke_key(
    State(state): State<ApiState>,
    parts: Parts,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    state
        .facade
        .revoke_key_with_auth(caller, id)
        .await
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/auth/config` — report the active auth configuration, no secrets.
///
/// # Known limitation
///
/// `resolver` reflects `HaciendaConfig::auth.resolver` — the *declared* configuration
/// — not necessarily the resolver instance actually wired into the live `AuthState`.
/// `AuthState` (in `hacienda-core`) holds its resolver behind a private
/// `Arc<dyn TokenResolver>` field with no introspection getter, and at least one
/// resolver (`ApiKeyTokenResolver`) is deliberately constructed by embedders directly
/// via `::new` rather than through `TokenResolverType`/`build_token_resolver`, because
/// it needs a live store handle `AuthConfig`'s serializable fields cannot carry (see
/// that type's doc comment). An embedder who wires a store-backed resolver in by hand
/// makes this field stale. Adding a real introspection API to `AuthState` would need a
/// `hacienda-core` change, which is out of scope for this endpoint — see the test
/// `auth_config_resolver_reflects_declared_config_not_the_live_resolver` below, which
/// demonstrates the gap rather than hiding it.
#[utoipa::path(
    get,
    path = "/v1/auth/config",
    tag = "auth",
    operation_id = "getAuthConfig",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Declared auth configuration (no secrets)", body = AuthConfigResponse),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks auth:manage")
    )
)]
pub async fn get_auth_config(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<AuthConfigResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    require_auth_manage(caller)?;

    let auth_config = &state.facade.config().auth;
    let resolver = match auth_config.resolver {
        hacienda_core::auth::authn::TokenResolverType::Memory => "memory",
        hacienda_core::auth::authn::TokenResolverType::Dev => "dev",
    };

    Ok(Json(AuthConfigResponse {
        enabled: auth_config.enabled,
        resolver: resolver.to_string(),
    }))
}

/// `GET /v1/auth/whoami` — report the *calling* principal's own granted capabilities.
///
/// Unlike `GET /v1/auth/config` (server-wide declared configuration, gated on
/// `auth:manage`, which most callers do not hold), this reports only what the
/// presented token itself grants — the self-describing probe a client SDK needs to
/// adapt its method surface to the tenant, without requiring an elevated capability
/// just to ask "what can I do." Gated on `documents:process` for now (the capability
/// nearly every issued key holds, same precedent as presets/RAG/versions/uploads)
/// rather than left open to any authenticated token with zero capabilities — see the
/// design-spec correction this addresses: `_resolve_tier`-style capability probing
/// against `/v1/auth/config` does not work for a normal SDK caller.
#[utoipa::path(
    get,
    path = "/v1/auth/whoami",
    tag = "auth",
    operation_id = "getWhoami",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "The calling principal's own granted capabilities", body = WhoamiResponse),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn whoami(parts: Parts) -> Json<WhoamiResponse> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let (principal_id, capabilities) = match caller {
        Caller::Trusted => (None, Capability::all()),
        Caller::Principal(auth_ctx) => (
            Some(auth_ctx.principal_id.clone()),
            auth_ctx.capabilities.iter().collect(),
        ),
    };

    let mut capability_strings: Vec<String> =
        capabilities.iter().map(ToString::to_string).collect();
    capability_strings.sort();

    Json(WhoamiResponse {
        principal_id,
        capabilities: capability_strings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        routes::build_router,
        state::{ApiLimits, ApiState},
    };
    use axum::body::Body;
    use axum::http::Request;
    use hacienda_core::{
        auth::{authn::ApiKeyTokenResolver, ApiKeyStore, AuthState, InMemoryApiKeyStore},
        jobs::InMemoryJobStore,
        HaciendaConfig, HaciendaFacade,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Build a router wired to a shared in-memory `ApiKeyStore`, authenticated via
    /// `ApiKeyTokenResolver` (the resolver production `hacienda serve` uses for
    /// issued keys — not `DevTokenResolver`). Mints one bootstrap `auth:manage` key
    /// directly against the facade as `Caller::Trusted`, bypassing HTTP: this is
    /// exactly how a real deployment bootstraps its first admin key (CLI or direct
    /// database seed), since there is no key yet to authenticate the first `POST
    /// /v1/auth/keys` call with.
    ///
    /// Returns `(router, bootstrap_admin_raw_key)`.
    async fn app_with_bootstrap_admin() -> (axum::Router, String) {
        let store = Arc::new(InMemoryApiKeyStore::new());
        let facade = Arc::new(
            HaciendaFacade::with_stores(
                HaciendaConfig::default(),
                None,
                None,
                Some(Arc::clone(&store) as Arc<dyn ApiKeyStore>),
            )
            .expect("facade construction must succeed"),
        );

        let (pair, _record) = facade
            .issue_key_with_auth(
                Caller::Trusted,
                "bootstrap-admin",
                None,
                vec![Capability::AuthManage],
            )
            .await
            .expect("bootstrap issuance must succeed");

        let resolver = ApiKeyTokenResolver::new(store as Arc<dyn ApiKeyStore>);
        let auth = AuthState::new(Arc::new(resolver));
        let jobs = InMemoryJobStore::new().into_arc();
        let router = build_router(ApiState::new(facade, jobs, auth, ApiLimits::default()));
        (router, pair.raw_key)
    }

    /// Drive one request through `router`, returning `(status, json body)`.
    async fn request(
        router: &axum::Router,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&str>,
    ) -> (u16, serde_json::Value) {
        let mut builder = Request::builder().method(method).uri(path);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        let req = builder
            .body(Body::from(body.unwrap_or_default().to_string()))
            .unwrap();
        let response = router.clone().oneshot(req).await.unwrap();
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn should_issue_a_key_and_expose_the_raw_key_exactly_once() {
        let (router, admin_key) = app_with_bootstrap_admin().await;

        let (status, body) = request(
            &router,
            "POST",
            "/v1/auth/keys",
            Some(&admin_key),
            Some(r#"{"owner":"service-a","capabilities":["documents:process"]}"#),
        )
        .await;

        assert_eq!(
            status, 200,
            "issuance must succeed for an auth:manage caller"
        );
        let raw_key = body["raw_key"]
            .as_str()
            .expect("raw_key must be present in the issuance response");
        assert!(raw_key.starts_with("hcd_live_"));
        assert_eq!(body["owner"], "service-a");
        assert_eq!(
            body["capabilities"],
            serde_json::json!(["documents:process"])
        );
        assert!(body["id"].is_string());
        assert!(body["created_at"].is_string());
    }

    #[tokio::test]
    async fn should_reject_an_unrecognised_capability_name_with_400() {
        let (router, admin_key) = app_with_bootstrap_admin().await;

        let (status, _) = request(
            &router,
            "POST",
            "/v1/auth/keys",
            Some(&admin_key),
            Some(r#"{"owner":"service-a","capabilities":["not:a:real:capability"]}"#),
        )
        .await;

        assert_eq!(status, 400);
    }

    #[tokio::test]
    async fn should_reject_issuance_without_auth_manage_capability() {
        let (router, admin_key) = app_with_bootstrap_admin().await;

        // Mint a low-privilege key, then try to use it to issue another key.
        let (_, body) = request(
            &router,
            "POST",
            "/v1/auth/keys",
            Some(&admin_key),
            Some(r#"{"owner":"low-priv","capabilities":["documents:process"]}"#),
        )
        .await;
        let low_priv_key = body["raw_key"].as_str().unwrap().to_string();

        let (status, _) = request(
            &router,
            "POST",
            "/v1/auth/keys",
            Some(&low_priv_key),
            Some(r#"{"owner":"x","capabilities":["documents:process"]}"#),
        )
        .await;

        assert_eq!(status, 403);
    }

    /// End-to-end through the real router and store: issuing a key succeeds, the
    /// freshly issued key authenticates, revoking it succeeds, and the same key can no
    /// longer authenticate afterwards.
    #[tokio::test]
    async fn revoked_key_can_no_longer_authenticate() {
        let (router, admin_key) = app_with_bootstrap_admin().await;

        let (status, body) = request(
            &router,
            "POST",
            "/v1/auth/keys",
            Some(&admin_key),
            Some(r#"{"owner":"service-b","capabilities":["auth:manage"]}"#),
        )
        .await;
        assert_eq!(status, 200);
        let key_id = body["id"].as_str().unwrap().to_string();
        let raw_key = body["raw_key"].as_str().unwrap().to_string();

        // The freshly issued key authenticates against a capability-guarded route.
        let (status, _) = request(&router, "GET", "/v1/auth/config", Some(&raw_key), None).await;
        assert_eq!(status, 200, "a freshly issued key must authenticate");

        let (status, _) = request(
            &router,
            "DELETE",
            &format!("/v1/auth/keys/{key_id}"),
            Some(&admin_key),
            None,
        )
        .await;
        assert_eq!(status, 204);

        let (status, _) = request(&router, "GET", "/v1/auth/config", Some(&raw_key), None).await;
        assert_eq!(status, 401, "a revoked key must no longer authenticate");
    }

    #[tokio::test]
    async fn should_reject_revocation_without_auth_manage_capability() {
        let (router, admin_key) = app_with_bootstrap_admin().await;

        let (_, issued) = request(
            &router,
            "POST",
            "/v1/auth/keys",
            Some(&admin_key),
            Some(r#"{"owner":"target","capabilities":["documents:process"]}"#),
        )
        .await;
        let target_id = issued["id"].as_str().unwrap().to_string();

        let (_, low) = request(
            &router,
            "POST",
            "/v1/auth/keys",
            Some(&admin_key),
            Some(r#"{"owner":"low-priv","capabilities":["documents:process"]}"#),
        )
        .await;
        let low_priv_key = low["raw_key"].as_str().unwrap().to_string();

        let (status, _) = request(
            &router,
            "DELETE",
            &format!("/v1/auth/keys/{target_id}"),
            Some(&low_priv_key),
            None,
        )
        .await;

        assert_eq!(status, 403);
    }

    #[tokio::test]
    async fn get_auth_config_reports_fields_without_leaking_key_material() {
        let (router, admin_key) = app_with_bootstrap_admin().await;

        let (status, body) =
            request(&router, "GET", "/v1/auth/config", Some(&admin_key), None).await;

        assert_eq!(status, 200);
        assert!(body.get("enabled").is_some());
        assert!(body.get("resolver").is_some());
        let rendered = body.to_string();
        assert!(
            !rendered.contains("hcd_live_"),
            "auth config response must never contain key material: {rendered}"
        );
    }

    #[tokio::test]
    async fn get_auth_config_requires_auth_manage_capability() {
        let (router, admin_key) = app_with_bootstrap_admin().await;

        let (_, low) = request(
            &router,
            "POST",
            "/v1/auth/keys",
            Some(&admin_key),
            Some(r#"{"owner":"low-priv","capabilities":["documents:process"]}"#),
        )
        .await;
        let low_priv_key = low["raw_key"].as_str().unwrap().to_string();

        let (status, _) =
            request(&router, "GET", "/v1/auth/config", Some(&low_priv_key), None).await;

        assert_eq!(status, 403);
    }

    #[tokio::test]
    async fn get_auth_config_without_a_token_is_unauthenticated() {
        let (router, _admin_key) = app_with_bootstrap_admin().await;

        let (status, _) = request(&router, "GET", "/v1/auth/config", None, None).await;

        assert_eq!(status, 401);
    }

    /// Documents the known accuracy gap described on `get_auth_config`: `resolver`
    /// reports the *declared* `HaciendaConfig::auth.resolver` (`Dev`, the default),
    /// even though every other test in this module proves the live router is in fact
    /// authenticating through a store-backed `ApiKeyTokenResolver`, not `DevTokenResolver`.
    #[tokio::test]
    async fn auth_config_resolver_reflects_declared_config_not_the_live_resolver() {
        let (router, admin_key) = app_with_bootstrap_admin().await;

        let (status, body) =
            request(&router, "GET", "/v1/auth/config", Some(&admin_key), None).await;

        assert_eq!(status, 200);
        assert_eq!(
            body["resolver"], "dev",
            "reflects HaciendaConfig::default().auth.resolver, not the live \
             ApiKeyTokenResolver this test's router actually authenticates through"
        );
    }
}
