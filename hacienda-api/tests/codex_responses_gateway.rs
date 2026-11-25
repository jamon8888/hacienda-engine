//! Loopback contract for the server-side Codex Responses gateway.
//!
//! These tests drive the real router, including the auth middleware and the
//! single route table, with an injected provider. They assert the security
//! boundary that matters: the gateway receives the caller's original prompt,
//! the provider receives only pseudonymous text, and no real value reaches the
//! SSE client or a client-chosen account/tenant context.

use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration as StdDuration,
};

use async_trait::async_trait;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use hacienda_api::ViewerEvent;
use hacienda_api::{
    router, ApiLimits, ApiState, CodexResponsesProvider, CodexResponsesProviderRequest,
    CodexResponsesProviderResponse, CodexResponsesProviderStream, ProviderGatewayError,
    ProviderSseEvent,
};
use hacienda_core::{
    auth::{authn::DevTokenResolver, AuthState},
    jobs::InMemoryJobStore,
    pii::PipelineConfig,
    redaction::{EnvKeyResolver, RedactionConfig, RedactionMode},
    HaciendaConfig, HaciendaFacade,
};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{timeout, Duration},
};
use tower::ServiceExt;

const EMAIL: &str = "alice@example.com";
const LOCAL_SECRET: &str = "local-gateway-secret-123";
const CHATGPT_AUTH: &str = "Bearer chatgpt-user-token";
const ACCOUNT_ID: &str = "acct-123";
type RecordedHeaders = Vec<Vec<(String, String)>>;

#[derive(Clone, Default)]
struct RecordingProvider {
    bodies: Arc<Mutex<Vec<Value>>>,
    headers: Arc<Mutex<RecordedHeaders>>,
    auths: Arc<Mutex<Vec<String>>>,
    accounts: Arc<Mutex<Vec<String>>>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl CodexResponsesProvider for RecordingProvider {
    async fn stream(
        &self,
        request: CodexResponsesProviderRequest,
    ) -> Result<CodexResponsesProviderStream, ProviderGatewayError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.bodies.lock().unwrap().push(request.body);
        self.headers.lock().unwrap().push(request.headers);
        self.auths.lock().unwrap().push(request.authorization);
        self.accounts.lock().unwrap().push(request.account_id);

        let event = |kind: &str, data: Value| {
            ProviderSseEvent::new(Some(kind.to_owned()), None, data.to_string())
        };
        let events = vec![
            event(
                "response.created",
                serde_json::json!({
                    "type": "response.created",
                    "response": {"id": "resp_1", "object": "response", "status": "in_progress", "output": []}
                }),
            ),
            event(
                "response.output_text.delta",
                serde_json::json!({
                    "type": "response.output_text.delta", "item_id": "item_1",
                    "output_index": 0, "content_index": 0, "delta": "alice@"
                }),
            ),
            event(
                "response.output_text.delta",
                serde_json::json!({
                    "type": "response.output_text.delta", "item_id": "item_1",
                    "output_index": 0, "content_index": 0, "delta": "example.com"
                }),
            ),
            event(
                "response.output_text.done",
                serde_json::json!({
                    "type": "response.output_text.done", "item_id": "item_1",
                    "output_index": 0, "content_index": 0, "text": EMAIL
                }),
            ),
            event(
                "response.completed",
                serde_json::json!({
                    "type": "response.completed",
                    "response": {
                        "id": "resp_1", "object": "response", "status": "completed",
                        "output": [{"type": "message", "role": "assistant", "content": [
                            {"type": "output_text", "text": EMAIL}
                        ]}]
                    }
                }),
            ),
        ];
        Ok(CodexResponsesProviderStream {
            response: CodexResponsesProviderResponse {
                status: 200,
                headers: Vec::new(),
            },
            events: Box::pin(futures::stream::iter(
                events.into_iter().map(Ok::<_, ProviderGatewayError>),
            )),
        })
    }

    fn models(&self) -> Vec<String> {
        vec!["hacienda-codex".to_owned()]
    }
}

