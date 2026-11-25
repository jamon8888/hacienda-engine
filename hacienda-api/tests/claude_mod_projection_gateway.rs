//! Loopback contract for the authenticated Claude Mod projection channel.
//!
//! These tests drive the real router over a real loopback TCP socket. They prove
//! that a mapping minted while redacting `/v1/messages` reaches exactly the
//! bootstrapped Mod (never the provider), and that a wrong capability, an
//! expired session, or a different principal all fail closed.

use std::{
    sync::{Arc, Mutex},
    time::Duration as StdDuration,
};

use async_trait::async_trait;
use futures::stream;
use hacienda_api::{
    router, ApiLimits, ApiState, ClaudeMessagesProvider, ClaudeMessagesProviderRequest,
    ClaudeMessagesProviderResponse, ClaudeMessagesProviderStream, ModSessionStore,
    ProviderGatewayError, ProviderSseEvent, CAPABILITY_HEADER, CORRELATION_HEADER,
};
use hacienda_core::{
    auth::{
        authn::{InMemoryTokenStore, Token},
        AuthState, CapabilitySet,
    },
    jobs::InMemoryJobStore,
    pii::PipelineConfig,
    redaction::{EnvKeyResolver, RedactionConfig, RedactionMode},
    HaciendaConfig, HaciendaFacade,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{timeout, Duration},
};

const EMAIL: &str = "alice@example.com";
const CORRELATION: &str = "hcm_loopback_test";
const AUTH_ALICE: &str = "Bearer alice-secret";
const AUTH_BOB: &str = "Bearer bob-secret";

#[derive(Clone)]
struct RecordingProvider {
    bodies: Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl ClaudeMessagesProvider for RecordingProvider {
    async fn stream(
        &self,
        request: ClaudeMessagesProviderRequest,
    ) -> Result<ClaudeMessagesProviderStream, ProviderGatewayError> {
        self.bodies
            .lock()
            .expect("body lock")
            .push(request.body.clone());
        let events = vec![
            ProviderSseEvent::new(
                Some("message_start".to_owned()),
                None,
                json!({
                    "type": "message_start",
                    "message": {
                        "id": "msg_1", "type": "message", "role": "assistant",
                        "model": "claude-code", "content": [], "stop_reason": null,
                        "stop_sequence": null,
                        "usage": {"input_tokens": 1, "output_tokens": 0}
                    }
                })
                .to_string(),
            ),
            ProviderSseEvent::new(
                Some("content_block_start".to_owned()),
                None,
                json!({
                    "type": "content_block_start", "index": 0,
                    "content_block": {"type": "text", "text": ""}
                })
                .to_string(),
            ),
            ProviderSseEvent::new(
                Some("content_block_delta".to_owned()),
                None,
                json!({
                    "type": "content_block_delta", "index": 0,
                    "delta": {"type": "text_delta", "text": format!("Contact {EMAIL}")}
                })
                .to_string(),
            ),
            ProviderSseEvent::new(
                Some("content_block_stop".to_owned()),
                None,
                json!({"type": "content_block_stop", "index": 0}).to_string(),
            ),
            ProviderSseEvent::new(
                Some("message_delta".to_owned()),
                None,
                json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                    "usage": {"output_tokens": 2}
                })
                .to_string(),
            ),
            ProviderSseEvent::new(
                Some("message_stop".to_owned()),
                None,
                json!({"type": "message_stop"}).to_string(),
            ),
        ];
        let stream = stream::iter(events.into_iter().map(Ok));
        Ok(ClaudeMessagesProviderStream {
            response: ClaudeMessagesProviderResponse {
                status: 200,
                headers: Vec::new(),
            },
            events: Box::pin(stream),
        })
    }
}

fn state(provider: RecordingProvider) -> ApiState {
    let capabilities = CapabilitySet::new([
        hacienda_core::auth::Capability::DocumentsProcess,
        hacienda_core::auth::Capability::PiiReveal,
    ]);
    let mut tokens = InMemoryTokenStore::new();
    tokens.insert(
        "alice-secret",
        Token::new("alice-token", "alice", capabilities.clone()),
    );
    tokens.insert("bob-secret", Token::new("bob-token", "bob", capabilities));
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
    let facade = HaciendaFacade::with_key_resolver(config, Arc::new(resolver), &[])
        .expect("pseudonymizing facade");
    ApiState::new(
        Arc::new(facade),
        InMemoryJobStore::new().into_arc(),
        AuthState::new(Arc::new(tokens)),
        ApiLimits::default(),
    )
    .with_claude_messages_provider(Arc::new(provider))
}

