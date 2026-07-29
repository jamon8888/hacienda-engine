//! Authorization (authz) middleware for Axum.
//!
//! Extracts tokens from requests, resolves them via [`TokenResolver`], and enforces
//! capability requirements on routes.

use crate::auth::authn::{build_token_resolver, AuthConfig, TokenResolver};
use crate::auth::{AuthContext, AuthzError, CapabilitySet};
use axum::{
    extract::{FromRequestParts, Request, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use std::collections::HashMap;
use std::sync::Arc;

/// Extension type for storing the authenticated context in request extensions.
#[derive(Clone)]
pub struct AuthExtension(pub Arc<AuthContext>);

impl std::ops::Deref for AuthExtension {
    type Target = Arc<AuthContext>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// How specifically a pattern matched a path, used to break ties between
/// overlapping patterns deterministically.
///
/// An exact pattern beats a parameterised one, which beats a wildcard; between two
/// wildcards the one with the longer prefix wins, and between two parameterised patterns
/// the one with more literal segments wins. Without this, iteration order over a
/// `HashMap` decided which capability guarded a path, so the answer could differ
/// between processes.
///
/// The variant order below *is* the precedence order — `derive(Ord)` reads it top to
/// bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MatchRank {
    /// Matched via `prefix/*` or a trailing `{*rest}` capture; ordered by prefix length.
    Prefix(usize),
    /// Matched a pattern containing `{param}` segments; ordered by literal segment count.
    Param(usize),
    /// Matched the whole path literally.
    Exact,
}

/// Match `path` against a route `pattern`, returning how specific the match was.
///
/// Three pattern forms are understood:
///
/// - **Literal** — `/v1/documents` matches only itself.
/// - **Parameterised** — axum's `{id}` syntax, as in `/v1/jobs/{id}`. Each `{name}`
///   matches exactly one non-empty segment, and axum's `{*rest}` matches one or more
///   trailing segments.
/// - **Prefix** — a trailing `*`, as in `/v1/audit*`.
///
/// The parameterised form is not optional sugar. The route table is written in axum's
/// syntax, so a table entry like `/v1/jobs/{id}` would otherwise match no request at
/// all; combined with deny-by-default that makes the route return 403 to *every* caller,
/// including its rightful owner. Registering a route and having it be unreachable is
/// the failure this arm prevents.
///
/// Prefix matching is anchored at a segment boundary: `/v1/audit*` covers `/v1/audit`
/// and `/v1/audit/export` and does *not* cover `/v1/auditor-backdoor`. Plain
/// `starts_with` treats that backdoor as an audit route, which silently hands it
/// whatever capability guards the audit trail.
fn match_pattern(pattern: &str, path: &str) -> Option<MatchRank> {
    if let Some(raw_prefix) = pattern.strip_suffix('*') {
        // Accept both `/v1/audit*` and `/v1/audit/*` spellings.
        let prefix = raw_prefix.strip_suffix('/').unwrap_or(raw_prefix);
        if path == prefix {
            return Some(MatchRank::Prefix(prefix.len()));
        }
        let rest = path.strip_prefix(prefix)?;
        return rest
            .starts_with('/')
            .then_some(MatchRank::Prefix(prefix.len()));
    }

    if !pattern.contains('{') {
        return (pattern == path).then_some(MatchRank::Exact);
    }

    match_parameterised(pattern, path)
}

/// Match an axum-style parameterised pattern such as `/v1/jobs/{id}`.
fn match_parameterised(pattern: &str, path: &str) -> Option<MatchRank> {
    let mut literals = 0usize;
    let mut pattern_segments = pattern.split('/');
    let mut path_segments = path.split('/');

    loop {
        match (pattern_segments.next(), path_segments.next()) {
            (None, None) => return Some(MatchRank::Param(literals)),
            (Some(pattern_segment), Some(path_segment)) => {
                let Some(name) = parameter_name(pattern_segment) else {
                    if pattern_segment != path_segment {
                        return None;
                    }
                    literals += 1;
                    continue;
                };
                // A parameter never matches an empty segment: `/v1/jobs/` must not
                // resolve as "job with an empty id" and inherit the route's capability.
                if path_segment.is_empty() {
                    return None;
                }
                if name.starts_with('*') {
                    // Trailing capture: consumes the rest of the path. Ranked as a
                    // prefix match because it is exactly as unspecific as `prefix/*`.
                    let prefix_len = pattern.len() - pattern_segment.len();
                    return Some(MatchRank::Prefix(prefix_len));
                }
            }
            // One ran out before the other: different segment counts, no match.
            _ => return None,
        }
    }
}

/// The parameter name inside a `{...}` segment, or `None` if the segment is a literal.
fn parameter_name(segment: &str) -> Option<&str> {
    segment.strip_prefix('{')?.strip_suffix('}')
}

/// State for the auth middleware.
#[derive(Clone)]
pub struct AuthState {
    resolver: Arc<dyn TokenResolver>,
    /// Capabilities required for each route pattern.
    /// Key is the route pattern, value is required capabilities.
    route_requirements: Arc<HashMap<String, CapabilitySet>>,
    /// Patterns that are reachable without a token. Everything not listed here and not
    /// listed in `route_requirements` is denied.
    public_routes: Arc<Vec<String>>,
    /// Whether auth is enabled.
    enabled: bool,
}

impl AuthState {
    /// Create a new auth state with the given resolver.
    pub fn new(resolver: Arc<dyn TokenResolver>) -> Self {
        Self {
            resolver,
            route_requirements: Arc::new(HashMap::new()),
            public_routes: Arc::new(Vec::new()),
            enabled: true,
        }
    }

    /// Create auth state from config.
    pub fn from_config(config: &AuthConfig) -> Result<Self, AuthzError> {
        let resolver = build_token_resolver(config)?;
        Ok(Self {
            resolver,
            route_requirements: Arc::new(HashMap::new()),
            public_routes: Arc::new(Vec::new()),
            enabled: config.enabled,
        })
    }

    /// Add a route requirement.
    pub fn with_route_requirement(
        mut self,
        pattern: impl Into<String>,
        capabilities: CapabilitySet,
    ) -> Self {
        Arc::make_mut(&mut self.route_requirements).insert(pattern.into(), capabilities);
        self
    }

    /// Declare a route reachable without authentication.
    ///
    /// Being public is an explicit, auditable decision rather than the consequence of
    /// forgetting to register a requirement.
    pub fn with_public_route(mut self, pattern: impl Into<String>) -> Self {
        Arc::make_mut(&mut self.public_routes).push(pattern.into());
        self
    }

    /// Enable or disable auth.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Whether `path` has been explicitly declared public.
    pub fn is_public(&self, path: &str) -> bool {
        self.public_routes
            .iter()
            .any(|pattern| match_pattern(pattern, path).is_some())
    }

    /// Get the required capabilities for a path, choosing the most specific pattern.
    fn required_capabilities(&self, path: &str) -> Option<&CapabilitySet> {
        self.route_requirements
            .iter()
            .filter_map(|(pattern, caps)| Some((match_pattern(pattern, path)?, caps)))
            .max_by_key(|(rank, _)| *rank)
            .map(|(_, caps)| caps)
    }
}

/// Extract token from Authorization header.
fn extract_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

/// Axum middleware for authentication and authorization.
///
/// Deny by default: a path reaches the handler only if it was declared public or it was
/// registered with capabilities the caller holds. A route nobody remembered to register
/// is refused rather than served, so forgetting an entry costs an outage instead of a
/// disclosure.
pub async fn auth_middleware(
    State(state): State<AuthState>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Auth disabled entirely — single-tenant desktop mode, where the process boundary
    // is the trust boundary.
    if !state.enabled {
        return Ok(next.run(request).await);
    }

    let path = request.uri().path().to_string();

    if state.is_public(&path) {
        return Ok(next.run(request).await);
    }

    let Some(required) = state.required_capabilities(&path) else {
        tracing::warn!(
            path = %path,
            "denying request to a route with no registered capability requirement"
        );
        return Err(StatusCode::FORBIDDEN);
    };

    let token = extract_token(request.headers()).ok_or(StatusCode::UNAUTHORIZED)?;

    let token_obj = state
        .resolver
        .resolve(&token)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Expiry lives here: an expired credential is an authentication failure, not an
    // authorization one, so it must not be reported as "insufficient capabilities".
    let auth_ctx = token_obj
        .to_auth_context()
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    for cap in required.iter() {
        if !auth_ctx.has_capability(cap) {
            tracing::warn!(
                path = %path,
                principal = %auth_ctx.principal_id,
                capability = %cap,
                "denying request: principal lacks required capability"
            );
            return Err(StatusCode::FORBIDDEN);
        }
    }

    request
        .extensions_mut()
        .insert(AuthExtension(Arc::new(auth_ctx)));

    Ok(next.run(request).await)
}

/// Trait for extracting capabilities required by a handler.
pub trait RequireCapability {
    /// The capabilities required.
    fn required_capabilities() -> CapabilitySet;
}

/// Extractor for authenticated context.
impl<S> FromRequestParts<S> for AuthExtension
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<AuthExtension>().cloned().ok_or((
            StatusCode::UNAUTHORIZED,
            "Authentication required".to_string(),
        ))
    }
}

