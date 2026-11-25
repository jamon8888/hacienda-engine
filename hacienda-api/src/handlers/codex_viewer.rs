//! Same-origin, loopback-only viewer channel for the Codex Responses gateway.
//!
//! The browser never talks to the inference provider and never receives a
//! bearer credential. Loading `/viewer` mints a short-lived, unguessable
//! session, returns it in an HttpOnly cookie and embeds a matching one-time
//! token in the page. The SSE and reveal endpoints require both values, a
//! loopback Host, and a loopback/same-origin `Origin` when one is present.
//!
//! Original PII values live only in this in-memory session and in the browser
//! process. They are published to the viewer over a separate channel from the
//! tokenized text Codex receives; they are never written to the provider body,
//! an SSE frame sent to Codex, a log line, or a persistent store. Audit
//! records contain only action/clock/reference metadata.

use std::{
    collections::HashMap,
    convert::Infallible,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use futures::stream;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{error::ApiError, extract::Json as SafeJson, state::ApiState};

/// Viewer sessions are deliberately shorter than a working session: a stale
/// tab cannot replay a mapping after this window.
pub const VIEWER_SESSION_TTL: Duration = Duration::from_secs(10 * 60);
const SESSION_COOKIE: &str = "hacienda_viewer_session";
const SESSION_HEADER: &str = "x-hacienda-viewer-session";
const MAX_AUDIT_ENTRIES: usize = 2048;

/// One viewer event. `mapping` carries the local-only original; `text` carries
/// already-tokenized text that is also being sent to Codex.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewerEvent {
    Mapping {
        turn_id: String,
        token: String,
        original: String,
    },
    Text {
        turn_id: String,
        text: String,
    },
    End {
        turn_id: String,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct ViewerAuditRecord {
    pub at: String,
    pub action: String,
    pub tenant: String,
    pub principal: String,
    pub session_ref: String,
    pub token_ref: String,
    pub outcome: String,
}

struct ViewerSession {
    tenant: String,
    principal: String,
    expires: Instant,
    tx: tokio::sync::mpsc::UnboundedSender<ViewerEvent>,
}

#[derive(Default)]
struct ViewerInner {
    sessions: HashMap<String, ViewerSession>,
    /// Exactly one session is the display target for Codex turns. Opening a
    /// viewer page selects it; original mappings are never broadcast to other
    /// valid sessions.
    active: Option<String>,
    audit: Vec<ViewerAuditRecord>,
}

/// Process-local viewer sessions. `Clone` shares the same store, so every
/// router clone sees the same sessions.
#[derive(Clone)]
pub struct ViewerSessionStore {
    inner: Arc<Mutex<ViewerInner>>,
    ttl: Duration,
}

impl Default for ViewerSessionStore {
    fn default() -> Self {
        Self::new(VIEWER_SESSION_TTL)
    }
}

impl ViewerSessionStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ViewerInner::default())),
            ttl,
        }
    }

    /// Mint a session bound to the authenticated gateway caller's scope. The
    /// token is two UUIDs behind a typed prefix: it carries no tenant, path,
    /// or PII information and is not derivable from browser input.
    pub fn create_session(&self, tenant: &str, principal: &str) -> (String, String) {
        let token = format!(
            "vses_{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        let expires_at = (chrono::Utc::now()
            + chrono::Duration::from_std(self.ttl).unwrap_or_default())
        .to_rfc3339();
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .sessions
            .retain(|_, session| session.expires > Instant::now());
        inner.sessions.insert(
            token.clone(),
            ViewerSession {
                tenant: tenant.to_owned(),
                principal: principal.to_owned(),
                expires: Instant::now() + self.ttl,
                tx,
            },
        );
        // The freshly opened page is the only display target until another
        // viewer page is opened. This keeps a second tab from receiving the
        // first tab's local rehydration values.
        inner.active = Some(token.clone());
        (token, expires_at)
    }

    /// Explicitly bind subsequent Codex turns to one already-valid session.
    pub fn activate(&self, token: &str) -> Result<(), ApiError> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .sessions
            .retain(|_, session| session.expires > Instant::now());
        let session = inner
            .sessions
            .get(token)
            .ok_or_else(ApiError::unauthenticated)?;
        if session.expires <= Instant::now() {
            return Err(ApiError::unauthenticated());
        }
        inner.active = Some(token.to_owned());
        Ok(())
    }

    /// Validate a presented token and return a receiver for its event stream.
    /// An expired, unknown or malformed token is refused identically.
    pub fn subscribe(
        &self,
        token: &str,
        cookie_token: Option<&str>,
    ) -> Result<tokio::sync::mpsc::UnboundedReceiver<ViewerEvent>, ApiError> {
        if !is_session_token(token) || cookie_token != Some(token) {
            return Err(ApiError::unauthenticated());
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .sessions
            .retain(|_, session| session.expires > Instant::now());
        // Replace the session's sender so a replayed SSE connection cannot
        // keep a stale stream alive; the previous receiver simply ends.
        let session = inner
            .sessions
            .get_mut(token)
            .ok_or_else(ApiError::unauthenticated)?;
        session.tx = tx;
        Ok(rx)
    }

    /// Record a local reveal. Only the token reference and scopes are stored;
    /// the original value is never passed to this function.
    pub fn audit_reveal(&self, token: &str, token_ref: &str) -> Result<(), ApiError> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let (tenant, principal) = {
            let session = inner
                .sessions
                .get(token)
                .ok_or_else(ApiError::unauthenticated)?;
            if session.expires <= Instant::now() {
                return Err(ApiError::unauthenticated());
            }
            (session.tenant.clone(), session.principal.clone())
        };
        inner.audit.push(ViewerAuditRecord {
            at: chrono::Utc::now().to_rfc3339(),
            action: "reveal".to_owned(),
            tenant,
            principal,
            session_ref: session_ref(token),
            token_ref: token_ref.to_owned(),
            outcome: "audited_without_plaintext".to_owned(),
        });
        if inner.audit.len() > MAX_AUDIT_ENTRIES {
            inner.audit.remove(0);
        }
        Ok(())
    }

    /// Publish to the single active session only. A value that cannot be bound
    /// to a live viewer is dropped rather than leaked to another session.
    pub fn publish_to_active(&self, event: ViewerEvent) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .sessions
            .retain(|_, session| session.expires > Instant::now());
        let Some(token) = inner.active.clone() else {
            return;
        };
        let Some(session) = inner.sessions.get(&token) else {
            inner.active = None;
            return;
        };
        if session.expires <= Instant::now() {
            inner.active = None;
            return;
        }
        let _ = session.tx.send(event);
    }

    /// Content-free audit snapshot for the session's tenant, for tests and
    /// operators. Never includes a plaintext value.
    pub fn audit_for_tenant(&self, tenant: &str) -> Vec<ViewerAuditRecord> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .audit
            .iter()
            .filter(|record| record.tenant == tenant)
            .cloned()
            .collect()
    }
}