// ── Minimal HTTP/1.1 client over a loopback socket ────────────────────────────

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn read_headers(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = timeout(StdDuration::from_secs(10), stream.read(&mut chunk))
            .await
            .expect("server answered headers")
            .expect("read ok");
        assert!(read > 0, "connection closed before response headers");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = find_bytes(&buffer, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buffer[..end + 4]).into_owned();
            return (headers, buffer[end + 4..].to_vec());
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

async fn read_json(stream: &mut TcpStream, raw: Vec<u8>) -> Value {
    let mut raw = raw;
    let mut chunk = [0_u8; 4096];
    while find_bytes(&raw, b"}").is_none() {
        let read = timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .expect("json body")
            .expect("read ok");
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
    }
    let bytes = if raw.starts_with(b"{") {
        raw
    } else {
        decode_chunks(&raw)
    };
    serde_json::from_slice(&bytes).expect("bootstrap JSON")
}

async fn read_until(stream: &mut TcpStream, raw: &mut Vec<u8>, needle: &[u8]) {
    let mut chunk = [0_u8; 4096];
    while find_bytes(&decode_chunks(raw), needle).is_none() {
        let read = timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .expect("expected frame arrived")
            .expect("read ok");
        assert!(read > 0, "stream closed before expected frame");
        raw.extend_from_slice(&chunk[..read]);
    }
}

async fn read_to_message_stop(stream: &mut TcpStream, mut raw: Vec<u8>) -> String {
    let mut chunk = [0_u8; 4096];
    while find_bytes(&decode_chunks(&raw), b"event: message_stop").is_none() {
        let read = timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .expect("messages stream ended")
            .unwrap_or(0);
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
    }
    String::from_utf8(decode_chunks(&raw)).expect("UTF-8 SSE")
}

/// Read a whole response body (the request sets `connection: close`), decoding
/// any transfer-encoding chunks. Used for JSON bodies with nested objects, where
/// stopping at the first `}` would parse an incomplete value.
async fn read_whole_body(stream: &mut TcpStream, headers: &str, mut raw: Vec<u8>) -> Vec<u8> {
    let mut chunk = [0_u8; 8192];
    loop {
        let read = timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .expect("response body")
            .unwrap_or(0);
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
    }
    // A JSON response normally carries content-length, not chunked encoding.
    if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        decode_chunks(&raw)
    } else {
        raw
    }
}

async fn send(host: u16, request: String) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", host))
        .await
        .expect("connect gateway");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("send request");
    stream
}

async fn serve_with_state(api_state: ApiState) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("address");
    // The /mod/* handlers require the real TCP peer via `ConnectInfo`; without
    // this service conversion the extractor fails closed and every Mod route
    // would 500. Existing non-Mod routes are unaffected.
    let app = router(api_state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve gateway");
    });
    addr.port()
}

async fn serve(provider: RecordingProvider) -> u16 {
    serve_with_state(state(provider)).await
}

fn loopback_headers(port: u16, auth: &str, extra: &str) -> String {
    format!(
        "host: 127.0.0.1:{port}\r\norigin: http://127.0.0.1:{port}\r\nauthorization: {auth}\r\n{CORRELATION_HEADER}: {CORRELATION}\r\n{extra}"
    )
}

async fn bootstrap(port: u16, auth: &str) -> Value {
    let headers = loopback_headers(port, auth, "content-type: application/json\r\n");
    let request = format!(
        "POST /mod/session HTTP/1.1\r\n{headers}content-length: 2\r\nconnection: close\r\n\r\n{{}}"
    );
    let mut stream = send(port, request).await;
    let (response, body) = read_headers(&mut stream).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    read_json(&mut stream, body).await
}

