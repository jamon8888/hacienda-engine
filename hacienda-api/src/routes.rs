//! The single route table: path, access decision, and handler in one place.
//!
//! # Invariant
//!
//! Both the axum [`Router`] and the [`AuthState`] are derived from [`ROUTE_TABLE`].
//! It is structurally impossible to add a route without also deciding who may call it,
//! because the same array entry that registers the handler also sets the access policy.
//!
//! A reviewer can audit the entire API surface in one screen. The test
//! `every_guarded_route_reflected_in_auth_state` asserts that every non-public entry in
//! the table is reflected in the built `AuthState`, so a bug in `build_router` cannot
//! silently omit a capability requirement.

use axum::{
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, post},
    Router,
};
use hacienda_core::{
    auth::authz::AuthRouterBuilder,
    auth::{authz::auth_middleware, AuthState, Capability, CapabilitySet},
};

use crate::{
    handlers::{
        audit, audit_review, auth, documents, info, jobs, openapi, pii, presets, rag, rag_stream,
        uploads, usage, versions,
    },
    quota::quota_middleware,
    state::ApiState,
};

/// How a route is accessed.
#[derive(Debug, Clone)]
pub enum Access {
    /// Reachable without a token.
    Public,
    /// Requires the caller to hold this capability (or more).
    Capability(Capability),
}

/// One row of the route table: path, access decision, and method router.
pub struct RouteSpec {
    pub path: &'static str,
    pub access: Access,
    /// Axum method router. Stored as a closure so `ApiState` need not be `Clone + 'static`
    /// at table construction time. We use a type-erased function pointer.
    pub(crate) make_router: fn() -> axum::routing::MethodRouter<ApiState>,
}

/// The single source of truth for the entire API surface.
///
/// Every Phase 4 route appears here exactly once. Audit and review endpoints are
/// Phase 5 and must not be added to this table until then.
///
/// Column order: path, access, handler factory.
pub static ROUTE_TABLE: &[RouteSpec] = &[
    // ── Public endpoints (no token required) ──────────────────────────────────
    RouteSpec {
        path: "/health",
        access: Access::Public,
        make_router: || get(info::health),
    },
    RouteSpec {
        path: "/version",
        access: Access::Public,
        make_router: || get(info::version),
    },
    RouteSpec {
        path: "/info",
        access: Access::Public,
        make_router: || get(info::info),
    },
    RouteSpec {
        path: "/openapi.json",
        access: Access::Public,
        make_router: || get(openapi::openapi),
    },
    // ── documents:process endpoints ───────────────────────────────────────────
    RouteSpec {
        path: "/v1/documents",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || post(documents::process_documents),
    },
    RouteSpec {
        path: "/v1/documents/async",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || post(documents::process_documents_async),
    },
    RouteSpec {
        path: "/v1/jobs",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(jobs::list_jobs),
    },
    RouteSpec {
        path: "/v1/jobs/{id}",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(jobs::get_job),
    },
    RouteSpec {
        path: "/v1/jobs/{id}/result",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(jobs::get_job_result),
    },
    RouteSpec {
        path: "/v1/pii/scan",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || post(pii::scan_text),
    },
    RouteSpec {
        path: "/v1/pii/redact",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || post(pii::redact_text),
    },
    RouteSpec {
        path: "/v1/pii/reveal",
        access: Access::Capability(Capability::PiiReveal),
        make_router: || post(pii::reveal_token),
    },
    RouteSpec {
        path: "/v1/pii/config",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(pii::pii_config),
    },
    // ── audit:read endpoints (Phase 10) ─────────────────────────────────────────
    // `/v1/audit` is the coarse Phase 10 shape (`AuditResponse`, open segment only) —
    // kept as-is because `sdks/typescript/src/client.ts`'s hand-written
    // `audit.getAudit()` is typed against it, and `/v1/audit/entries` below covers the
    // same ground with cursor pagination for anyone who wants the fuller surface.
    // `/v1/audit/verify` now uses the richer `handlers::audit::audit_verify`
    // (broken-chain-as-200, names the offending entry/seal) instead of Phase 10's
    // `audit_review::verify_audit` (valid: bool only) — restoring the behaviour
    // documented in `hacienda-api/README.md` and covered by `handlers::audit`'s own
    // test suite. `sdks/typescript/src/client.ts`'s `audit.verifyAudit()` return type
    // was updated to match (`Schemas["VerifyResponse"]`); `audit_review::verify_audit`
    // itself is kept, `#[allow(dead_code)]`, as a smaller reference implementation.
    RouteSpec {
        path: "/v1/audit",
        access: Access::Capability(Capability::AuditRead),
        make_router: || get(audit_review::get_audit),
    },
    RouteSpec {
        path: "/v1/audit/verify",
        access: Access::Capability(Capability::AuditRead),
        make_router: || get(audit::audit_verify),
    },
    RouteSpec {
        path: "/v1/audit/entries",
        access: Access::Capability(Capability::AuditRead),
        make_router: || get(audit::audit_entries),
    },
    RouteSpec {
        path: "/v1/audit/seals",
        access: Access::Capability(Capability::AuditRead),
        make_router: || get(audit::audit_seals),
    },
    RouteSpec {
        path: "/v1/audit/export",
        access: Access::Capability(Capability::AuditExport),
        make_router: || get(audit::audit_export),
    },
    RouteSpec {
        path: "/v1/audit/tip",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(audit::audit_tip),
    },
    RouteSpec {
        path: "/v1/review",
        access: Access::Capability(Capability::AuditRead),
        make_router: || get(audit_review::get_review),
    },
    RouteSpec {
        path: "/v1/review/{id}/decide",
        access: Access::Capability(Capability::ReviewDecide),
        make_router: || post(audit_review::decide_review),
    },
    RouteSpec {
        path: "/v1/compliance/dpia",
        access: Access::Capability(Capability::AuditRead),
        make_router: || get(audit_review::get_compliance_dpia),
    },
    RouteSpec {
        path: "/v1/compliance/report",
        access: Access::Capability(Capability::AuditRead),
        make_router: || get(audit_review::get_compliance_report),
    },
    RouteSpec {
        path: "/v1/glossary",
        access: Access::Capability(Capability::AuditRead),
        make_router: || get(audit_review::get_glossary),
    },
    // ── auth:manage endpoints (Phase 11 Task 3) ─────────────────────────────────
    RouteSpec {
        path: "/v1/auth/keys",
        access: Access::Capability(Capability::AuthManage),
        make_router: || post(auth::issue_key),
    },
    RouteSpec {
        path: "/v1/auth/keys/{id}",
        access: Access::Capability(Capability::AuthManage),
        make_router: || delete(auth::revoke_key),
    },
    RouteSpec {
        path: "/v1/auth/config",
        access: Access::Capability(Capability::AuthManage),
        make_router: || get(auth::get_auth_config),
    },
    // Self-describing capability probe for SDK clients — see `handlers::auth::whoami`'s
    // doc comment for why this exists separately from `/v1/auth/config` (that route
    // needs `auth:manage`, which a normal SDK caller does not hold).
    RouteSpec {
        path: "/v1/auth/whoami",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(auth::whoami),
    },
    // ── documents:process endpoints, RAG (Phase 12 Task 3) ───────────────────────
    // Reuses `DocumentsProcess`, not a dedicated capability: RAG collections carry
    // the same class of redacted document content `/v1/documents` already gates
    // under it — see Task 3 Step 2's verification note in the plan file. Every
    // method `RagStore` exposes now has a route, including the async
    // migrate-embeddings job pair added in Task 3 Step 3.
    RouteSpec {
        path: "/v1/rag/collections",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(rag::list_collections).post(rag::create_collection),
    },
    RouteSpec {
        path: "/v1/rag/collections/{name}",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(rag::get_collection).delete(rag::delete_collection),
    },
    RouteSpec {
        path: "/v1/rag/collections/{name}/documents",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(rag::list_documents).post(rag::upsert_document),
    },
    // Hacienda-only addition closing the xberg-sdks `reindex_rag_document` gap
    // (platform-parity design spec §3.1/§4.1) — see `handlers::rag::reindex_document`'s
    // doc comment. Same `DocumentsProcess` capability as every other RAG route.
    RouteSpec {
        path: "/v1/rag/collections/{name}/documents/{id}/reindex",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || post(rag::reindex_document),
    },
    RouteSpec {
        path: "/v1/rag/collections/{name}/retrieve",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || post(rag::retrieve),
    },
    RouteSpec {
        path: "/v1/rag/collections/{name}/migrate-embeddings",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || post(rag::migrate_embeddings),
    },
    RouteSpec {
        path: "/v1/rag/collections/{name}/migrate-embeddings/{job_id}",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(rag::get_migrate_status),
    },
    // Streaming answer synthesis (Phase 12 Track 3) — see
    // `handlers/rag_stream.rs`'s module doc for the mandatory PII redaction gate
    // this route applies before any retrieved content reaches an LLM.
    RouteSpec {
        path: "/v1/rag/collections/{name}/answer",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || post(rag_stream::answer),
    },
    // ── documents:process endpoints, presets (Phase 13 Task 2) ──────────────────
    // Presets are inert config (a saved pipeline configuration), not part of the
    // audit-bearing pipeline itself — gated under the same DocumentsProcess
    // capability as every other config/inert-data route, matching RAG's rationale
    // above. `PresetStore` has no in-memory backend (Postgres-only), so these
    // routes 400 when `ApiState::preset_store` is `None`.
    RouteSpec {
        path: "/v1/presets",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(presets::list_presets).post(presets::create_preset),
    },
    RouteSpec {
        path: "/v1/presets/{id}",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(presets::get_preset).delete(presets::delete_preset),
    },
    // ── documents:process endpoints, document versions/diff (Phase 13 Task 3) ──
    // `DocumentVersionStore` has no in-memory backend (Postgres-only), same opt-in
    // shape as presets/RAG — these routes 400 when `ApiState::version_store` is
    // `None`. Gated under the same `DocumentsProcess` capability as `/v1/documents`
    // itself, since a version's content is exactly that route's redacted output.
    RouteSpec {
        path: "/v1/documents/{id}/versions",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(versions::list_document_versions),
    },
    RouteSpec {
        path: "/v1/documents/{id}",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(versions::get_document),
    },
    RouteSpec {
        path: "/v1/documents/{id}/diff",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(versions::diff_document),
    },
    RouteSpec {
        path: "/v1/documents/{id}/diff/{diff_job_id}",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(versions::get_diff_job),
    },
    // ── documents:process endpoints, presigned uploads (Phase 13 Task 4) ────────
    // A presigned upload is a precursor step to `/v1/documents` — the bytes never
    // transit this server — so it is gated under the same `DocumentsProcess`
    // capability as the route it feeds, matching presets/RAG/versions above.
    // `ObjectStore` has no in-memory backend (S3-compatible only), same opt-in
    // shape as `PresetStore`/`DocumentVersionStore`: these routes 400 when
    // `ApiState::object_store` is `None`.
    RouteSpec {
        path: "/v1/uploads/presign",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || post(uploads::presign_upload),
    },
    RouteSpec {
        path: "/v1/uploads/confirm",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || post(uploads::confirm_upload),
    },
    // ── audit:read endpoints, usage/metering (Phase 13 Task 5) ──────────────────
    // A read-model over the audit chain (Decision 3, platform-parity spec §10) —
    // gated under the same `AuditRead` capability as `/v1/audit`, `/v1/audit/verify`,
    // and `/v1/compliance/*` above, since usage is derived from that same data, not
    // a separate source of truth. `UsageStore` has no in-memory backend
    // (Postgres-only), same opt-in shape as presets/versions/uploads: this route
    // 400s when `ApiState::usage_store` is `None`.
    RouteSpec {
        path: "/v1/usage",
        access: Access::Capability(Capability::AuditRead),
        make_router: || get(usage::get_usage),
    },
];

