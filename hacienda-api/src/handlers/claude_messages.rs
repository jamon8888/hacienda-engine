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
//! provider seam is reached.  Provider SSE frames are similarly validated: text,
//! tool-use JSON and thinking deltas are redacted as one logical block before any
//! of its deltas are emitted, so a sensitive value split across deltas cannot escape.
//! Lifecycle frames and safe `ping` keepalives are relayed as they arrive; Claude
//! Code stalls on a buffered response and trips its idle watchdog on missing pings.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    convert::Infallible,
    fmt,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use axum::{
    extract::State,
    http::{request::Parts, HeaderMap, HeaderName},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
};
use futures::{stream, Stream, StreamExt};
use hacienda_client_core::{
    ProviderHttpClient, ProviderHttpRequest, ProviderSseEvent, ReqwestHttpClient, TransportError,
};
use hacienda_core::{
    auth::{AuthContext, Caller, Capability, CapabilitySet},
    HaciendaError, HaciendaFacade,
};
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::{
    error::ApiError,
    extract::Json as SafeJson,
    handlers::{
        claude_mod::{correlation as mod_correlation, ModSessionStore},
        extract_auth_context,
    },
    state::ApiState,
};

const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";

/// The authenticated scope a projection session must match before it may
/// receive any local mapping. Derived only from the request `AuthContext`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ModScope {
    tenant: String,
    principal: String,
}

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

/// Response metadata which is safe to copy to the Anthropic-compatible client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeMessagesProviderResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

pub type ClaudeMessagesEventStream =
    Pin<Box<dyn Stream<Item = Result<ProviderSseEvent, ProviderGatewayError>> + Send + 'static>>;

pub struct ClaudeMessagesProviderStream {
    pub response: ClaudeMessagesProviderResponse,
    pub events: ClaudeMessagesEventStream,
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
    ) -> Result<ClaudeMessagesProviderStream, ProviderGatewayError>;
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
        let parsed = Url::parse(&endpoint).map_err(|_| ProviderGatewayError::Unavailable)?;
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
    ) -> Result<ClaudeMessagesProviderStream, ProviderGatewayError> {
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
        let (event_tx, event_rx) = mpsc::channel(32);
        let (response_tx, response_rx) = oneshot::channel();
        tokio::task::spawn_blocking(move || {
            let mut decoder = SseDecoder::default();
            let mut response_tx = Some(response_tx);
            let event_tx_for_decoder = event_tx.clone();
            let mut guard = match http.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    let _ = event_tx.blocking_send(Err(ProviderGatewayError::Transport));
                    if let Some(sender) = response_tx.take() {
                        let _ = sender.send(Err(ProviderGatewayError::Transport));
                    }
                    return;
                }
            };
            let result = guard
                .send_stream(
                    http_request,
                    &mut |code, headers| {
                        let valid_status = (200..300).contains(&code);
                        let valid_content_type = headers.iter().all(|(name, value)| {
                            !name.eq_ignore_ascii_case("content-type")
                                || value.to_ascii_lowercase().starts_with("text/event-stream")
                        });
                        if let Some(sender) = response_tx.take() {
                            let _ = sender.send(if valid_status && valid_content_type {
                                Ok(ClaudeMessagesProviderResponse {
                                    status: code,
                                    headers,
                                })
                            } else {
                                Err(ProviderGatewayError::InvalidResponse)
                            });
                        }
                    },
                    &mut |bytes| {
                        decoder.push(bytes, &mut |event| {
                            event_tx_for_decoder
                                .blocking_send(Ok(event))
                                .map_err(|_| TransportError::Failed)
                        })
                    },
                )
                .map_err(|_| ProviderGatewayError::Transport)
                .and_then(|()| {
                    decoder
                        .finish()
                        .map_err(|_| ProviderGatewayError::InvalidResponse)
                });
            if let Err(error) = result {
                let _ = event_tx.blocking_send(Err(error));
                if let Some(sender) = response_tx.take() {
                    let _ = sender.send(Err(ProviderGatewayError::InvalidResponse));
                }
            }
        });
        let response = response_rx
            .await
            .map_err(|_| ProviderGatewayError::Transport)??;
        let events = stream::unfold(event_rx, |mut receiver| async move {
            receiver.recv().await.map(|item| (item, receiver))
        });
        Ok(ClaudeMessagesProviderStream {
            response,
            events: Box::pin(events),
        })
    }
}