fn session_ref(token: &str) -> String {
    // A stable, non-reversible prefix: enough to correlate audit events, not
    // enough to act as a bearer token.
    let mut bytes = [0u8; 32];
    let digest = blake3::hash(token.as_bytes());
    bytes.copy_from_slice(digest.as_bytes());
    format!("vses_{}", data_encoding::HEXLOWER.encode(&bytes[..8]))
}

fn is_session_token(token: &str) -> bool {
    token.len() == 69
        && token.starts_with("vses_")
        && token[5..].chars().all(|c| c.is_ascii_hexdigit())
}

// ─── Loopback / same-origin guard ────────────────────────────────────────────

fn loopback_host(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "[::1]" | "::1"
    )
}

/// Parse a Host header into `(host, port)` with HTTP's default port 80.
fn host_and_port(host: &str) -> Option<(String, u16)> {
    if let Some(rest) = host.strip_prefix('[') {
        let end = rest.find(']')?;
        let name = format!("[{}]", &rest[..end]);
        let port = match rest[end + 1..].strip_prefix(':') {
            Some(port) => port.parse().ok()?,
            None => 80,
        };
        Some((name, port))
    } else {
        match host.rsplit_once(':') {
            Some((name, port)) => Some((name.to_owned(), port.parse().ok()?)),
            None => Some((host.to_owned(), 80)),
        }
    }
}