/// Macro to create a route requirement.
#[macro_export]
macro_rules! require_capability {
    ($($cap:expr),+ $(,)?) => {
        CapabilitySet::new([$($cap),+])
    };
}

/// Builder for adding auth requirements to a router.
pub struct AuthRouterBuilder {
    state: AuthState,
}

impl AuthRouterBuilder {
    pub fn new(state: AuthState) -> Self {
        Self { state }
    }

    /// Add a route requirement.
    pub fn require(mut self, pattern: impl Into<String>, capabilities: CapabilitySet) -> Self {
        self.state = self.state.with_route_requirement(pattern, capabilities);
        self
    }

    /// Declare a route reachable without authentication.
    pub fn public(mut self, pattern: impl Into<String>) -> Self {
        self.state = self.state.with_public_route(pattern);
        self
    }

    /// Build the state.
    pub fn build(self) -> AuthState {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{Capability, CapabilitySet};

    #[test]
    fn auth_state_route_matching() {
        let state = AuthState::new(Arc::new(crate::auth::authn::DevTokenResolver))
            .with_route_requirement(
                "/v1/documents*",
                require_capability![Capability::DocumentsProcess],
            )
            .with_route_requirement("/v1/pii/scan", require_capability![Capability::PiiReveal])
            .with_route_requirement(
                "/v1/audit/export",
                require_capability![Capability::AuditExport],
            );

        // Exact match
        assert!(state
            .required_capabilities("/v1/pii/scan")
            .unwrap()
            .has(Capability::PiiReveal));

        // Prefix match
        assert!(state
            .required_capabilities("/v1/documents/123")
            .unwrap()
            .has(Capability::DocumentsProcess));
        assert!(state
            .required_capabilities("/v1/documents/batch")
            .unwrap()
            .has(Capability::DocumentsProcess));

        // No match
        assert!(state.required_capabilities("/health").is_none());
    }

    /// The route table is written in axum's syntax, so a pattern like `/v1/jobs/{id}`
    /// must match a concrete request path.
    ///
    /// Before this arm existed, `match_pattern` compared `/v1/jobs/{id}` to
    /// `/v1/jobs/abc` literally, found no match, and deny-by-default turned the whole
    /// endpoint into a permanent 403 — for its owner as much as for an intruder. The
    /// route was registered, reviewed, and unreachable.
    #[test]
    fn should_match_an_axum_path_parameter_against_a_concrete_segment() {
        let state = AuthState::new(Arc::new(crate::auth::authn::DevTokenResolver))
            .with_route_requirement(
                "/v1/jobs/{id}",
                require_capability![Capability::DocumentsProcess],
            );

        assert!(state
            .required_capabilities("/v1/jobs/9f2c1e0a")
            .unwrap()
            .has(Capability::DocumentsProcess));
    }

    /// A parameter matches exactly one non-empty segment — no more, no less.
    ///
    /// Matching zero segments would let `/v1/jobs/` inherit the route's capability as a
    /// job with an empty id; matching several would let `/v1/jobs/a/b` inherit it too,
    /// silently extending the grant to a sub-resource nobody registered.
    #[test]
    fn should_not_let_a_path_parameter_span_more_or_fewer_than_one_segment() {
        let state = AuthState::new(Arc::new(crate::auth::authn::DevTokenResolver))
            .with_route_requirement(
                "/v1/jobs/{id}",
                require_capability![Capability::DocumentsProcess],
            );

        assert!(state.required_capabilities("/v1/jobs").is_none());
        assert!(state.required_capabilities("/v1/jobs/").is_none());
        assert!(state
            .required_capabilities("/v1/jobs/abc/results")
            .is_none());
        assert!(state.required_capabilities("/v1/jobsX/abc").is_none());
    }

    /// A literal pattern must win over a parameterised one that also matches, and a
    /// parameterised one must win over a prefix wildcard. Otherwise the capability
    /// guarding a path would depend on `HashMap` iteration order.
    #[test]
    fn should_prefer_the_most_specific_pattern() {
        let state = AuthState::new(Arc::new(crate::auth::authn::DevTokenResolver))
            .with_route_requirement("/v1/jobs*", require_capability![Capability::AuditRead])
            .with_route_requirement(
                "/v1/jobs/{id}",
                require_capability![Capability::DocumentsProcess],
            )
            .with_route_requirement(
                "/v1/jobs/stats",
                require_capability![Capability::AuditExport],
            );

        // Exact beats parameterised beats prefix.
        assert!(state
            .required_capabilities("/v1/jobs/stats")
            .unwrap()
            .has(Capability::AuditExport));
        assert!(state
            .required_capabilities("/v1/jobs/abc")
            .unwrap()
            .has(Capability::DocumentsProcess));
        assert!(state
            .required_capabilities("/v1/jobs/abc/raw")
            .unwrap()
            .has(Capability::AuditRead));
    }

    /// axum's trailing capture `{*rest}` consumes the remainder of the path and is as
    /// unspecific as a `prefix/*` wildcard, so it must rank as one.
    #[test]
    fn should_treat_a_trailing_capture_as_a_prefix_match() {
        let state = AuthState::new(Arc::new(crate::auth::authn::DevTokenResolver))
            .with_route_requirement(
                "/v1/audit/{*rest}",
                require_capability![Capability::AuditRead],
            );

        assert!(state.required_capabilities("/v1/audit/a").is_some());
        assert!(state.required_capabilities("/v1/audit/a/b/c").is_some());
        // The capture requires at least one segment.
        assert!(state.required_capabilities("/v1/audit").is_none());
    }

    #[test]
    fn capability_set_macro() {
        let caps = require_capability![Capability::DocumentsProcess, Capability::PiiReveal];
        assert!(caps.has(Capability::DocumentsProcess));
        assert!(caps.has(Capability::PiiReveal));
        assert!(!caps.has(Capability::AuditRead));
    }

    /// A route with no explicit requirement must not be reachable without a
    /// token. The middleware used to `return Ok(next.run(request).await)` when
    /// `required_capabilities` found no entry, so every endpoint an operator
    /// forgot to register was silently public — the opposite of §7's deny-by-
    /// default, and exactly the failure mode OWASP A01 describes.
    #[test]
    fn unregistered_route_is_denied_not_allowed() {
        let state = AuthState::new(Arc::new(crate::auth::authn::DevTokenResolver))
            .with_route_requirement("/v1/pii/scan", require_capability![Capability::PiiReveal]);

        assert!(state.required_capabilities("/v1/pii/scan").is_some());
        // Unregistered: must resolve to "deny", not "no requirement".
        assert!(state.required_capabilities("/v1/documents/leak").is_none());
        assert!(!state.is_public("/v1/documents/leak"));
    }

    /// Prefix matching must respect path segment boundaries. `/v1/audit*`
    /// otherwise also matches `/v1/auditor-backdoor`, and because the map is a
    /// HashMap the *first* pattern to match in arbitrary iteration order wins —
    /// so which capability guards a path could change between runs.
    #[test]
    fn prefix_match_does_not_cross_segment_boundaries() {
        let state = AuthState::new(Arc::new(crate::auth::authn::DevTokenResolver))
            .with_route_requirement("/v1/audit/*", require_capability![Capability::AuditRead]);

        assert!(state.required_capabilities("/v1/audit/entries").is_some());
        assert!(state
            .required_capabilities("/v1/auditor-backdoor")
            .is_none());
    }

    /// With overlapping patterns the most specific one must win, deterministically.
    #[test]
    fn most_specific_pattern_wins() {
        let state = AuthState::new(Arc::new(crate::auth::authn::DevTokenResolver))
            .with_route_requirement("/v1/*", require_capability![Capability::DocumentsProcess])
            .with_route_requirement("/v1/audit/*", require_capability![Capability::AuditRead]);

        let caps = state.required_capabilities("/v1/audit/export").unwrap();
        assert!(caps.has(Capability::AuditRead));
        assert!(!caps.has(Capability::DocumentsProcess));
    }

    #[test]
    fn public_routes_bypass_auth_only_when_declared() {
        let state = AuthState::new(Arc::new(crate::auth::authn::DevTokenResolver))
            .with_public_route("/health");

        assert!(state.is_public("/health"));
        assert!(!state.is_public("/v1/documents"));
    }

    /// End-to-end checks against a real `Router`. The route-matching helper can be
    /// perfectly correct while the middleware still forwards to the handler on the
    /// deny path, so these drive the actual request pipeline.
    mod middleware {
        use super::*;
        use axum::{body::Body, http::Request, routing::get, Router};
        use tower::ServiceExt;

        /// Token minted by `DevTokenResolver` carrying exactly `documents:process`.
        const DOCS_TOKEN: &str = "hcd_documents:process_testsuite";

        fn app(state: AuthState) -> Router {
            Router::new()
                .route("/health", get(|| async { "ok" }))
                .route("/v1/pii/scan", get(|| async { "scanned" }))
                .route("/v1/documents/leak", get(|| async { "SECRET" }))
                .layer(axum::middleware::from_fn_with_state(state, auth_middleware))
        }

        async fn status(state: AuthState, path: &str, token: Option<&str>) -> StatusCode {
            let mut builder = Request::builder().uri(path);
            if let Some(token) = token {
                builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
            }
            app(state)
                .oneshot(builder.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status()
        }

        fn guarded() -> AuthState {
            AuthState::new(Arc::new(crate::auth::authn::DevTokenResolver))
                .with_public_route("/health")
                .with_route_requirement(
                    "/v1/pii/scan",
                    require_capability![Capability::DocumentsProcess],
                )
        }

        #[tokio::test]
        async fn unregistered_route_is_not_served() {
            // `/v1/documents/leak` has a handler but no requirement. Previously the
            // middleware ran it for anyone, token or not.
            assert_eq!(
                status(guarded(), "/v1/documents/leak", None).await,
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                status(guarded(), "/v1/documents/leak", Some(DOCS_TOKEN)).await,
                StatusCode::FORBIDDEN
            );
        }

        #[tokio::test]
        async fn declared_public_route_is_served_without_a_token() {
            assert_eq!(status(guarded(), "/health", None).await, StatusCode::OK);
        }

        #[tokio::test]
        async fn guarded_route_requires_a_token_then_the_capability() {
            assert_eq!(
                status(guarded(), "/v1/pii/scan", None).await,
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                status(guarded(), "/v1/pii/scan", Some("hcd_bogus")).await,
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                status(guarded(), "/v1/pii/scan", Some(DOCS_TOKEN)).await,
                StatusCode::OK
            );
            // Holds a valid token, but not the capability this route demands.
            let audit_only = guarded().with_route_requirement(
                "/v1/pii/scan",
                require_capability![Capability::AuditExport],
            );
            assert_eq!(
                status(audit_only, "/v1/pii/scan", Some(DOCS_TOKEN)).await,
                StatusCode::FORBIDDEN
            );
        }

        #[tokio::test]
        async fn disabling_auth_bypasses_everything() {
            let state = guarded().with_enabled(false);
            assert_eq!(
                status(state, "/v1/documents/leak", None).await,
                StatusCode::OK
            );
        }
    }
}