/// `POST /v1/messages`.
#[utoipa::path(
    post,
    path = "/v1/messages",
    tag = "messages",
    operation_id = "claudeMessages",
    security(("bearerAuth" = [])),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Redacted Anthropic Messages Server-Sent Events", content_type = "text/event-stream"),
        (status = 400, description = "Invalid or unsupported Messages request"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn messages(
    State(state): State<ApiState>,
    parts: Parts,
    headers: HeaderMap,
    SafeJson(mut body): SafeJson<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let auth_context = extract_auth_context(&parts).ok_or_else(ApiError::unauthenticated)?;
    let caller = Caller::Principal(auth_context.as_ref());
    caller
        .require(hacienda_core::auth::Capability::DocumentsProcess)
        .map_err(HaciendaError::from)
        .map_err(ApiError::from)?;

    let provider = state
        .claude_messages_provider
        .ok_or_else(|| ApiError::invalid_request("The provider gateway is not configured."))?;
    let text_paths = validate_and_collect_text_paths(&body)
        .map_err(|_| ApiError::invalid_request("The request contains unsupported content."))?;

    // Local-only projection correlation. It is a non-secret hint; if it is
    // absent or malformed no mapping is published and the Mod keeps pseudonyms.
    let correlation = mod_correlation(&headers).map(str::to_owned);
    let scope = ModScope {
        tenant: auth_context.tenant.as_str().to_owned(),
        principal: auth_context.principal_id.clone(),
    };
    // Reveal is re-scoped to the authenticated tenant under a server-side
    // context. The gateway caller does not gain `pii:reveal`, and the originals
    // go only to the authenticated local Mod channel, never to the provider.
    let reveal_context = AuthContext::with_tenant(
        auth_context.principal_id.clone(),
        auth_context.tenant.clone(),
        CapabilitySet::new([Capability::PiiReveal]),
    );
    redact_paths(
        &mut body,
        &text_paths,
        &state.facade,
        caller,
        &reveal_context,
        &state.mod_sessions,
        &scope,
        correlation.as_deref(),
    )
    .await?;

    let tenant = scope.tenant.clone();
    let provider_request = ClaudeMessagesProviderRequest {
        body,
        headers: safe_routing_headers(&headers)?,
        tenant,
    };
    let provider_stream = provider.stream(provider_request).await.map_err(|error| {
        tracing::error!(error = %error, "claude messages provider failed");
        ApiError::internal()
    })?;
    let response_headers = safe_response_headers(&provider_stream.response.headers)?;
    let facade = Arc::clone(&state.facade);
    let relay = ResponseRelay::new(
        provider_stream.events,
        facade,
        auth_context,
        reveal_context,
        Arc::clone(&state.mod_sessions),
        scope,
        correlation,
    );
    let output = stream::unfold(relay, |mut relay| async move {
        relay
            .next_event()
            .await
            .map(|event| (Ok::<Event, Infallible>(event), relay))
    });
    // Claude Code reads SSE frames as they arrive and uses `ping` as liveness
    // during a long thinking pause. Emit our own named ping while idle instead
    // of only a comment, and keep the wire content type explicitly Anthropic.
    let keep_alive = KeepAlive::new()
        .interval(Duration::from_secs(15))
        .event(Event::default().event("ping").data(r#"{"type":"ping"}"#));
    Ok((response_headers, Sse::new(output).keep_alive(keep_alive)))
}

fn safe_response_headers(headers: &[(String, String)]) -> Result<HeaderMap, ApiError> {
    const ALLOWED: &[&str] = &[
        "anthropic-version",
        "anthropic-beta",
        "cache-control",
        "request-id",
        "retry-after",
        "x-request-id",
    ];
    let mut safe = HeaderMap::new();
    safe.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/event-stream"),
    );
    for (name, value) in headers {
        if !ALLOWED
            .iter()
            .any(|allowed| name.eq_ignore_ascii_case(allowed))
        {
            continue;
        }
        if value.len() > 4096 || value.contains(['\r', '\n']) {
            return Err(ApiError::internal());
        }
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| ApiError::internal())?;
        let value = value.parse().map_err(|_| ApiError::internal())?;
        safe.append(name, value);
    }
    Ok(safe)
}

fn safe_routing_headers(headers: &HeaderMap) -> Result<Vec<(String, String)>, ApiError> {
    let mut safe = Vec::new();
    for name in ["anthropic-version", "anthropic-beta", "x-request-id"] {
        let header_name = HeaderName::from_static(name);
        for value in headers.get_all(&header_name).iter() {
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
            vec!["messages".into(), index.to_string(), "content".into()],
            &mut paths,
        )?;
    }
    if let Some(system) = object.get("system") {
        collect_content(system, vec!["system".into()], &mut paths)?;
    }
    if let Some(metadata) = object.get("metadata") {
        collect_metadata(metadata, vec!["metadata".into()], &mut paths)?;
    }
    if let Some(tools) = object.get("tools") {
        collect_tools(tools, vec!["tools".into()], &mut paths)?;
    }
    if let Some(tool_choice) = object.get("tool_choice") {
        collect_tool_choice(tool_choice, vec!["tool_choice".into()], &mut paths)?;
    }
    for key in [
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

fn collect_metadata(value: &Value, path: Vec<String>, paths: &mut Vec<String>) -> Result<(), ()> {
    let object = value.as_object().ok_or(())?;
    reject_unknown_keys(object, &["user_id"])?;
    if object.contains_key("user_id") {
        require_string(object, "user_id")?;
        paths.push(path_value(&path_with(&path, "user_id")));
    }
    Ok(())
}

fn collect_tools(value: &Value, path: Vec<String>, paths: &mut Vec<String>) -> Result<(), ()> {
    let tools = value.as_array().ok_or(())?;
    for (index, tool) in tools.iter().enumerate() {
        let tool = tool.as_object().ok_or(())?;
        reject_unknown_keys(
            tool,
            &["name", "description", "input_schema", "cache_control"],
        )?;
        let tool_path = path_with(&path, &index.to_string());
        require_string(tool, "name")?;
        paths.push(path_value(&path_with(&tool_path, "name")));
        if let Some(description) = tool.get("description") {
            if !description.is_string() {
                return Err(());
            }
            paths.push(path_value(&path_with(&tool_path, "description")));
        }
        let input_schema = tool.get("input_schema").ok_or(())?;
        if !input_schema.is_object() {
            return Err(());
        }
        collect_all_strings(input_schema, path_with(&tool_path, "input_schema"), paths)?;
        if let Some(cache_control) = tool.get("cache_control") {
            validate_cache_control(cache_control)?;
        }
    }
    Ok(())
}

fn collect_tool_choice(
    value: &Value,
    path: Vec<String>,
    paths: &mut Vec<String>,
) -> Result<(), ()> {
    match value {
        Value::String(choice) if matches!(choice.as_str(), "auto" | "any" | "none") => Ok(()),
        Value::Object(object) => {
            reject_unknown_keys(object, &["type", "name"])?;
            if require_string(object, "type")? != "tool" {
                return Err(());
            }
            require_string(object, "name")?;
            paths.push(path_value(&path_with(&path, "name")));
            Ok(())
        }
        _ => Err(()),
    }
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

fn path_value(path: &[String]) -> String {
    pointer(&path.iter().map(String::as_str).collect::<Vec<_>>())
}

// These are request-scoped services and authorization scopes that must stay
// bound to the same redaction operation.
#[allow(clippy::too_many_arguments)]
async fn redact_paths(
    body: &mut Value,
    paths: &[String],
    facade: &HaciendaFacade,
    caller: Caller<'_>,
    reveal_context: &AuthContext,
    mod_sessions: &ModSessionStore,
    scope: &ModScope,
    correlation: Option<&str>,
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
        publish_mod_mappings(
            facade,
            reveal_context,
            mod_sessions,
            scope,
            correlation,
            &redacted,
        )
        .await;
        let slot = body.pointer_mut(path).ok_or_else(ApiError::internal)?;
        *slot = Value::String(redacted);
    }
    Ok(())
}

/// Extract tokens minted by this gateway from a redacted string. Unknown text
/// is ignored: a token the facade cannot resolve locally stays unrevealed.
fn extract_mod_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let Some(relative_end) = rest[start..].find(']') else {
            break;
        };
        let end = start + relative_end;
        let token = &rest[start..=end];
        let parts: Vec<&str> = token[1..token.len() - 1].split(':').collect();
        let valid = parts.len() == 3
            && !parts[0].is_empty()
            && parts[0]
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            && !parts[1].is_empty()
            && parts[1]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            && parts[2]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
        if valid {
            tokens.push(token.to_owned());
        }
        rest = &rest[end + 1..];
    }
    tokens
}

/// Resolve token → original locally and publish to the one active, fully
/// scoped Mod session. No value reaches this function's caller except through
/// that in-memory channel, and no original is logged or persisted.
async fn publish_mod_mappings(
    facade: &HaciendaFacade,
    reveal_context: &AuthContext,
    mod_sessions: &ModSessionStore,
    scope: &ModScope,
    correlation: Option<&str>,
    redacted: &str,
) {
    let Some(correlation) = correlation else {
        return;
    };
    let mut mappings: HashMap<String, String> = HashMap::new();
    for token in extract_mod_tokens(redacted) {
        if mappings.contains_key(&token) {
            continue;
        }
        if let Ok(original) = facade
            .reveal_token_with_auth(Caller::Principal(reveal_context), &token)
            .await
        {
            mappings.insert(token, original);
        }
    }
    for (token, original) in mappings {
        mod_sessions.publish(
            &scope.tenant,
            &scope.principal,
            correlation,
            &token,
            original,
        );
    }
}

/// Owned-argument wrapper for the streaming relay. Taking `&self` across an
/// await would require `ResponseRelay` to be `Sync`, but its boxed provider
/// stream is only `Send`; cloning the small scope values keeps the relay future
/// sendable without changing the client-visible frames.
async fn relay_publish_mappings(
    facade: Arc<HaciendaFacade>,
    reveal_context: AuthContext,
    mod_sessions: Arc<ModSessionStore>,
    scope: ModScope,
    correlation: Option<String>,
    redacted: String,
) {
    publish_mod_mappings(
        &facade,
        &reveal_context,
        &mod_sessions,
        &scope,
        correlation.as_deref(),
        &redacted,
    )
    .await;
}

impl ResponseRelay {
    fn project_side_channel(&self, redacted: &str) -> impl std::future::Future<Output = ()> + Send {
        relay_publish_mappings(
            Arc::clone(&self.facade),
            self.reveal_context.clone(),
            Arc::clone(&self.mod_sessions),
            self.scope.clone(),
            self.correlation.clone(),
            redacted.to_owned(),
        )
    }
}

const MAX_BLOCK_BUFFER: usize = 1024 * 1024;
const MAX_SSE_FRAME: usize = 1024 * 1024;
const MAX_BLOCK_DELTAS: usize = 4096;

struct ResponseRelay {
    events: ClaudeMessagesEventStream,
    facade: Arc<HaciendaFacade>,
    auth_context: Arc<hacienda_core::auth::AuthContext>,
    /// Server-side reveal context, scoped to the caller's tenant only.
    reveal_context: AuthContext,
    mod_sessions: Arc<ModSessionStore>,
    scope: ModScope,
    correlation: Option<String>,
    validator: ResponseValidator,
    active: Option<BufferedBlock>,
    pending: VecDeque<ProviderSseEvent>,
    closed: bool,
}

struct BufferedBlock {
    kind: BlockKind,
    joined: String,
    deltas: Vec<ProviderSseEvent>,
    retained_bytes: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    ToolUse,
    Thinking,
}

impl ResponseRelay {
    fn new(
        events: ClaudeMessagesEventStream,
        facade: Arc<HaciendaFacade>,
        auth_context: Arc<hacienda_core::auth::AuthContext>,
        reveal_context: AuthContext,
        mod_sessions: Arc<ModSessionStore>,
        scope: ModScope,
        correlation: Option<String>,
    ) -> Self {
        Self {
            events,
            facade,
            auth_context,
            reveal_context,
            mod_sessions,
            scope,
            correlation,
            validator: ResponseValidator::default(),
            active: None,
            pending: VecDeque::new(),
            closed: false,
        }
    }

    async fn next_event(&mut self) -> Option<Event> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some(render_event(event));
            }
            if self.closed {
                return None;
            }
            let Some(result) = self.events.next().await else {
                // A body that stops before `message_delta`/`message_stop` is a
                // dropped connection to Claude Code, not a clean completion.
                self.closed = true;
                if self.validator.finish().is_err() || self.active.is_some() {
                    self.pending.push_back(invalid_response_event());
                    continue;
                }
                return None;
            };
            let Ok(event) = result else {
                self.closed = true;
                self.pending.push_back(invalid_response_event());
                continue;
            };
            if self.accept(event).await.is_err() {
                self.closed = true;
                self.pending.push_back(invalid_response_event());
                continue;
            }
        }
    }

    async fn accept(&mut self, event: ProviderSseEvent) -> Result<(), ()> {
        let action = self.validator.accept(&event)?;
        match action {
            ValidationAction::BlockStart { kind } => {
                self.active = Some(BufferedBlock {
                    kind,
                    joined: String::new(),
                    deltas: Vec::new(),
                    retained_bytes: 0,
                });
                let mut safe = event;
                if kind == BlockKind::ToolUse {
                    // Tool names are model-generated free-form text.
                    let mut value: Value = serde_json::from_str(&safe.data).map_err(|_| ())?;
                    let name = value
                        .pointer("/content_block/name")
                        .and_then(Value::as_str)
                        .ok_or(())?
                        .to_owned();
                    let caller = Caller::Principal(self.auth_context.as_ref());
                    let redacted = self
                        .facade
                        .redact_text_with_auth(caller, &name)
                        .await
                        .map_err(|_| ())?
                        .redacted_text;
                    *value.pointer_mut("/content_block/name").ok_or(())? =
                        Value::String(redacted.clone());
                    self.project_side_channel(&redacted).await;
                    safe.data = serde_json::to_string(&value).map_err(|_| ())?;
                }
                self.pending.push_back(safe);
            }
            ValidationAction::BlockDelta { kind } => {
                let value: Value = serde_json::from_str(&event.data).map_err(|_| ())?;
                let fragment = match kind {
                    BlockKind::Text => value.pointer("/delta/text").and_then(Value::as_str),
                    BlockKind::ToolUse => {
                        value.pointer("/delta/partial_json").and_then(Value::as_str)
                    }
                    // A signature delta carries no inspectable text but stays
                    // in the buffered block until the whole block is redacted.
                    BlockKind::Thinking => {
                        match value.pointer("/delta/type").and_then(Value::as_str) {
                            Some("thinking_delta") => {
                                value.pointer("/delta/thinking").and_then(Value::as_str)
                            }
                            Some("signature_delta") => None,
                            _ => return Err(()),
                        }
                    }
                };
                if let Some(fragment) = fragment {
                    let active = self.active.as_mut().ok_or(())?;
                    active.joined.push_str(fragment);
                    if active.joined.len() > MAX_BLOCK_BUFFER {
                        return Err(());
                    }
                }
                let active = self.active.as_mut().ok_or(())?;
                active.retained_bytes = active
                    .retained_bytes
                    .checked_add(event.data.len())
                    .filter(|bytes| *bytes <= MAX_BLOCK_BUFFER)
                    .ok_or(())?;
                if active.deltas.len() >= MAX_BLOCK_DELTAS {
                    return Err(());
                }
                active.deltas.push(event);
            }
            ValidationAction::BlockStop => {
                let active = self.active.take().ok_or(())?;
                let redacted = self.redact_block(&active).await?;
                for event in
                    redact_block_deltas(active.deltas, active.kind, redacted, &active.joined)?
                {
                    self.pending.push_back(event);
                }
                self.pending.push_back(event);
            }
            ValidationAction::Error => {
                let mut value: Value = serde_json::from_str(&event.data).map_err(|_| ())?;
                let message = value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .ok_or(())?;
                let caller = Caller::Principal(self.auth_context.as_ref());
                let redacted = self
                    .facade
                    .redact_text_with_auth(caller, message)
                    .await
                    .map_err(|_| ())?
                    .redacted_text;
                *value.pointer_mut("/error/message").ok_or(())? = Value::String(redacted.clone());
                self.project_side_channel(&redacted).await;
                let mut safe = event;
                safe.data = serde_json::to_string(&value).map_err(|_| ())?;
                self.pending.push_back(safe);
            }
            ValidationAction::Immediate => {
                let mut safe = event;
                if safe.comment.is_some() {
                    safe.comment = Some("ping".to_owned());
                }
                if safe.event.as_deref() == Some("message_delta") {
                    let mut value: Value = serde_json::from_str(&safe.data).map_err(|_| ())?;
                    if value
                        .pointer("/delta/stop_sequence")
                        .and_then(Value::as_str)
                        .is_some()
                    {
                        let stop = value
                            .pointer("/delta/stop_sequence")
                            .and_then(Value::as_str)
                            .ok_or(())?
                            .to_owned();
                        let caller = Caller::Principal(self.auth_context.as_ref());
                        let redacted = self
                            .facade
                            .redact_text_with_auth(caller, &stop)
                            .await
                            .map_err(|_| ())?
                            .redacted_text;
                        *value.pointer_mut("/delta/stop_sequence").ok_or(())? =
                            Value::String(redacted.clone());
                        self.project_side_channel(&redacted).await;
                        safe.data = serde_json::to_string(&value).map_err(|_| ())?;
                    }
                }
                self.pending.push_back(safe);
            }
        }
        Ok(())
    }

    async fn redact_block(&mut self, block: &BufferedBlock) -> Result<String, ()> {
        let caller = Caller::Principal(self.auth_context.as_ref());
        let redacted = if block.kind == BlockKind::ToolUse {
            let mut value: Value = serde_json::from_str(&block.joined).map_err(|_| ())?;
            if !value.is_object() {
                return Err(());
            }
            redact_tool_argument_values(&self.facade, self.auth_context.as_ref(), &mut value)
                .await?;
            serde_json::to_string(&value).map_err(|_| ())?
        } else {
            self.facade
                .redact_text_with_auth(caller, &block.joined)
                .await
                .map_err(|_| ())?
                .redacted_text
        };
        if redacted.len() > MAX_BLOCK_BUFFER {
            return Err(());
        }
        if block.kind == BlockKind::ToolUse
            && !serde_json::from_str::<Value>(&redacted)
                .ok()
                .is_some_and(|value| value.is_object())
        {
            return Err(());
        }
        self.project_side_channel(&redacted).await;
        Ok(redacted)
    }
}