pub(crate) fn is_loopback_request(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some((host_name, host_port)) = host_and_port(host) else {
        return false;
    };
    if !loopback_host(&host_name) {
        return false;
    }
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let Ok(parsed) = url::Url::parse(origin) else {
            return false;
        };
        let Some(origin_host) = parsed.host_str() else {
            return false;
        };
        let origin_host = if origin_host.contains(':') {
            format!("[{origin_host}]")
        } else {
            origin_host.to_owned()
        };
        if parsed.scheme() != "http"
            || !loopback_host(&origin_host)
            || origin_host.to_ascii_lowercase() != host_name.to_ascii_lowercase()
            || parsed.port_or_known_default().unwrap_or(80) != host_port
        {
            return false;
        }
    }
    if let Some(site) = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
    {
        if !matches!(site, "same-origin" | "none") {
            return false;
        }
    }
    true
}

fn cookie_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|cookie| cookie.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == SESSION_COOKIE).then_some(value)
        })
}

fn viewer_headers(content_type: &'static str, session: Option<&str>) -> HeaderMap {
    let mut map = HeaderMap::new();
    map.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    map.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    map.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    map.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    map.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; connect-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; form-action 'none'",
        ),
    );
    if let Some(token) = session {
        let cookie = format!(
            "{SESSION_COOKIE}={token}; Path=/viewer; HttpOnly; SameSite=Strict; Max-Age={}",
            VIEWER_SESSION_TTL.as_secs()
        );
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            map.insert(header::SET_COOKIE, value);
        }
    }
    map
}

// ─── Handlers ────────────────────────────────────────────────────────────────

/// `GET /viewer` — same-origin page. Mint a session and bind it to a cookie.
#[utoipa::path(
    get,
    path = "/viewer",
    tag = "codex-viewer",
    operation_id = "codexViewerPage",
    responses(
        (status = 200, description = "Viewer HTML with an embedded one-time session token"),
        (status = 403, description = "Viewer is not enabled or request is not loopback")
    )
)]
pub async fn viewer_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if !is_loopback_request(&headers) {
        return Err(ApiError::invalid_request(
            "Viewer requests must originate from loopback.",
        ));
    }
    let dir = state
        .viewer_assets_dir
        .as_ref()
        .ok_or_else(|| ApiError::not_found())?;
    let html = tokio::fs::read_to_string(dir.join("index.html"))
        .await
        .map_err(|_| ApiError::not_found())?;
    // The page is bound to a server-owned local principal: it never handles
    // the inference credential or the caller's bearer token.
    let (token, expires_at) = state
        .viewer_sessions
        .create_session("viewer-local", "codex-viewer");
    let config = serde_json::json!({
        "sessionToken": token,
        "expiresAt": expires_at,
        "eventsUrl": format!("/viewer/events?token={token}"),
        "revealUrl": "/viewer/reveal",
    })
    .to_string();
    // The config is embedded in an HTML attribute: escape quotes so the JSON
    // string survives HTML parsing without an inline script (strict CSP).
    let html = html.replace("__VIEWER_CONFIG__", &config.replace('"', "&quot;"));
    Ok((
        viewer_headers("text/html; charset=utf-8", Some(&token)),
        html,
    )
        .into_response())
}

async fn asset(
    dir: Option<&PathBuf>,
    name: &str,
    content_type: &'static str,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if !is_loopback_request(&headers) {
        return Err(ApiError::invalid_request(
            "Viewer requests must originate from loopback.",
        ));
    }
    let bytes = tokio::fs::read(dir.ok_or_else(ApiError::not_found)?.join(name))
        .await
        .map_err(|_| ApiError::not_found())?;
    Ok((viewer_headers(content_type, None), bytes).into_response())
}