fn messages_request(body: &str, port: u16, auth: &str) -> String {
    let headers = loopback_headers(port, auth, "content-type: application/json\r\n");
    format!(
        "POST /v1/messages HTTP/1.1\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn messages_body() -> String {
    json!({
        "model": "claude-code",
        "max_tokens": 64,
        "stream": true,
        "messages": [{
            "role": "user",
            "content": [{"type": "text", "text": format!("Please email {EMAIL}")}]
        }]
    })
    .to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn local_mapping_reaches_mod_while_provider_only_sees_tokens() {
    let provider = RecordingProvider {
        bodies: Arc::new(Mutex::new(Vec::new())),
    };
    let provider_view = provider.clone();
    let port = serve(provider).await;
    let bootstrap = bootstrap(port, AUTH_ALICE).await;
    let session = bootstrap["sessionId"].as_str().unwrap().to_owned();
    let capability = bootstrap["capability"].as_str().unwrap().to_owned();
    assert_eq!(
        bootstrap["origin"].as_str().unwrap(),
        format!("http://127.0.0.1:{port}")
    );

    let extra = format!("{CAPABILITY_HEADER}: {capability}\r\naccept: text/event-stream\r\n");
    let headers = loopback_headers(port, AUTH_ALICE, &extra);
    let mut events = send(
        port,
        format!("GET /mod/events?session={session} HTTP/1.1\r\n{headers}connection: close\r\n\r\n"),
    )
    .await;
    let (events_response, mut events_body) = read_headers(&mut events).await;
    assert!(
        events_response.starts_with("HTTP/1.1 200"),
        "{events_response}"
    );

    let mut messages = send(port, messages_request(&messages_body(), port, AUTH_ALICE)).await;
    let (messages_response, messages_raw) = read_headers(&mut messages).await;
    assert!(
        messages_response.starts_with("HTTP/1.1 200"),
        "{messages_response}"
    );
    let sse = read_to_message_stop(&mut messages, messages_raw).await;
    assert!(!sse.contains(EMAIL), "client received a real value");
    let token = {
        let text = sse
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter_map(|value| value["delta"]["text"].as_str().map(str::to_owned))
            .collect::<String>();
        let start = text.find('[').expect("assistant token in SSE");
        let end = text[start..].find(']').expect("assistant token in SSE") + start;
        text[start..=end].to_owned()
    };
    assert!(token.contains("EMAIL") && token.contains(']'), "{token}");

    read_until(&mut events, &mut events_body, b"\"type\":\"Mapping\"").await;
    let frame = String::from_utf8(decode_chunks(&events_body)).expect("UTF-8 mapping frame");
    let data = frame
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .expect("mapping data line");
    let mapping: Value = serde_json::from_str(data).expect("mapping JSON");
    assert_eq!(mapping["type"], "Mapping");
    assert_eq!(mapping["sessionId"], session);
    assert_eq!(mapping["token"], token);
    assert_eq!(mapping["original"], EMAIL);
    assert_eq!(mapping["capability"], capability);

    let bodies = provider_view.bodies.lock().expect("provider bodies");
    let upstream = serde_json::to_string(&bodies[0]).expect("upstream JSON");
    assert!(!upstream.contains(EMAIL), "provider received a real value");
    assert!(upstream.contains(&token), "provider received the token");
}

#[tokio::test]
async fn wrong_capability_and_cross_principal_sessions_are_refused() {
    let port = serve(RecordingProvider {
        bodies: Arc::new(Mutex::new(Vec::new())),
    })
    .await;
    let bootstrap = bootstrap(port, AUTH_ALICE).await;
    let session = bootstrap["sessionId"].as_str().unwrap();
    let capability = bootstrap["capability"].as_str().unwrap();

    let wrong = format!(
        "{CAPABILITY_HEADER}: {}",
        "cmcap_".to_owned() + &"c".repeat(64)
    );
    let headers = loopback_headers(port, AUTH_ALICE, &wrong);
    let mut stream = send(
        port,
        format!("GET /mod/events?session={session} HTTP/1.1\r\n{headers}connection: close\r\n\r\n"),
    )
    .await;
    let (response, _) = read_headers(&mut stream).await;
    assert!(response.starts_with("HTTP/1.1 401"), "{response}");

    // A different principal with the same correlation must not receive the
    // mapping published by Alice's /v1/messages call.
    let mut bob_messages = send(port, messages_request(&messages_body(), port, AUTH_BOB)).await;
    let (response, body) = read_headers(&mut bob_messages).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let _ = read_to_message_stop(&mut bob_messages, body).await;

    let extra = format!("{CAPABILITY_HEADER}: {capability}\r\naccept: text/event-stream\r\n");
    let headers = loopback_headers(port, AUTH_ALICE, &extra);
    let mut alice_events = send(
        port,
        format!("GET /mod/events?session={session} HTTP/1.1\r\n{headers}connection: close\r\n\r\n"),
    )
    .await;
    let (response, mut events_body) = read_headers(&mut alice_events).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let mut chunk = [0_u8; 4096];
    let read = match timeout(Duration::from_millis(400), alice_events.read(&mut chunk)).await {
        Ok(Ok(read)) => read,
        Ok(Err(_)) | Err(_) => 0,
    };
    events_body.extend_from_slice(&chunk[..read]);
    let frame = String::from_utf8(decode_chunks(&events_body)).expect("UTF-8");
    assert!(!frame.contains(EMAIL), "cross-principal mapping leaked");
    assert!(!frame.contains("\"type\":\"Mapping\""));
}

#[tokio::test]
async fn expired_and_revoked_sessions_fail_closed() {
    let port = serve(RecordingProvider {
        bodies: Arc::new(Mutex::new(Vec::new())),
    })
    .await;
    let bootstrap = bootstrap(port, AUTH_ALICE).await;
    let session = bootstrap["sessionId"].as_str().unwrap().to_owned();
    let capability = bootstrap["capability"].as_str().unwrap().to_owned();

    let extra = format!("{CAPABILITY_HEADER}: {capability}\r\ncontent-type: application/json\r\n");
    let headers = loopback_headers(port, AUTH_ALICE, &extra);
    let revoke = format!(
        "POST /mod/revoke?session={session} HTTP/1.1\r\n{headers}content-length: 0\r\nconnection: close\r\n\r\n"
    );
    let mut stream = send(port, revoke).await;
    let (response, _) = read_headers(&mut stream).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");

    let headers = loopback_headers(
        port,
        AUTH_ALICE,
        &format!("{CAPABILITY_HEADER}: {capability}\r\n"),
    );
    let mut stream = send(
        port,
        format!("GET /mod/events?session={session} HTTP/1.1\r\n{headers}connection: close\r\n\r\n"),
    )
    .await;
    let (response, _) = read_headers(&mut stream).await;
    assert!(response.starts_with("HTTP/1.1 401"), "{response}");

    // A malformed correlation hint never mints a session.
    let request = format!(
        "POST /mod/session HTTP/1.1\r\nhost: 127.0.0.1:{port}\r\norigin: http://127.0.0.1:{port}\r\nauthorization: {AUTH_ALICE}\r\n{CORRELATION_HEADER}: bad value\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}"
    );
    let mut stream = send(port, request).await;
    let (response, _) = read_headers(&mut stream).await;
    assert!(response.starts_with("HTTP/1.1 401"), "{response}");
}

#[tokio::test]
async fn an_expired_session_cannot_subscribe() {
    let provider = RecordingProvider {
        bodies: Arc::new(Mutex::new(Vec::new())),
    };
    let port = serve_with_state(
        state(provider)
            .with_mod_sessions(Arc::new(ModSessionStore::new(Duration::from_millis(20)))),
    )
    .await;
    let bootstrap = bootstrap(port, AUTH_ALICE).await;
    let session = bootstrap["sessionId"].as_str().unwrap();
    let capability = bootstrap["capability"].as_str().unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;

    let headers = loopback_headers(
        port,
        AUTH_ALICE,
        &format!("{CAPABILITY_HEADER}: {capability}\r\n"),
    );
    let mut stream = send(
        port,
        format!("GET /mod/events?session={session} HTTP/1.1\r\n{headers}connection: close\r\n\r\n"),
    )
    .await;
    let (response, _) = read_headers(&mut stream).await;
    assert!(response.starts_with("HTTP/1.1 401"), "{response}");
}

#[tokio::test]
async fn non_loopback_requests_are_refused() {
    let port = serve(RecordingProvider {
        bodies: Arc::new(Mutex::new(Vec::new())),
    })
    .await;
    let request = format!(
        "POST /mod/session HTTP/1.1\r\nhost: evil.test:{port}\r\nauthorization: {AUTH_ALICE}\r\n{CORRELATION_HEADER}: {CORRELATION}\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}"
    );
    let mut stream = send(port, request).await;
    let (response, _) = read_headers(&mut stream).await;
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
}

#[tokio::test]
async fn mod_routes_never_redirect() {
    let port = serve(RecordingProvider {
        bodies: Arc::new(Mutex::new(Vec::new())),
    })
    .await;
    // Cross-origin redirects drop Authorization and custom headers, which
    // would silently turn an authenticated bootstrap into an anonymous one.
    for path in [
        "/mod/session/",
        "/mod/events/",
        "/mod/poll/",
        "/mod/revoke/",
    ] {
        let headers = loopback_headers(port, AUTH_ALICE, "");
        let request = format!("GET {path} HTTP/1.1\r\n{headers}connection: close\r\n\r\n");
        let mut stream = send(port, request).await;
        let (response, _) = read_headers(&mut stream).await;
        assert!(
            !response.starts_with("HTTP/1.1 3"),
            "{path} redirected: {response}"
        );
    }
}

#[tokio::test]
async fn bounded_poll_drains_mappings_and_reports_closed_after_revoke() {
    let port = serve(RecordingProvider {
        bodies: Arc::new(Mutex::new(Vec::new())),
    })
    .await;
    let bootstrap = bootstrap(port, AUTH_ALICE).await;
    let session = bootstrap["sessionId"].as_str().unwrap().to_owned();
    let capability = bootstrap["capability"].as_str().unwrap().to_owned();

    // No SSE subscriber is needed: the bounded poll queue is filled at publish
    // time and drained atomically by the one-shot endpoint.
    let mut messages = send(port, messages_request(&messages_body(), port, AUTH_ALICE)).await;
    let (response, body) = read_headers(&mut messages).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let _ = read_to_message_stop(&mut messages, body).await;

    let extra = format!("{CAPABILITY_HEADER}: {capability}\r\n");
    let headers = loopback_headers(port, AUTH_ALICE, &extra);
    let request = format!(
        "GET /mod/poll?session={session}&limit=32 HTTP/1.1\r\n{headers}accept: application/json\r\nconnection: close\r\n\r\n"
    );
    let mut poll = send(port, request).await;
    let (response, raw) = read_headers(&mut poll).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let body = read_whole_body(&mut poll, &response, raw).await;
    let value: Value = serde_json::from_slice(&body).expect("poll JSON");
    assert_eq!(value["expired"], false);
    assert_eq!(value["closed"], false);
    let events = value["events"].as_array().expect("poll events");
    assert!(!events.is_empty(), "poll must drain at least one mapping");
    assert_eq!(events[0]["type"], "Mapping");
    assert_eq!(events[0]["original"], EMAIL);
    assert_eq!(events[0]["sessionId"], session);
    assert_eq!(events[0]["capability"], capability);
    assert!(
        !format!("{value}").contains("__never__"),
        "body is content-free JSON"
    );

    // A second poll drains nothing and stays live.
    let request = format!(
        "GET /mod/poll?session={session}&limit=32 HTTP/1.1\r\n{headers}accept: application/json\r\nconnection: close\r\n\r\n"
    );
    let mut poll = send(port, request).await;
    let (response, raw) = read_headers(&mut poll).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let value: Value =
        serde_json::from_slice(&read_whole_body(&mut poll, &response, raw).await).unwrap();
    assert_eq!(value["events"].as_array().unwrap().len(), 0);

    // Revoke then poll: no originals replayed, closed is signalled.
    let extra = format!("{CAPABILITY_HEADER}: {capability}\r\ncontent-type: application/json\r\n");
    let headers = loopback_headers(port, AUTH_ALICE, &extra);
    let revoke = format!(
        "POST /mod/revoke?session={session} HTTP/1.1\r\n{headers}content-length: 0\r\nconnection: close\r\n\r\n"
    );
    let mut stream = send(port, revoke).await;
    let (response, _) = read_headers(&mut stream).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");

    let extra = format!("{CAPABILITY_HEADER}: {capability}\r\n");
    let headers = loopback_headers(port, AUTH_ALICE, &extra);
    let request = format!(
        "GET /mod/poll?session={session}&limit=32 HTTP/1.1\r\n{headers}accept: application/json\r\nconnection: close\r\n\r\n"
    );
    let mut poll = send(port, request).await;
    let (response, raw) = read_headers(&mut poll).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let value: Value =
        serde_json::from_slice(&read_whole_body(&mut poll, &response, raw).await).unwrap();
    assert_eq!(value["closed"], true);
    assert_eq!(value["expired"], false);
    assert_eq!(value["events"].as_array().unwrap().len(), 0);
    assert!(
        !format!("{value}").contains(EMAIL),
        "revoked original replayed"
    );
}
