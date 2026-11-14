//! Authenticated Anthropic Messages gateway for TeamOnPremise Claude Code.
//!
//! The handler accepts the Anthropic JSON shape only long enough to walk every
//! inspectable text-bearing field.  The provider receives a second JSON value whose
//! text fields have already passed through [`HaciendaFacade::redact_text_with_auth`].
//! There is intentionally no tenant field in the request type: tenant routing comes
//! from the authenticated [`Caller`] and is carried only in a server-generated header.
//!
//! Strict mode is structural rather than a caller option.  Unknown content blocks,
//! attachments, and fields which this module cannot inspect are rejected before the
//! provider seam is reached.  Provider SSE frames are similarly validated and their
//! text deltas are redacted as one logical response so a sensitive value split across
//! deltas cannot escape.

use std::{
    collections::HashSet,
    fmt,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use axum::{
    extract::State,
    http::{HeaderMap, header::HeaderName, request::Parts},
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::stream;
use hacienda_client_core::{
    ProviderHttpClient, ProviderHttpRequest, ProviderSseEvent, ReqwestHttpClient, TransportError,
};
use hacienda_core::{HaciendaError, HaciendaFacade, auth::Caller};
use serde_json::{Map, Value};

use crate::{
    error::ApiError,
    extract::Json as SafeJson,
    handlers::{caller_from_arc, extract_auth_context},
    state::ApiState,
};

const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";

/// Content-free provider failure.  The variant deliberately carries no endpoint,
/// credential, request body, tenant, or provider response text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderGatewayError {
    Transport,
    InvalidResponse,
    Unavailable,
}

impl fmt::Display for ProviderGatewayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Transport => "provider transport failed",
            Self::InvalidResponse => "provider response was invalid",
            Self::Unavailable => "provider is unavailable",
        })
    }
}

/// Redacted request handed to a Claude Code provider implementation.
///
/// `body` contains only tokenized text.  Implementations must not log or persist it.
#[derive(Clone)]
pub struct ClaudeMessagesProviderRequest {
    pub body: Value,
    pub headers: Vec<(String, String)>,
    pub tenant: String,
}

impl fmt::Debug for ClaudeMessagesProviderRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaudeMessagesProviderRequest")
            .field("body", &"<token-only>")
            .field("headers", &"<safe-routing-only>")
            .field("tenant", &"<authenticated>")
            .finish()
    }
}

/// Provider seam used by the API handler and by focused transport fixtures.
#[async_trait]
pub trait ClaudeMessagesProvider: Send + Sync {
    async fn stream(
        &self,
        request: ClaudeMessagesProviderRequest,
    ) -> Result<Vec<ProviderSseEvent>, ProviderGatewayError>;
}

/// Anthropic-compatible HTTP provider backed by Hacienda's existing provider HTTP seam.
///
/// The HTTP client is injected so tests can prove the provider payload is token-only
/// without opening a network connection.  Real deployments can use
/// [`Self::with_reqwest`].
pub struct AnthropicMessagesProvider<H> {
    endpoint: String,
    credential: String,
    http: Arc<Mutex<H>>,
}

impl<H> AnthropicMessagesProvider<H> {
    pub fn new(
        endpoint: impl Into<String>,
        credential: impl Into<String>,
        http: H,
    ) -> Result<Self, ProviderGatewayError> {
        let endpoint = endpoint.into();
        let parsed =
            reqwest::Url::parse(&endpoint).map_err(|_| ProviderGatewayError::Unavailable)?;
        let host = parsed.host_str().ok_or(ProviderGatewayError::Unavailable)?;
        let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
        if parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback) {
            return Err(ProviderGatewayError::Unavailable);
        }
        let credential = credential.into();
        if credential.trim().is_empty() {
            return Err(ProviderGatewayError::Unavailable);
        }
        Ok(Self {
            endpoint,
            credential,
            http: Arc::new(Mutex::new(http)),
        })
    }
}

impl AnthropicMessagesProvider<ReqwestHttpClient> {
    pub fn with_reqwest(
        endpoint: impl Into<String>,
        credential: impl Into<String>,
    ) -> Result<Self, ProviderGatewayError> {
        Self::new(
            endpoint,
            credential,
            ReqwestHttpClient::new().map_err(|_| ProviderGatewayError::Unavailable)?,
        )
    }
}