fn state(provider: RecordingProvider) -> ApiState {
    let facade = Arc::new(
        HaciendaFacade::new(HaciendaConfig::default().with_pii(PipelineConfig::default()))
            .expect("PII facade"),
    );
    let jobs = InMemoryJobStore::new().into_arc();
    ApiState::new(
        facade,
        jobs,
        AuthState::new(Arc::new(DevTokenResolver)),
        ApiLimits::default(),
    )
    .with_codex_responses_provider(Arc::new(provider))
    .with_codex_gateway_secret(LOCAL_SECRET)
}

fn request() -> Value {
    serde_json::json!({
        "model": "hacienda-codex",
        "stream": true,
        "instructions": format!("Ask {EMAIL}"),
        "input": [{
            "type": "message", "role": "user",
            "content": [{"type": "input_text", "text": format!("email {EMAIL}")}]
        }],
        "tools": [{
            "type": "function", "name": "lookup",
            "description": format!("look up {EMAIL}"),
            "parameters": {"type": "object", "properties": {"q": {"type": "string"}}}
        }]
    })
}

/// Build a Responses request with independently controllable local-secret,
/// ChatGPT bearer and account headers.
fn gateway_request(
    local: Option<&str>,
    chatgpt: Option<&str>,
    account: Option<&str>,
    body: Value,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json");
    if let Some(local) = local {
        builder = builder.header("x-hacienda-gateway-token", local);
    }
    if let Some(chatgpt) = chatgpt {
        builder = builder.header("authorization", chatgpt);
    }
    if let Some(account) = account {
        builder = builder.header("chatgpt-account-id", account);
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

async fn post(
    app: &axum::Router,
    local: Option<&str>,
    chatgpt: Option<&str>,
    account: Option<&str>,
    body: Value,
) -> axum::response::Response {
    app.clone()
        .oneshot(gateway_request(local, chatgpt, account, body))
        .await
        .unwrap()
}

#[tokio::test]
async fn gateway_redacts_before_egress_and_in_spite_of_split_deltas() {
    let provider = RecordingProvider::default();
    let app = router(state(provider.clone()));
    let response = post(
        &app,
        Some(LOCAL_SECRET),
        Some(CHATGPT_AUTH),
        Some(ACCOUNT_ID),
        request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("response body");
    let sse = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(!sse.contains(EMAIL), "SSE client received a real value");
    assert!(sse.contains("response.created"));
    assert!(sse.contains("response.completed"));

    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let bodies = provider.bodies.lock().unwrap();
    let upstream = serde_json::to_string(&bodies[0]).unwrap();
    assert!(!upstream.contains(EMAIL), "provider received a real value");
    assert_eq!(bodies[0]["store"], serde_json::json!(false));
    assert!(bodies[0].get("tenant").is_none());
    // The ChatGPT token and account id are forwarded unchanged upstream; the
    // local gateway secret is not part of the provider request at all.
    assert_eq!(provider.auths.lock().unwrap()[0], CHATGPT_AUTH);
    assert_eq!(provider.accounts.lock().unwrap()[0], ACCOUNT_ID);
    let headers = provider.headers.lock().unwrap();
    assert!(headers[0]
        .iter()
        .all(|(name, value)| name != "x-hacienda-gateway-token" && !value.contains(LOCAL_SECRET)));
}

#[tokio::test]
async fn gateway_refuses_missing_or_wrong_local_secret_before_provider() {
    let provider = RecordingProvider::default();
    let app = router(state(provider.clone()));

    // No local secret header at all.
    let anonymous = post(&app, None, Some(CHATGPT_AUTH), Some(ACCOUNT_ID), request()).await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    // A wrong local secret, and a local secret presented in Authorization.
    let wrong = post(
        &app,
        Some("wrong-local-secret"),
        Some(CHATGPT_AUTH),
        Some(ACCOUNT_ID),
        request(),
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    let wrong_header = post(
        &app,
        None,
        Some(&format!("Bearer {LOCAL_SECRET}")),
        Some(ACCOUNT_ID),
        request(),
    )
    .await;
    assert_eq!(wrong_header.status(), StatusCode::UNAUTHORIZED);

    // Inference credentials alone never authorize a provider call.
    let no_account = post(
        &app,
        Some(LOCAL_SECRET),
        Some(CHATGPT_AUTH),
        None,
        request(),
    )
    .await;
    assert_eq!(no_account.status(), StatusCode::UNAUTHORIZED);

    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn uninspectable_content_and_client_account_header_fail_closed() {
    let provider = RecordingProvider::default();
    let app = router(state(provider.clone()));

    let image = serde_json::json!({
        "model": "hacienda-codex", "stream": true,
        "input": [{"type": "message", "role": "user", "content": [
            {"type": "input_image", "image_url": "https://example.test/a.png"}
        ]}]
    });
    assert_eq!(
        post(
            &app,
            Some(LOCAL_SECRET),
            Some(CHATGPT_AUTH),
            Some(ACCOUNT_ID),
            image
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    // A malformed account id fails closed after local auth but before egress.
    assert_eq!(
        post(
            &app,
            Some(LOCAL_SECRET),
            Some(CHATGPT_AUTH),
            Some("bad account injected"),
            request()
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn models_discovery_is_authenticated_and_server_owned() {
    let provider = RecordingProvider::default();
    let app = router(state(provider.clone()));

    let anonymous = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let authenticated = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .header("x-hacienda-gateway-token", LOCAL_SECRET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authenticated.status(), StatusCode::OK);
    let bytes = to_bytes(authenticated.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["data"][0]["id"], serde_json::json!("hacienda-codex"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn only_the_dedicated_local_header_authorizes_the_gateway() {
    let provider = RecordingProvider::default();
    let app = router(state(provider.clone()));

    for (local, chatgpt) in [
        (None, Some(CHATGPT_AUTH)),
        (Some("wrong-local-secret"), Some(CHATGPT_AUTH)),
        (Some(LOCAL_SECRET), None),
        (None, None),
    ] {
        let response = post(&app, local, chatgpt, Some(ACCOUNT_ID), request()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }

    let response = post(
        &app,
        Some(LOCAL_SECRET),
        Some(CHATGPT_AUTH),
        Some(ACCOUNT_ID),
        request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider.auths.lock().unwrap()[0], CHATGPT_AUTH);

    let models = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .header("x-hacienda-gateway-token", LOCAL_SECRET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(models.status(), StatusCode::OK);
}

#[tokio::test]
async fn viewer_mapping_and_tokenized_text_share_one_turn_without_leaking_to_codex() {
    let resolver = EnvKeyResolver::with_lookup(|name| match name {
        "HACIENDA_PSEUDONYM_ACTIVE_KEY" => Some("k1".to_owned()),
        "HACIENDA_PSEUDONYM_KEY_K1" => Some("07".repeat(64)),
        _ => None,
    });
    let config = HaciendaConfig::default().with_pii(PipelineConfig {
        redaction: RedactionConfig {
            mode: RedactionMode::Pseudonymize,
            ..Default::default()
        },
        ..Default::default()
    });
    let facade =
        Arc::new(HaciendaFacade::with_key_resolver(config, Arc::new(resolver), &[]).unwrap());
    let provider = RecordingProvider::default();
    let api_state = ApiState::new(
        facade,
        InMemoryJobStore::new().into_arc(),
        AuthState::new(Arc::new(DevTokenResolver)),
        ApiLimits::default(),
    )
    .with_codex_responses_provider(Arc::new(provider.clone()))
    .with_codex_gateway_secret(LOCAL_SECRET);
    let (session, _) = api_state
        .viewer_sessions()
        .create_session("default", "viewer-local");
    let mut viewer_rx = api_state
        .viewer_sessions()
        .subscribe(&session, Some(&session))
        .unwrap();
    // A second valid session must never receive the first session's originals.
    let (other_session, _) = api_state
        .viewer_sessions()
        .create_session("default", "other-viewer");
    let mut other_rx = api_state
        .viewer_sessions()
        .subscribe(&other_session, Some(&other_session))
        .unwrap();
    api_state.viewer_sessions().activate(&session).unwrap();
    let app = router(api_state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header("x-hacienda-gateway-token", LOCAL_SECRET)
                .header("authorization", CHATGPT_AUTH)
                .header("chatgpt-account-id", ACCOUNT_ID)
                .header("content-type", "application/json")
                .body(Body::from(request().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    let sse = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(!sse.contains(EMAIL), "Codex received a real value");

    let mut mappings = Vec::new();
    let mut texts = Vec::new();
    let mut turn_ids = Vec::new();
    while let Ok(event) = viewer_rx.try_recv() {
        match event {
            ViewerEvent::Mapping {
                turn_id,
                token,
                original,
            } => {
                assert!(token.starts_with("[EMAIL:k1:"));
                assert_eq!(original, EMAIL);
                mappings.push(token);
                turn_ids.push(turn_id);
            }
            ViewerEvent::Text { turn_id, text } => {
                assert!(!text.contains(EMAIL), "viewer text event leaked plaintext");
                texts.push(text);
                turn_ids.push(turn_id);
            }
            ViewerEvent::End { turn_id } => turn_ids.push(turn_id),
        }
    }
    assert!(!mappings.is_empty(), "viewer must receive local mappings");
    assert!(
        !texts.is_empty(),
        "viewer must receive tokenized response text"
    );
    assert!(texts.iter().any(|text| text.contains("[EMAIL:k1:")));
    assert_eq!(
        turn_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        1
    );
    let other_events: Vec<_> = std::iter::from_fn(|| other_rx.try_recv().ok()).collect();
    assert!(
        other_events.is_empty(),
        "the second session received another session's mapping or text"
    );
    let upstream = serde_json::to_string(&provider.bodies.lock().unwrap()[0]).unwrap();
    assert!(!upstream.contains(EMAIL));
}

// ─── Real loopback HTTP (no synthetic oneshot requests) ──────────────────────

struct RawResponse {
    status: String,
    headers: String,
    body: Vec<u8>,
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn read_until_headers(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = timeout(StdDuration::from_secs(10), stream.read(&mut chunk))
            .await
            .expect("server answered headers")
            .expect("read ok");
        assert!(read > 0, "connection closed before response headers");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = find_bytes(&buffer, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buffer[..end + 4]).into_owned();
            let rest = buffer[end + 4..].to_vec();
            return (headers, rest);
        }
    }
}

fn decode_chunks(raw: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    let mut rest = raw;
    while let Some(end) = find_bytes(rest, b"\r\n") {
        let Ok(size) =
            usize::from_str_radix(std::str::from_utf8(&rest[..end]).unwrap_or("0").trim(), 16)
        else {
            break;
        };
        if size == 0 {
            break;
        }
        let start = end + 2;
        if rest.len() < start + size + 2 {
            break;
        }
        decoded.extend_from_slice(&rest[start..start + size]);
        rest = &rest[start + size + 2..];
    }
    decoded
}

fn header_value(headers: &str, name: &str) -> Option<String> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

async fn read_response(stream: &mut TcpStream, headers: String, initial: Vec<u8>) -> RawResponse {
    let status = headers.lines().next().unwrap_or("HTTP/1.1 000").to_owned();
    let body = if let Some(length) = header_value(&headers, "content-length") {
        let length: usize = length.parse().unwrap();
        let mut body = initial;
        let mut chunk = [0u8; 4096];
        while body.len() < length {
            let read = timeout(StdDuration::from_secs(5), stream.read(&mut chunk))
                .await
                .expect("server answered body")
                .expect("read ok");
            assert!(read > 0, "connection closed before full body");
            body.extend_from_slice(&chunk[..read]);
        }
        body.truncate(length);
        body
    } else {
        let mut raw = initial;
        let mut chunk = [0u8; 4096];
        loop {
            if find_bytes(&raw, b"0\r\n\r\n").is_some() {
                break;
            }
            let read = timeout(StdDuration::from_secs(5), stream.read(&mut chunk))
                .await
                .expect("server answered streamed body")
                .unwrap_or(0);
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&chunk[..read]);
        }
        decode_chunks(&raw)
    };
    RawResponse {
        status,
        headers,
        body,
    }
}

fn http_request(
    host: &str,
    method: &str,
    path: &str,
    extra: &[(&str, &str)],
    body: &[u8],
) -> String {
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: keep-alive\r\n");
    for (name, value) in extra {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    if !body.is_empty() {
        request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
        request.push_str(std::str::from_utf8(body).expect("UTF-8 test body"));
    } else {
        request.push_str("\r\n");
    }
    request
}

async fn request_over_tcp(addr: SocketAddr, request: String) -> RawResponse {
    let mut stream = TcpStream::connect(addr).await.expect("loopback connect");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("request written");
    let (headers, initial) = read_until_headers(&mut stream).await;
    if !headers.starts_with("HTTP/1.1") {
        panic!("invalid response headers for {request:?}: {headers:?}");
    }
    read_response(&mut stream, headers, initial).await
}

fn session_token_from_page(html: &str) -> String {
    let start = html.find("vses_").expect("session token embedded in page") + 5;
    let hex = &html[start..];
    let length = hex
        .find(|character: char| !character.is_ascii_hexdigit())
        .unwrap_or(hex.len());
    format!("vses_{}", &hex[..length])
}

fn cookie_from_headers(headers: &str) -> String {
    header_value(headers, "set-cookie")
        .and_then(|cookie| {
            cookie
                .split(';')
                .next()
                .and_then(|pair| pair.strip_prefix("hacienda_viewer_session="))
                .map(str::to_owned)
        })
        .expect("session cookie")
}

fn sse_events(decoded: &str) -> Vec<Value> {
    decoded
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str(data).ok())
        .collect()
}

#[tokio::test]
async fn real_loopback_http_links_codex_turn_to_viewer_rehydration() {
    let resolver = EnvKeyResolver::with_lookup(|name| match name {
        "HACIENDA_PSEUDONYM_ACTIVE_KEY" => Some("k1".to_owned()),
        "HACIENDA_PSEUDONYM_KEY_K1" => Some("07".repeat(64)),
        _ => None,
    });
    let config = HaciendaConfig::default().with_pii(PipelineConfig {
        redaction: RedactionConfig {
            mode: RedactionMode::Pseudonymize,
            ..Default::default()
        },
        ..Default::default()
    });
    let facade =
        Arc::new(HaciendaFacade::with_key_resolver(config, Arc::new(resolver), &[]).unwrap());
    let provider = RecordingProvider::default();
    let viewer_assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../viewer");
    let api_state = ApiState::new(
        facade,
        InMemoryJobStore::new().into_arc(),
        AuthState::new(Arc::new(DevTokenResolver)),
        ApiLimits::default(),
    )
    .with_codex_responses_provider(Arc::new(provider.clone()))
    .with_codex_gateway_secret(LOCAL_SECRET)
    .with_viewer_assets(viewer_assets);

    let sessions = api_state.viewer_sessions().clone();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback bind");
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(api_state))
            .await
            .expect("serve");
    });
    let host = addr.to_string();

    // 1. Real GET /viewer: capture the HttpOnly cookie and embedded page token.
    let page = request_over_tcp(addr, http_request(&host, "GET", "/viewer", &[], &[])).await;
    assert!(page.status.starts_with("HTTP/1.1 200"), "{}", page.status);
    assert!(page
        .headers
        .to_ascii_lowercase()
        .contains("content-security-policy: default-src 'self'"));
    let cookie = cookie_from_headers(&page.headers);
    let token = session_token_from_page(&String::from_utf8_lossy(&page.body));
    assert!(cookie.ends_with(&token));
    assert!(
        sessions.activate(&token).is_ok(),
        "the minted session must exist in the shared store"
    );

    // 2. Real GET /viewer/events with the cookie + query token.
    let events_path = format!("/viewer/events?token={token}");
    let mut sse_stream = TcpStream::connect(addr)
        .await
        .expect("loopback SSE connect");
    let cookie_header = format!("hacienda_viewer_session={cookie}");
    sse_stream
        .write_all(
            http_request(
                &host,
                "GET",
                &events_path,
                &[("Cookie", &cookie_header)],
                &[],
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let (sse_headers, sse_initial) = read_until_headers(&mut sse_stream).await;
    assert!(
        sse_headers.starts_with("HTTP/1.1 200"),
        "token={token} (len {}), cookie={cookie} (len {}), headers={sse_headers}",
        token.len(),
        cookie.len()
    );

    // 3. Real POST /v1/responses with the original PII and the local bearer.
    let body = request().to_string();
    let codex_response = request_over_tcp(
        addr,
        http_request(
            &host,
            "POST",
            "/v1/responses",
            &[
                ("X-Hacienda-Gateway-Token", LOCAL_SECRET),
                ("Authorization", CHATGPT_AUTH),
                ("ChatGPT-Account-ID", ACCOUNT_ID),
                ("Content-Type", "application/json"),
            ],
            body.as_bytes(),
        ),
    )
    .await;
    assert!(
        codex_response.status.starts_with("HTTP/1.1 200"),
        "{}",
        codex_response.status
    );
    let codex_body = String::from_utf8_lossy(&codex_response.body);
    assert!(
        !codex_body.contains(EMAIL),
        "Codex HTTP response leaked PII"
    );
    assert!(codex_body.contains("response.completed"));

    // 4. The viewer SSE must receive the original mapping and tokenized text
    //    for the same turn, while the provider only ever saw pseudonyms.
    let mut raw_sse = sse_initial;
    let mut chunk = [0u8; 4096];
    loop {
        let events = sse_events(&String::from_utf8_lossy(&decode_chunks(&raw_sse)));
        if events.iter().any(|event| event["type"] == "mapping")
            && events.iter().any(|event| event["type"] == "text")
        {
            break;
        }
        let read = timeout(Duration::from_secs(5), sse_stream.read(&mut chunk))
            .await
            .expect("viewer SSE answered")
            .unwrap_or(0);
        if read == 0 {
            break;
        }
        raw_sse.extend_from_slice(&chunk[..read]);
    }
    let events = sse_events(&String::from_utf8_lossy(&decode_chunks(&raw_sse)));
    let mappings: Vec<_> = events
        .iter()
        .filter(|event| event["type"] == "mapping")
        .collect();
    let texts: Vec<_> = events
        .iter()
        .filter(|event| event["type"] == "text")
        .collect();
    assert!(!mappings.is_empty(), "viewer SSE carried no mapping event");
    assert!(!texts.is_empty(), "viewer SSE carried no text event");
    assert!(mappings.iter().any(|event| event["original"] == EMAIL));
    assert!(mappings.iter().any(|event| event["token"]
        .as_str()
        .is_some_and(|t| t.starts_with("[EMAIL:k1:"))));
    assert!(texts.iter().any(|event| event["text"]
        .as_str()
        .is_some_and(|t| t.contains("[EMAIL:k1:"))));
    assert!(texts
        .iter()
        .all(|event| !event["text"].as_str().unwrap_or_default().contains(EMAIL)));
    let mut turn_ids: Vec<String> = mappings
        .iter()
        .chain(texts.iter())
        .filter_map(|event| event["turn_id"].as_str().map(str::to_owned))
        .collect();
    turn_ids.sort();
    turn_ids.dedup();
    assert_eq!(turn_ids.len(), 1, "mapping and text must share a turn id");

    let upstream = serde_json::to_string(&provider.bodies.lock().unwrap()[0]).unwrap();
    assert!(!upstream.contains(EMAIL), "provider received original PII");
    assert!(upstream.contains("[EMAIL:k1:"));
    assert_eq!(provider.auths.lock().unwrap()[0], CHATGPT_AUTH);
    assert_eq!(provider.accounts.lock().unwrap()[0], ACCOUNT_ID);
    assert!(!upstream.contains(LOCAL_SECRET));

    // 5. Static assets actually served over the same loopback origin.
    let css = request_over_tcp(
        addr,
        http_request(&host, "GET", "/viewer/viewer.css", &[], &[]),
    )
    .await;
    assert!(css.status.starts_with("HTTP/1.1 200"));
    let css = String::from_utf8_lossy(&css.body);
    assert!(css.contains("--entity-gray: #9ca3af"));
    assert!(css.contains(".output .entity"));
    assert!(css.contains("color: var(--entity-gray)"));
    let js = request_over_tcp(
        addr,
        http_request(&host, "GET", "/viewer/viewer.js", &[], &[]),
    )
    .await;
    assert!(js.status.starts_with("HTTP/1.1 200"));
    let js = String::from_utf8_lossy(&js.body);
    assert!(js.contains("data.type === 'mapping'"));
    assert!(js.contains("data.type === 'text'"));

    server.abort();
}
