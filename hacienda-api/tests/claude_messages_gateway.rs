//! Loopback contract for the Anthropic Messages gateway used by Claude Code.
//!
//! These tests drive the real router (auth middleware included) with a scripted
//! provider. They cover the streaming contract Claude Code depends on — frames
//! arrive event by event, pings are not buffered, the full lifecycle reaches
//! `message_stop`, and a malformed stream ends in an error frame — while
//! proving that text split across provider deltas is redacted as one logical
//! block and never sent upstream in the clear.

use std::{
    sync::{Arc, Mutex},
    time::Duration as StdDuration,
};

use async_trait::async_trait;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use futures::stream;
use hacienda_api::{
    router, ApiLimits, ApiState, ClaudeMessagesProvider, ClaudeMessagesProviderRequest,
    ClaudeMessagesProviderResponse, ClaudeMessagesProviderStream, ProviderGatewayError,
    ProviderSseEvent,
};
use hacienda_core::{
    auth::{authn::DevTokenResolver, AuthState},
    jobs::InMemoryJobStore,
    pii::PipelineConfig,
    HaciendaConfig, HaciendaFacade,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
    time::{timeout, Duration},
};
use tower::ServiceExt;

const EMAIL: &str = "alice@example.com";
const AUTH: &str = "Bearer hcd_documents:process_testsuffix";

#[derive(Clone)]
struct ScriptedProvider {
    bodies: Arc<Mutex<Vec<Value>>>,
    script: Vec<ProviderSseEvent>,
    gated: bool,
    /// When `gated`, the stream pauses before its first delta and waits for the
    /// test to release it. That makes incremental relay observable without
    /// relying on wall-clock timing.
    gate: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl ScriptedProvider {
    fn new(script: Vec<ProviderSseEvent>) -> Self {
        Self {
            bodies: Arc::new(Mutex::new(Vec::new())),
            script,
            gated: false,
            gate: Arc::new(Mutex::new(None)),
        }
    }

    fn gated(mut self) -> Self {
        self.gated = true;
        self
    }
}

#[async_trait]
impl ClaudeMessagesProvider for ScriptedProvider {
    async fn stream(
        &self,
        request: ClaudeMessagesProviderRequest,
    ) -> Result<ClaudeMessagesProviderStream, ProviderGatewayError> {
        self.bodies
            .lock()
            .expect("body lock")
            .push(request.body.clone());
        let gate = if self.gated {
            let (tx, rx) = oneshot::channel::<()>();
            *self.gate.lock().expect("gate lock") = Some(tx);
            Some(rx)
        } else {
            None
        };
        let script = self.script.clone();
        let (tx, rx) = mpsc::channel(32);
        tokio::spawn(async move {
            let mut gate = gate;
            for (index, event) in script.into_iter().enumerate() {
                if index == 2 {
                    if let Some(gate) = gate.take() {
                        let _ = gate.await;
                    }
                }
                if tx.send(Ok(event)).await.is_err() {
                    break;
                }
            }
        });
        let events = stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });
        Ok(ClaudeMessagesProviderStream {
            response: ClaudeMessagesProviderResponse {
                status: 200,
                headers: Vec::new(),
            },
            events: Box::pin(events),
        })
    }
}

fn state(provider: ScriptedProvider) -> ApiState {
    let facade = Arc::new(
        HaciendaFacade::new(HaciendaConfig::default().with_pii(PipelineConfig::default()))
            .expect("PII facade"),
    );
    ApiState::new(
        facade,
        InMemoryJobStore::new().into_arc(),
        AuthState::new(Arc::new(DevTokenResolver)),
        ApiLimits::default(),
    )
    .with_claude_messages_provider(Arc::new(provider))
}

fn event(kind: &str, data: Value) -> ProviderSseEvent {
    ProviderSseEvent::new(Some(kind.to_owned()), None, data.to_string())
}

fn message_start() -> ProviderSseEvent {
    event(
        "message_start",
        json!({
            "type": "message_start",
            "message": {
                "id": "msg_1", "type": "message", "role": "assistant",
                "model": "claude-code", "content": [], "stop_reason": null,
                "stop_sequence": null,
                "usage": {"input_tokens": 1, "output_tokens": 0}
            }
        }),
    )
}

fn message_end() -> Vec<ProviderSseEvent> {
    vec![
        event(
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                "usage": {"output_tokens": 2}
            }),
        ),
        event("message_stop", json!({"type": "message_stop"})),
    ]
}

