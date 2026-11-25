//! Authenticated OpenAI Responses gateway for Codex inside ChatGPT Desktop.
//!
//! Codex is configured with a provider whose `base_url` ends in `/v1` and whose
//! `wire_api` is `responses`. It therefore sends the *original* prompt to this
//! server. The only place where that prompt may become pseudonymous is here,
//! before any egress: [`responses`] walks every inspectable text-bearing field
//! of the Responses request, redacts it through the caller's tenant-scoped
//! [`HaciendaFacade`], and hands the provider a token-only JSON body.
//!
//! The upstream is the official Codex Responses path. Inference authentication
//! belongs to Codex: the ChatGPT access token and account id arrive on the
//! request and are forwarded, unchanged, to that upstream only after the local
//! gateway secret has been validated. `require_gateway_caller` below enforces
//! that secret independently of `Authorization`, so a process with auth disabled
//! still cannot use this gateway as an unauthenticated inference proxy.
//!
//! The response side is deliberately symmetric. Provider SSE frames are decoded
//! locally, validated against a strict event allowlist, and every content delta
//! is buffered until its logical item finishes. A PII value split across
//! several deltas is therefore redacted as one string before any frame is
//! released to Codex. Unknown event types, attachments, ids/comments on SSE
//! frames, and unfinished streams fail closed: the stream ends without the
//! relay carrying any unreviewed content.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    convert::Infallible,
    fmt,
    pin::Pin,
    sync::{Arc, Mutex},
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
    HaciendaFacade,
};
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};
use url::Url;
use uuid::Uuid;

use crate::{
    error::ApiError,
    extract::Json as SafeJson,
    handlers::{
        claude_messages::ProviderGatewayError,
        codex_viewer::{ViewerEvent, ViewerSessionStore},
    },
    state::ApiState,
};

/// Default upstream. Overridable by the host for tests and self-hosted gateways.
pub const DEFAULT_CODEX_RESPONSES_ENDPOINT: &str =
    "https://chatgpt.com/backend-api/codex/responses";
/// Model advertised by `/v1/models` when the host does not configure a list.
pub const DEFAULT_CODEX_MODEL: &str = "hacienda-codex";
/// Local gateway secret header. `Authorization` is reserved for the ChatGPT
/// token that Codex obtained through OpenAI sign-in.
pub const GATEWAY_SECRET_HEADER: &str = "x-hacienda-gateway-token";
/// Account header emitted by Codex alongside the ChatGPT bearer token.
pub const CODEX_ACCOUNT_HEADER: &str = "chatgpt-account-id";

/// Token-only request handed to a Codex Responses provider implementation.
///
/// `Debug` never renders the body or headers: endpoints, credentials, tenant
/// ids, and prompt text must not reach logs through a diagnostic format.
#[derive(Clone)]
pub struct CodexResponsesProviderRequest {
    pub body: Value,
    /// Bounded routing headers only; never the local gateway secret.
    pub headers: Vec<(String, String)>,
    /// The user's ChatGPT bearer token, forwarded unchanged to the upstream.
    pub authorization: String,
    /// The account id Codex supplied in `ChatGPT-Account-ID`.
    pub account_id: String,
}

impl fmt::Debug for CodexResponsesProviderRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodexResponsesProviderRequest")
            .field("body", &"<token-only>")
            .field("headers", &"<routing-only>")
            .field("authorization", &"<redacted>")
            .field("account_id", &"<redacted>")
            .finish()
    }
}

/// Response metadata safe to copy back to the local client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexResponsesProviderResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

pub type CodexResponsesEventStream =
    Pin<Box<dyn Stream<Item = Result<ProviderSseEvent, ProviderGatewayError>> + Send + 'static>>;

pub struct CodexResponsesProviderStream {
    pub response: CodexResponsesProviderResponse,
    pub events: CodexResponsesEventStream,
}

/// Provider seam used by the handler and by focused transport fixtures.
#[async_trait]
pub trait CodexResponsesProvider: Send + Sync {
    /// Start a token-only streaming Responses call.
    async fn stream(
        &self,
        request: CodexResponsesProviderRequest,
    ) -> Result<CodexResponsesProviderStream, ProviderGatewayError>;

    /// Server-owned model ids advertised by `GET /v1/models`. No network call,
    /// no client input, and no credential in the returned strings.
    fn models(&self) -> Vec<String>;
}

/// HTTP provider for the official Codex Responses path.
///
/// No credential is stored here: the ChatGPT bearer token and account id come
/// from the authenticated Codex request and are forwarded only to the official
/// upstream. The client is injected so tests can prove the payload is
/// token-only without opening a socket. `with_reqwest` is the production
/// constructor.
pub struct CodexResponsesHttpClientProvider<H> {
    endpoint: String,
    models: Vec<String>,
    http: Arc<Mutex<H>>,
}

impl<H> CodexResponsesHttpClientProvider<H> {
    pub fn new(
        endpoint: impl Into<String>,
        models: Vec<String>,
        http: H,
    ) -> Result<Self, ProviderGatewayError> {
        let endpoint = endpoint.into();
        let parsed = Url::parse(&endpoint).map_err(|_| ProviderGatewayError::Unavailable)?;
        let host = parsed.host_str().ok_or(ProviderGatewayError::Unavailable)?;
        let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
        if parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback) {
            return Err(ProviderGatewayError::Unavailable);
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            // Credentials in a URL would leak through transport diagnostics.
            return Err(ProviderGatewayError::Unavailable);
        }
        if models.is_empty()
            || models
                .iter()
                .any(|model| model.is_empty() || model.len() > 128 || !safe_header_value(model))
        {
            return Err(ProviderGatewayError::Unavailable);
        }
        Ok(Self {
            endpoint,
            models,
            http: Arc::new(Mutex::new(http)),
        })
    }
}

impl CodexResponsesHttpClientProvider<ReqwestHttpClient> {
    pub fn with_reqwest(
        endpoint: impl Into<String>,
        models: Vec<String>,
    ) -> Result<Self, ProviderGatewayError> {
        let http = ReqwestHttpClient::new().map_err(|_| ProviderGatewayError::Unavailable)?;
        Self::new(endpoint, models, http)
    }

    /// Host bootstrap helper. The gateway is opt-in: it is enabled only when
    /// `HACIENDA_CODEX_RESPONSES_ENDPOINT` is set. No inference credential is
    /// read here — the ChatGPT token and account id arrive per request from
    /// Codex and are forwarded to the upstream only.
    ///
    /// - `HACIENDA_CODEX_RESPONSES_ENDPOINT` — official Codex Responses path.
    /// - `HACIENDA_CODEX_MODELS` — optional comma-separated model ids.
    pub fn from_env() -> Result<Option<Self>, ProviderGatewayError> {
        let read = |name: &str| -> Result<Option<String>, ProviderGatewayError> {
            match std::env::var(name) {
                Ok(value) => Ok(Some(value)),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(std::env::VarError::NotUnicode(_)) => Err(ProviderGatewayError::Unavailable),
            }
        };
        let Some(endpoint) = read("HACIENDA_CODEX_RESPONSES_ENDPOINT")? else {
            return Ok(None);
        };
        let models: Vec<String> = read("HACIENDA_CODEX_MODELS")?
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|model| !model.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .filter(|models: &Vec<String>| !models.is_empty())
            .unwrap_or_else(|| vec![DEFAULT_CODEX_MODEL.to_owned()]);
        Self::with_reqwest(endpoint, models).map(Some)
    }
}

fn safe_header_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.contains(['\r', '\n', '\0'])
        && !value.chars().any(char::is_control)
}