fn redact_tool_argument_values<'a>(
    facade: &'a HaciendaFacade,
    auth_context: &'a AuthContext,
    value: &'a mut Value,
) -> Pin<Box<dyn std::future::Future<Output = Result<(), ()>> + Send + 'a>> {
    Box::pin(async move {
        match value {
            Value::String(text) => {
                if text.len() > MAX_BLOCK_BUFFER {
                    return Err(());
                }
                *text = facade
                    .redact_text_with_auth(Caller::Principal(auth_context), text)
                    .await
                    .map_err(|_| ())?
                    .redacted_text;
                Ok(())
            }
            Value::Array(items) => {
                for item in items {
                    redact_tool_argument_values(facade, auth_context, item).await?;
                }
                Ok(())
            }
            Value::Object(object) => {
                let keys: Vec<String> = object.keys().cloned().collect();
                for key in keys {
                    let safe_key = facade
                        .redact_text_with_auth(Caller::Principal(auth_context), &key)
                        .await
                        .map_err(|_| ())?
                        .redacted_text;
                    if safe_key != key {
                        return Err(());
                    }
                    redact_tool_argument_values(
                        facade,
                        auth_context,
                        object.get_mut(&key).ok_or(())?,
                    )
                    .await?;
                }
                Ok(())
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
        }
    })
}