#[async_trait]
impl<H> ClaudeMessagesProvider for AnthropicMessagesProvider<H>
where
    H: ProviderHttpClient + Send + 'static,
{
    async fn stream(
        &self,
        request: ClaudeMessagesProviderRequest,
    ) -> Result<Vec<ProviderSseEvent>, ProviderGatewayError> {
        let body =
            serde_json::to_vec(&request.body).map_err(|_| ProviderGatewayError::Transport)?;
        let mut headers = vec![
            ("content-type".to_owned(), "application/json".to_owned()),
            ("accept".to_owned(), "text/event-stream".to_owned()),
            ("x-api-key".to_owned(), self.credential.clone()),
            ("x-hacienda-tenant-id".to_owned(), request.tenant),
        ];
        headers.extend(request.headers);
        let http_request = ProviderHttpRequest::new("POST", self.endpoint.clone(), headers, body);
        let http = Arc::clone(&self.http);
        tokio::task::spawn_blocking(move || {
            let mut decoder = SseDecoder::default();
            let mut status = None;
            let mut response_headers = Vec::new();
            let mut guard = http.lock().map_err(|_| ProviderGatewayError::Transport)?;
            guard
                .send_stream(
                    http_request,
                    &mut |code, headers| {
                        status = Some(code);
                        response_headers = headers;
                    },
                    &mut |bytes| decoder.push(bytes),
                )
                .map_err(|_| ProviderGatewayError::Transport)?;
            let status = status.ok_or(ProviderGatewayError::InvalidResponse)?;
            if !(200..300).contains(&status)
                || response_headers.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("content-type")
                        && !value.to_ascii_lowercase().starts_with("text/event-stream")
                })
            {
                return Err(ProviderGatewayError::InvalidResponse);
            }
            decoder.finish()
        })
        .await
        .map_err(|_| ProviderGatewayError::Transport)?
    }
}

/// `POST /v1/messages`.
pub async fn messages(
    State(state): State<ApiState>,
    parts: Parts,
    headers: HeaderMap,
    SafeJson(mut body): SafeJson<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let auth_context = extract_auth_context(&parts);
    let caller = caller_from_arc(&auth_context);
    caller
        .require(hacienda_core::auth::Capability::DocumentsProcess)
        .map_err(HaciendaError::from)
        .map_err(ApiError::from)?;

    let provider = state
        .claude_messages_provider
        .ok_or_else(|| ApiError::invalid_request("The provider gateway is not configured."))?;
    let text_paths = validate_and_collect_text_paths(&body)
        .map_err(|_| ApiError::invalid_request("The request contains unsupported content."))?;
    redact_paths(&mut body, &text_paths, &state.facade, caller).await?;

    let tenant = caller.tenant_ctx().tenant.as_str().to_owned();
    let provider_request = ClaudeMessagesProviderRequest {
        body,
        headers: safe_routing_headers(&headers)?,
        tenant,
    };
    let events = provider.stream(provider_request).await.map_err(|error| {
        tracing::error!(error = %error, "claude messages provider failed");
        ApiError::internal()
    })?;
    let events = redact_and_validate_provider_events(&events, &state.facade, caller).await?;
    let output = events.into_iter().map(|event| {
        let mut rendered = Event::default();
        if let Some(kind) = event.event {
            rendered = rendered.event(kind);
        }
        if let Some(id) = event.id {
            rendered = rendered.id(id);
        }
        rendered.data(event.data)
    });
    Ok(Sse::new(stream::iter(output.map(Ok))).keep_alive(KeepAlive::default()))
}