#[async_trait]
impl<H> CodexResponsesProvider for CodexResponsesHttpClientProvider<H>
where
    H: ProviderHttpClient + Send + 'static,
{
    async fn stream(
        &self,
        request: CodexResponsesProviderRequest,
    ) -> Result<CodexResponsesProviderStream, ProviderGatewayError> {
        let body =
            serde_json::to_vec(&request.body).map_err(|_| ProviderGatewayError::Transport)?;
        let mut headers = vec![
            ("content-type".to_owned(), "application/json".to_owned()),
            ("accept".to_owned(), "text/event-stream".to_owned()),
            ("authorization".to_owned(), request.authorization),
            ("chatgpt-account-id".to_owned(), request.account_id),
        ];
        // Only the ChatGPT credential and bounded routing headers are
        // forwarded. The local gateway secret never reaches this seam.
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
                                Ok(CodexResponsesProviderResponse {
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
        Ok(CodexResponsesProviderStream {
            response,
            events: Box::pin(events),
        })
    }

    fn models(&self) -> Vec<String> {
        self.models.clone()
    }
}

/// `POST /v1/responses` — the Codex Responses gateway.
#[utoipa::path(
    post,
    path = "/v1/responses",
    tag = "responses",
    operation_id = "codexResponses",
    security(("bearerAuth" = [])),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Redacted Responses Server-Sent Events", content_type = "text/event-stream"),
        (status = 400, description = "Invalid or uninspectable Responses request"),
        (status = 401, description = "Missing or invalid local gateway credentials"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn responses(
    State(state): State<ApiState>,
    parts: Parts,
    headers: HeaderMap,
    SafeJson(mut body): SafeJson<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let auth_context = require_gateway_caller(&parts, &state)?;
    let caller = Caller::Principal(auth_context.as_ref());

    // Only after the local secret has been validated do we read the user's
    // inference credential. It is forwarded upstream, never logged or used for
    // local authorization.
    let (authorization, account_id, routing_headers) = forwarded_credentials(&headers)?;

    let provider = state
        .codex_responses_provider
        .as_ref()
        .ok_or_else(|| ApiError::invalid_request("The Responses gateway is not configured."))?;

    let text_paths = validate_and_collect_text_paths(&body)
        .map_err(|_| ApiError::invalid_request("The request contains unsupported content."))?;

    // Local viewer mapping only. Reveal is re-scoped to the same tenant under a
    // server-side context; the gateway caller does not gain `pii:reveal`, and
    // the original values never enter the provider request or a log line.
    let reveal_context = AuthContext::with_tenant(
        auth_context.principal_id.clone(),
        auth_context.tenant.clone(),
        CapabilitySet::new([Capability::PiiReveal]),
    );
    let mut mappings = HashMap::new();
    redact_paths(
        &mut body,
        &text_paths,
        &state.facade,
        caller,
        &reveal_context,
        &mut mappings,
    )
    .await?;

    let turn_id = Uuid::new_v4().simple().to_string();
    for (token, original) in &mappings {
        state
            .viewer_sessions
            .publish_to_active(ViewerEvent::Mapping {
                turn_id: turn_id.clone(),
                token: token.clone(),
                original: original.clone(),
            });
    }

    // Provider-side context must never outlive this call: a later turn cannot
    // reference an earlier pseudonymous response by id, and a client cannot
    // smuggle provider-held content back in through `previous_response_id`.
    body.as_object_mut()
        .ok_or_else(ApiError::internal)?
        .insert("store".to_owned(), Value::Bool(false));

    let provider_request = CodexResponsesProviderRequest {
        body,
        headers: routing_headers,
        authorization,
        account_id,
    };
    let provider_stream = provider.stream(provider_request).await.map_err(|error| {
        tracing::error!(error = %error, "codex responses provider failed");
        ApiError::internal()
    })?;
    let response_headers = safe_response_headers(&provider_stream.response.headers)?;
    let facade = Arc::clone(&state.facade);
    let relay = ResponseRelay::new(
        provider_stream.events,
        facade,
        auth_context,
        Arc::clone(&state.viewer_sessions),
        turn_id,
    );
    let output = stream::unfold(relay, |mut relay| async move {
        relay
            .next_event()
            .await
            .map(|event| (Ok::<Event, Infallible>(event), relay))
    });
    Ok((
        response_headers,
        Sse::new(output).keep_alive(KeepAlive::default()),
    ))
}

/// `GET /v1/models` — server-owned model discovery for the Responses provider.
#[utoipa::path(
    get,
    path = "/v1/models",
    tag = "responses",
    operation_id = "codexModels",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Server-owned model identifiers"),
        (status = 401, description = "Missing or invalid local gateway credentials"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn models(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<axum::Json<Value>, ApiError> {
    require_gateway_caller(&parts, &state)?;
    let provider = state
        .codex_responses_provider
        .as_ref()
        .ok_or_else(|| ApiError::invalid_request("The Responses gateway is not configured."))?;
    let data = provider
        .models()
        .into_iter()
        .map(|id| {
            serde_json::json!({
                "id": id,
                "object": "model",
                "created": 0,
                "owned_by": "hacienda",
            })
        })
        .collect::<Vec<_>>();
    Ok(axum::Json(serde_json::json!({
        "object": "list",
        "data": data,
    })))
}

/// Local gateway authentication is independent of the inference credential.
/// The dedicated header carries the locally issued secret; `Authorization`
/// stays untouched so the ChatGPT token can be forwarded upstream.
fn require_gateway_caller(parts: &Parts, state: &ApiState) -> Result<Arc<AuthContext>, ApiError> {
    let expected = state
        .codex_gateway_secret
        .as_ref()
        .ok_or_else(|| ApiError::invalid_request("The Responses gateway is not configured."))?;
    let presented = parts
        .headers
        .get(GATEWAY_SECRET_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(ApiError::unauthenticated)?;
    if !safe_header_value(presented) || *blake3::hash(presented.as_bytes()).as_bytes() != *expected
    {
        return Err(ApiError::unauthenticated());
    }
    // Server-owned context: never derived from client-supplied tenant fields.
    Ok(Arc::new(AuthContext::new(
        "codex-local-gateway",
        CapabilitySet::new([Capability::DocumentsProcess]),
    )))
}

fn safe_response_headers(headers: &[(String, String)]) -> Result<HeaderMap, ApiError> {
    const ALLOWED: &[&str] = &[
        "cache-control",
        "request-id",
        "retry-after",
        "x-request-id",
        "openai-processing-ms",
        "openai-version",
    ];
    let mut safe = HeaderMap::new();
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

/// Extract the two credentials Codex supplies and the allowlisted routing
/// headers. Both credentials are validated locally but never logged; only the
/// ChatGPT bearer and account id are forwarded to the official upstream.
fn forwarded_credentials(
    headers: &HeaderMap,
) -> Result<(String, String, Vec<(String, String)>), ApiError> {
    let authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| !token.trim().is_empty() && token.len() <= 8192)
        .filter(|token| safe_header_value(token))
        .ok_or_else(ApiError::unauthenticated)?
        .to_owned();
    let account_id = headers
        .get(CODEX_ACCOUNT_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            safe_header_value(value)
                && value.len() <= 256
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
        .ok_or_else(ApiError::unauthenticated)?
        .to_owned();

    let mut routing = Vec::new();
    for name in ["x-request-id", "openai-beta"] {
        let header_name = HeaderName::from_static(name);
        for value in headers.get_all(&header_name).iter() {
            let value = value
                .to_str()
                .map_err(|_| ApiError::invalid_request("The request contains invalid headers."))?;
            if !safe_header_value(value) {
                return Err(ApiError::invalid_request(
                    "The request contains invalid headers.",
                ));
            }
            routing.push((name.to_owned(), value.to_owned()));
        }
    }
    Ok((format!("Bearer {authorization}"), account_id, routing))
}

// ─── Request inspection ──────────────────────────────────────────────────────

const TOP_LEVEL_FIELDS: &[&str] = &[
    "model",
    "input",
    "instructions",
    "tools",
    "tool_choice",
    "max_output_tokens",
    "max_tokens",
    "temperature",
    "top_p",
    "n",
    "parallel_tool_calls",
    "stream",
    "store",
    "metadata",
    "user",
    "service_tier",
    "text",
    "reasoning",
    "include",
    "stream_options",
    "safety_identifier",
    "prompt_cache_key",
    "background",
];

fn validate_and_collect_text_paths(body: &Value) -> Result<Vec<String>, ()> {
    let object = body.as_object().ok_or(())?;
    reject_unknown_keys(object, TOP_LEVEL_FIELDS)?;
    let model = require_string(object, "model")?;
    if model.len() > 256 {
        return Err(());
    }
    if object.get("stream").and_then(Value::as_bool) != Some(true) {
        return Err(());
    }
    if object
        .get("store")
        .is_some_and(|value| value.as_bool() != Some(false))
    {
        return Err(());
    }
    if let Some(n) = object.get("n") {
        if !matches!(n.as_u64(), Some(1..=16)) {
            return Err(());
        }
    }
    for key in ["max_output_tokens", "max_tokens"] {
        if let Some(value) = object.get(key) {
            if value.as_u64().is_none_or(|count| count == 0) {
                return Err(());
            }
        }
    }
    for key in ["temperature", "top_p"] {
        if let Some(value) = object.get(key) {
            let number = value.as_f64().ok_or(())?;
            if !(0.0..=2.0).contains(&number) {
                return Err(());
            }
        }
    }
    if let Some(value) = object.get("parallel_tool_calls") {
        if !value.is_boolean() {
            return Err(());
        }
    }
    if let Some(value) = object.get("background") {
        if !value.is_boolean() {
            return Err(());
        }
    }

    let mut paths = Vec::new();
    match object.get("input").ok_or(())? {
        Value::String(text) => {
            if text.len() > MAX_TEXT_FIELD {
                return Err(());
            }
            paths.push("/input".to_owned());
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, item) in items.iter().enumerate() {
                collect_input_item(item, vec!["input".into(), index.to_string()], &mut paths)?;
            }
        }
        _ => return Err(()),
    }
    if let Some(instructions) = object.get("instructions") {
        require_string_of(instructions, MAX_TEXT_FIELD)?;
        paths.push("/instructions".to_owned());
    }
    if let Some(tools) = object.get("tools") {
        collect_tools(tools, vec!["tools".into()], &mut paths)?;
    }
    if let Some(choice) = object.get("tool_choice") {
        collect_tool_choice(choice, vec!["tool_choice".into()], &mut paths)?;
    }
    if let Some(metadata) = object.get("metadata") {
        collect_metadata(metadata, vec!["metadata".into()], &mut paths)?;
    }
    for key in [
        "text",
        "reasoning",
        "include",
        "stream_options",
        "safety_identifier",
        "prompt_cache_key",
        "service_tier",
        "user",
    ] {
        if let Some(value) = object.get(key) {
            collect_all_strings(value, vec![key.to_owned()], &mut paths)?;
        }
    }
    Ok(paths)
}

const MAX_TEXT_FIELD: usize = 1024 * 1024;
const MAX_METADATA_ENTRIES: usize = 64;

fn collect_input_item(value: &Value, path: Vec<String>, paths: &mut Vec<String>) -> Result<(), ()> {
    let object = value.as_object().ok_or(())?;
    reject_tenant_keys(object)?;
    let kind = require_string(object, "type")?;
    match kind {
        "message" => {
            reject_unknown_keys(object, &["type", "role", "content"])?;
            let role = require_string(object, "role")?;
            if !matches!(role, "user" | "assistant" | "system" | "developer") {
                return Err(());
            }
            collect_content(
                object.get("content").ok_or(())?,
                path_with(&path, "content"),
                paths,
            )?;
        }
        "function_call" => {
            reject_unknown_keys(object, &["type", "call_id", "name", "arguments"])?;
            require_string(object, "call_id")?;
            require_string_of(object.get("name").ok_or(())?, 256)?;
            require_string_of(object.get("arguments").ok_or(())?, MAX_TEXT_FIELD)?;
            paths.push(path_value(&path_with(&path, "name")));
            paths.push(path_value(&path_with(&path, "arguments")));
        }
        "function_call_output" => {
            reject_unknown_keys(object, &["type", "call_id", "output"])?;
            require_string(object, "call_id")?;
            let output = object.get("output").ok_or(())?;
            match output {
                Value::String(text) if text.len() <= MAX_TEXT_FIELD => {
                    paths.push(path_value(&path_with(&path, "output")));
                }
                Value::Array(parts) => {
                    for (index, part) in parts.iter().enumerate() {
                        collect_content_part(
                            part,
                            path_with(&path_with(&path, "output"), &index.to_string()),
                            paths,
                        )?;
                    }
                }
                _ => return Err(()),
            }
        }
        // Images, files, provider-held item references, and future item types
        // cannot be proven inspectable before egress.
        _ => return Err(()),
    }
    Ok(())
}

fn collect_content(value: &Value, path: Vec<String>, paths: &mut Vec<String>) -> Result<(), ()> {
    match value {
        Value::String(text) if text.len() <= MAX_TEXT_FIELD => {
            paths.push(path_value(&path));
            Ok(())
        }
        Value::Array(parts) => {
            for (index, part) in parts.iter().enumerate() {
                collect_content_part(part, path_with(&path, &index.to_string()), paths)?;
            }
            Ok(())
        }
        _ => Err(()),
    }
}

fn collect_content_part(
    value: &Value,
    path: Vec<String>,
    paths: &mut Vec<String>,
) -> Result<(), ()> {
    let object = value.as_object().ok_or(())?;
    reject_tenant_keys(object)?;
    let kind = require_string(object, "type")?;
    match kind {
        "input_text" | "output_text" | "text" => {
            reject_unknown_keys(object, &["type", "text"])?;
            require_string_of(object.get("text").ok_or(())?, MAX_TEXT_FIELD)?;
            paths.push(path_value(&path_with(&path, "text")));
        }
        "refusal" => {
            reject_unknown_keys(object, &["type", "refusal"])?;
            require_string_of(object.get("refusal").ok_or(())?, MAX_TEXT_FIELD)?;
            paths.push(path_value(&path_with(&path, "refusal")));
        }
        _ => return Err(()),
    }
    Ok(())
}

fn collect_tools(value: &Value, path: Vec<String>, paths: &mut Vec<String>) -> Result<(), ()> {
    let tools = value.as_array().ok_or(())?;
    for (index, tool) in tools.iter().enumerate() {
        let object = tool.as_object().ok_or(())?;
        reject_tenant_keys(object)?;
        if require_string(object, "type")? != "function" {
            // Hosted tools can produce content the gateway has not inspected.
            return Err(());
        }
        reject_unknown_keys(
            object,
            &["type", "name", "description", "parameters", "strict"],
        )?;
        let tool_path = path_with(&path, &index.to_string());
        require_string_of(object.get("name").ok_or(())?, 256)?;
        paths.push(path_value(&path_with(&tool_path, "name")));
        if let Some(description) = object.get("description") {
            require_string_of(description, MAX_TEXT_FIELD)?;
            paths.push(path_value(&path_with(&tool_path, "description")));
        }
        if let Some(parameters) = object.get("parameters") {
            if !parameters.is_object() {
                return Err(());
            }
            collect_all_strings(parameters, path_with(&tool_path, "parameters"), paths)?;
        }
        if let Some(strict) = object.get("strict") {
            if !strict.is_boolean() {
                return Err(());
            }
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
        Value::String(choice) if matches!(choice.as_str(), "auto" | "none" | "required") => Ok(()),
        Value::Object(object) => {
            reject_unknown_keys(object, &["type", "name"])?;
            if require_string(object, "type")? != "function" {
                return Err(());
            }
            require_string_of(object.get("name").ok_or(())?, 256)?;
            paths.push(path_value(&path_with(&path, "name")));
            Ok(())
        }
        _ => Err(()),
    }
}

fn collect_metadata(value: &Value, path: Vec<String>, paths: &mut Vec<String>) -> Result<(), ()> {
    let object = value.as_object().ok_or(())?;
    reject_tenant_keys(object)?;
    if object.len() > MAX_METADATA_ENTRIES {
        return Err(());
    }
    for (key, value) in object {
        require_string_of(value, 4096)?;
        paths.push(path_value(&path_with(&path, key)));
    }
    Ok(())
}

fn collect_all_strings(
    value: &Value,
    path: Vec<String>,
    paths: &mut Vec<String>,
) -> Result<(), ()> {
    match value {
        Value::String(_) => {
            if value
                .as_str()
                .is_some_and(|text| text.len() > MAX_TEXT_FIELD)
            {
                return Err(());
            }
            paths.push(path_value(&path));
        }
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
        .filter(|text| !text.is_empty())
        .ok_or(())
}

fn require_string_of(value: &Value, max_len: usize) -> Result<&str, ()> {
    value
        .as_str()
        .filter(|text| text.len() <= max_len)
        .ok_or(())
}

fn path_with(path: &[String], child: &str) -> Vec<String> {
    let mut path = path.to_vec();
    path.push(child.to_owned());
    path
}

fn path_value(path: &[String]) -> String {
    path.iter()
        .map(String::as_str)
        .fold(String::new(), |mut output, segment| {
            output.push('/');
            output.push_str(&segment.replace('~', "~0").replace('/', "~1"));
            output
        })
}

async fn redact_paths(
    body: &mut Value,
    paths: &[String],
    facade: &HaciendaFacade,
    caller: Caller<'_>,
    reveal_context: &AuthContext,
    mappings: &mut HashMap<String, String>,
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
        collect_local_mappings(facade, reveal_context, &redacted, mappings).await;
        let slot = body.pointer_mut(path).ok_or_else(ApiError::internal)?;
        *slot = Value::String(redacted);
    }
    Ok(())
}

/// Collect `token -> original` pairs for tokens this gateway just minted. The
/// reveal uses the facade's local key material only; a failed reveal (mask or
/// hash mode, unreadable token) simply leaves the token unrehydratable in the
/// viewer and never fails the provider call.
async fn collect_local_mappings(
    facade: &HaciendaFacade,
    reveal_context: &AuthContext,
    redacted: &str,
    mappings: &mut HashMap<String, String>,
) {
    for token in extract_hacienda_tokens(redacted) {
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
}

fn extract_hacienda_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let Some(end) = rest[start..].find(']') else {
            break;
        };
        let end = start + end;
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

// ─── Response inspection ─────────────────────────────────────────────────────

const MAX_SSE_FRAME: usize = 1024 * 1024;
const MAX_GROUP_BUFFER: usize = 1024 * 1024;
const MAX_ACTIVE_GROUPS: usize = 256;

struct ResponseRelay {
    events: CodexResponsesEventStream,
    facade: Arc<HaciendaFacade>,
    auth_context: Arc<hacienda_core::auth::AuthContext>,
    viewer_sessions: Arc<ViewerSessionStore>,
    turn_id: String,
    validator: ResponseValidator,
    groups: HashMap<GroupKey, BufferedGroup>,
    pending: VecDeque<ProviderSseEvent>,
    closed: bool,
}

struct BufferedGroup {
    joined: String,
    deltas: Vec<ProviderSseEvent>,
    total: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum DeltaKind {
    OutputText,
    Refusal,
    FunctionArguments,
    ReasoningSummary,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct GroupKey {
    item_id: String,
    output_index: u64,
    index: u64,
    kind: DeltaKind,
}

impl ResponseRelay {
    fn new(
        events: CodexResponsesEventStream,
        facade: Arc<HaciendaFacade>,
        auth_context: Arc<hacienda_core::auth::AuthContext>,
        viewer_sessions: Arc<ViewerSessionStore>,
        turn_id: String,
    ) -> Self {
        Self {
            events,
            facade,
            auth_context,
            viewer_sessions,
            turn_id,
            validator: ResponseValidator::default(),
            groups: HashMap::new(),
            pending: VecDeque::new(),
            closed: false,
        }
    }

    async fn next_event(&mut self) -> Option<Event> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                self.publish_viewer_text(&event);
                return Some(render_event(event));
            }
            if self.closed {
                return None;
            }
            let Some(result) = self.events.next().await else {
                self.closed = self.validator.finish().is_err() || !self.groups.is_empty();
                if !self.closed {
                    self.viewer_sessions.publish_to_active(ViewerEvent::End {
                        turn_id: self.turn_id.clone(),
                    });
                }
                return None;
            };
            let Ok(event) = result else {
                self.closed = true;
                return None;
            };
            if self.accept(event).await.is_err() {
                self.closed = true;
                return None;
            }
        }
    }

    /// Only text that has already passed the gateway's redaction is published
    /// to the viewer; this is a separate in-memory channel from Codex's SSE.
    fn publish_viewer_text(&self, event: &ProviderSseEvent) {
        let Ok(value) = serde_json::from_str::<Value>(&event.data) else {
            return;
        };
        for pointer in ["/delta", "/text", "/refusal", "/error/message"] {
            if let Some(text) = value.pointer(pointer).and_then(Value::as_str) {
                if !text.is_empty() && text.len() <= MAX_TEXT_FIELD {
                    self.viewer_sessions.publish_to_active(ViewerEvent::Text {
                        turn_id: self.turn_id.clone(),
                        text: text.to_owned(),
                    });
                }
                return;
            }
        }
    }

    async fn accept(&mut self, event: ProviderSseEvent) -> Result<(), ()> {
        let action = self.validator.accept(&event)?;
        match action {
            ValidationAction::Immediate => {
                let safe = self.redact_event(event).await?;
                self.pending.push_back(safe);
            }
            ValidationAction::Delta { key, fragment } => {
                if self.groups.len() >= MAX_ACTIVE_GROUPS {
                    return Err(());
                }
                let group = self.groups.entry(key).or_insert_with(|| BufferedGroup {
                    joined: String::new(),
                    deltas: Vec::new(),
                    total: 0,
                });
                group.total = group.total.saturating_add(fragment.len());
                if group.total > MAX_GROUP_BUFFER {
                    return Err(());
                }
                group.joined.push_str(&fragment);
                group.deltas.push(event);
            }
            ValidationAction::Done { key, field } => {
                // A terminal event cannot arrive before the content it closes.
                let group = self.groups.remove(&key).ok_or(())?;
                let caller = Caller::Principal(self.auth_context.as_ref());
                let mut redacted = self
                    .facade
                    .redact_text_with_auth(caller, &group.joined)
                    .await
                    .map_err(|_| ())?
                    .redacted_text;
                if key.kind == DeltaKind::FunctionArguments && !is_json_object(&redacted) {
                    return Err(());
                }
                if redacted.len() > MAX_GROUP_BUFFER {
                    return Err(());
                }
                if key.kind == DeltaKind::FunctionArguments {
                    // Keep the stable JSON spelling so the done event and the
                    // rewritten deltas agree exactly.
                    redacted = serde_json::to_string(
                        &serde_json::from_str::<Value>(&redacted).map_err(|_| ())?,
                    )
                    .map_err(|_| ())?;
                }
                for mut delta in group.deltas {
                    let mut value: Value = serde_json::from_str(&delta.data).map_err(|_| ())?;
                    *value.pointer_mut("/delta").ok_or(())? = Value::String(String::new());
                    delta.data = serde_json::to_string(&value).map_err(|_| ())?;
                    self.pending.push_back(delta);
                }
                if let Some(last) = self.pending.back_mut() {
                    let mut value: Value = serde_json::from_str(&last.data).map_err(|_| ())?;
                    *value.pointer_mut("/delta").ok_or(())? = Value::String(redacted.clone());
                    last.data = serde_json::to_string(&value).map_err(|_| ())?;
                }
                let mut done: Value = serde_json::from_str(&event.data).map_err(|_| ())?;
                let pointer = format!("/{field}");
                *done.pointer_mut(&pointer).ok_or(())? = Value::String(redacted);
                let mut safe = event;
                safe.data = serde_json::to_string(&done).map_err(|_| ())?;
                self.pending.push_back(safe);
            }
        }
        Ok(())
    }

    async fn redact_event(&mut self, mut event: ProviderSseEvent) -> Result<ProviderSseEvent, ()> {
        let mut value: Value = serde_json::from_str(&event.data).map_err(|_| ())?;
        let caller = Caller::Principal(self.auth_context.as_ref());
        redact_value_strings(&mut value, caller, &self.facade).await?;
        event.data = serde_json::to_string(&value).map_err(|_| ())?;
        Ok(event)
    }
}

fn is_json_object(text: &str) -> bool {
    serde_json::from_str::<Value>(text).is_ok_and(|value| value.is_object())
}

fn render_event(event: ProviderSseEvent) -> Event {
    let mut rendered = Event::default();
    if let Some(kind) = event.event {
        rendered = rendered.event(kind);
    }
    let _ = event.id; // ids are refused by the validator; never forwarded.
    rendered.data(event.data)
}

/// Fields whose values are protocol metadata, not model-visible content.
/// Everything else is passed through the redaction pipeline before egress.
const STRUCTURAL_KEYS: &[&str] = &[
    "type",
    "id",
    "object",
    "created_at",
    "status",
    "model",
    "output_index",
    "content_index",
    "summary_index",
    "item_id",
    "sequence_number",
    "call_id",
    "index",
    "code",
    "param",
    "finish_reason",
    "role",
    "reasoning_effort",
    "store",
    "stream",
    "parallel_tool_calls",
    "temperature",
    "top_p",
    "max_output_tokens",
    "n",
    "input_tokens",
    "output_tokens",
    "total_tokens",
    "reasoning_tokens",
    "cached_tokens",
    "service_tier",
];

fn redact_value_strings<'a>(
    value: &'a mut Value,
    caller: Caller<'a>,
    facade: &'a HaciendaFacade,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ()>> + Send + 'a>> {
    Box::pin(async move {
        match value {
            Value::String(text) => {
                if text.len() > MAX_TEXT_FIELD {
                    return Err(());
                }
                Ok(())
            }
            Value::Array(items) => {
                for item in items.iter_mut() {
                    if let Some(text) = item.as_str() {
                        if text.len() > MAX_TEXT_FIELD {
                            return Err(());
                        }
                        let redacted = facade
                            .redact_text_with_auth(caller, text)
                            .await
                            .map_err(|_| ())?
                            .redacted_text;
                        *item = Value::String(redacted);
                    } else {
                        redact_value_strings(item, caller, facade).await?;
                    }
                }
                Ok(())
            }
            Value::Object(object) => {
                reject_tenant_keys(object)?;
                let keys: Vec<String> = object.keys().cloned().collect();
                for key in keys {
                    let needs_redaction = !STRUCTURAL_KEYS.contains(&key.as_str())
                        && object.get(&key).is_some_and(|value| value.is_string());
                    if needs_redaction {
                        let redacted = {
                            let text = object
                                .get(&key)
                                .and_then(Value::as_str)
                                .filter(|text| text.len() <= MAX_TEXT_FIELD)
                                .ok_or(())?;
                            facade
                                .redact_text_with_auth(caller, text)
                                .await
                                .map_err(|_| ())?
                                .redacted_text
                        };
                        if key == "arguments" && !is_json_object(&redacted) {
                            return Err(());
                        }
                        object.insert(key, Value::String(redacted));
                    } else {
                        let child = object.get_mut(&key).ok_or(())?;
                        redact_value_strings(child, caller, facade).await?;
                    }
                }
                Ok(())
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
        }
    })
}

#[derive(Default)]
struct ResponseValidator {
    phase: ResponsePhase,
}

enum ValidationAction {
    Immediate,
    Delta { key: GroupKey, fragment: String },
    Done { key: GroupKey, field: &'static str },
}

impl ResponseValidator {
    fn accept(&mut self, event: &ProviderSseEvent) -> Result<ValidationAction, ()> {
        if event.comment.is_some() || event.id.is_some() || event.retry.is_some() {
            return Err(());
        }
        let kind = event.event.as_deref().ok_or(())?;
        let value: Value = serde_json::from_str(&event.data).map_err(|_| ())?;
        if !value.is_object() || value.get("type").and_then(Value::as_str) != Some(kind) {
            return Err(());
        }
        let object = value.as_object().ok_or(())?;
        reject_tenant_keys(object)?;
        // Unknown event types are handled by the catch-all arm below, which
        // admits only provably token-only payloads. Known types are restricted
        // to their exact field allowlist.
        if let Some(allowed) = allowed_event_keys(kind) {
            if object.keys().any(|key| !allowed.contains(&key.as_str())) {
                return Err(());
            }
        }
        if let Some(sequence) = object.get("sequence_number") {
            if sequence.as_u64().is_none() {
                return Err(());
            }
        }
        match kind {
            "response.created" => {
                if self.phase != ResponsePhase::AwaitCreated {
                    return Err(());
                }
                validate_response_shell(&value)?;
                self.phase = ResponsePhase::Ready;
                Ok(ValidationAction::Immediate)
            }
            "response.in_progress" => {
                if self.phase != ResponsePhase::Ready {
                    return Err(());
                }
                validate_response_shell(&value)?;
                Ok(ValidationAction::Immediate)
            }
            "ping" => {
                if self.phase == ResponsePhase::Done {
                    return Err(());
                }
                Ok(ValidationAction::Immediate)
            }
            "response.output_item.added" | "response.output_item.done" => {
                if self.phase != ResponsePhase::Ready {
                    return Err(());
                }
                let index = object
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .ok_or(())?;
                let _ = index;
                if !object.get("item").is_some_and(Value::is_object) {
                    return Err(());
                }
                Ok(ValidationAction::Immediate)
            }
            "response.content_part.added" | "response.content_part.done" => {
                if self.phase != ResponsePhase::Ready {
                    return Err(());
                }
                require_indexes(object, &["output_index", "content_index"])?;
                if !object.get("part").is_some_and(Value::is_object) {
                    return Err(());
                }
                if require_string(object, "item_id")?.len() > 256 {
                    return Err(());
                }
                Ok(ValidationAction::Immediate)
            }
            "response.reasoning_summary_part.added" | "response.reasoning_summary_part.done" => {
                if self.phase != ResponsePhase::Ready {
                    return Err(());
                }
                require_indexes(object, &["output_index", "summary_index"])?;
                if !object.get("part").is_some_and(Value::is_object) {
                    return Err(());
                }
                if require_string(object, "item_id")?.len() > 256 {
                    return Err(());
                }
                Ok(ValidationAction::Immediate)
            }
            "response.output_text.delta"
            | "response.refusal.delta"
            | "response.function_call_arguments.delta"
            | "response.reasoning_summary_text.delta" => {
                if self.phase != ResponsePhase::Ready {
                    return Err(());
                }
                let (key, fragment) = parse_delta(kind, &value)?;
                Ok(ValidationAction::Delta { key, fragment })
            }
            "response.output_text.done"
            | "response.refusal.done"
            | "response.function_call_arguments.done"
            | "response.reasoning_summary_text.done" => {
                if self.phase != ResponsePhase::Ready {
                    return Err(());
                }
                let (key, field) = parse_done(kind, &value)?;
                Ok(ValidationAction::Done { key, field })
            }
            "response.completed"
            | "response.failed"
            | "response.incomplete"
            | "response.error"
            | "error" => {
                if self.phase != ResponsePhase::Ready {
                    return Err(());
                }
                if !matches!(kind, "response.error" | "error")
                    && !object.get("response").is_some_and(Value::is_object)
                {
                    return Err(());
                }
                if kind == "response.error" {
                    let error = value.get("error").and_then(Value::as_object).ok_or(())?;
                    if !error.get("message").is_some_and(Value::is_string) {
                        return Err(());
                    }
                } else if kind == "error" {
                    let flat = object.get("message").is_some_and(Value::is_string);
                    let nested = value
                        .get("error")
                        .and_then(Value::as_object)
                        .is_some_and(|error| error.get("message").is_some_and(Value::is_string));
                    if !flat && !nested {
                        return Err(());
                    }
                }
                self.phase = ResponsePhase::Done;
                Ok(ValidationAction::Immediate)
            }
            // Unknown events may pass only when their payload is provably
            // token-only. Any free-form text is refused before egress.
            _ => {
                if self.phase == ResponsePhase::Done || !is_token_only_unknown_event(object) {
                    return Err(());
                }
                Ok(ValidationAction::Immediate)
            }
        }
    }

    fn finish(&self) -> Result<(), ()> {
        (self.phase == ResponsePhase::Done).then_some(()).ok_or(())
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ResponsePhase {
    #[default]
    AwaitCreated,
    Ready,
    Done,
}

fn allowed_event_keys(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "response.created"
        | "response.in_progress"
        | "response.completed"
        | "response.failed"
        | "response.incomplete" => Some(&["type", "response", "sequence_number"]),
        "response.output_item.added" | "response.output_item.done" => {
            Some(&["type", "output_index", "item", "sequence_number"])
        }
        "response.content_part.added"
        | "response.content_part.done"
        | "response.reasoning_summary_part.added"
        | "response.reasoning_summary_part.done" => Some(&[
            "type",
            "item_id",
            "output_index",
            "content_index",
            "summary_index",
            "part",
            "sequence_number",
        ]),
        "response.output_text.delta"
        | "response.refusal.delta"
        | "response.function_call_arguments.delta" => Some(&[
            "type",
            "item_id",
            "output_index",
            "content_index",
            "delta",
            "sequence_number",
        ]),
        "response.output_text.done" => Some(&[
            "type",
            "item_id",
            "output_index",
            "content_index",
            "text",
            "sequence_number",
        ]),
        "response.refusal.done" => Some(&[
            "type",
            "item_id",
            "output_index",
            "content_index",
            "refusal",
            "sequence_number",
        ]),
        "response.function_call_arguments.done" => Some(&[
            "type",
            "item_id",
            "output_index",
            "content_index",
            "arguments",
            "sequence_number",
        ]),
        "response.reasoning_summary_text.delta" => Some(&[
            "type",
            "item_id",
            "output_index",
            "summary_index",
            "delta",
            "sequence_number",
        ]),
        "response.reasoning_summary_text.done" => Some(&[
            "type",
            "item_id",
            "output_index",
            "summary_index",
            "text",
            "sequence_number",
        ]),
        "response.error" => Some(&["type", "error", "sequence_number"]),
        // The Responses `error` event is flat: code/message/param at the top
        // level. The nested form is accepted as well for proxy-compatible feeds.
        "error" => Some(&[
            "type",
            "error",
            "code",
            "message",
            "param",
            "sequence_number",
        ]),
        "ping" => Some(&["type", "sequence_number"]),
        _ => None,
    }
}

/// At `created`/`in_progress` the response shell must be empty: content can
/// only arrive through the deltas the gateway inspects.
fn validate_response_shell(value: &Value) -> Result<(), ()> {
    let response = value.get("response").and_then(Value::as_object).ok_or(())?;
    if !response
        .get("output")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        return Err(());
    }
    Ok(())
}

fn require_indexes(object: &Map<String, Value>, keys: &[&str]) -> Result<(), ()> {
    for key in keys {
        if object.get(*key).and_then(Value::as_u64).is_none() {
            return Err(());
        }
    }
    Ok(())
}

fn delta_kind(kind: &str) -> Result<(DeltaKind, &'static str, &'static str), ()> {
    match kind {
        "response.output_text.delta" => Ok((DeltaKind::OutputText, "content_index", "delta")),
        "response.refusal.delta" => Ok((DeltaKind::Refusal, "content_index", "delta")),
        "response.function_call_arguments.delta" => {
            Ok((DeltaKind::FunctionArguments, "content_index", "delta"))
        }
        "response.reasoning_summary_text.delta" => {
            Ok((DeltaKind::ReasoningSummary, "summary_index", "delta"))
        }
        _ => Err(()),
    }
}

fn parse_delta(kind: &str, value: &Value) -> Result<(GroupKey, String), ()> {
    let (delta_kind, index_key, content_key) = delta_kind(kind)?;
    let object = value.as_object().ok_or(())?;
    let item_id = require_string(object, "item_id")?;
    if item_id.len() > 256 {
        return Err(());
    }
    let output_index = object
        .get("output_index")
        .and_then(Value::as_u64)
        .ok_or(())?;
    let index = object.get(index_key).and_then(Value::as_u64).ok_or(())?;
    let fragment = object.get(content_key).and_then(Value::as_str).ok_or(())?;
    if fragment.len() > MAX_GROUP_BUFFER {
        return Err(());
    }
    Ok((
        GroupKey {
            item_id: item_id.to_owned(),
            output_index,
            index,
            kind: delta_kind,
        },
        fragment.to_owned(),
    ))
}

fn parse_done(kind: &str, value: &Value) -> Result<(GroupKey, &'static str), ()> {
    let (delta_kind, index_key, field) = match kind {
        "response.output_text.done" => Ok((DeltaKind::OutputText, "content_index", "text")),
        "response.refusal.done" => Ok((DeltaKind::Refusal, "content_index", "refusal")),
        "response.function_call_arguments.done" => {
            Ok((DeltaKind::FunctionArguments, "content_index", "arguments"))
        }
        "response.reasoning_summary_text.done" => {
            Ok((DeltaKind::ReasoningSummary, "summary_index", "text"))
        }
        _ => Err(()),
    }?;
    let object = value.as_object().ok_or(())?;
    let item_id = require_string(object, "item_id")?;
    if item_id.len() > 256 {
        return Err(());
    }
    let output_index = object
        .get("output_index")
        .and_then(Value::as_u64)
        .ok_or(())?;
    let index = object.get(index_key).and_then(Value::as_u64).ok_or(())?;
    if !object.get(field).is_some_and(Value::is_string) {
        return Err(());
    }
    Ok((
        GroupKey {
            item_id: item_id.to_owned(),
            output_index,
            index,
            kind: delta_kind,
        },
        field,
    ))
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

/// Bounded SSE decoder. Frames, multi-line data, comments, ids, and retries are
/// decoded but only validated events reach the relay.
#[derive(Default)]
struct SseDecoder {
    pending: Vec<u8>,
    data: Vec<String>,
    event: Option<String>,
    id: Option<String>,
    comment: Option<String>,
}

impl SseDecoder {
    fn push(
        &mut self,
        bytes: &[u8],
        callback: &mut dyn FnMut(ProviderSseEvent) -> Result<(), TransportError>,
    ) -> Result<(), TransportError> {
        self.pending.extend_from_slice(bytes);
        if self.pending.len() > MAX_SSE_FRAME {
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
            "data" => {
                if self.data.iter().map(String::len).sum::<usize>() + value.len() > MAX_SSE_FRAME {
                    return Err(TransportError::Failed);
                }
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
pub(crate) async fn redact_and_validate_provider_events(
    events: &[ProviderSseEvent],
    facade: &HaciendaFacade,
    auth_context: &hacienda_core::auth::AuthContext,
) -> Result<Vec<ProviderSseEvent>, ()> {
    let mut validator = ResponseValidator::default();
    let mut output = Vec::new();
    let mut groups: HashMap<GroupKey, BufferedGroup> = HashMap::new();
    for event in events {
        match validator.accept(event)? {
            ValidationAction::Immediate => {
                let mut value: Value = serde_json::from_str(&event.data).map_err(|_| ())?;
                redact_value_strings(&mut value, Caller::Principal(auth_context), facade).await?;
                let mut safe = event.clone();
                safe.data = serde_json::to_string(&value).map_err(|_| ())?;
                output.push(safe);
            }
            ValidationAction::Delta { key, fragment } => {
                let group = groups.entry(key).or_insert_with(|| BufferedGroup {
                    joined: String::new(),
                    deltas: Vec::new(),
                    total: 0,
                });
                group.total = group.total.saturating_add(fragment.len());
                if group.total > MAX_GROUP_BUFFER {
                    return Err(());
                }
                group.joined.push_str(&fragment);
                group.deltas.push(event.clone());
            }
            ValidationAction::Done { key, field } => {
                let group = groups.remove(&key).ok_or(())?;
                let redacted = facade
                    .redact_text_with_auth(Caller::Principal(auth_context), &group.joined)
                    .await
                    .map_err(|_| ())?
                    .redacted_text;
                if key.kind == DeltaKind::FunctionArguments && !is_json_object(&redacted) {
                    return Err(());
                }
                for mut delta in group.deltas {
                    let mut value: Value = serde_json::from_str(&delta.data).map_err(|_| ())?;
                    *value.pointer_mut("/delta").ok_or(())? = Value::String(String::new());
                    delta.data = serde_json::to_string(&value).map_err(|_| ())?;
                    output.push(delta);
                }
                if let Some(last) = output.last_mut() {
                    let mut value: Value = serde_json::from_str(&last.data).map_err(|_| ())?;
                    *value.pointer_mut("/delta").ok_or(())? = Value::String(redacted.clone());
                    last.data = serde_json::to_string(&value).map_err(|_| ())?;
                }
                let mut done: Value = serde_json::from_str(&event.data).map_err(|_| ())?;
                let pointer = format!("/{field}");
                *done.pointer_mut(&pointer).ok_or(())? = Value::String(redacted);
                let mut safe = event.clone();
                safe.data = serde_json::to_string(&done).map_err(|_| ())?;
                output.push(safe);
            }
        }
    }
    validator.finish()?;
    if !groups.is_empty() {
        return Err(());
    }
    Ok(output)
}

#[cfg(test)]
fn validate_provider_events(events: &[ProviderSseEvent]) -> Result<(), ()> {
    let mut validator = ResponseValidator::default();
    let mut open = HashSet::new();
    for event in events {
        match validator.accept(event)? {
            ValidationAction::Immediate => {}
            ValidationAction::Delta { key, .. } => {
                open.insert(key);
            }
            ValidationAction::Done { key, .. } => {
                if !open.remove(&key) {
                    return Err(());
                }
            }
        }
    }
    validator.finish()?;
    if open.is_empty() {
        Ok(())
    } else {
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, http::Request};
    use hacienda_core::{
        auth::{authn::DevTokenResolver, AuthContext, AuthState, Capability, CapabilitySet},
        jobs::InMemoryJobStore,
        pii::PipelineConfig,
        HaciendaConfig,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    const EMAIL: &str = "bob@example.com";
    const LOCAL_SECRET: &str = "local-gateway-secret-123";
    const CHATGPT_AUTH: &str = "Bearer chatgpt-user-token";
    const ACCOUNT_ID: &str = "acct-123";

    fn pii_facade() -> HaciendaFacade {
        HaciendaFacade::new(HaciendaConfig::default().with_pii(PipelineConfig::default()))
            .expect("PII facade")
    }

    fn auth_context() -> Arc<AuthContext> {
        Arc::new(AuthContext::new(
            "codex-test",
            CapabilitySet::new([Capability::DocumentsProcess]),
        ))
    }

    fn valid_request() -> Value {
        serde_json::json!({
            "model": "hacienda-codex",
            "stream": true,
            "store": false,
            "instructions": format!("Contact {EMAIL} first"),
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": format!("ask {EMAIL}")}]
            }, {
                "type": "function_call",
                "call_id": "call-1",
                "name": "lookup",
                "arguments": format!("{{\"query\":\"{EMAIL}\"}}")
            }, {
                "type": "function_call_output",
                "call_id": "call-1",
                "output": format!("result for {EMAIL}")
            }],
            "tools": [{
                "type": "function",
                "name": "lookup",
                "description": format!("lookup for {EMAIL}"),
                "parameters": {
                    "type": "object",
                    "properties": {"query": {"type": "string", "enum": [EMAIL]}}
                }
            }],
            "metadata": {"ticket": EMAIL},
        })
    }

    fn shell_event(kind: &str, output: Value) -> ProviderSseEvent {
        ProviderSseEvent::new(
            Some(kind.to_owned()),
            None,
            serde_json::json!({
                "type": kind,
                "response": {"id": "resp_1", "object": "response", "status": "in_progress", "output": output}
            })
            .to_string(),
        )
    }

    fn valid_response_events() -> Vec<ProviderSseEvent> {
        let mut events = vec![shell_event("response.created", serde_json::json!([]))];
        events.push(ProviderSseEvent::new(
            Some("response.output_text.delta".into()),
            None,
            serde_json::json!({
                "type": "response.output_text.delta",
                "item_id": "item_1", "output_index": 0, "content_index": 0, "delta": "hello"
            })
            .to_string(),
        ));
        events.push(ProviderSseEvent::new(
            Some("response.output_text.done".into()),
            None,
            serde_json::json!({
                "type": "response.output_text.done",
                "item_id": "item_1", "output_index": 0, "content_index": 0, "text": "hello"
            })
            .to_string(),
        ));
        events.push(shell_event(
            "response.completed",
            serde_json::json!([{"type": "message", "role": "assistant", "content": []}]),
        ));
        events
    }

    #[test]
    fn nested_request_content_is_collected_and_redacted() {
        let mut body = valid_request();
        let paths = validate_and_collect_text_paths(&body).expect("supported request");
        assert!(paths.iter().any(|path| path == "/instructions"));
        assert!(paths.iter().any(|path| path.ends_with("/content/0/text")));
        assert!(paths.iter().any(|path| path.ends_with("/arguments")));
        assert!(paths.iter().any(|path| path.ends_with("/output")));
        assert!(paths.iter().any(|path| path.contains("parameters")));
        let reveal = AuthContext::new("test", CapabilitySet::new([Capability::PiiReveal]));
        let mut mappings = HashMap::new();
        block_on(redact_paths(
            &mut body,
            &paths,
            &pii_facade(),
            Caller::Trusted,
            &reveal,
            &mut mappings,
        ))
        .expect("redaction");
        assert!(!serde_json::to_string(&body).unwrap().contains(EMAIL));
    }

    #[test]
    fn uninspectable_request_content_fails_closed() {
        let image = serde_json::json!({
            "model": "hacienda-codex", "stream": true,
            "input": [{"type": "message", "role": "user", "content": [
                {"type": "input_image", "image_url": "https://example.test/a.png"}
            ]}]
        });
        assert!(validate_and_collect_text_paths(&image).is_err());

        let hosted_tool = serde_json::json!({
            "model": "hacienda-codex", "stream": true, "input": "hi",
            "tools": [{"type": "web_search"}]
        });
        assert!(validate_and_collect_text_paths(&hosted_tool).is_err());

        let non_streaming = serde_json::json!({
            "model": "hacienda-codex", "stream": false, "input": "hi"
        });
        assert!(validate_and_collect_text_paths(&non_streaming).is_err());

        let bad_tool_args = serde_json::json!({
            "model": "hacienda-codex", "stream": true,
            "input": [{"type": "function_call", "call_id": "c", "name": "f", "arguments": 7}]
        });
        assert!(validate_and_collect_text_paths(&bad_tool_args).is_err());
    }

    #[test]
    fn provider_context_and_session_fields_are_refused() {
        for extra in [
            serde_json::json!({"previous_response_id": "resp_old"}),
            serde_json::json!({"conversation": "conv_1"}),
            serde_json::json!({"tenant_id": "other"}),
            serde_json::json!({"store": true}),
        ] {
            let mut body = serde_json::json!({
                "model": "hacienda-codex", "stream": true, "input": "hi"
            });
            body.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(
                validate_and_collect_text_paths(&body).is_err(),
                "unsupported field must fail closed: {extra}"
            );
        }
    }

    #[tokio::test]
    async fn split_deltas_are_redacted_as_one_logical_string() {
        let mut events = valid_response_events();
        events[1].data = serde_json::json!({
            "type": "response.output_text.delta",
            "item_id": "item_1", "output_index": 0, "content_index": 0, "delta": "bob@"
        })
        .to_string();
        events.insert(
            2,
            ProviderSseEvent::new(
                Some("response.output_text.delta".into()),
                None,
                serde_json::json!({
                    "type": "response.output_text.delta",
                    "item_id": "item_1", "output_index": 0, "content_index": 0,
                    "delta": "example.com"
                })
                .to_string(),
            ),
        );
        let safe = redact_and_validate_provider_events(&events, &pii_facade(), &auth_context())
            .await
            .expect("redaction");
        assert!(safe.iter().all(|event| !event.data.contains(EMAIL)));
        let final_delta = safe
            .iter()
            .filter(|event| event.event.as_deref() == Some("response.output_text.delta"))
            .filter_map(|event| {
                let value: Value = serde_json::from_str(&event.data).ok()?;
                value
                    .pointer("/delta")
                    .and_then(Value::as_str)
                    .filter(|delta| !delta.is_empty())
                    .map(str::to_owned)
            })
            .collect::<Vec<_>>();
        assert_eq!(final_delta.len(), 1, "only the last delta carries content");
        assert!(!final_delta[0].contains(EMAIL));
        assert!(final_delta[0].contains("[EMAIL]"));
    }

    #[tokio::test]
    async fn function_call_arguments_are_grouped_and_revalidated() {
        let mut events = vec![shell_event("response.created", serde_json::json!([]))];
        events.push(ProviderSseEvent::new(
            Some("response.function_call_arguments.delta".into()),
            None,
            serde_json::json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "item_1", "output_index": 0, "content_index": 0,
                "delta": "{\"email\":\"bob@"
            })
            .to_string(),
        ));
        events.push(ProviderSseEvent::new(
            Some("response.function_call_arguments.delta".into()),
            None,
            serde_json::json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "item_1", "output_index": 0, "content_index": 0,
                "delta": "example.com\"}"
            })
            .to_string(),
        ));
        events.push(ProviderSseEvent::new(
            Some("response.function_call_arguments.done".into()),
            None,
            serde_json::json!({
                "type": "response.function_call_arguments.done",
                "item_id": "item_1", "output_index": 0, "content_index": 0,
                "arguments": "{\"email\":\"bob@example.com\"}"
            })
            .to_string(),
        ));
        events.push(shell_event(
            "response.completed",
            serde_json::json!([{"type": "function_call", "arguments": "{\"email\":\"bob@example.com\"}"}]),
        ));
        let safe = redact_and_validate_provider_events(&events, &pii_facade(), &auth_context())
            .await
            .expect("arguments redaction");
        assert!(safe.iter().all(|event| !event.data.contains(EMAIL)));
        let arguments = safe
            .iter()
            .find_map(|event| {
                let value: Value = serde_json::from_str(&event.data).ok()?;
                value
                    .get("arguments")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .expect("redacted arguments");
        assert!(is_json_object(&arguments));
    }

    #[test]
    fn unknown_events_fail_closed_unless_token_only() {
        let mut events = valid_response_events();
        events.insert(
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
        assert!(validate_provider_events(&events).is_ok());

        let mut plaintext = events;
        plaintext[1].data = serde_json::json!({
            "type": "future_token", "token": EMAIL
        })
        .to_string();
        assert!(validate_provider_events(&plaintext).is_err());
    }

    #[test]
    fn malformed_events_and_lifecycle_violations_fail_closed() {
        let mut out_of_order = valid_response_events();
        out_of_order.swap(0, 1);
        assert!(validate_provider_events(&out_of_order).is_err());

        let mut with_id = valid_response_events();
        with_id[0].id = Some("provider-id".into());
        assert!(validate_provider_events(&with_id).is_err());

        let mut unknown_field = valid_response_events();
        unknown_field[1].data = serde_json::json!({
            "type": "response.output_text.delta",
            "item_id": "item_1", "output_index": 0, "content_index": 0,
            "delta": "hi", "debug": EMAIL
        })
        .to_string();
        assert!(validate_provider_events(&unknown_field).is_err());

        let mut unfinished = valid_response_events();
        unfinished.truncate(2);
        assert!(validate_provider_events(&unfinished).is_err());

        let mut missing_done = valid_response_events();
        missing_done.remove(2);
        assert!(validate_provider_events(&missing_done).is_err());
    }

    #[tokio::test]
    async fn error_messages_are_redacted() {
        let mut events = vec![shell_event("response.created", serde_json::json!([]))];
        events.push(ProviderSseEvent::new(
            Some("response.error".into()),
            None,
            serde_json::json!({
                "type": "response.error",
                "error": {"code": "server_error", "message": format!("failed for {EMAIL}")}
            })
            .to_string(),
        ));
        let safe = redact_and_validate_provider_events(&events, &pii_facade(), &auth_context())
            .await
            .expect("error redaction");
        assert!(!safe[1].data.contains(EMAIL));
    }

    #[tokio::test]
    async fn responses_handler_requires_the_local_gateway_secret() {
        let state =
            crate::routes::tests::test_state_no_auth().with_codex_gateway_secret(LOCAL_SECRET);
        let (parts, _) = Request::new(()).into_parts();
        let result = responses(
            State(state),
            parts,
            HeaderMap::new(),
            SafeJson(valid_request()),
        )
        .await;
        match result {
            Err(error) => assert_eq!(error.code, crate::error::ApiErrorCode::Unauthenticated),
            Ok(_) => panic!("a caller without the local secret must be refused"),
        }
    }

    #[derive(Clone)]
    struct RecordingProvider {
        captured: Arc<Mutex<Vec<Value>>>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl CodexResponsesProvider for RecordingProvider {
        async fn stream(
            &self,
            request: CodexResponsesProviderRequest,
        ) -> Result<CodexResponsesProviderStream, ProviderGatewayError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.captured.lock().unwrap().push(request.body);
            let events = stream::iter(
                valid_response_events()
                    .into_iter()
                    .map(Ok::<_, ProviderGatewayError>)
                    .collect::<Vec<_>>(),
            );
            Ok(CodexResponsesProviderStream {
                response: CodexResponsesProviderResponse {
                    status: 200,
                    headers: Vec::new(),
                },
                events: Box::pin(events),
            })
        }

        fn models(&self) -> Vec<String> {
            vec![DEFAULT_CODEX_MODEL.to_owned()]
        }
    }

    fn gateway_state(provider: Arc<dyn CodexResponsesProvider>) -> ApiState {
        let facade = Arc::new(pii_facade());
        let jobs = InMemoryJobStore::new().into_arc();
        ApiState::new(
            facade,
            jobs,
            AuthState::new(Arc::new(DevTokenResolver)),
            crate::state::ApiLimits::default(),
        )
        .with_codex_responses_provider(provider)
        .with_codex_gateway_secret(LOCAL_SECRET)
    }

    fn gateway_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(GATEWAY_SECRET_HEADER, LOCAL_SECRET.parse().unwrap());
        headers.insert(
            axum::http::header::AUTHORIZATION,
            CHATGPT_AUTH.parse().unwrap(),
        );
        headers.insert(CODEX_ACCOUNT_HEADER, ACCOUNT_ID.parse().unwrap());
        headers
    }

    fn authenticated_parts() -> Parts {
        let (mut parts, _) = Request::new(()).into_parts();
        parts.headers = gateway_headers();
        parts
    }

    #[tokio::test]
    async fn provider_sees_only_tokenized_prompt_and_redacted_sse() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(RecordingProvider {
            captured: Arc::clone(&captured),
            calls,
        });
        let state = gateway_state(provider);
        let response = responses(
            State(state),
            authenticated_parts(),
            gateway_headers(),
            SafeJson(valid_request()),
        )
        .await
        .expect("gateway response");

        let body = to_bytes(response.into_response().into_body(), 8 * 1024 * 1024)
            .await
            .expect("sse body");
        let body = String::from_utf8(body.to_vec()).expect("utf8 sse");
        assert!(!body.contains(EMAIL), "relayed SSE leaked a real value");
        assert!(body.contains("response.created"));
        assert!(body.contains("response.completed"));

        let sent = captured.lock().unwrap();
        assert_eq!(sent.len(), 1);
        let sent = &sent[0];
        assert!(!serde_json::to_string(sent).unwrap().contains(EMAIL));
        assert_eq!(sent["store"], serde_json::json!(false));
        assert!(sent.get("tenant").is_none());
        assert!(sent.get("previous_response_id").is_none());
    }

    #[tokio::test]
    async fn wrong_local_secret_is_rejected_before_provider_call() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(RecordingProvider {
            captured,
            calls: Arc::clone(&calls),
        });
        let state = gateway_state(provider);
        let mut headers = gateway_headers();
        headers.insert(GATEWAY_SECRET_HEADER, "wrong-local-secret".parse().unwrap());
        let mut parts = authenticated_parts();
        parts.headers = headers.clone();
        let result = responses(State(state), parts, headers, SafeJson(valid_request())).await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn models_requires_authentication_and_returns_server_owned_ids() {
        let provider = Arc::new(RecordingProvider {
            captured: Arc::new(Mutex::new(Vec::new())),
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let state = gateway_state(provider);
        let (unauthenticated, _) = Request::new(()).into_parts();
        assert!(models(State(state.clone()), unauthenticated).await.is_err());

        let json = models(State(state), authenticated_parts())
            .await
            .expect("model list")
            .0;
        assert_eq!(
            json["data"][0]["id"],
            serde_json::json!(DEFAULT_CODEX_MODEL)
        );
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(future)
    }
}