/// Content-free error emitted in place of a silently truncated stream. It
/// never echoes provider text, so it cannot leak unredacted content.
fn invalid_response_event() -> ProviderSseEvent {
    ProviderSseEvent::new(
        Some("error".to_owned()),
        None,
        serde_json::json!({
            "type": "error",
            "error": {
                "type": "api_error",
                "message": "The provider response was invalid."
            }
        })
        .to_string(),
    )
}

/// Only content-free SSE heartbeats are safe to relay to the client.
fn is_safe_keepalive_comment(comment: &str) -> bool {
    let comment = comment.trim();
    if comment.is_empty() || comment == "ping" {
        return true;
    }
    let Some(remainder) = comment.strip_prefix("ping - ") else {
        return false;
    };
    !remainder.is_empty()
        && remainder.bytes().all(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'-' | b':' | b'.' | b'T' | b'Z' | b' ')
        })
        && remainder.bytes().any(|byte| byte.is_ascii_digit())
}

fn render_event(event: ProviderSseEvent) -> Event {
    let mut rendered = Event::default();
    if let Some(kind) = event.event {
        rendered = rendered.event(kind);
    }
    if let Some(id) = event.id {
        rendered = rendered.id(id);
    }
    if let Some(retry) = event.retry {
        rendered = rendered.retry(Duration::from_millis(retry));
    }
    if let Some(comment) = event.comment {
        rendered = rendered.comment(comment);
        if event.data.is_empty() {
            return rendered;
        }
    }
    rendered.data(event.data)
}