fn safe_routing_headers(headers: &HeaderMap) -> Result<Vec<(String, String)>, ApiError> {
    let mut safe = Vec::new();
    for name in ["anthropic-version", "anthropic-beta", "x-request-id"] {
        let header_name = HeaderName::from_static(name);
        let Some(value) = headers.get(&header_name) else {
            continue;
        };
        let value = value
            .to_str()
            .map_err(|_| ApiError::invalid_request("The request contains invalid headers."))?;
        if value.len() > 4096 || value.contains(['\r', '\n']) {
            return Err(ApiError::invalid_request(
                "The request contains invalid headers.",
            ));
        }
        safe.push((name.to_owned(), value.to_owned()));
    }
    if !safe.iter().any(|(name, _)| name == "anthropic-version") {
        safe.push((
            "anthropic-version".to_owned(),
            DEFAULT_ANTHROPIC_VERSION.to_owned(),
        ));
    }
    Ok(safe)
}

const TOP_LEVEL_FIELDS: &[&str] = &[
    "model",
    "messages",
    "max_tokens",
    "system",
    "metadata",
    "stop_sequences",
    "stream",
    "temperature",
    "top_p",
    "top_k",
    "tools",
    "tool_choice",
    "service_tier",
    "thinking",
    "output_config",
    "container",
    "context_management",
];

fn validate_and_collect_text_paths(body: &Value) -> Result<Vec<String>, ()> {
    let object = body.as_object().ok_or(())?;
    reject_unknown_keys(object, TOP_LEVEL_FIELDS)?;
    require_string(object, "model")?;
    if object
        .get("max_tokens")
        .and_then(Value::as_u64)
        .is_none_or(|tokens| tokens == 0)
    {
        return Err(());
    }
    if object.get("stream").and_then(Value::as_bool) != Some(true) {
        return Err(());
    }
    let mut paths = Vec::new();
    let messages = object.get("messages").and_then(Value::as_array).ok_or(())?;
    if messages.is_empty() {
        return Err(());
    }
    for (index, message) in messages.iter().enumerate() {
        let message = message.as_object().ok_or(())?;
        reject_unknown_keys(message, &["role", "content", "name"])?;
        let role = require_string(message, "role")?;
        if !matches!(role, "user" | "assistant") {
            return Err(());
        }
        if let Some(name) = message.get("name") {
            if !name.is_string() {
                return Err(());
            }
            paths.push(pointer(&["messages", &index.to_string(), "name"]));
        }
        let content = message.get("content").ok_or(())?;
        collect_content(
            content,
            vec!["messages".into(), index.to_string(), "content"],
            &mut paths,
        )?;
    }
    if let Some(system) = object.get("system") {
        collect_content(system, vec!["system".into()], &mut paths)?;
    }
    for key in [
        "metadata",
        "tools",
        "tool_choice",
        "thinking",
        "output_config",
        "container",
        "context_management",
        "stop_sequences",
        "service_tier",
    ] {
        if let Some(value) = object.get(key) {
            collect_all_strings(value, vec![key.into()], &mut paths)?;
        }
    }
    Ok(paths)
}

fn collect_content(
    value: &Value,
    mut path: Vec<String>,
    paths: &mut Vec<String>,
) -> Result<(), ()> {
    match value {
        Value::String(_) => paths.push(pointer(
            &path.iter().map(String::as_str).collect::<Vec<_>>(),
        )),
        Value::Array(blocks) => {
            for (index, block) in blocks.iter().enumerate() {
                let block_path = {
                    path.push(index.to_string());
                    let next = path.clone();
                    path.pop();
                    next
                };
                collect_block(block, block_path, paths)?;
            }
        }
        _ => return Err(()),
    }
    Ok(())
}

