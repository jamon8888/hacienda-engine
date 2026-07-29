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
    routing::{get, post},
    Router,
};
use hacienda_core::{
    auth::authz::AuthRouterBuilder,
    auth::{authz::auth_middleware, AuthState, Capability, CapabilitySet},
};

use crate::{
    handlers::{documents, info, jobs, openapi, pii},
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
        path: "/v1/jobs/{id}",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(jobs::get_job),
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
        path: "/v1/pii/config",
        access: Access::Capability(Capability::DocumentsProcess),
        make_router: || get(pii::pii_config),
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

/// Build the axum `Router` from the route table, applying body limits and auth middleware.
pub fn build_router(state: ApiState) -> Router {
    // Build the AuthState from the table — same source, so they cannot drift.
    let auth = build_auth_state(state.auth.clone());

    let mut router = Router::new();
    for spec in ROUTE_TABLE {
        router = router.route(spec.path, (spec.make_router)());
    }

    router
        .layer(DefaultBodyLimit::max(state.limits.max_body_bytes))
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

    /// Turn an axum route pattern into a concrete, requestable path by replacing every
    /// `{param}` segment with a placeholder.
    fn substitute_path_parameters(pattern: &str) -> String {
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
}