fn redact_block_deltas(
    mut deltas: Vec<ProviderSseEvent>,
    kind: BlockKind,
    redacted: String,
    original: &str,
) -> Result<Vec<ProviderSseEvent>, ()> {
    // A thinking signature authenticates the original text. If redaction changes
    // that text, drop the signature instead of forwarding a mismatched pair.
    if kind == BlockKind::Thinking && redacted != original {
        deltas.retain(|event| {
            serde_json::from_str::<Value>(&event.data)
                .ok()
                .and_then(|value| {
                    value
                        .pointer("/delta/type")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .as_deref()
                != Some("signature_delta")
        });
    }
    let pointer_for = |event: &ProviderSseEvent| -> Result<Option<&'static str>, ()> {
        match kind {
            BlockKind::Text => Ok(Some("/delta/text")),
            BlockKind::ToolUse => Ok(Some("/delta/partial_json")),
            BlockKind::Thinking => {
                let value: Value = serde_json::from_str(&event.data).map_err(|_| ())?;
                match value.pointer("/delta/type").and_then(Value::as_str) {
                    Some("thinking_delta") => Ok(Some("/delta/thinking")),
                    Some("signature_delta") => Ok(None),
                    _ => Err(()),
                }
            }
        }
    };
    let redactable = deltas
        .iter()
        .map(pointer_for)
        .collect::<Result<Vec<_>, ()>>()?;
    let last = redactable
        .iter()
        .rposition(|pointer| pointer.is_some())
        .ok_or(())?;
    for (event, pointer) in deltas.iter_mut().zip(redactable.iter()) {
        let Some(pointer) = pointer else {
            continue;
        };
        let mut value: Value = serde_json::from_str(&event.data).map_err(|_| ())?;
        *value.pointer_mut(pointer).ok_or(())? = Value::String(String::new());
        event.data = serde_json::to_string(&value).map_err(|_| ())?;
    }
    let pointer = redactable[last].ok_or(())?;
    let mut value: Value = serde_json::from_str(&deltas[last].data).map_err(|_| ())?;
    *value.pointer_mut(pointer).ok_or(())? = Value::String(redacted);
    deltas[last].data = serde_json::to_string(&value).map_err(|_| ())?;
    Ok(deltas)
}

#[derive(Default)]
struct ResponseValidator {
    phase: ResponsePhase,
    active: Option<(u64, BlockKind)>,
}

enum ValidationAction {
    Immediate,
    BlockStart { kind: BlockKind },
    BlockDelta { kind: BlockKind },
    BlockStop,
    Error,
}

impl ResponseValidator {
    fn accept(&mut self, event: &ProviderSseEvent) -> Result<ValidationAction, ()> {
        if let Some(comment) = event.comment.as_deref() {
            if self.phase == ResponsePhase::Done
                || !is_safe_keepalive_comment(comment)
                || !event.data.is_empty()
                || event.event.is_some()
                || event.id.is_some()
                || event.retry.is_some()
            {
                return Err(());
            }
            return Ok(ValidationAction::Immediate);
        }
        if event.id.is_some() || event.retry.is_some() {
            return Err(());
        }
        let kind = event.event.as_deref().ok_or(())?;
        let value: Value = serde_json::from_str(&event.data).map_err(|_| ())?;
        if !value.is_object() || value.get("type").and_then(Value::as_str) != Some(kind) {
            return Err(());
        }
        let object = value.as_object().ok_or(())?;
        match kind {
            "ping" => {
                if self.phase == ResponsePhase::Done {
                    return Err(());
                }
                reject_unknown_keys(object, &["type"])?;
                Ok(ValidationAction::Immediate)
            }
            "message_start" => {
                if self.phase != ResponsePhase::AwaitMessageStart {
                    return Err(());
                }
                reject_unknown_keys(object, &["type", "message"])?;
                let message = value.get("message").and_then(Value::as_object).ok_or(())?;
                reject_unknown_keys(
                    message,
                    &[
                        "id",
                        "type",
                        "role",
                        "model",
                        "content",
                        "stop_reason",
                        "stop_sequence",
                        "usage",
                    ],
                )?;
                if !message
                    .get("content")
                    .is_some_and(|value| value.as_array().is_some_and(Vec::is_empty))
                {
                    return Err(());
                }
                validate_usage(message.get("usage").ok_or(())?)?;
                self.phase = ResponsePhase::ReadyForBlock;
                Ok(ValidationAction::Immediate)
            }
            "content_block_start" => {
                if self.phase != ResponsePhase::ReadyForBlock {
                    return Err(());
                }
                reject_unknown_keys(object, &["type", "index", "content_block"])?;
                let index = value.get("index").and_then(Value::as_u64).ok_or(())?;
                let block = value
                    .get("content_block")
                    .and_then(Value::as_object)
                    .ok_or(())?;
                let block_kind = match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        reject_unknown_keys(block, &["type", "text"])?;
                        if block.get("text").and_then(Value::as_str) != Some("") {
                            return Err(());
                        }
                        BlockKind::Text
                    }
                    Some("tool_use") => {
                        reject_unknown_keys(block, &["type", "id", "name", "input"])?;
                        if block.get("id").and_then(Value::as_str).is_none()
                            || block.get("name").and_then(Value::as_str).is_none()
                            || !block
                                .get("input")
                                .is_some_and(|value| value.as_object().is_some_and(Map::is_empty))
                        {
                            return Err(());
                        }
                        BlockKind::ToolUse
                    }
                    Some("thinking") => {
                        reject_unknown_keys(block, &["type", "thinking", "signature"])?;
                        if block.get("thinking").and_then(Value::as_str) != Some("")
                            || !block.get("signature").is_some_and(Value::is_string)
                        {
                            return Err(());
                        }
                        BlockKind::Thinking
                    }
                    _ => return Err(()),
                };
                self.active = Some((index, block_kind));
                self.phase = ResponsePhase::InBlock;
                Ok(ValidationAction::BlockStart { kind: block_kind })
            }
            "content_block_delta" => {
                if self.phase != ResponsePhase::InBlock {
                    return Err(());
                }
                reject_unknown_keys(object, &["type", "index", "delta"])?;
                let index = value.get("index").and_then(Value::as_u64).ok_or(())?;
                let (_, block_kind) = self.active.ok_or(())?;
                if self.active.map(|(active, _)| active) != Some(index) {
                    return Err(());
                }
                let delta = value.get("delta").and_then(Value::as_object).ok_or(())?;
                match block_kind {
                    BlockKind::Text => {
                        reject_unknown_keys(delta, &["type", "text"])?;
                        if delta.get("type").and_then(Value::as_str) != Some("text_delta")
                            || !delta.get("text").is_some_and(Value::is_string)
                        {
                            return Err(());
                        }
                    }
                    BlockKind::ToolUse => {
                        reject_unknown_keys(delta, &["type", "partial_json"])?;
                        if delta.get("type").and_then(Value::as_str) != Some("input_json_delta")
                            || !delta.get("partial_json").is_some_and(Value::is_string)
                        {
                            return Err(());
                        }
                    }
                    BlockKind::Thinking => match delta.get("type").and_then(Value::as_str) {
                        Some("thinking_delta") => {
                            reject_unknown_keys(delta, &["type", "thinking"])?;
                            if !delta.get("thinking").is_some_and(Value::is_string) {
                                return Err(());
                            }
                        }
                        Some("signature_delta") => {
                            reject_unknown_keys(delta, &["type", "signature"])?;
                            if !delta.get("signature").is_some_and(Value::is_string) {
                                return Err(());
                            }
                        }
                        _ => return Err(()),
                    },
                }
                Ok(ValidationAction::BlockDelta { kind: block_kind })
            }
            "content_block_stop" => {
                if self.phase != ResponsePhase::InBlock {
                    return Err(());
                }
                reject_unknown_keys(object, &["type", "index"])?;
                let index = value.get("index").and_then(Value::as_u64).ok_or(())?;
                if self.active.map(|(active, _)| active) != Some(index) {
                    return Err(());
                }
                self.active = None;
                self.phase = ResponsePhase::ReadyForBlock;
                Ok(ValidationAction::BlockStop)
            }
            "message_delta" => {
                if !matches!(
                    self.phase,
                    ResponsePhase::ReadyForBlock | ResponsePhase::AfterMessageDelta
                ) {
                    return Err(());
                }
                reject_unknown_keys(object, &["type", "delta", "usage"])?;
                let delta = value.get("delta").and_then(Value::as_object).ok_or(())?;
                reject_unknown_keys(delta, &["stop_reason", "stop_sequence"])?;
                validate_usage(value.get("usage").ok_or(())?)?;
                self.phase = ResponsePhase::AfterMessageDelta;
                Ok(ValidationAction::Immediate)
            }
            "message_stop" => {
                if self.phase != ResponsePhase::AfterMessageDelta || self.active.is_some() {
                    return Err(());
                }
                reject_unknown_keys(object, &["type"])?;
                self.phase = ResponsePhase::Done;
                Ok(ValidationAction::Immediate)
            }
            "error" => {
                if self.phase == ResponsePhase::Done || self.active.is_some() {
                    return Err(());
                }
                reject_unknown_keys(object, &["type", "error"])?;
                let error = value.get("error").and_then(Value::as_object).ok_or(())?;
                reject_unknown_keys(error, &["type", "message"])?;
                if !error.get("message").is_some_and(Value::is_string) {
                    return Err(());
                }
                self.phase = ResponsePhase::Done;
                Ok(ValidationAction::Error)
            }
            _ => {
                if self.phase == ResponsePhase::Done || !is_token_only_unknown_event(object) {
                    return Err(());
                }
                Ok(ValidationAction::Immediate)
            }
        }
    }

    fn finish(&self) -> Result<(), ()> {
        (self.phase == ResponsePhase::Done && self.active.is_none())
            .then_some(())
            .ok_or(())
    }
}

fn is_token_only_unknown_event(object: &Map<String, Value>) -> bool {
    let Some(token) = object.get("token").and_then(Value::as_str) else {
        return false;
    };
    object.len() == 2 && object.contains_key("type") && is_hacienda_token_only(token)
}