fn text_block(deltas: &[&str]) -> Vec<ProviderSseEvent> {
    let mut events = vec![
        message_start(),
        event(
            "content_block_start",
            json!({
                "type": "content_block_start", "index": 0,
                "content_block": {"type": "text", "text": ""}
            }),
        ),
    ];
    for text in deltas {
        events.push(event(
            "content_block_delta",
            json!({
                "type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": text}
            }),
        ));
    }
    events.push(event(
        "content_block_stop",
        json!({"type": "content_block_stop", "index": 0}),
    ));
    events.extend(message_end());
    events
}

fn request() -> Value {
    json!({
        "model": "claude-code",
        "max_tokens": 64,
        "stream": true,
        "messages": [{
            "role": "user",
            "content": [{"type": "text", "text": format!("Please email {EMAIL}")}]
        }]
    })
}

async fn post(app: &Router, body: Value) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .header("authorization", AUTH)
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("router answered")
}

async fn sse_body(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("SSE body");
    String::from_utf8(bytes.to_vec()).expect("UTF-8 SSE")
}

fn assert_frames_in_order(sse: &str, frames: &[&str]) {
    let mut cursor = 0;
    for frame in frames {
        let next = sse[cursor..].find(frame).expect("frame in order");
        cursor += next + frame.len();
    }
}

// ── Real loopback HTTP helpers for the incremental-delivery test ─────────────

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