fn collect_block(value: &Value, path: Vec<String>, paths: &mut Vec<String>) -> Result<(), ()> {
    let object = value.as_object().ok_or(())?;
    reject_tenant_keys(object)?;
    let kind = require_string(object, "type")?;
    match kind {
        "text" => {
            reject_unknown_keys(object, &["type", "text", "cache_control"])?;
            require_string(object, "text")?;
            paths.push(pointer(
                &path
                    .iter()
                    .map(String::as_str)
                    .chain(["text"])
                    .collect::<Vec<_>>(),
            ));
            if let Some(cache) = object.get("cache_control") {
                validate_cache_control(cache)?;
            }
        }
        "thinking" => {
            reject_unknown_keys(object, &["type", "thinking", "signature"])?;
            require_string(object, "thinking")?;
            paths.push(pointer(
                &path
                    .iter()
                    .map(String::as_str)
                    .chain(["thinking"])
                    .collect::<Vec<_>>(),
            ));
            if let Some(signature) = object.get("signature") {
                if !signature.is_string() {
                    return Err(());
                }
            }
        }
        "tool_use" => {
            reject_unknown_keys(object, &["type", "id", "name", "input"])?;
            require_string(object, "id")?;
            require_string(object, "name")?;
            let input = object
                .get("input")
                .filter(|value| value.is_object())
                .ok_or(())?;
            collect_all_strings(input, path_with(&path, "input"), paths)?;
        }
        "tool_result" => {
            reject_unknown_keys(object, &["type", "tool_use_id", "content", "is_error"])?;
            require_string(object, "tool_use_id")?;
            if let Some(is_error) = object.get("is_error") {
                if !is_error.is_boolean() {
                    return Err(());
                }
            }
            let content = object.get("content").ok_or(())?;
            collect_content(content, path_with(&path, "content"), paths)?;
        }
        // Images, files, documents, audio, and future block types are intentionally
        // refused: this gateway cannot prove they are inspectable before egress.
        _ => return Err(()),
    }
    Ok(())
}

fn collect_all_strings(
    value: &Value,
    path: Vec<String>,
    paths: &mut Vec<String>,
) -> Result<(), ()> {
    match value {
        Value::String(_) => paths.push(pointer(
            &path.iter().map(String::as_str).collect::<Vec<_>>(),
        )),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                collect_all_strings(item, path_with(&path, &index.to_string()), paths)?;
            }
        }
        Value::Object(object) => {
            reject_tenant_keys(object)?;
            for (key, child) in object {
                collect_all_strings(child, path_with(&path, key), paths)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

fn validate_cache_control(value: &Value) -> Result<(), ()> {
    let object = value.as_object().ok_or(())?;
    reject_unknown_keys(object, &["type"])?;
    if !matches!(
        object.get("type").and_then(Value::as_str),
        Some("ephemeral")
    ) {
        return Err(());
    }
    Ok(())
}

fn reject_unknown_keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<(), ()> {
    reject_tenant_keys(object)?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(());
    }
    Ok(())
}

fn reject_tenant_keys(object: &Map<String, Value>) -> Result<(), ()> {
    if object
        .keys()
        .any(|key| key.to_ascii_lowercase().contains("tenant"))
    {
        return Err(());
    }
    Ok(())
}

fn require_string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, ()> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(())
}

fn path_with(path: &[String], child: &str) -> Vec<String> {
    let mut path = path.to_vec();
    path.push(child.to_owned());
    path
}

fn pointer(path: &[&str]) -> String {
    path.iter()
        .map(|segment| segment.replace('~', "~0").replace('/', "~1"))
        .fold(String::new(), |mut output, segment| {
            output.push('/');
            output.push_str(&segment);
            output
        })
}

async fn redact_paths(
    body: &mut Value,
    paths: &[String],
    facade: &HaciendaFacade,
    caller: Caller<'_>,
) -> Result<(), ApiError> {
    let mut seen = HashSet::new();
    for path in paths {
        if !seen.insert(path) {
            continue;
        }
        let text = body
            .pointer(path)
            .and_then(Value::as_str)
            .ok_or_else(ApiError::internal)?;
        let redacted = facade
            .redact_text_with_auth(caller, text)
            .await
            .map_err(ApiError::from)?
            .redacted_text;
        let slot = body.pointer_mut(path).ok_or_else(ApiError::internal)?;
        *slot = Value::String(redacted);
    }
    Ok(())
}