fn is_hacienda_token_only(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let mut rest = value;
    while let Some(start) = rest.find('[') {
        if start != 0 {
            return false;
        }
        let Some(end) = rest.find(']') else {
            return false;
        };
        let token = &rest[1..end];
        let mut parts = token.split(':');
        if parts.next().is_none_or(|part| part.is_empty())
            || parts.next().is_none_or(|part| part.is_empty())
            || parts.next().is_none_or(|part| part.is_empty())
            || parts.next().is_some()
            || token
                .chars()
                .any(|char| !(char.is_ascii_alphanumeric() || matches!(char, ':' | '_' | '-')))
        {
            return false;
        }
        rest = &rest[end + 1..];
    }
    rest.is_empty()
}

#[cfg(test)]
async fn redact_and_validate_provider_events(
    events: &[ProviderSseEvent],
    facade: &HaciendaFacade,
    caller: Caller<'_>,
) -> Result<Vec<ProviderSseEvent>, ApiError> {
    let mut validator = ResponseValidator::default();
    let mut output = Vec::new();
    let mut active: Option<BufferedBlock> = None;
    for event in events {
        let action = validator.accept(event).map_err(|_| ApiError::internal())?;
        match action {
            ValidationAction::BlockStart { kind } => {
                active = Some(BufferedBlock {
                    kind,
                    joined: String::new(),
                    deltas: Vec::new(),
                    retained_bytes: 0,
                });
                let mut safe = event.clone();
                if kind == BlockKind::ToolUse {
                    let mut value: Value =
                        serde_json::from_str(&safe.data).map_err(|_| ApiError::internal())?;
                    let name = value
                        .pointer("/content_block/name")
                        .and_then(Value::as_str)
                        .ok_or_else(ApiError::internal)?
                        .to_owned();
                    let redacted = facade
                        .redact_text_with_auth(caller, &name)
                        .await
                        .map_err(ApiError::from)?
                        .redacted_text;
                    *value
                        .pointer_mut("/content_block/name")
                        .ok_or_else(ApiError::internal)? = Value::String(redacted);
                    safe.data = serde_json::to_string(&value).map_err(|_| ApiError::internal())?;
                }
                output.push(safe);
            }
            ValidationAction::BlockDelta { kind } => {
                let value: Value =
                    serde_json::from_str(&event.data).map_err(|_| ApiError::internal())?;
                let fragment = match kind {
                    BlockKind::Text => value.pointer("/delta/text").and_then(Value::as_str),
                    BlockKind::ToolUse => {
                        value.pointer("/delta/partial_json").and_then(Value::as_str)
                    }
                    BlockKind::Thinking => {
                        match value.pointer("/delta/type").and_then(Value::as_str) {
                            Some("thinking_delta") => {
                                value.pointer("/delta/thinking").and_then(Value::as_str)
                            }
                            Some("signature_delta") => None,
                            _ => return Err(ApiError::internal()),
                        }
                    }
                };
                if let Some(fragment) = fragment {
                    let block = active.as_mut().ok_or_else(ApiError::internal)?;
                    block.joined.push_str(fragment);
                    if block.joined.len() > MAX_BLOCK_BUFFER {
                        return Err(ApiError::internal());
                    }
                }
                let block = active.as_mut().ok_or_else(ApiError::internal)?;
                block.retained_bytes = block
                    .retained_bytes
                    .checked_add(event.data.len())
                    .filter(|bytes| *bytes <= MAX_BLOCK_BUFFER)
                    .ok_or_else(ApiError::internal)?;
                if block.deltas.len() >= MAX_BLOCK_DELTAS {
                    return Err(ApiError::internal());
                }
                block.deltas.push(event.clone());
            }
            ValidationAction::BlockStop => {
                let block = active.take().ok_or_else(ApiError::internal)?;
                let joined = if block.kind == BlockKind::ToolUse {
                    serde_json::from_str::<Value>(&block.joined)
                        .ok()
                        .filter(Value::is_object)
                        .ok_or_else(ApiError::internal)?;
                    block.joined.clone()
                } else {
                    block.joined.clone()
                };
                let redacted = facade
                    .redact_text_with_auth(caller, &joined)
                    .await
                    .map_err(ApiError::from)?
                    .redacted_text;
                if block.kind == BlockKind::ToolUse
                    && !serde_json::from_str::<Value>(&redacted)
                        .ok()
                        .is_some_and(|value| value.is_object())
                {
                    return Err(ApiError::internal());
                }
                output.extend(
                    redact_block_deltas(block.deltas, block.kind, redacted, &block.joined)
                        .map_err(|_| ApiError::internal())?,
                );
                output.push(event.clone());
            }
            ValidationAction::Error => {
                let mut value: Value =
                    serde_json::from_str(&event.data).map_err(|_| ApiError::internal())?;
                let message = value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .ok_or_else(ApiError::internal)?;
                let redacted = facade
                    .redact_text_with_auth(caller, message)
                    .await
                    .map_err(ApiError::from)?
                    .redacted_text;
                *value
                    .pointer_mut("/error/message")
                    .ok_or_else(ApiError::internal)? = Value::String(redacted);
                let mut safe = event.clone();
                safe.data = serde_json::to_string(&value).map_err(|_| ApiError::internal())?;
                output.push(safe);
            }
            ValidationAction::Immediate => output.push(event.clone()),
        }
    }
    validator.finish().map_err(|_| ApiError::internal())?;
    Ok(output)
}