/// Build the `AuthState` from the route table.
///
/// Public routes are registered via `with_public_route`; guarded routes are registered
/// via `with_route_requirement`. The result is a deny-by-default state: any path not
/// in this table is refused with 403.
pub fn build_auth_state(base: AuthState) -> AuthState {
    let mut builder = AuthRouterBuilder::new(base);
    for spec in ROUTE_TABLE {
        match &spec.access {
            Access::Public => {
                builder = builder.public(spec.path);
            }
            Access::Capability(cap) => {
                builder = builder.require(spec.path, CapabilitySet::new([*cap]));
            }
        }
    }
    builder.build()
}

/// Build the axum `Router` from the route table, applying body limits, quota, and auth
/// middleware.
///
/// # Layer ordering
///
/// `.layer()` calls wrap outward-in: the *last* one added runs *first* on an incoming
/// request. So `auth_middleware` (added last) runs first and attaches the caller's
/// `AuthExtension`; `quota_middleware` (added second-to-last) runs next and reads that
/// extension to enforce the S1 §7 per-tenant request-rate quota; `DefaultBodyLimit` runs
/// after that, and the handler's own body-decoding extractors run last of all. A quota
/// rejection therefore always happens before the body is read — the S1 spec's
/// `quota_exceeded_returns_429_before_body_decode` exit criterion — because
/// `quota_middleware` returns its own `429` response directly instead of calling
/// `next.run(request)`.
pub fn build_router(state: ApiState) -> Router {
    // Build the AuthState from the table — same source, so they cannot drift.
    let auth = build_auth_state(state.auth.clone());
    let quota = state.quota_store.clone();

    let mut router = Router::new();
    for spec in ROUTE_TABLE {
        router = router.route(spec.path, (spec.make_router)());
    }

    router
        .layer(DefaultBodyLimit::max(state.limits.max_body_bytes))
        .layer(middleware::from_fn_with_state(quota, quota_middleware))
        .layer(middleware::from_fn_with_state(auth, auth_middleware))
        .with_state(state)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::state::{ApiLimits, ApiState};
    use axum::{body::Body, http::Request};
    use hacienda_core::{
        auth::{authn::DevTokenResolver, AuthState},
        jobs::InMemoryJobStore,
        pii::PipelineConfig,
        redaction::{EnvKeyResolver, RedactionConfig, RedactionMode},
        HaciendaConfig, HaciendaFacade,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    /// A state with auth **enabled** and the dev resolver, so a `hcd_<caps>_<rand>`
    /// bearer token grants exactly the capabilities named in it.
    fn test_state() -> ApiState {
        state_with(
            AuthState::new(Arc::new(DevTokenResolver)),
            ApiLimits::default(),
        )
    }

    /// A state with auth **disabled**, for exercising extractor and handler behaviour
    /// without a token. Every request is `Caller::Trusted`.
    pub(crate) fn test_state_no_auth() -> ApiState {
        state_with(
            AuthState::new(Arc::new(DevTokenResolver)).with_enabled(false),
            ApiLimits::default(),
        )
    }

    fn state_with(auth: AuthState, limits: ApiLimits) -> ApiState {
        let facade = Arc::new(HaciendaFacade::new(HaciendaConfig::default()).unwrap());
        let jobs = InMemoryJobStore::new().into_arc();
        ApiState::new(facade, jobs, auth, limits)
    }

    /// Creates a test state with PII enabled in pseudonymize mode and a key resolver.
    fn state_with_pii() -> ApiState {
        let resolver = EnvKeyResolver::with_lookup(|name| match name {
            "HACIENDA_PSEUDONYM_ACTIVE_KEY" => Some("k1".to_string()),
            "HACIENDA_PSEUDONYM_KEY_K1" => Some("07".repeat(64)),
            _ => None,
        });

        let pii_config = PipelineConfig {
            redaction: RedactionConfig {
                mode: RedactionMode::Pseudonymize,
                ..Default::default()
            },
            ..Default::default()
        };

        let config = HaciendaConfig::default().with_pii(pii_config);
        let facade = Arc::new(HaciendaFacade::with_key_resolver(config, &resolver, &[]).unwrap());

        let auth = AuthState::new(Arc::new(DevTokenResolver));
        let jobs = InMemoryJobStore::new().into_arc();
        ApiState::new(facade, jobs, auth, ApiLimits::default())
    }

    /// A state with auth disabled and an in-memory RAG store attached.
    fn state_with_rag() -> ApiState {
        use hacienda_rag::{InMemoryVectorStore, RagStore};
        let store: Arc<dyn RagStore> = Arc::new(InMemoryVectorStore::new("test"));
        test_state_no_auth().with_rag_store(store)
    }

    /// A state with auth disabled, PII detection enabled (mask mode — no key
    /// resolver needed), and an in-memory RAG store attached. Used by the
    /// streaming-answer tests: `handlers::rag_stream::answer` calls
    /// `redact_text_with_auth` unconditionally, which fails closed with
    /// `PiiDisabled` against the plain `state_with_rag()` facade.
    fn state_with_rag_and_pii() -> ApiState {
        use hacienda_rag::{InMemoryVectorStore, RagStore};

        let pii_config = PipelineConfig {
            redaction: RedactionConfig {
                mode: RedactionMode::Mask,
                ..Default::default()
            },
            ..Default::default()
        };
        let config = HaciendaConfig::default().with_pii(pii_config);
        let facade = Arc::new(HaciendaFacade::new(config).unwrap());
        let jobs = InMemoryJobStore::new().into_arc();
        let auth = AuthState::new(Arc::new(DevTokenResolver)).with_enabled(false);
        let store: Arc<dyn RagStore> = Arc::new(InMemoryVectorStore::new("test"));
        ApiState::new(facade, jobs, auth, ApiLimits::default()).with_rag_store(store)
    }

    /// Turn an axum route pattern into a concrete, requestable path by replacing every
    /// `{param}` segment with a placeholder.
    pub(crate) fn substitute_path_parameters(pattern: &str) -> String {
        pattern
            .split('/')
            .map(|segment| {
                if segment.starts_with('{') {
                    "placeholder"
                } else {
                    segment
                }
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Every non-public entry in the route table must be reflected in the built `AuthState`.
    ///
    /// We verify this by driving requests through the full router:
    ///
    /// - A public route is reachable without a token (200 or 405).
    /// - A guarded route is refused without a token (401).
    ///
    /// This is an end-to-end test: it catches both a missing capability registration
    /// in `build_auth_state` AND a broken middleware, so a regression in either fails
    /// this test rather than only the dedicated middleware test.
    #[tokio::test]
    async fn every_guarded_route_reflected_in_auth_state() {
        let state = test_state();
        let app = build_router(state);

        // Gather public and guarded paths, de-duplicated.
        let public_paths: Vec<&str> = ROUTE_TABLE
            .iter()
            .filter(|s| matches!(s.access, Access::Public))
            .map(|s| s.path)
            .collect();

        // Parameterised paths are *not* skipped. Substituting a placeholder segment is
        // enough to drive them, and the skip that used to be here is precisely where a
        // bug hid: `/v1/jobs/{id}` was registered with a capability the middleware's
        // matcher could not resolve, so the route answered 403 to every caller —
        // including its owner — and no test noticed.
        let guarded_paths: Vec<String> = ROUTE_TABLE
            .iter()
            .filter(|s| matches!(s.access, Access::Capability(_)))
            .map(|s| substitute_path_parameters(s.path))
            .collect();

        // Public routes must succeed without a token (GET /health → 200; POST routes with
        // no body → 422 or 405, which are past the auth gate).
        for path in &public_paths {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(*path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_ne!(
                resp.status().as_u16(),
                401,
                "public route {path} must not require auth, got {}",
                resp.status()
            );
            assert_ne!(
                resp.status().as_u16(),
                403,
                "public route {path} must not be forbidden, got {}",
                resp.status()
            );
        }

        // Guarded routes must refuse without a token (401).
        for path in &guarded_paths {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(path.as_str())
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status().as_u16(),
                401,
                "guarded route {path} must return 401 without a token, got {}",
                resp.status()
            );
        }
    }

    /// A guarded handler must observe the `AuthExtension` — not silently run as
    /// `Caller::Trusted`.
    ///
    /// The previous version of this test sent *no* token and asserted the request was
    /// refused. That only proves the middleware runs; it says nothing about what the
    /// handler does afterwards, because the request never reached one.
    ///
    /// This version sends a **valid** token that satisfies the route's declared
    /// requirement (`documents:process`) but lacks `pii:reveal`, and asks for
    /// `include_text: true` — a second capability that only the facade enforces, using
    /// the `Caller` the handler derived. A handler that lost the extension would be
    /// `Caller::Trusted`, sail past `require(PiiReveal)`, and fail later on the absent
    /// pipeline with 400. So 403 is the only status that proves the caller was carried
    /// through.
    #[tokio::test]
    async fn guarded_handler_observes_the_caller_not_trusted() {
        let app = build_router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pii/scan")
                    .header("authorization", "Bearer hcd_documents:process_testsuffix")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"hello","include_text":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status().as_u16(),
            403,
            "include_text without pii:reveal must be 403; a 400 means the handler ran as \
             Caller::Trusted and reached the pipeline check instead"
        );
    }

    /// The same token, without `include_text`, must get past authorisation.
    ///
    /// Paired with the test above so that a blanket "always 403" regression — which would
    /// satisfy that assertion trivially — fails here.
    #[tokio::test]
    async fn documents_process_alone_is_authorised_for_a_scan_without_text() {
        let app = build_router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pii/scan")
                    .header("authorization", "Bearer hcd_documents:process_testsuffix")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(
            response.status().as_u16(),
            403,
            "documents:process alone must authorise a scan with include_text absent"
        );
    }

    /// `process_documents` used to zip `result.extraction.results` with `result.pii`
    /// three ways with `body.documents` — `Iterator::zip` truncates to the shortest
    /// input, and `result.pii` is empty whenever no PII pipeline is configured, so the
    /// whole zip silently produced zero documents regardless of what was submitted.
    /// Pins the fix: extraction still runs and is returned even with PII disabled,
    /// each document simply carries no entities.
    #[tokio::test]
    async fn process_documents_returns_extraction_when_pii_is_not_configured() {
        use data_encoding::BASE64;

        let app = build_router(test_state_no_auth());

        let body_content = BASE64.encode(b"hello world");
        let request_body = format!(
            r#"{{"documents":[{{"filename":"a.txt","mime_type":"text/plain","content_base64":"{body_content}"}}]}}"#
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/documents")
                    .header("content-type", "application/json")
                    .body(Body::from(request_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 200);

        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let documents = json["documents"]
            .as_array()
            .expect("documents must be an array");

        assert_eq!(
            documents.len(),
            1,
            "one submitted document must yield one result even with PII disabled, got {json}"
        );
        assert_eq!(
            documents[0]["content"],
            serde_json::json!("hello world"),
            "the extracted content must be the submitted content, not an empty stand-in"
        );
        assert_eq!(
            documents[0]["entities"],
            serde_json::json!([]),
            "no PII pipeline configured means no entities, not a missing document"
        );
    }

    /// `GET /v1/review` and `POST /v1/review/{id}/decide` are declared under two
    /// different capabilities in the route table (`audit:read` and `review:decide`
    /// respectively) — reading the queue and deciding on an item are different
    /// privileges. Before `HaciendaFacade::review_queue_read_with_auth` existed,
    /// `get_review` called the same `review_queue_with_auth` as `decide_review`, which
    /// unconditionally required `review:decide` — so a caller with only `audit:read`
    /// passed the route-level guard and was then rejected by the facade. This test pins
    /// the fixed, distinct behaviour at the HTTP layer (Phase 10 Task 2 Step 3).
    #[tokio::test]
    async fn review_read_and_decide_require_distinct_capabilities() {
        let app = build_router(test_state());

        // audit:read alone must be authorised to list the queue.
        let list_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/review")
                    .header("authorization", "Bearer hcd_audit:read_testsuffix")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(
            list_response.status().as_u16(),
            403,
            "audit:read alone must authorise GET /v1/review"
        );

        // audit:read alone must NOT be authorised to decide on an item.
        let decide_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/review/some-id/decide")
                    .header("authorization", "Bearer hcd_audit:read_testsuffix")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"decision":"approve","reviewer":"r","comment":"c"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            decide_response.status().as_u16(),
            403,
            "audit:read alone must not authorise POST /v1/review/{{id}}/decide"
        );
    }

    /// An oversized body must yield 413 with the error envelope, not axum's default
    /// `text/plain` rejection.
    ///
    /// The previous version of this test asserted `200` on `/health` and deferred the
    /// real assertion to a test that was never written. It passed while proving nothing.
    #[tokio::test]
    async fn oversized_body_yields_413_with_envelope() {
        use crate::state::ApiLimits;

        let limits = ApiLimits {
            max_body_bytes: 32,
            max_documents: 64,
        };
        let app = build_router(state_with(
            AuthState::new(Arc::new(DevTokenResolver)).with_enabled(false),
            limits,
        ));

        let oversized = format!(r#"{{"text":"{}"}}"#, "x".repeat(4096));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pii/redact")
                    .header("content-type", "application/json")
                    .body(Body::from(oversized))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status().as_u16(),
            413,
            "a body over max_body_bytes must be refused with 413"
        );

        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(
            text.contains("\"code\":\"payload_too_large\""),
            "413 must use the error envelope, got: {text}"
        );
    }

    /// A body inside the limit must not be refused — otherwise the test above would pass
    /// with a limit of zero.
    #[tokio::test]
    async fn body_within_the_limit_is_not_refused() {
        let app = build_router(test_state_no_auth());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pii/redact")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(response.status().as_u16(), 413);
    }

    /// S1 §9: `quota_exceeded_returns_429_before_body_decode`.
    ///
    /// The request body is malformed JSON — if the handler's extractor ever saw it,
    /// the response would be `400` (the existing `extractor_rejections_use_the_json_envelope`
    /// behaviour in `error.rs`'s test module). Getting `429` instead proves
    /// `quota_middleware` rejected the request before `next.run` ever handed control to
    /// an extractor, exactly as the ordering comment on `build_router` documents.
    #[tokio::test]
    async fn quota_exceeded_returns_429_before_body_decode() {
        use crate::quota::{InMemoryQuotaStore, QuotaLimits};

        let quota = Arc::new(InMemoryQuotaStore::new(QuotaLimits {
            requests_per_minute: 1,
        }));
        let app = build_router(test_state_no_auth().with_quota_store(quota));

        let request = || {
            Request::builder()
                .method("POST")
                .uri("/v1/pii/redact")
                .header("content-type", "application/json")
                .body(Body::from("{not valid json"))
                .unwrap()
        };

        let first = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(
            first.status().as_u16(),
            400,
            "the first request is within quota, so the malformed body reaches the extractor"
        );

        let second = app.oneshot(request()).await.unwrap();
        assert_eq!(
            second.status().as_u16(),
            429,
            "the second request exceeds the one-per-minute quota"
        );
        assert!(
            second.headers().contains_key("retry-after"),
            "a 429 must carry Retry-After"
        );
        let body = axum::body::to_bytes(second.into_body(), 64 * 1024)
            .await
            .unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(
            text.contains("\"code\":\"rate_limited\""),
            "429 must use the error envelope, got: {text}"
        );
    }

    /// Reveal route requires `pii:reveal` capability, not just `documents:process`.
    ///
    /// Mirrors `guarded_handler_observes_the_caller_not_trusted` but for the reveal
    /// endpoint: a token with only `documents:process` must get 403.
    #[tokio::test]
    async fn reveal_route_requires_pii_reveal_not_just_documents_process() {
        let app = build_router(state_with_pii());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pii/reveal")
                    .header("authorization", "Bearer hcd_documents:process_testsuffix")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"token":"[EMAIL:k1:AAAA]"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status().as_u16(),
            403,
            "reveal without pii:reveal must be 403; got {}",
            response.status()
        );
    }

    /// Reveal route returns plaintext for a valid token with correct capabilities.
    #[tokio::test]
    async fn reveal_route_returns_plaintext_for_a_valid_token() {
        let app = build_router(state_with_pii());

        // First, redact something to get a valid token
        let redact_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pii/redact")
                    .header(
                        "authorization",
                        "Bearer hcd_documents:process,pii:reveal_testsuffix",
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"mail bob@example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(redact_response.status().as_u16(), 200);

        let redact_body = axum::body::to_bytes(redact_response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let redact_json: serde_json::Value = serde_json::from_slice(&redact_body).unwrap();
        let redacted_text = redact_json["redacted_text"].as_str().unwrap();

        // Extract the token from the redacted text
        let token_start = redacted_text.find('[').unwrap();
        let token_end = redacted_text.find(']').unwrap() + 1;
        let token = &redacted_text[token_start..token_end];

        // Now reveal it
        let reveal_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pii/reveal")
                    .header(
                        "authorization",
                        "Bearer hcd_documents:process,pii:reveal_testsuffix",
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"token":"{token}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(reveal_response.status().as_u16(), 200);

        let reveal_body = axum::body::to_bytes(reveal_response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let reveal_json: serde_json::Value = serde_json::from_slice(&reveal_body).unwrap();
        assert_eq!(
            reveal_json["plaintext"].as_str().unwrap(),
            "bob@example.com"
        );
    }

    /// Reveal route returns 400 for a malformed token.
    #[tokio::test]
    async fn reveal_route_returns_400_for_a_malformed_token() {
        let app = build_router(state_with_pii());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pii/reveal")
                    .header(
                        "authorization",
                        "Bearer hcd_documents:process,pii:reveal_testsuffix",
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"token":"not-a-token"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 400);
    }

    /// The route table's path set must be internally consistent (no duplicate paths).
    #[test]
    fn route_table_has_no_duplicate_paths() {
        let mut seen = std::collections::HashSet::new();
        for spec in ROUTE_TABLE {
            assert!(
                seen.insert(spec.path),
                "duplicate path in ROUTE_TABLE: {}",
                spec.path
            );
        }
    }

    /// Full lifecycle: create a collection, upsert a document, retrieve it back,
    /// then delete the collection. Exercises every RAG route end to end against the
    /// real router (not the store directly), so a routing/DTO wiring bug fails here.
    #[tokio::test]
    async fn rag_collection_document_retrieve_and_delete_round_trip() {
        let app = build_router(state_with_rag());

        let create = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"name":"c1","embedding_dim":3,"distance_metric":"cosine","index_method":"flat"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create.status().as_u16(), 201);

        let get = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/rag/collections/c1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status().as_u16(), 200);

        let upsert_body = r#"{
            "document": {"external_id": "doc-1", "full_text": "hello world"},
            "chunks": [
                {"ordinal": 0, "content": "hello world", "embedding": [1.0, 0.0, 0.0]}
            ]
        }"#;
        let upsert = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections/c1/documents")
                    .header("content-type", "application/json")
                    .body(Body::from(upsert_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upsert.status().as_u16(), 201);

        let retrieve_body =
            r#"{"mode":"vector","query_vector":[1.0,0.0,0.0],"top_k":5,"include_content":true}"#;
        let retrieve = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections/c1/retrieve")
                    .header("content-type", "application/json")
                    .body(Body::from(retrieve_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retrieve.status().as_u16(), 200);
        let retrieve_bytes = axum::body::to_bytes(retrieve.into_body(), 64 * 1024)
            .await
            .unwrap();
        let retrieve_json: serde_json::Value = serde_json::from_slice(&retrieve_bytes).unwrap();
        assert_eq!(
            retrieve_json["chunks"][0]["content"].as_str().unwrap(),
            "hello world"
        );

        let delete = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/v1/rag/collections/c1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete.status().as_u16(), 204);

        let get_after_delete = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/rag/collections/c1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_after_delete.status().as_u16(), 404);
    }

    /// Upserting a document with `full_text` set and no `chunks` field must succeed by
    /// chunking server-side (`hacienda_rag::chunk_full_text`), not by rejecting the
    /// request or silently storing zero chunks. Uses an `embedding_dim: 0` collection
    /// (full-text/keyword only) because the server-side chunker leaves `embedding`
    /// empty — a caller wanting embedded chunks still submits `chunks` itself or
    /// re-embeds afterward, per `handlers::rag::upsert_document`'s doc comment.
    ///
    /// Asserts on the stored chunks directly via `get_document_chunks` (bypassing HTTP,
    /// which has no chunk-returning read path) rather than only checking the document
    /// exists — a store that silently kept zero chunks would still pass a
    /// document-exists-only check.
    #[tokio::test]
    async fn upsert_document_without_chunks_is_chunked_server_side() {
        use hacienda_rag::{InMemoryVectorStore, RagStore};
        let store = Arc::new(InMemoryVectorStore::new("test"));
        let rag_store: Arc<dyn RagStore> = store.clone();
        let app = build_router(test_state_no_auth().with_rag_store(rag_store));

        assert_eq!(create_collection(&app, "c1", 0).await, 201);

        let long_text = "Sentence one. ".repeat(50);
        let upsert_body = serde_json::json!({
            "document": {"external_id": "doc-1", "full_text": long_text},
        });
        let upsert = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections/c1/documents")
                    .header("content-type", "application/json")
                    .body(Body::from(upsert_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            upsert.status().as_u16(),
            201,
            "upsert without chunks must succeed by chunking full_text server-side"
        );
        let upsert = json_body(upsert).await;
        let document_id: hacienda_rag::DocumentId =
            serde_json::from_value(upsert["document_id"].clone()).expect("document_id");

        // The API scopes collection names by tenant (S1) before they reach `RagStore` —
        // an unauthenticated caller resolves to the default tenant, so the store holds
        // this collection under "default:c1", not the caller-facing "c1". See
        // `handlers::rag::scope_collection_name`.
        let scoped_name = crate::handlers::rag::scope_collection_name(
            &hacienda_core::tenancy::TenantCtx::default_tenant(
                hacienda_core::tenancy::ActorId::new("test"),
            ),
            "c1",
        );
        let (_document, chunks) = store
            .get_document_chunks(&scoped_name, &document_id)
            .await
            .expect("get_document_chunks")
            .expect("document must exist");
        assert!(
            !chunks.is_empty(),
            "full_text must have been split into at least one chunk"
        );
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.ordinal, i as u32, "chunks must be stored in order");
            assert!(!chunk.content.is_empty());
        }
    }

    /// Reindexing a document with `external_id` set re-chunks its stored
    /// `full_text` and re-upserts under the same identity, not a duplicate.
    #[tokio::test]
    async fn rag_reindex_document_rechunks_in_place() {
        let app = build_router(state_with_rag());
        assert_eq!(create_collection(&app, "c1", 0).await, 201);

        let upsert_body = serde_json::json!({
            "document": {"external_id": "doc-1", "full_text": "hello world"},
        });
        let upsert = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections/c1/documents")
                    .header("content-type", "application/json")
                    .body(Body::from(upsert_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upsert.status().as_u16(), 201);
        let upsert = json_body(upsert).await;
        let document_id: hacienda_rag::DocumentId =
            serde_json::from_value(upsert["document_id"].clone()).expect("document_id");

        let reindex = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/v1/rag/collections/c1/documents/{}/reindex",
                        document_id.0
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reindex.status().as_u16(), 200);
        let reindex = json_body(reindex).await;
        let reindexed_id: hacienda_rag::DocumentId =
            serde_json::from_value(reindex["document_id"].clone()).expect("document_id");
        assert_eq!(
            reindexed_id, document_id,
            "reindex must update the same document (matched by external_id), not create a new one"
        );
    }

    /// Reindexing a document with no `external_id` must be refused (400), not
    /// silently create a duplicate — see `handlers::rag::reindex_document`'s doc.
    #[tokio::test]
    async fn rag_reindex_document_without_external_id_is_400() {
        let app = build_router(state_with_rag());
        assert_eq!(create_collection(&app, "c1", 0).await, 201);

        let upsert_body = serde_json::json!({
            "document": {"full_text": "hello world"},
        });
        let upsert = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections/c1/documents")
                    .header("content-type", "application/json")
                    .body(Body::from(upsert_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upsert.status().as_u16(), 201);
        let upsert = json_body(upsert).await;
        let document_id: hacienda_rag::DocumentId =
            serde_json::from_value(upsert["document_id"].clone()).expect("document_id");

        let reindex = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/v1/rag/collections/c1/documents/{}/reindex",
                        document_id.0
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reindex.status().as_u16(), 400);
    }

    /// `POST .../reindex` on a document that does not exist must be 404.
    #[tokio::test]
    async fn rag_reindex_missing_document_is_404() {
        let app = build_router(state_with_rag());
        assert_eq!(create_collection(&app, "c1", 0).await, 201);

        let reindex = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections/c1/documents/missing/reindex")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reindex.status().as_u16(), 404);
    }

    /// `GET` on a collection that was never created must be 404, not 500 or 200
    /// with a null body.
    #[tokio::test]
    async fn rag_get_missing_collection_is_404() {
        let app = build_router(state_with_rag());

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/rag/collections/missing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 404);
    }

    /// End to end over the real router: create a collection, upsert a chunk, then
    /// `POST .../answer` with a model string no provider can route (mirrors
    /// `hacienda_rag::stream`'s own `answer_stream_emits_citations_before_llm_error`
    /// test, which already proves this fails fast with no live network call). The
    /// SSE body must still carry a `citation` event for the upserted chunk before
    /// the `error` event — proving retrieval, the redaction gate, and the SSE
    /// encoding all ran, even though the LLM call itself was never reachable.
    #[tokio::test]
    async fn rag_answer_streams_citation_then_error_without_a_routable_model() {
        let app = build_router(state_with_rag_and_pii());

        assert_eq!(create_collection(&app, "c1", 3).await, 201);

        let upsert_body = r#"{
            "document": {"external_id": "doc-1", "full_text": "hello world"},
            "chunks": [
                {"ordinal": 0, "content": "hello world", "embedding": [1.0, 0.0, 0.0]}
            ]
        }"#;
        let upsert = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections/c1/documents")
                    .header("content-type", "application/json")
                    .body(Body::from(upsert_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upsert.status().as_u16(), 201);

        let answer_body = r#"{
            "prompt": "what is this?",
            "retrieve": {"mode":"vector","query_vector":[1.0,0.0,0.0],"top_k":5},
            "llm": {"model": "not-a-real-provider/does-not-exist"}
        }"#;
        let answer = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections/c1/answer")
                    .header("content-type", "application/json")
                    .body(Body::from(answer_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(answer.status().as_u16(), 200);
        let content_type = answer
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(
            content_type.starts_with("text/event-stream"),
            "expected an SSE response, got content-type {content_type}"
        );

        let bytes = axum::body::to_bytes(answer.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        let citation_pos = body
            .find("event: citation")
            .expect("expected a citation event in the SSE body");
        let error_pos = body
            .find("event: error")
            .expect("expected an error event in the SSE body");
        assert!(
            citation_pos < error_pos,
            "citation event must precede the error event, got: {body}"
        );
        assert!(
            body.contains("doc-1") || body.contains("\"document_id\""),
            "citation event should carry the chunk's document id, got: {body}"
        );
        // Never let the internal liter-llm error message reach the wire.
        assert!(
            !body.contains("not-a-real-provider"),
            "error event must not echo the internal LLM error, got: {body}"
        );
    }

    /// `POST .../answer` on a collection that was never created must be 404.
    #[tokio::test]
    async fn rag_answer_missing_collection_is_404() {
        let app = build_router(state_with_rag());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections/missing/answer")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"prompt":"hi","retrieve":{"top_k":5},"llm":{"model":"x/y"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 404);
    }

    /// Without a configured RAG store, every `/v1/rag/*` route must fail cleanly
    /// (400), not panic or 500 — RAG is opt-in per `ApiState::rag_store`.
    #[tokio::test]
    async fn rag_routes_without_a_configured_store_yield_400() {
        let app = build_router(test_state_no_auth());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"c1","embedding_dim":3}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 400);
    }

    /// RAG routes require `documents:process`, same as every other content-bearing
    /// route — a token without it must be 403.
    #[tokio::test]
    async fn rag_routes_require_documents_process_capability() {
        // Auth is disabled in `state_with_rag`; build with auth enabled instead so
        // a missing capability is actually observable.
        let facade = std::sync::Arc::new(
            hacienda_core::HaciendaFacade::new(hacienda_core::HaciendaConfig::default()).unwrap(),
        );
        let jobs = InMemoryJobStore::new().into_arc();
        let auth = AuthState::new(Arc::new(DevTokenResolver));
        let store: Arc<dyn hacienda_rag::RagStore> =
            Arc::new(hacienda_rag::InMemoryVectorStore::new("test"));
        let state = ApiState::new(facade, jobs, auth, ApiLimits::default()).with_rag_store(store);
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections")
                    .header("authorization", "Bearer hcd_pii:reveal_testsuffix")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"c1","embedding_dim":3}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 403);
    }

    async fn create_collection(app: &axum::Router, name: &str, embedding_dim: u32) -> u16 {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"name":"{name}","embedding_dim":{embedding_dim}}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    async fn json_body(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// `GET /v1/rag/collections` must page over the backend (not in-process) and
    /// report `total` before `limit`/`offset` slicing, same contract as
    /// `GET /v1/jobs`.
    #[tokio::test]
    async fn rag_list_collections_paginates() {
        let app = build_router(state_with_rag());
        for name in ["c1", "c2", "c3"] {
            assert_eq!(create_collection(&app, name, 3).await, 201);
        }

        let first_page = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/rag/collections?limit=2&offset=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first_page.status().as_u16(), 200);
        let first_page = json_body(first_page).await;
        assert_eq!(first_page["total"], 3);
        assert_eq!(first_page["collections"].as_array().unwrap().len(), 2);

        let second_page = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/rag/collections?limit=2&offset=2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(second_page.status().as_u16(), 200);
        let second_page = json_body(second_page).await;
        assert_eq!(second_page["total"], 3);
        assert_eq!(second_page["collections"].as_array().unwrap().len(), 1);
    }

    /// `GET /v1/rag/collections/{name}/documents` must page like
    /// `list_collections` above, and 404 for a collection that does not exist —
    /// same not-found contract as `GET /v1/rag/collections/{name}` itself.
    #[tokio::test]
    async fn rag_list_documents_paginates_and_404s_on_unknown_collection() {
        let app = build_router(state_with_rag());
        assert_eq!(create_collection(&app, "c1", 3).await, 201);

        for i in 0..3 {
            let upsert_body = format!(
                r#"{{"document":{{"external_id":"doc-{i}","full_text":"text {i}"}},
                     "chunks":[{{"ordinal":0,"content":"text {i}","embedding":[1.0,0.0,0.0]}}]}}"#
            );
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/rag/collections/c1/documents")
                        .header("content-type", "application/json")
                        .body(Body::from(upsert_body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), 201);
        }

        let page = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/rag/collections/c1/documents?limit=2&offset=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(page.status().as_u16(), 200);
        let page = json_body(page).await;
        assert_eq!(page["total"], 3);
        assert_eq!(page["documents"].as_array().unwrap().len(), 2);

        let missing = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/rag/collections/missing/documents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status().as_u16(), 404);
    }

    /// `to_source` must name a preset whose `dimensions` matches the collection's
    /// `embedding_dim` — a mismatch is a client-supplied-body fault (400), not a
    /// job that gets created only to fail once it starts running. The "fast"
    /// preset is 384-dimensional; the collection here is 3-dimensional.
    #[tokio::test]
    async fn rag_migrate_embeddings_dimension_mismatch_is_400() {
        let app = build_router(state_with_rag());
        assert_eq!(create_collection(&app, "c1", 3).await, 201);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections/c1/migrate-embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"to_source":"fast","to_version":2}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 400);
    }

    /// `to_version` must be strictly greater than the collection's current
    /// `embedding_version` (which starts at 1) — requesting the same version is
    /// a 400, not a job that races another migration to the same version number.
    #[tokio::test]
    async fn rag_migrate_embeddings_non_increasing_version_is_400() {
        let app = build_router(state_with_rag());
        // "fast" is 384-dimensional — match it so the dimension check passes and
        // only the version check is under test.
        assert_eq!(create_collection(&app, "c1", 384).await, 201);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections/c1/migrate-embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"to_source":"fast","to_version":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 400);
    }

    /// Full migrate-embeddings lifecycle: create a collection, upsert a document,
    /// kick off a migration, poll until it succeeds, and confirm the collection's
    /// provenance was updated.
    ///
    /// Ignored: `xberg::embed_texts_async` requires downloading real ONNX model
    /// weights over the network and running them through ONNX Runtime, neither of
    /// which is available in this sandbox. The validation-only paths above
    /// (dimension mismatch, non-increasing version) exercise everything that
    /// runs before the background job touches the embedding backend, and run
    /// unconditionally.
    #[tokio::test]
    #[ignore = "requires network access to download ONNX model weights and a working ONNX Runtime install"]
    async fn rag_migrate_embeddings_full_round_trip() {
        let app = build_router(state_with_rag());
        assert_eq!(create_collection(&app, "c1", 384).await, 201);

        let upsert_body = r#"{
            "document": {"external_id": "doc-1", "full_text": "hello world"},
            "chunks": [{"ordinal": 0, "content": "hello world", "embedding": [0.0]}]
        }"#;
        // Deliberately wrong embedding length: this route only exercises the
        // migrate-embeddings job, not upsert's dimension validation, so any
        // upsert body accepted at collection creation time is enough — adjust
        // if `embedding_dim` validation ever tightens to reject this at upsert.
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections/c1/documents")
                    .header("content-type", "application/json")
                    .body(Body::from(upsert_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        let migrate = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/rag/collections/c1/migrate-embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"to_source":"fast","to_version":2}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(migrate.status().as_u16(), 202);
        let migrate = json_body(migrate).await;
        let job_id = migrate["job_id"].as_str().unwrap().to_string();

        let mut status = String::new();
        for _ in 0..50 {
            let poll = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(format!(
                            "/v1/rag/collections/c1/migrate-embeddings/{job_id}"
                        ))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = json_body(poll).await;
            status = body["status"].as_str().unwrap().to_string();
            if status == "succeeded" || status == "failed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        assert_eq!(status, "succeeded");

        let get = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/rag/collections/c1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let spec = json_body(get).await;
        assert_eq!(spec["embedding_source"], "fast");
        assert_eq!(spec["embedding_version"], 2);
    }

    /// Without a configured preset store, every `/v1/presets*` route must fail
    /// cleanly (400), not panic or 500 — presets are opt-in per
    /// `ApiState::preset_store`, same as RAG.
    #[tokio::test]
    async fn presets_routes_without_a_configured_store_yield_400() {
        let app = build_router(test_state_no_auth());

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/presets")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 400);
    }

    /// Preset routes require `documents:process`, same as every other
    /// content-bearing route — a token without it must be 403. Auth enforcement
    /// happens before the handler runs, so this does not need a live store.
    #[tokio::test]
    async fn presets_routes_require_documents_process_capability() {
        let facade = std::sync::Arc::new(
            hacienda_core::HaciendaFacade::new(hacienda_core::HaciendaConfig::default()).unwrap(),
        );
        let jobs = InMemoryJobStore::new().into_arc();
        let auth = AuthState::new(Arc::new(DevTokenResolver));
        let state = ApiState::new(facade, jobs, auth, ApiLimits::default());
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/presets")
                    .header("authorization", "Bearer hcd_pii:reveal_testsuffix")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 403);
    }

    /// Full lifecycle against a real `PostgresPresetStore`: create, list, get,
    /// delete, then confirm the deleted preset 404s. Requires a live Postgres —
    /// ignored by default, same convention as
    /// `hacienda-core`'s `store::postgres::presets` tests. Run with:
    ///   DATABASE_URL=postgres://... cargo test -p hacienda-api --lib \
    ///     routes::tests::preset_route_create_list_get_delete_round_trip -- --ignored
    #[tokio::test]
    #[ignore]
    async fn preset_route_create_list_get_delete_round_trip() {
        use hacienda_core::store::postgres::connection::{connect, migrate};
        use hacienda_core::store::postgres::presets::PostgresPresetStore;

        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = connect(&database_url).await.expect("connect failed");
        migrate(&pool).await.expect("migrate failed");
        let store: Arc<dyn hacienda_core::store::postgres::presets::PresetStore> =
            Arc::new(PostgresPresetStore::new(pool));

        let app = build_router(test_state_no_auth().with_preset_store(store));

        let name = format!("preset-{}", uuid::Uuid::new_v4());
        let create_body = format!(r#"{{"name":"{name}","config":{{"mode":"mask"}}}}"#);
        let create = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/presets")
                    .header("content-type", "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create.status().as_u16(), 201);
        let create_bytes = axum::body::to_bytes(create.into_body(), 64 * 1024)
            .await
            .unwrap();
        let created: serde_json::Value = serde_json::from_slice(&create_bytes).unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        assert_eq!(created["name"].as_str().unwrap(), name);

        let list = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/presets")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list.status().as_u16(), 200);
        let list_bytes = axum::body::to_bytes(list.into_body(), 64 * 1024)
            .await
            .unwrap();
        let list_json: serde_json::Value = serde_json::from_slice(&list_bytes).unwrap();
        assert!(list_json["presets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"].as_str().unwrap() == id));

        let get = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/presets/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status().as_u16(), 200);

        let delete = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/presets/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete.status().as_u16(), 204);

        let get_after_delete = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/presets/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_after_delete.status().as_u16(), 404);
    }

    /// Without a configured version store, every `/v1/documents/{id}/versions`,
    /// `/{id}`, and `/diff` route must fail cleanly (400), not panic or 500 —
    /// versioning is opt-in per `ApiState::version_store`, same as presets/RAG.
    ///
    /// `/v1/documents/{id}/diff/{diff_job_id}` is deliberately excluded here: it
    /// polls `JobStore` directly (mirroring `GET /v1/jobs/{id}`) and never touches
    /// `version_store`, so its behaviour with no store configured is "404, no such
    /// job" rather than "400, versioning disabled" — covered separately below.
    ///
    /// `/v1/documents/{id}` and `/v1/documents` are distinct route table entries
    /// (static `async` segment vs. `{id}` param — matchit disambiguates them), so
    /// this only exercises the version-specific paths, not the base upload route.
    #[tokio::test]
    async fn document_version_routes_without_a_configured_store_yield_400() {
        let app = build_router(test_state_no_auth());
        let id = uuid::Uuid::new_v4();

        for path in [
            format!("/v1/documents/{id}/versions"),
            format!("/v1/documents/{id}"),
            format!("/v1/documents/{id}/diff?from=1&to=2"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(path.as_str())
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status().as_u16(),
                400,
                "path {path} without a configured version store must 400, got {}",
                response.status()
            );
        }
    }

    /// The diff-job polling route does not depend on `version_store` — it looks up
    /// `JobStore` directly, same as `/v1/jobs/{id}`. With no store configured and no
    /// such job, the correct outcome is 404 (unknown job id), not 400.
    #[tokio::test]
    async fn document_version_diff_job_poll_without_a_store_is_404_not_400() {
        let app = build_router(test_state_no_auth());
        let id = uuid::Uuid::new_v4();

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/documents/{id}/diff/some-job-id"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 404);
    }

    /// Document version routes require `documents:process`, same as every other
    /// content-bearing route — a token without it must be 403. Auth enforcement
    /// happens before the handler runs, so this does not need a live store.
    #[tokio::test]
    async fn document_version_routes_require_documents_process_capability() {
        let facade = std::sync::Arc::new(
            hacienda_core::HaciendaFacade::new(hacienda_core::HaciendaConfig::default()).unwrap(),
        );
        let jobs = InMemoryJobStore::new().into_arc();
        let auth = AuthState::new(Arc::new(DevTokenResolver));
        let state = ApiState::new(facade, jobs, auth, ApiLimits::default());
        let app = build_router(state);
        let id = uuid::Uuid::new_v4();

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/documents/{id}/versions"))
                    .header("authorization", "Bearer hcd_pii:reveal_testsuffix")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 403);
    }

    /// Full lifecycle against a real `PostgresDocumentVersionStore`: submit two
    /// versions of the same `document_id` via `POST /v1/documents`, list them,
    /// fetch the latest full envelope, diff the two versions, and poll the diff
    /// job id even though the diff completed synchronously (still `200` with
    /// results, not a dead end). Requires a live Postgres — ignored by default,
    /// same convention as the preset lifecycle test above. Run with:
    ///   DATABASE_URL=postgres://... cargo test -p hacienda-api --lib \
    ///     routes::tests::document_version_create_list_get_diff_round_trip -- --ignored
    #[tokio::test]
    #[ignore]
    async fn document_version_create_list_get_diff_round_trip() {
        use data_encoding::BASE64;
        use hacienda_core::store::postgres::connection::{connect, migrate};
        use hacienda_core::store::postgres::versions::DocumentVersionStore;

        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = connect(&database_url).await.expect("connect failed");
        migrate(&pool).await.expect("migrate failed");
        let store: Arc<dyn DocumentVersionStore> = Arc::new(
            hacienda_core::store::postgres::versions::PostgresDocumentVersionStore::new(pool),
        );

        // PII enabled with the default `Mask` mode (needs no key resolver) — not required
        // for `process_documents` to return a populated response any more (see
        // `process_documents_returns_extraction_when_pii_is_not_configured`), but this
        // test is about the versioning path, and exercising it with entities present is
        // marginally more representative of a real deployment.
        let pii_config = hacienda_core::pii::PipelineConfig::default();
        let config = hacienda_core::HaciendaConfig::default().with_pii(pii_config);
        let facade = Arc::new(hacienda_core::HaciendaFacade::new(config).unwrap());
        let jobs = InMemoryJobStore::new().into_arc();
        let auth = AuthState::new(Arc::new(DevTokenResolver)).with_enabled(false);
        let state =
            ApiState::new(facade, jobs, auth, ApiLimits::default()).with_version_store(store);

        let app = build_router(state);
        let document_id = uuid::Uuid::new_v4();

        let submit = |text: &str| {
            let body_content = BASE64.encode(text.as_bytes());
            format!(
                r#"{{"documents":[{{"filename":"a.txt","mime_type":"text/plain","content_base64":"{body_content}","document_id":"{document_id}"}}]}}"#
            )
        };

        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/documents")
                    .header("content-type", "application/json")
                    .body(Body::from(submit("line one\nline two")))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first.status().as_u16(), 200);

        let second = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/documents")
                    .header("content-type", "application/json")
                    .body(Body::from(submit("line one\nline three")))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(second.status().as_u16(), 200);

        let list = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/documents/{document_id}/versions"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list.status().as_u16(), 200);
        let list_bytes = axum::body::to_bytes(list.into_body(), 64 * 1024)
            .await
            .unwrap();
        let list_json: serde_json::Value = serde_json::from_slice(&list_bytes).unwrap();
        let versions = list_json["versions"].as_array().unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0]["version_sequence"].as_i64().unwrap(), 2);
        assert_eq!(versions[1]["version_sequence"].as_i64().unwrap(), 1);

        let get = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/documents/{document_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status().as_u16(), 200);
        let get_bytes = axum::body::to_bytes(get.into_body(), 64 * 1024)
            .await
            .unwrap();
        let get_json: serde_json::Value = serde_json::from_slice(&get_bytes).unwrap();
        assert_eq!(get_json["version_sequence"].as_i64().unwrap(), 2);
        assert!(get_json["content"].as_str().unwrap().contains("line three"));

        let diff = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/documents/{document_id}/diff?from=1&to=2"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Small documents finish inside the 2-second budget: expect a synchronous 200.
        assert_eq!(diff.status().as_u16(), 200);
        let diff_bytes = axum::body::to_bytes(diff.into_body(), 64 * 1024)
            .await
            .unwrap();
        let diff_json: serde_json::Value = serde_json::from_slice(&diff_bytes).unwrap();
        let lines = diff_json["lines"].as_array().unwrap();
        assert!(lines
            .iter()
            .any(|l| l["op"] == "equal" && l["text"] == "line one"));
        assert!(lines
            .iter()
            .any(|l| l["op"] == "delete" && l["text"] == "line two"));
        assert!(lines
            .iter()
            .any(|l| l["op"] == "insert" && l["text"] == "line three"));
    }

    /// Diffing an out-of-range version sequence must be 404, not 500 or a panic.
    #[tokio::test]
    #[ignore]
    async fn document_version_diff_with_missing_version_is_404() {
        use hacienda_core::store::postgres::connection::{connect, migrate};
        use hacienda_core::store::postgres::versions::DocumentVersionStore;

        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = connect(&database_url).await.expect("connect failed");
        migrate(&pool).await.expect("migrate failed");
        let store: Arc<dyn DocumentVersionStore> = Arc::new(
            hacienda_core::store::postgres::versions::PostgresDocumentVersionStore::new(pool),
        );

        let app = build_router(test_state_no_auth().with_version_store(store));
        let document_id = uuid::Uuid::new_v4();

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/documents/{document_id}/diff?from=1&to=2"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 404);
    }

    /// Without a configured object store, both `/v1/uploads/*` routes must fail
    /// cleanly (400), not panic or 500 — uploads are opt-in per
    /// `ApiState::object_store`, same as presets/RAG/versions.
    #[tokio::test]
    async fn upload_routes_without_a_configured_store_yield_400() {
        let app = build_router(test_state_no_auth());

        let presign = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/uploads/presign")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"filename":"a.txt","mime_type":"text/plain"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(presign.status().as_u16(), 400);

        let confirm = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/uploads/confirm")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"upload_id":"{}"}}"#,
                        uuid::Uuid::new_v4()
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(confirm.status().as_u16(), 400);
    }

    /// Upload routes require `documents:process`, same as every other
    /// content-bearing route — a token without it must be 403. Auth enforcement
    /// happens before the handler runs, so this does not need a live store.
    #[tokio::test]
    async fn upload_routes_require_documents_process_capability() {
        let facade = std::sync::Arc::new(
            hacienda_core::HaciendaFacade::new(hacienda_core::HaciendaConfig::default()).unwrap(),
        );
        let jobs = InMemoryJobStore::new().into_arc();
        let auth = AuthState::new(Arc::new(DevTokenResolver));
        let state = ApiState::new(facade, jobs, auth, ApiLimits::default());
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/uploads/presign")
                    .header("authorization", "Bearer hcd_pii:reveal_testsuffix")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"filename":"a.txt","mime_type":"text/plain"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 403);
    }

    /// Without a configured usage store, `GET /v1/usage` must fail cleanly (400),
    /// not panic or 500 — usage is opt-in per `ApiState::usage_store`, same as
    /// presets/versions/uploads.
    #[tokio::test]
    async fn usage_route_without_a_configured_store_yields_400() {
        let app = build_router(test_state_no_auth());

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/usage")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 400);
    }

    /// The usage route requires `audit:read`, same as `/v1/audit` and
    /// `/v1/compliance/*` — a token without it must be 403. Auth enforcement
    /// happens before the handler runs, so this does not need a live store.
    #[tokio::test]
    async fn usage_route_requires_audit_read_capability() {
        let facade = std::sync::Arc::new(
            hacienda_core::HaciendaFacade::new(hacienda_core::HaciendaConfig::default()).unwrap(),
        );
        let jobs = InMemoryJobStore::new().into_arc();
        let auth = AuthState::new(Arc::new(DevTokenResolver));
        let state = ApiState::new(facade, jobs, auth, ApiLimits::default());
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/usage")
                    .header("authorization", "Bearer hcd_documents:process_testsuffix")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 403);
    }

    /// Full round trip against a real `PostgresUsageStore`: append audit entries for
    /// a principal via `PostgresAuditStore`, then confirm `GET /v1/usage` aggregates
    /// them into the expected entity/byte counts. Requires a live Postgres — ignored
    /// by default, same convention as the preset/version lifecycle tests above. Run
    /// with:
    ///   DATABASE_URL=postgres://... cargo test -p hacienda-api --lib \
    ///     routes::tests::usage_route_aggregates_audit_entries -- --ignored
    #[tokio::test]
    #[ignore]
    async fn usage_route_aggregates_audit_entries() {
        use hacienda_core::audit::{AuditEntryInput, AuditStore, EntitySource, RedactionAction};
        use hacienda_core::store::postgres::audit::PostgresAuditStore;
        use hacienda_core::store::postgres::connection::{connect, migrate};
        use hacienda_core::store::postgres::usage::{PostgresUsageStore, UsageStore};

        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = connect(&database_url).await.expect("connect failed");
        migrate(&pool).await.expect("migrate failed");

        let audit_store = PostgresAuditStore::new(pool.clone());
        let principal = format!("avocat-{}", uuid::Uuid::new_v4());
        audit_store
            .append(vec![AuditEntryInput {
                id: uuid::Uuid::new_v4().to_string(),
                category: "Email".to_string(),
                action: RedactionAction::Mask,
                span_hash: "hash".to_string(),
                span_length: 42,
                confidence: Some(1.0),
                source: EntitySource::Regex,
                pipeline_version: "1.0".to_string(),
                config_hash: "cfg".to_string(),
                principal: Some(principal.clone()),
                vertical: None,
            }])
            .await
            .expect("append failed");

        let store: Arc<dyn UsageStore> = Arc::new(PostgresUsageStore::new(pool));
        let app = build_router(test_state_no_auth().with_usage_store(store));

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/usage")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);

        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let record = json["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["principal"].as_str() == Some(principal.as_str()))
            .expect("principal missing from usage response");
        assert_eq!(record["entity_count"].as_i64().unwrap(), 1);
        assert_eq!(record["byte_count"].as_i64().unwrap(), 42);
    }

    /// Phase 10 Task 2 Step 5: proves the seven audit/review/compliance/glossary routes
    /// are backend-agnostic by re-running them against real `PostgresAuditStore` and
    /// `PostgresReviewStore` backends instead of the default in-memory stores. The
    /// handlers in `handlers/audit_review.rs` call only `HaciendaFacade` methods, never a
    /// store directly, so this is expected to pass without any handler changes — this
    /// test is what proves that expectation rather than assuming it.
    ///
    /// Requires a live Postgres — ignored by default, same convention as
    /// `usage_route_aggregates_audit_entries` above. Run with:
    ///   DATABASE_URL=postgres://... cargo test -p hacienda-api --lib \
    ///     routes::tests::audit_review_compliance_glossary_routes_work_against_postgres \
    ///     -- --ignored
    #[tokio::test]
    #[ignore]
    async fn audit_review_compliance_glossary_routes_work_against_postgres() {
        use hacienda_core::audit::{AuditEntryInput, AuditStore, EntitySource, RedactionAction};
        use hacienda_core::compliance::ComplianceConfig;
        use hacienda_core::glossary::GlossaryConfig;
        use hacienda_core::review::{
            Priority, ReviewConfig, ReviewQueueItem, ReviewStatus, ReviewStore,
        };
        use hacienda_core::store::postgres::audit::PostgresAuditStore;
        use hacienda_core::store::postgres::connection::{connect, migrate};
        use hacienda_core::store::postgres::review::PostgresReviewStore;

        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = connect(&database_url).await.expect("connect failed");
        migrate(&pool).await.expect("migrate failed");

        let audit_store = Arc::new(PostgresAuditStore::new(pool.clone()));
        let review_store = Arc::new(PostgresReviewStore::new(pool.clone()));

        // Seed one audit entry so GET /v1/audit has content to return. `entries()`
        // returns the whole open segment, which is shared with every other test that
        // appends to this database — the principal is unique per run so the seeded
        // entry can be found by identity rather than assuming an empty table.
        let principal = format!("avocat-postgres-route-test-{}", uuid::Uuid::new_v4());
        audit_store
            .append(vec![AuditEntryInput {
                id: uuid::Uuid::new_v4().to_string(),
                category: "Email".to_string(),
                action: RedactionAction::Mask,
                span_hash: "hash".to_string(),
                span_length: 42,
                confidence: Some(1.0),
                source: EntitySource::Regex,
                pipeline_version: "1.0".to_string(),
                config_hash: "cfg".to_string(),
                principal: Some(principal.clone()),
                vertical: None,
            }])
            .await
            .expect("append failed");

        // Seed one pending review item so GET /v1/review and POST /v1/review/{id}/decide
        // have something to act on.
        let review_id = uuid::Uuid::new_v4().to_string();
        review_store
            .submit(ReviewQueueItem {
                id: review_id.clone(),
                text_snippet: "jane@example.com".to_string(),
                category: "email".to_string(),
                start: 0,
                end: 16,
                confidence: 0.4,
                source: "regex".to_string(),
                status: ReviewStatus::Pending,
                priority: Priority::High,
                assigned_reviewer: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                deadline: None,
                decision: None,
                decided_by: None,
                decided_at: None,
                comment: None,
                tenant_id: "default".to_string(),
            })
            .await
            .expect("submit failed");

        let config = HaciendaConfig {
            pii: Some(PipelineConfig::default()),
            compliance: Some(ComplianceConfig::default()),
            review: Some(ReviewConfig::default()),
            glossary: Some(GlossaryConfig::default()),
            ..Default::default()
        };

        let facade = Arc::new(
            HaciendaFacade::with_stores(
                config,
                Some(audit_store.clone() as Arc<dyn AuditStore>),
                Some(review_store.clone() as Arc<dyn ReviewStore>),
                None,
            )
            .expect("facade construction failed"),
        );

        let jobs = InMemoryJobStore::new().into_arc();
        let auth = AuthState::new(Arc::new(DevTokenResolver)).with_enabled(false);
        let state = ApiState::new(facade, jobs, auth, ApiLimits::default());
        let app = build_router(state);

        // GET /v1/audit — the seeded entry comes back.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/audit")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let entries = json["entries"].as_array().unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e["principal"].as_str() == Some(principal.as_str())),
            "seeded principal missing from audit response"
        );
        assert!(json["audit_chain_tip"].is_string());

        // GET /v1/audit/verify — the chain built from the seeded entry verifies clean.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/audit/verify")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["verified"], serde_json::Value::Bool(true));

        // GET /v1/review — the seeded item comes back pending. `list(None)` returns the
        // whole table, shared with every other test that submits to this database, so
        // the seeded item is found by its (uuid, so unique) id rather than assuming an
        // otherwise-empty queue.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/review")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let items = json["items"].as_array().unwrap();
        let seeded_item = items
            .iter()
            .find(|i| i["id"].as_str() == Some(review_id.as_str()))
            .expect("seeded review item missing from queue response");
        assert_eq!(seeded_item["status"].as_str(), Some("pending"));

        // POST /v1/review/{id}/decide — approve the seeded item.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/review/{review_id}/decide"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"decision":"approve","reviewer":"r","comment":"looks fine"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["item"]["status"].as_str(), Some("approved"));
        assert_eq!(json["item"]["decision"].as_str(), Some("approve"));

        // GET /v1/compliance/dpia
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/compliance/dpia")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(json["report"]["processing_description"]
            .as_str()
            .unwrap()
            .contains("hacienda-pii"));

        // GET /v1/compliance/report
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/compliance/report")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            json["report"]["model_card"]["model_details"]["name"].as_str(),
            Some("hacienda-pii")
        );

        // GET /v1/glossary — no document has been processed, so the glossary is empty.
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/glossary")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["entries"].as_array().unwrap().len(), 0);
    }
}