async fn redact_and_validate_provider_events(
    events: &[ProviderSseEvent],
    facade: &HaciendaFacade,
    caller: Caller<'_>,
) -> Result<Vec<ProviderSseEvent>, ApiError> {
    validate_provider_events(events).map_err(|_| ApiError::internal())?;
    let mut output = events.to_vec();
    let mut text_slots = Vec::new();
    let mut joined = String::new();
    for (index, event) in output.iter().enumerate() {
        let pointer = match event.event.as_deref() {
            Some("content_block_delta") => "/delta/text",
            Some("error") => "/error/message",
            _ => continue,
        };
        let value: Value = serde_json::from_str(&event.data).map_err(|_| ApiError::internal())?;
        let text = value
            .pointer(pointer)
            .and_then(Value::as_str)
            .ok_or_else(ApiError::internal)?;
        joined.push_str(text);
        text_slots.push((index, pointer.to_owned()));
    }
    if text_slots.is_empty() {
        return Ok(output);
    }
    let redacted = facade
        .redact_text_with_auth(caller, &joined)
        .await
        .map_err(ApiError::from)?
        .redacted_text;
    if redacted == joined {
        return Ok(output);
    }
    for (index, pointer) in &text_slots {
        let mut value: Value =
            serde_json::from_str(&output[*index].data).map_err(|_| ApiError::internal())?;
        *value.pointer_mut(pointer).ok_or_else(ApiError::internal)? = Value::String(String::new());
        output[*index].data = serde_json::to_string(&value).map_err(|_| ApiError::internal())?;
    }
    let (index, pointer) = text_slots.last().ok_or_else(ApiError::internal)?;
    let mut value: Value =
        serde_json::from_str(&output[*index].data).map_err(|_| ApiError::internal())?;
    *value.pointer_mut(pointer).ok_or_else(ApiError::internal)? = Value::String(redacted);
    output[*index].data = serde_json::to_string(&value).map_err(|_| ApiError::internal())?;
    Ok(output)
}

fn validate_provider_events(events: &[ProviderSseEvent]) -> Result<(), ()> {
    if events.is_empty()
        || !matches!(
            events.last().and_then(|event| event.event.as_deref()),
            Some("message_stop" | "error")
        )
    {
        return Err(());
    }
    for event in events {
        if event.comment.is_some() || event.id.as_deref().is_some_and(|id| id.contains('\0')) {
            return Err(());
        }
        let kind = event.event.as_deref().ok_or(())?;
        let value: Value = serde_json::from_str(&event.data).map_err(|_| ())?;
        if !value.is_object() || value.get("type").and_then(Value::as_str) != Some(kind) {
            return Err(());
        }
        match kind {
            "ping" | "content_block_stop" | "message_stop" => {}
            "content_block_delta" => {
                let delta = value.get("delta").and_then(Value::as_object).ok_or(())?;
                if delta.get("type").and_then(Value::as_str) != Some("text_delta")
                    || !delta.get("text").is_some_and(Value::is_string)
                {
                    return Err(());
                }
            }
            "content_block_start" => {
                let block = value
                    .get("content_block")
                    .and_then(Value::as_object)
                    .ok_or(())?;
                if block.get("type").and_then(Value::as_str) != Some("text")
                    || block.get("text").and_then(Value::as_str) != Some("")
                {
                    return Err(());
                }
            }
            "message_start" => {
                let message = value.get("message").and_then(Value::as_object).ok_or(())?;
                if !message
                    .get("content")
                    .is_some_and(|value| value.as_array().is_some_and(Vec::is_empty))
                {
                    return Err(());
                }
            }
            "message_delta" => {
                if value.get("delta").and_then(Value::as_object).is_none() {
                    return Err(());
                }
            }
            "error" => {
                let error = value.get("error").and_then(Value::as_object).ok_or(())?;
                if !error.get("message").is_some_and(Value::is_string) {
                    return Err(());
                }
            }
            _ => return Err(()),
        }
    }
    Ok(())
}

#[derive(Default)]
struct SseDecoder {
    pending: Vec<u8>,
    data: Vec<String>,
    event: Option<String>,
    id: Option<String>,
    comment: Option<String>,
    events: Vec<ProviderSseEvent>,
}