/// `GET /viewer/viewer.js`.
#[utoipa::path(
    get,
    path = "/viewer/viewer.js",
    tag = "codex-viewer",
    operation_id = "codexViewerScript",
    responses((status = 200, description = "Viewer script"), (status = 404, description = "Viewer not enabled"))
)]
pub async fn viewer_script(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    asset(
        state.viewer_assets_dir.as_ref(),
        "viewer.js",
        "application/javascript; charset=utf-8",
        headers,
    )
    .await
}

/// `GET /viewer/viewer.css`.
#[utoipa::path(
    get,
    path = "/viewer/viewer.css",
    tag = "codex-viewer",
    operation_id = "codexViewerStyles",
    responses((status = 200, description = "Viewer stylesheet"), (status = 404, description = "Viewer not enabled"))
)]
pub async fn viewer_styles(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    asset(
        state.viewer_assets_dir.as_ref(),
        "viewer.css",
        "text/css; charset=utf-8",
        headers,
    )
    .await
}

#[derive(Debug, Deserialize)]
pub struct ViewerEventsQuery {
    token: String,
}

/// `GET /viewer/events` — authenticated, same-origin SSE channel.
#[utoipa::path(
    get,
    path = "/viewer/events",
    tag = "codex-viewer",
    operation_id = "codexViewerEvents",
    responses(
        (status = 200, description = "Viewer SSE stream", content_type = "text/event-stream"),
        (status = 401, description = "Missing, expired or cross-session token"),
        (status = 403, description = "Request is not loopback")
    )
)]
pub async fn viewer_events(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<ViewerEventsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if !is_loopback_request(&headers) {
        return Err(ApiError::invalid_request(
            "Viewer requests must originate from loopback.",
        ));
    }
    let receiver = state
        .viewer_sessions
        .subscribe(&query.token, cookie_token(&headers))?;
    let stream = stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|event| {
            let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_owned());
            (
                Ok::<Event, Infallible>(Event::default().data(data)),
                receiver,
            )
        })
    });
    Ok((
        viewer_headers("text/event-stream", None),
        Sse::new(stream).keep_alive(KeepAlive::default()),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewerRevealRequest {
    token_ref: String,
}

/// `POST /viewer/reveal` — audit a local reveal without sending its content.
#[utoipa::path(
    post,
    path = "/viewer/reveal",
    tag = "codex-viewer",
    operation_id = "codexViewerReveal",
    responses(
        (status = 200, description = "Content-free reveal audit recorded"),
        (status = 401, description = "Missing, expired or cross-session token"),
        (status = 403, description = "Request is not loopback")
    )
)]
pub async fn viewer_reveal(
    State(state): State<ApiState>,
    headers: HeaderMap,
    SafeJson(request): SafeJson<ViewerRevealRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !is_loopback_request(&headers) {
        return Err(ApiError::invalid_request(
            "Viewer requests must originate from loopback.",
        ));
    }
    let token = headers
        .get(SESSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(ApiError::unauthenticated)?;
    if cookie_token(&headers) != Some(token) || request.token_ref.len() > 256 {
        return Err(ApiError::unauthenticated());
    }
    state
        .viewer_sessions
        .audit_reveal(token, &request.token_ref)?;
    Ok(Json(serde_json::json!({"audited": true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacienda_core::{
        auth::{authn::DevTokenResolver, AuthState},
        jobs::InMemoryJobStore,
        HaciendaConfig, HaciendaFacade,
    };
    use std::sync::Arc;

    fn state() -> ApiState {
        let facade = Arc::new(HaciendaFacade::new(HaciendaConfig::default()).unwrap());
        let jobs = InMemoryJobStore::new().into_arc();
        ApiState::new(
            facade,
            jobs,
            AuthState::new(Arc::new(DevTokenResolver)),
            crate::state::ApiLimits::default(),
        )
    }

    fn loopback_headers(token: Option<&str>, cookie: bool) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:8787".parse().unwrap());
        headers.insert(header::ORIGIN, "http://127.0.0.1:8787".parse().unwrap());
        if let Some(token) = token {
            if cookie {
                headers.insert(
                    header::COOKIE,
                    format!("{SESSION_COOKIE}={token}").parse().unwrap(),
                );
            }
        }
        headers
    }

    #[test]
    fn session_token_shape_and_expiry_are_enforced() {
        let store = ViewerSessionStore::new(Duration::from_millis(1));
        let (token, _) = store.create_session("acme", "alice");
        assert!(is_session_token(&token));
        std::thread::sleep(Duration::from_millis(5));
        assert!(store.subscribe(&token, Some(&token)).is_err());
    }

    #[test]
    fn missing_or_cross_session_cookie_is_rejected() {
        let store = ViewerSessionStore::new(VIEWER_SESSION_TTL);
        let (first, _) = store.create_session("acme", "alice");
        let (second, _) = store.create_session("acme", "bob");
        assert!(store.subscribe(&first, None).is_err());
        assert!(store.subscribe(&first, Some(&second)).is_err());
        assert!(store.subscribe(&first, Some(&first)).is_ok());
    }

    #[test]
    fn reveal_audit_carries_no_plaintext() {
        let store = ViewerSessionStore::new(VIEWER_SESSION_TTL);
        let (token, _) = store.create_session("acme", "alice");
        store
            .audit_reveal(&token, "[EMAIL:k1:MFRGG2LTMVRXEZLU]")
            .unwrap();
        let audit = store.audit_for_tenant("acme");
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].action, "reveal");
        assert_eq!(audit[0].tenant, "acme");
        assert!(audit[0].token_ref.contains("[EMAIL:"));
        assert!(!format!("{:?}", audit).contains("bob@example.com"));
    }

    #[test]
    fn mapping_is_delivered_only_to_the_active_session() {
        let store = ViewerSessionStore::new(VIEWER_SESSION_TTL);
        let (first, _) = store.create_session("acme", "alice");
        let (second, _) = store.create_session("acme", "bob");
        let mut first_rx = store.subscribe(&first, Some(&first)).unwrap();
        let mut second_rx = store.subscribe(&second, Some(&second)).unwrap();

        let mapping = ViewerEvent::Mapping {
            turn_id: "turn-1".to_owned(),
            token: "[EMAIL:k1:abc]".to_owned(),
            original: "alice@example.com".to_owned(),
        };
        // The last page opened is active by default: the second tab receives,
        // the first never sees the other session's original.
        store.publish_to_active(mapping.clone());
        assert_eq!(second_rx.try_recv().unwrap(), mapping);
        assert!(matches!(
            first_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        // Rebinding explicitly to the first session flips the isolation.
        store.activate(&first).unwrap();
        store.publish_to_active(mapping.clone());
        assert_eq!(first_rx.try_recv().unwrap(), mapping);
        assert!(matches!(
            second_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn loopback_guard_rejects_remote_origin_and_host() {
        let mut headers = loopback_headers(None, false);
        assert!(is_loopback_request(&headers));
        headers.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert!(!is_loopback_request(&headers));

        let mut headers = loopback_headers(None, false);
        headers.insert(header::HOST, "evil.example".parse().unwrap());
        assert!(!is_loopback_request(&headers));

        // A loopback origin on a different port is not the same origin.
        let mut headers = loopback_headers(None, false);
        headers.insert(header::ORIGIN, "http://127.0.0.1:9999".parse().unwrap());
        assert!(!is_loopback_request(&headers));

        let mut headers = loopback_headers(None, false);
        headers.remove(header::HOST);
        assert!(!is_loopback_request(&headers));
    }

    #[tokio::test]
    async fn events_requires_a_matching_session_cookie() {
        let state = state();
        let (token, _) = state.viewer_sessions.create_session("acme", "alice");
        let result = viewer_events(
            State(state),
            loopback_headers(Some(&token), false),
            Query(ViewerEventsQuery {
                token: token.clone(),
            }),
        )
        .await;
        assert!(result.is_err());
    }
}
