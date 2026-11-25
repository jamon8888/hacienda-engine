//! Loopback viewer contract: same-origin assets, cookie-bound short-lived
//! sessions, and content-free reveal auditing.

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use axum::{
    body::{to_bytes, Body},
    http::{header, HeaderMap, Method, Request, StatusCode},
};
use hacienda_api::{router, ApiLimits, ApiState, ViewerSessionStore};
use hacienda_core::{
    auth::{authn::DevTokenResolver, AuthState},
    jobs::InMemoryJobStore,
    HaciendaConfig, HaciendaFacade,
};
use tower::ServiceExt;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct ViewerDir(PathBuf);

impl ViewerDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "hacienda-viewer-contract-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("index.html"),
            "<html><body><script id=\"viewerConfig\" type=\"application/json\">__VIEWER_CONFIG__</script></body></html>",
        )
        .unwrap();
        fs::write(dir.join("viewer.js"), "// viewer").unwrap();
        fs::write(dir.join("viewer.css"), "body{}").unwrap();
        Self(dir)
    }

    fn path(&self) -> PathBuf {
        self.0.clone()
    }
}

impl Drop for ViewerDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn state(dir: &ViewerDir) -> ApiState {
    let facade = std::sync::Arc::new(HaciendaFacade::new(HaciendaConfig::default()).unwrap());
    let jobs = InMemoryJobStore::new().into_arc();
    ApiState::new(
        facade,
        jobs,
        AuthState::new(std::sync::Arc::new(DevTokenResolver)),
        ApiLimits::default(),
    )
    .with_viewer_assets(dir.path())
}

fn loopback_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::HOST, "127.0.0.1:8787".parse().unwrap());
    headers.insert(header::ORIGIN, "http://127.0.0.1:8787".parse().unwrap());
    headers
}

fn request(method: Method, uri: &str, headers: HeaderMap, body: Body) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .body(body)
        .unwrap();
    *request.headers_mut() = headers;
    request
}

#[tokio::test]
async fn viewer_page_mints_a_cookie_bound_session_with_strict_csp() {
    let dir = ViewerDir::new();
    let app = router(state(&dir));
    let response = app
        .oneshot(request(
            Method::GET,
            "/viewer",
            loopback_headers(),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "no-store, private"
    );
    let csp = response.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(csp.contains("connect-src 'self'"));
    assert!(csp.contains("default-src 'self'"));

    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    let token = cookie
        .split(';')
        .next()
        .unwrap()
        .split_once('=')
        .unwrap()
        .1
        .to_owned();
    assert!(token.starts_with("vses_"));

    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let html = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(html.contains(&token));
    assert!(html.contains("/viewer/events"));
}

#[tokio::test]
async fn viewer_assets_are_served_same_origin() {
    let dir = ViewerDir::new();
    let app = router(state(&dir));
    for (path, expected) in [
        ("/viewer/viewer.js", "// viewer"),
        ("/viewer/viewer.css", "body{}"),
    ] {
        let response = app
            .clone()
            .oneshot(request(
                Method::GET,
                path,
                loopback_headers(),
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "no-store, private"
        );
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert_eq!(String::from_utf8(bytes.to_vec()).unwrap(), expected);
    }
}

#[tokio::test]
async fn sse_channel_rejects_missing_cross_session_and_expired_tokens() {
    let dir = ViewerDir::new();
    let api_state = state(&dir);
    let (token, _) = api_state.viewer_sessions().create_session("acme", "alice");
    let (other, _) = api_state.viewer_sessions().create_session("acme", "bob");
    let app = router(api_state);

    let events_uri = format!("/viewer/events?token={token}");
    let response = app
        .clone()
        .oneshot(request(
            Method::GET,
            &events_uri,
            loopback_headers(),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let mut cross = loopback_headers();
    cross.insert(
        header::COOKIE,
        format!("hacienda_viewer_session={other}").parse().unwrap(),
    );
    let response = app
        .clone()
        .oneshot(request(Method::GET, &events_uri, cross, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let expired_store = ViewerSessionStore::new(Duration::from_millis(1));
    let (expired, _) = expired_store.create_session("acme", "alice");
    std::thread::sleep(Duration::from_millis(5));
    assert!(expired_store.subscribe(&expired, Some(&expired)).is_err());

    let mut valid = loopback_headers();
    valid.insert(
        header::COOKIE,
        format!("hacienda_viewer_session={token}").parse().unwrap(),
    );
    let response = app
        .oneshot(request(Method::GET, &events_uri, valid, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
}

#[tokio::test]
async fn reveal_audit_is_content_free_and_session_bound() {
    let dir = ViewerDir::new();
    let api_state = state(&dir);
    let (token, _) = api_state.viewer_sessions().create_session("acme", "alice");
    let app = router(api_state.clone());

    let mut headers = loopback_headers();
    headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    headers.insert("x-hacienda-viewer-session", token.parse().unwrap());
    headers.insert(
        header::COOKIE,
        format!("hacienda_viewer_session={token}").parse().unwrap(),
    );
    let body =
        Body::from(serde_json::json!({"token_ref": "[EMAIL:k1:MFRGG2LTMVRXEZLU]"}).to_string());
    let response = app
        .clone()
        .oneshot(request(Method::POST, "/viewer/reveal", headers, body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let audit = api_state.viewer_sessions().audit_for_tenant("acme");
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].action, "reveal");
    assert_eq!(audit[0].principal, "alice");
    assert!(audit[0].token_ref.contains("[EMAIL:"));
    assert!(!format!("{:?}", audit).contains("alice@example.com"));

    let response = app
        .oneshot(request(
            Method::POST,
            "/viewer/reveal",
            {
                let mut headers = loopback_headers();
                headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
                headers
            },
            Body::from(serde_json::json!({"token_ref": "[EMAIL:k1:MFRGG2LTMVRXEZLU]"}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn non_loopback_origin_is_refused() {
    let dir = ViewerDir::new();
    let app = router(state(&dir));
    let mut headers = loopback_headers();
    headers.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
    let response = app
        .oneshot(request(Method::GET, "/viewer", headers, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