#[cfg(test)]
fn validate_provider_events(events: &[ProviderSseEvent]) -> Result<(), ()> {
    if events.is_empty() {
        return Err(());
    }
    let mut validator = ResponseValidator::default();
    for event in events {
        validator.accept(event)?;
    }
    validator.finish()
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ResponsePhase {
    #[default]
    AwaitMessageStart,
    ReadyForBlock,
    InBlock,
    AfterMessageDelta,
    Done,
}

fn validate_usage(value: &Value) -> Result<(), ()> {
    const USAGE_FIELDS: &[&str] = &[
        "input_tokens",
        "output_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "cache_creation",
        "server_tool_use",
        "service_tier",
    ];
    let object = value.as_object().ok_or(())?;
    reject_unknown_keys(object, USAGE_FIELDS)?;
    if object.values().any(|field| {
        !(field.is_number() || field.is_null() || field.is_object() || field.is_string())
    }) {
        return Err(());
    }
    for key in ["cache_creation", "server_tool_use"] {
        if let Some(nested) = value.get(key) {
            let nested = nested.as_object().ok_or(())?;
            reject_unknown_keys(
                nested,
                &[
                    "ephemeral_5m_input_tokens",
                    "ephemeral_1h_input_tokens",
                    "web_search_requests",
                ],
            )?;
            if !nested.values().all(Value::is_number) {
                return Err(());
            }
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
    frame_bytes: usize,
}

impl SseDecoder {
    fn push(
        &mut self,
        bytes: &[u8],
        callback: &mut dyn FnMut(ProviderSseEvent) -> Result<(), TransportError>,
    ) -> Result<(), TransportError> {
        if self
            .pending
            .len()
            .saturating_add(self.frame_bytes)
            .saturating_add(bytes.len())
            > MAX_SSE_FRAME
        {
            return Err(TransportError::Failed);
        }
        self.pending.extend_from_slice(bytes);
        if self.pending.len().saturating_add(self.frame_bytes) > MAX_SSE_FRAME {
            return Err(TransportError::Failed);
        }
        while let Some(index) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line = self.pending.drain(..=index).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.line(&line, callback)?;
        }
        Ok(())
    }

    fn line(
        &mut self,
        line: &[u8],
        callback: &mut dyn FnMut(ProviderSseEvent) -> Result<(), TransportError>,
    ) -> Result<(), TransportError> {
        if line.len() > MAX_SSE_FRAME {
            return Err(TransportError::Failed);
        }
        let line = std::str::from_utf8(line).map_err(|_| TransportError::Failed)?;
        if line.is_empty() {
            if !self.data.is_empty() || self.comment.is_some() {
                let event = ProviderSseEvent {
                    event: self.event.take(),
                    id: self.id.clone(),
                    retry: None,
                    comment: self.comment.take(),
                    data: self.data.join("\n"),
                };
                callback(event)?;
            }
            self.data.clear();
            self.frame_bytes = 0;
            return Ok(());
        }
        self.frame_bytes = self
            .frame_bytes
            .checked_add(line.len().saturating_add(1))
            .filter(|size| *size <= MAX_SSE_FRAME)
            .ok_or(TransportError::Failed)?;
        if let Some(comment) = line.strip_prefix(':') {
            self.comment = Some(comment.strip_prefix(' ').unwrap_or(comment).to_owned());
            return Ok(());
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = Some(value.to_owned()),
            "id" if !value.contains('\0') => self.id = Some(value.to_owned()),
            "data" => {
                self.frame_bytes = self
                    .frame_bytes
                    .checked_add(std::mem::size_of::<String>())
                    .filter(|size| *size <= MAX_SSE_FRAME)
                    .ok_or(TransportError::Failed)?;
                self.data.push(value.to_owned());
            }
            _ => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<(), ProviderGatewayError> {
        if !self.pending.is_empty() || !self.data.is_empty() || self.comment.is_some() {
            return Err(ProviderGatewayError::InvalidResponse);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use hacienda_core::{auth::Caller, pii::PipelineConfig, HaciendaConfig};

    fn pii_facade() -> HaciendaFacade {
        HaciendaFacade::new(HaciendaConfig::default().with_pii(PipelineConfig::default()))
            .expect("PII facade")
    }

    fn valid_response_events() -> Vec<ProviderSseEvent> {
        vec![
            ProviderSseEvent::new(
                Some("message_start".into()),
                None,
                serde_json::json!({
                    "type": "message_start",
                    "message": {
                        "id": "msg_1",
                        "type": "message",
                        "role": "assistant",
                        "model": "claude-code",
                        "content": [],
                        "stop_reason": null,
                        "stop_sequence": null,
                        "usage": {"input_tokens": 1, "output_tokens": 0}
                    }
                })
                .to_string(),
            ),
            ProviderSseEvent::new(
                Some("content_block_start".into()),
                None,
                serde_json::json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {"type": "text", "text": ""}
                })
                .to_string(),
            ),
            ProviderSseEvent::new(
                Some("content_block_delta".into()),
                None,
                serde_json::json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": "hello"}
                })
                .to_string(),
            ),
            ProviderSseEvent::new(
                Some("content_block_stop".into()),
                None,
                serde_json::json!({"type": "content_block_stop", "index": 0}).to_string(),
            ),
            ProviderSseEvent::new(
                Some("message_delta".into()),
                None,
                serde_json::json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                    "usage": {"output_tokens": 1}
                })
                .to_string(),
            ),
            ProviderSseEvent::new(
                Some("message_stop".into()),
                None,
                serde_json::json!({"type": "message_stop"}).to_string(),
            ),
        ]
    }

    #[tokio::test]
    async fn messages_requires_an_authenticated_identity_when_auth_is_disabled() {
        let (parts, _) = Request::new(()).into_parts();
        let result = messages(
            State(crate::routes::tests::test_state_no_auth()),
            parts,
            HeaderMap::new(),
            SafeJson(serde_json::json!({})),
        )
        .await;

        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("the gateway must not accept a trusted fallback caller"),
        };
        assert_eq!(error.code, crate::error::ApiErrorCode::Unauthenticated);
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

    #[test]
    fn request_tool_and_metadata_fields_are_strictly_allowlisted() {
        let metadata = serde_json::json!({
            "model": "claude-code", "max_tokens": 32, "stream": true,
            "metadata": {"unexpected": "value"},
            "messages": [{"role": "user", "content": "hello"}]
        });
        assert!(validate_and_collect_text_paths(&metadata).is_err());

        let tool = serde_json::json!({
            "model": "claude-code", "max_tokens": 32, "stream": true,
            "tools": [{
                "name": "lookup", "input_schema": {}, "unexpected": "value"
            }],
            "messages": [{"role": "user", "content": "hello"}]
        });
        assert!(validate_and_collect_text_paths(&tool).is_err());
    }

    #[tokio::test]
    async fn provider_text_is_redacted_without_losing_sse_frame_order() {
        let mut events = valid_response_events();
        events[2].data = serde_json::json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "bob@"}
        })
        .to_string();
        events.insert(
            3,
            ProviderSseEvent::new(
                Some("content_block_delta".into()),
                None,
                serde_json::json!({
                    "type": "content_block_delta", "index": 0,
                    "delta": {"type": "text_delta", "text": "example.com"}
                })
                .to_string(),
            ),
        );
        let safe = redact_and_validate_provider_events(&events, &pii_facade(), Caller::Trusted)
            .await
            .expect("provider response redaction");
        assert!(safe
            .iter()
            .all(|event| !event.data.contains("bob@example.com")));
        assert_eq!(safe[0].event.as_deref(), Some("message_start"));
        assert_eq!(safe[6].event.as_deref(), Some("message_stop"));
    }

    fn valid_tool_response_events() -> Vec<ProviderSseEvent> {
        let mut events = valid_response_events();
        events[1].data = serde_json::json!({
            "type": "content_block_start", "index": 0,
            "content_block": {"type": "tool_use", "id": "tool-1", "name": "lookup", "input": {}}
        })
        .to_string();
        events[2].data = serde_json::json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": "{\"email\":\"bob@"}
        })
        .to_string();
        events.insert(
            3,
            ProviderSseEvent::new(
                Some("content_block_delta".into()),
                None,
                serde_json::json!({
                    "type": "content_block_delta", "index": 0,
                    "delta": {"type": "input_json_delta", "partial_json": "example.com\"}"}
                })
                .to_string(),
            ),
        );
        events
    }

    #[tokio::test]
    async fn tool_use_arguments_are_grouped_redacted_and_finalized() {
        let safe = redact_and_validate_provider_events(
            &valid_tool_response_events(),
            &pii_facade(),
            Caller::Trusted,
        )
        .await
        .expect("tool arguments redaction");
        assert!(safe
            .iter()
            .all(|event| !event.data.contains("bob@example.com")));
        let final_delta = safe.iter().find_map(|event| {
            if event.event.as_deref() != Some("content_block_delta") {
                return None;
            }
            let value: Value = serde_json::from_str(&event.data).ok()?;
            let partial = value.pointer("/delta/partial_json")?.as_str()?;
            (!partial.is_empty()).then(|| partial.to_owned())
        });
        assert!(final_delta.is_some_and(|json| serde_json::from_str::<Value>(&json).is_ok()));
    }

    #[tokio::test]
    async fn tool_use_redacts_json_escaped_email_values_after_decoding() {
        let mut events = valid_tool_response_events();
        events[2].data = serde_json::json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": "{\"email\":\"bob\\\\u0040example.com\"}"}
        })
        .to_string();
        events[3].data = serde_json::json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": ""}
        })
        .to_string();

        let safe = redact_and_validate_provider_events(&events, &pii_facade(), Caller::Trusted)
            .await
            .expect("escaped tool argument is redacted");
        let arguments = safe
            .iter()
            .filter_map(|event| serde_json::from_str::<Value>(&event.data).ok())
            .filter_map(|value| {
                value
                    .pointer("/delta/partial_json")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect::<String>();
        let decoded: Value = serde_json::from_str(&arguments).expect("valid finalized arguments");
        assert!(!decoded.to_string().contains("bob@example.com"));
    }

    #[test]
    fn repeated_message_deltas_are_valid_before_message_stop() {
        let mut events = valid_response_events();
        let repeated = events[4].clone();
        events.insert(5, repeated);
        assert!(validate_provider_events(&events).is_ok());
    }

    #[test]
    fn provider_unknown_fields_and_sse_ids_fail_closed() {
        let mut unknown = valid_response_events();
        unknown[2].data = serde_json::json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "hello", "debug": "value"}
        })
        .to_string();
        assert!(validate_provider_events(&unknown).is_err());

        let mut with_id = valid_response_events();
        with_id[0].id = Some("provider-id".into());
        assert!(validate_provider_events(&with_id).is_err());
    }

    #[test]
    fn provider_events_must_follow_the_message_lifecycle() {
        let mut out_of_order = valid_response_events();
        out_of_order.swap(0, 1);
        assert!(validate_provider_events(&out_of_order).is_err());

        let mut duplicate_terminal = valid_response_events();
        let terminal = duplicate_terminal[5].clone();
        duplicate_terminal.push(terminal);
        assert!(validate_provider_events(&duplicate_terminal).is_err());
    }

    #[test]
    fn unknown_events_are_allowed_only_for_explicit_token_payloads() {
        let mut token_only = valid_response_events();
        token_only.insert(
            1,
            ProviderSseEvent::new(
                Some("future_token".into()),
                None,
                serde_json::json!({
                    "type": "future_token", "token": "[PERSON:session:MFRGG2LTMVRXEZLU]"
                })
                .to_string(),
            ),
        );
        assert!(validate_provider_events(&token_only).is_ok());

        let mut plaintext = token_only;
        plaintext[1].data = serde_json::json!({
            "type": "future_token", "token": "hello"
        })
        .to_string();
        assert!(validate_provider_events(&plaintext).is_err());
    }

    #[tokio::test]
    async fn response_redaction_does_not_move_content_into_error_events() {
        let mut events = valid_response_events();
        events.truncate(3);
        events.push(ProviderSseEvent::new(
            Some("content_block_stop".into()),
            None,
            serde_json::json!({"type": "content_block_stop", "index": 0}).to_string(),
        ));
        events.push(ProviderSseEvent::new(
            Some("error".into()),
            None,
            serde_json::json!({
                "type": "error",
                "error": {"type": "overloaded_error", "message": "provider failed"}
            })
            .to_string(),
        ));
        let safe = redact_and_validate_provider_events(&events, &pii_facade(), Caller::Trusted)
            .await
            .expect("provider error response redaction");
        let error: Value = serde_json::from_str(&safe[4].data).unwrap();
        assert_eq!(error["error"]["message"], "provider failed");
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

    fn valid_thinking_response_events() -> Vec<ProviderSseEvent> {
        let mut events = valid_response_events();
        events[1].data = serde_json::json!({
            "type": "content_block_start", "index": 0,
            "content_block": {"type": "thinking", "thinking": "", "signature": ""}
        })
        .to_string();
        events[2].data = serde_json::json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "thinking_delta", "thinking": "note alice@"}
        })
        .to_string();
        events.insert(
            3,
            ProviderSseEvent::new(
                Some("content_block_delta".into()),
                None,
                serde_json::json!({
                    "type": "content_block_delta", "index": 0,
                    "delta": {"type": "signature_delta", "signature": "c2ln"}
                })
                .to_string(),
            ),
        );
        events.insert(
            4,
            ProviderSseEvent::new(
                Some("content_block_delta".into()),
                None,
                serde_json::json!({
                    "type": "content_block_delta", "index": 0,
                    "delta": {"type": "thinking_delta", "thinking": "example.com"}
                })
                .to_string(),
            ),
        );
        events
    }

    #[tokio::test]
    async fn thinking_deltas_are_redacted_as_one_logical_block() {
        let events = valid_thinking_response_events();
        let safe = redact_and_validate_provider_events(&events, &pii_facade(), Caller::Trusted)
            .await
            .expect("thinking redaction");
        assert!(safe
            .iter()
            .all(|event| !event.data.contains("alice@example.com")));
        let joined: String = safe
            .iter()
            .filter_map(|event| {
                let value = serde_json::from_str::<Value>(&event.data).ok()?;
                value
                    .pointer("/delta/thinking")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        assert!(!joined.contains("alice@example.com"));
        assert!(joined.contains("[EMAIL]"));
        assert!(!safe.iter().any(|event| {
            serde_json::from_str::<Value>(&event.data)
                .ok()
                .and_then(|value| {
                    value
                        .pointer("/delta/signature")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                == Some("c2ln".to_owned())
        }));
    }

    #[tokio::test]
    async fn keepalive_comments_are_relayed_but_plaintext_comments_fail_closed() {
        let mut events = valid_response_events();
        let keepalive = ProviderSseEvent {
            event: None,
            id: None,
            retry: None,
            comment: Some("ping - 1763990000".to_owned()),
            data: String::new(),
        };
        events.insert(1, keepalive.clone());
        let safe = redact_and_validate_provider_events(&events, &pii_facade(), Caller::Trusted)
            .await
            .expect("safe keepalive is relayed");
        assert!(safe
            .iter()
            .any(|event| event.comment.as_deref() == Some("ping - 1763990000")));
        let rendered = Sse::new(futures::stream::once(async move {
            Ok::<Event, Infallible>(render_event(keepalive))
        }))
        .into_response();
        let bytes = axum::body::to_bytes(rendered.into_body(), 1024)
            .await
            .expect("rendered keepalive");
        assert!(String::from_utf8_lossy(&bytes).contains(": ping - 1763990000"));

        let mut unsafe_events = valid_response_events();
        unsafe_events.insert(
            1,
            ProviderSseEvent {
                event: None,
                id: None,
                retry: None,
                comment: Some("alice@example.com".to_owned()),
                data: String::new(),
            },
        );
        assert!(validate_provider_events(&unsafe_events).is_err());
    }

    #[tokio::test]
    async fn tool_use_name_is_redacted_before_the_block_is_relayed() {
        let mut events = valid_tool_response_events();
        events[1].data = serde_json::json!({
            "type": "content_block_start", "index": 0,
            "content_block": {
                "type": "tool_use", "id": "tool-1",
                "name": "alice@example.com", "input": {}
            }
        })
        .to_string();
        let safe = redact_and_validate_provider_events(&events, &pii_facade(), Caller::Trusted)
            .await
            .expect("tool name redaction");
        let start: Value = serde_json::from_str(&safe[1].data).expect("block start JSON");
        let name = start["content_block"]["name"].as_str().expect("name");
        assert!(!name.contains("alice@example.com"));
        assert!(name.contains("[EMAIL]"));
    }
}