impl SseDecoder {
    fn push(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.pending.extend_from_slice(bytes);
        while let Some(index) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line = self.pending.drain(..=index).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.line(&line)?;
        }
        Ok(())
    }

    fn line(&mut self, line: &[u8]) -> Result<(), TransportError> {
        let line = std::str::from_utf8(line).map_err(|_| TransportError::Failed)?;
        if line.is_empty() {
            if !self.data.is_empty() || self.comment.is_some() {
                self.events.push(ProviderSseEvent {
                    event: self.event.take(),
                    id: self.id.clone(),
                    retry: None,
                    comment: self.comment.take(),
                    data: self.data.join("\n"),
                });
            }
            self.data.clear();
            return Ok(());
        }
        if let Some(comment) = line.strip_prefix(':') {
            self.comment = Some(comment.strip_prefix(' ').unwrap_or(comment).to_owned());
            return Ok(());
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = Some(value.to_owned()),
            "id" if !value.contains('\0') => self.id = Some(value.to_owned()),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<Vec<ProviderSseEvent>, ProviderGatewayError> {
        if !self.pending.is_empty() || !self.data.is_empty() || self.comment.is_some() {
            return Err(ProviderGatewayError::InvalidResponse);
        }
        if self.events.is_empty() {
            return Err(ProviderGatewayError::InvalidResponse);
        }
        Ok(self.events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacienda_core::{HaciendaConfig, auth::Caller, pii::PipelineConfig};

    fn pii_facade() -> HaciendaFacade {
        HaciendaFacade::new(HaciendaConfig::default().with_pii(PipelineConfig::default()))
            .expect("PII facade")
    }

    #[test]
    fn nested_tool_content_is_collected_and_unknown_attachments_fail_closed() {
        let body = serde_json::json!({
            "model": "claude-code",
            "max_tokens": 32,
            "stream": true,
            "system": [{"type": "text", "text": "system"}],
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "hello"},
                {"type": "tool_result", "tool_use_id": "tool-1", "content": [
                    {"type": "text", "text": "tool output"}
                ]}
            ]}, {"role": "assistant", "content": [{
                "type": "tool_use", "id": "tool-1", "name": "lookup",
                "input": {"query": "bob@example.com", "nested": ["Alice Martin"]}
            }]}]
        });
        let paths = validate_and_collect_text_paths(&body).expect("supported request");
        assert!(paths.iter().any(|path| path.ends_with("/input/query")));
        assert!(paths.iter().any(|path| path.ends_with("/content/0/text")));

        let attachment = serde_json::json!({
            "model": "claude-code", "max_tokens": 32, "stream": true,
            "messages": [{"role": "user", "content": [{"type": "image", "source": {}}]}]
        });
        assert!(validate_and_collect_text_paths(&attachment).is_err());
    }

    #[test]
    fn client_tenant_fields_are_not_accepted() {
        let body = serde_json::json!({
            "model": "claude-code", "max_tokens": 32, "stream": true,
            "tenant_id": "attacker",
            "messages": [{"role": "user", "content": "hello"}]
        });
        assert!(validate_and_collect_text_paths(&body).is_err());
    }

    #[tokio::test]
    async fn provider_text_is_redacted_without_losing_sse_frame_order() {
        let events = vec![
            ProviderSseEvent::new(
                Some("content_block_start".into()),
                None,
                serde_json::json!({
                    "type": "content_block_start", "index": 0,
                    "content_block": {"type": "text", "text": ""}
                })
                .to_string(),
            ),
            ProviderSseEvent::new(
                Some("content_block_delta".into()),
                None,
                serde_json::json!({
                    "type": "content_block_delta", "index": 0,
                    "delta": {"type": "text_delta", "text": "bob@example.com"}
                })
                .to_string(),
            ),
            ProviderSseEvent::new(
                Some("message_stop".into()),
                None,
                serde_json::json!({"type": "message_stop"}).to_string(),
            ),
        ];
        let safe = redact_and_validate_provider_events(&events, &pii_facade(), Caller::Trusted)
            .await
            .expect("provider response redaction");
        let payload = safe[1].data.clone();
        assert!(!payload.contains("bob@example.com"));
        assert_eq!(safe[0].event.as_deref(), Some("content_block_start"));
        assert_eq!(safe[2].event.as_deref(), Some("message_stop"));
    }

    #[test]
    fn malformed_or_unknown_provider_events_fail_closed() {
        let event = ProviderSseEvent::new(
            Some("content_block_delta".into()),
            None,
            serde_json::json!({
                "type": "content_block_delta", "delta": {"type": "image_delta"}
            })
            .to_string(),
        );
        assert!(validate_provider_events(&[event]).is_err());
    }
}