async fn read_until(stream: &mut TcpStream, raw: &mut Vec<u8>, needle: &[u8]) {
    let mut chunk = [0_u8; 4096];
    while find_bytes(raw, needle).is_none() {
        let read = timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .expect("expected SSE frame arrived while upstream was gated")
            .expect("read ok");
        assert!(read > 0, "stream closed before expected frame");
        raw.extend_from_slice(&chunk[..read]);
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

async fn read_all(stream: &mut TcpStream, mut raw: Vec<u8>) -> String {
    let mut chunk = [0_u8; 4096];
    while find_bytes(&raw, b"0\r\n\r\n").is_none() {
        let read = timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .expect("stream ended")
            .unwrap_or(0);
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
    }
    String::from_utf8(decode_chunks(&raw)).expect("UTF-8 SSE")
}

#[tokio::test]
async fn gateway_relays_chunked_frames_before_the_upstream_finishes() {
    let provider = ScriptedProvider::new(text_block(&["alice@", "example.com"])).gated();
    let gate = Arc::clone(&provider.gate);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("listener address");
    tokio::spawn(async move {
        axum::serve(listener, router(state(provider)))
            .await
            .expect("serve gateway");
    });

    let mut stream = TcpStream::connect(addr).await.expect("connect gateway");
    let payload = request().to_string();
    let request = format!(
        "POST /v1/messages HTTP/1.1\r\nhost: {addr}\r\nauthorization: {AUTH}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("send request");

    let (headers, mut raw) = read_headers(&mut stream).await;
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("content-type: text/event-stream"),
        "{headers}"
    );
    // The provider is parked before its first delta. If the gateway buffered
    // the response, this read would block until the gate opens — it does not.
    read_until(&mut stream, &mut raw, b"event: content_block_start").await;
    let early = String::from_utf8_lossy(&raw);
    assert!(early.contains("event: message_start"));
    assert!(!early.contains("event: message_stop"));

    gate.lock()
        .expect("gate lock")
        .take()
        .expect("provider installed gate")
        .send(())
        .expect("release gate");

    let sse = read_all(&mut stream, raw).await;
    assert!(!sse.contains(EMAIL), "client received a real value");
    assert!(sse.contains("[EMAIL]"));
    assert!(sse.contains("event: message_delta"));
    assert!(sse.contains("event: message_stop"));
    let start = sse.find("event: message_start").expect("message_start");
    let stop = sse.find("event: message_stop").expect("message_stop");
    assert!(start < stop);
}

#[tokio::test]
async fn split_pii_is_redacted_before_any_delta_and_upstream_stays_token_only() {
    let provider = ScriptedProvider::new(text_block(&["alice@", "example.com"]));
    let app = router(state(provider.clone()));
    let response = post(&app, request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/event-stream",
        "Claude Code requires an SSE content type"
    );
    let sse = sse_body(response).await;
    assert!(!sse.contains(EMAIL), "SSE client received a real value");
    assert!(sse.contains("[EMAIL]"));
    assert!(sse.contains("event: message_stop"));
    assert_frames_in_order(
        &sse,
        &[
            "event: message_start",
            "event: content_block_start",
            "event: content_block_delta",
            "event: content_block_stop",
            "event: message_delta",
            "event: message_stop",
        ],
    );

    let bodies = provider.bodies.lock().expect("bodies lock");
    assert_eq!(bodies.len(), 1);
    let upstream = serde_json::to_string(&bodies[0]).expect("upstream JSON");
    assert!(!upstream.contains(EMAIL), "provider received a real value");
}

#[tokio::test]
async fn keepalive_comments_are_relayed_as_content_free_frames() {
    let mut script = text_block(&["safe reply"]);
    let keepalive = ProviderSseEvent {
        event: None,
        id: None,
        retry: None,
        comment: Some("ping - 1763990000".to_owned()),
        data: String::new(),
    };
    script.insert(1, keepalive);
    let app = router(state(ScriptedProvider::new(script)));
    let response = post(&app, request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let sse = sse_body(response).await;
    assert!(sse.contains(": ping - 1763990000"));
    assert!(sse.contains("safe reply"));
}

#[tokio::test]
async fn tool_and_thinking_blocks_are_redacted_as_whole_logical_texts() {
    let mut script = vec![message_start()];
    script.push(event(
        "content_block_start",
        json!({
            "type": "content_block_start", "index": 0,
            "content_block": {
                "type": "tool_use", "id": "tool-1",
                "name": EMAIL, "input": {}
            }
        }),
    ));
    script.push(event(
        "content_block_delta",
        json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": "{\"email\":\"alice@"}
        }),
    ));
    script.push(event(
        "content_block_delta",
        json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": "example.com\"}"}
        }),
    ));
    script.push(event(
        "content_block_stop",
        json!({"type": "content_block_stop", "index": 0}),
    ));
    script.push(event(
        "content_block_start",
        json!({
            "type": "content_block_start", "index": 1,
            "content_block": {"type": "thinking", "thinking": "", "signature": ""}
        }),
    ));
    script.push(event(
        "content_block_delta",
        json!({
            "type": "content_block_delta", "index": 1,
            "delta": {"type": "thinking_delta", "thinking": "note alice@"}
        }),
    ));
    script.push(event(
        "content_block_delta",
        json!({
            "type": "content_block_delta", "index": 1,
            "delta": {"type": "signature_delta", "signature": "c2ln"}
        }),
    ));
    script.push(event(
        "content_block_delta",
        json!({
            "type": "content_block_delta", "index": 1,
            "delta": {"type": "thinking_delta", "thinking": "example.com"}
        }),
    ));
    script.push(event(
        "content_block_stop",
        json!({"type": "content_block_stop", "index": 1}),
    ));
    script.extend(message_end());

    let provider = ScriptedProvider::new(script);
    let app = router(state(provider.clone()));
    let response = post(&app, request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let sse = sse_body(response).await;
    assert!(!sse.contains(EMAIL), "client received a real value");
    assert!(sse.contains("[EMAIL]"));
    assert!(sse.contains("\"signature\":\"c2ln\""));

    let mut partial = String::new();
    let mut thinking = String::new();
    for line in sse.lines().filter_map(|line| line.strip_prefix("data: ")) {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            if let Some(json) = value.pointer("/delta/partial_json").and_then(Value::as_str) {
                partial.push_str(json);
            }
            if let Some(text) = value.pointer("/delta/thinking").and_then(Value::as_str) {
                thinking.push_str(text);
            }
        }
    }
    assert!(
        serde_json::from_str::<Value>(&partial).is_ok_and(|value| value.is_object()),
        "tool JSON must remain valid: {partial}"
    );
    assert!(partial.contains("[EMAIL]"));
    assert!(thinking.contains("[EMAIL]"));

    let bodies = provider.bodies.lock().expect("bodies lock");
    assert!(!serde_json::to_string(&bodies[0])
        .expect("upstream")
        .contains(EMAIL));
}

#[tokio::test]
async fn unknown_events_and_plaintext_fail_closed_with_an_error_frame() {
    let mut script = vec![message_start()];
    script.push(event(
        "content_block_start",
        json!({
            "type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}
        }),
    ));
    script.push(event(
        "content_block_delta",
        json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": EMAIL}
        }),
    ));
    script.push(event(
        "future_debug",
        json!({"type": "future_debug", "text": EMAIL}),
    ));
    script.extend(message_end());

    let app = router(state(ScriptedProvider::new(script)));
    let response = post(&app, request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let sse = sse_body(response).await;
    assert!(!sse.contains(EMAIL), "unredacted text escaped");
    assert!(!sse.contains("event: content_block_delta"));
    assert!(sse.contains("event: error"));
    assert!(sse.contains("The provider response was invalid."));
}
