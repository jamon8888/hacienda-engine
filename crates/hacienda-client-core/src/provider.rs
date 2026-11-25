//! Provider gateway seam for the local secrecy client.
//!
//! The public interface deliberately has two kinds of values: raw request values are
//! accepted only at the redaction seam, while provider transport receives
//! [`RedactedProviderRequest`] and returns token-only chunks. The client owns neither
//! a log sink nor durable transcript storage, so raw values have no persistence path.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

/// A stable conversation identifier shared by a client and its provider session.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversationId(String);

impl ConversationId {
    /// Construct an opaque conversation identifier.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the stable identifier for routing, never for plaintext content.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ConversationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ConversationId(<opaque>)")
    }
}

/// A stable context identifier used to continue the same redacted context.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContextId(String);

impl ContextId {
    /// Construct an opaque context identifier.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the stable identifier for routing, never for plaintext content.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ContextId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ContextId(<opaque>)")
    }
}

/// A provider supported by the local gateway seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderKind {
    /// Claude Desktop / Anthropic-compatible Messages transport.
    ClaudeDesktop,
    /// `OpenAI` Codex-compatible transport.
    Codex,
}

/// Raw context supplied by the UI before it crosses the redaction adapter.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PromptContext {
    pub label: String,
    pub content: String,
}

impl PromptContext {
    /// Construct a transient context item. Callers must not persist this value.
    #[must_use]
    pub fn new(label: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            content: content.into(),
        }
    }
}

impl fmt::Debug for PromptContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PromptContext")
            .field("label", &"<redacted>")
            .field("content", &"<redacted>")
            .finish()
    }
}

/// Raw Claude Desktop request. It exists only until [`RedactionAdapter::redact`].
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ClaudeDesktopRequest {
    pub conversation_id: ConversationId,
    pub context_id: ContextId,
    pub prompt: String,
    pub context: Vec<PromptContext>,
}

impl ClaudeDesktopRequest {
    /// Construct a request with no additional context.
    #[must_use]
    pub fn new(
        conversation_id: ConversationId,
        context_id: ContextId,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            conversation_id,
            context_id,
            prompt: prompt.into(),
            context: Vec::new(),
        }
    }

    /// Attach transient context before it crosses the redaction seam.
    #[must_use]
    pub fn with_context(mut self, context: Vec<PromptContext>) -> Self {
        self.context = context;
        self
    }
}

impl fmt::Debug for ClaudeDesktopRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaudeDesktopRequest")
            .field("conversation_id", &self.conversation_id)
            .field("context_id", &self.context_id)
            .field("prompt", &"<redacted>")
            .field("context", &"<redacted>")
            .finish()
    }
}

/// Raw Codex request. It exists only until [`RedactionAdapter::redact`].
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CodexRequest {
    pub conversation_id: ConversationId,
    pub context_id: ContextId,
    pub prompt: String,
    pub context: Vec<PromptContext>,
}

impl CodexRequest {
    /// Construct a request with no additional context.
    #[must_use]
    pub fn new(
        conversation_id: ConversationId,
        context_id: ContextId,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            conversation_id,
            context_id,
            prompt: prompt.into(),
            context: Vec::new(),
        }
    }

    /// Attach transient context before it crosses the redaction seam.
    #[must_use]
    pub fn with_context(mut self, context: Vec<PromptContext>) -> Self {
        self.context = context;
        self
    }
}

impl fmt::Debug for CodexRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodexRequest")
            .field("conversation_id", &self.conversation_id)
            .field("context_id", &self.context_id)
            .field("prompt", &"<redacted>")
            .field("context", &"<redacted>")
            .finish()
    }
}

/// Provider-specific raw request at the public seam.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum ProviderRequest {
    ClaudeDesktop(ClaudeDesktopRequest),
    Codex(CodexRequest),
}

impl ProviderRequest {
    /// Return the provider kind without exposing request content.
    #[must_use]
    pub fn provider(&self) -> ProviderKind {
        match self {
            Self::ClaudeDesktop(_) => ProviderKind::ClaudeDesktop,
            Self::Codex(_) => ProviderKind::Codex,
        }
    }

    /// Return the stable conversation identifier.
    #[must_use]
    pub fn conversation_id(&self) -> &ConversationId {
        match self {
            Self::ClaudeDesktop(request) => &request.conversation_id,
            Self::Codex(request) => &request.conversation_id,
        }
    }

    /// Return the stable context identifier.
    #[must_use]
    pub fn context_id(&self) -> &ContextId {
        match self {
            Self::ClaudeDesktop(request) => &request.context_id,
            Self::Codex(request) => &request.context_id,
        }
    }
}

impl fmt::Debug for ProviderRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClaudeDesktop(_) => {
                formatter.write_str("ProviderRequest::ClaudeDesktop(<redacted>)")
            }
            Self::Codex(_) => formatter.write_str("ProviderRequest::Codex(<redacted>)"),
        }
    }
}

/// A token reference safe to send to a provider and preserve in UI annotations.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OpaqueTokenReference {
    pub token: String,
    pub class: String,
}

impl OpaqueTokenReference {
    /// Construct a stable token reference; `token` must never contain plaintext.
    #[must_use]
    pub fn new(token: impl Into<String>, class: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            class: class.into(),
        }
    }
}

impl fmt::Debug for OpaqueTokenReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpaqueTokenReference")
            .field("token", &"<opaque>")
            .field("class", &self.class)
            .finish()
    }
}

/// Text whose sensitive values have been replaced by opaque token references.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenizedText {
    pub text: String,
    pub references: Vec<OpaqueTokenReference>,
}

impl TokenizedText {
    /// Construct token-only text after redaction has completed.
    #[must_use]
    pub fn new(text: impl Into<String>, references: Vec<OpaqueTokenReference>) -> Self {
        Self {
            text: text.into(),
            references,
        }
    }
}

impl fmt::Debug for TokenizedText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenizedText")
            .field("text", &"<token-only>")
            .field("references", &self.references)
            .finish()
    }
}

/// A context item after redaction. This is the only context form transport sees.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct RedactedContext {
    pub label: String,
    pub content: TokenizedText,
}

/// Provider request accepted by transport only after redaction.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct RedactedProviderRequest {
    pub provider: ProviderKind,
    pub conversation_id: ConversationId,
    pub context_id: ContextId,
    pub prompt: TokenizedText,
    pub context: Vec<RedactedContext>,
}

/// Explicit capability required for a human display rehydration operation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayCapability(String);

impl DisplayCapability {
    /// Construct a capability supplied by an authorized viewer session.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Debug for DisplayCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DisplayCapability(<redacted>)")
    }
}

/// A streamed provider response chunk. Token references are carried unchanged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct ProviderStreamChunk {
    pub conversation_id: ConversationId,
    pub context_id: ContextId,
    pub sequence: u64,
    pub content: TokenizedText,
}

/// Provider-specific Claude response.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct ClaudeDesktopResponse {
    pub conversation_id: ConversationId,
    pub context_id: ContextId,
    pub chunks: Vec<ProviderStreamChunk>,
}

/// Provider-specific Codex response.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct CodexResponse {
    pub conversation_id: ConversationId,
    pub context_id: ContextId,
    pub chunks: Vec<ProviderStreamChunk>,
}

/// Provider-specific response at the public seam.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum ProviderResponse {
    ClaudeDesktop(ClaudeDesktopResponse),
    Codex(CodexResponse),
}

/// Ephemeral display result. The client does not store or log this value.
#[derive(Clone, PartialEq, Eq)]
pub struct DisplayChunk {
    pub conversation_id: ConversationId,
    pub context_id: ContextId,
    pub sequence: u64,
    pub text: String,
}

impl fmt::Debug for DisplayChunk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DisplayChunk")
            .field("conversation_id", &self.conversation_id)
            .field("context_id", &self.context_id)
            .field("sequence", &self.sequence)
            .field("text", &"<display-only>")
            .finish()
    }
}

/// Redaction failures are intentionally content-free and safe to log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error, Serialize)]
pub enum RedactionError {
    #[error("redaction adapter rejected the request")]
    Rejected,
}

/// Provider transport failures are intentionally content-free and safe to log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error, Serialize)]
pub enum TransportError {
    #[error("provider transport failed")]
    Failed,
}

/// Human display rehydration failures are intentionally content-free and safe to log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error, Serialize)]
pub enum RehydrationError {
    #[error("display rehydration failed")]
    Failed,
}

/// Errors from the small client interface. No variant contains user content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error, Serialize)]
pub enum ClientError {
    #[error("redaction capability is unavailable")]
    RedactionUnavailable,
    #[error(transparent)]
    Redaction(#[from] RedactionError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("display rehydration capability is unavailable")]
    RehydrationUnavailable,
    #[error(transparent)]
    Rehydration(#[from] RehydrationError),
}

/// Adapter seam that turns transient raw requests into provider-safe requests.
pub trait RedactionAdapter {
    /// Redact every prompt and context field before provider transport.
    ///
    /// # Errors
    /// Returns an error if any prompt or context cannot be safely tokenized.
    fn redact(&self, request: &ProviderRequest) -> Result<RedactedProviderRequest, RedactionError>;

    /// Redact provider output before it is exposed to the renderer.
    ///
    /// Implementations must use the same local Hacienda policy as [`Self::redact`].
    /// The default rejects the operation so an adapter that has not implemented the
    /// response boundary cannot accidentally return raw provider text.
    ///
    /// # Errors
    /// Returns an error when response redaction is unavailable or fails.
    fn redact_text(&self, _text: &str) -> Result<TokenizedText, RedactionError> {
        Err(RedactionError::Rejected)
    }
}

/// Adapter seam for provider streaming. It receives token-only data only.
pub trait ProviderTransport {
    /// Stream a complete set of chunks while preserving opaque references.
    ///
    /// # Errors
    /// Returns an error if transport fails or the provider response is invalid.
    fn stream(
        &mut self,
        request: &RedactedProviderRequest,
    ) -> Result<ProviderResponse, TransportError>;
}

/// An outbound request at the provider HTTP seam. The body and headers are kept
/// private to the transport implementation and are never rendered by `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderHttpRequest {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl ProviderHttpRequest {
    /// Construct an HTTP request for an injected provider client.
    #[must_use]
    pub fn new(
        method: impl Into<String>,
        url: impl Into<String>,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Self {
        Self {
            method: method.into(),
            url: url.into(),
            headers,
            body,
        }
    }

    /// Inspect the method in a test seam without exposing payloads in formatting.
    #[must_use]
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Inspect the destination in an injected HTTP implementation.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Inspect headers only inside the injected HTTP implementation.
    #[must_use]
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// Inspect the serialized body only inside the injected HTTP implementation.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

impl fmt::Debug for ProviderHttpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderHttpRequest")
            .field("method", &self.method)
            .field("url", &"<opaque>")
            .field("headers", &"<redacted>")
            .field("body", &"<redacted>")
            .finish()
    }
}

/// A response from the injected HTTP seam. The body is consumed by a provider
/// adapter and is never included in `Debug` or a transport error.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderHttpResponse {
    status: u16,
    body: String,
}

/// One decoded Server-Sent Event. Fields are preserved verbatim after the client
/// has validated and redacted its data payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderSseEvent {
    pub event: Option<String>,
    pub id: Option<String>,
    pub retry: Option<u64>,
    pub comment: Option<String>,
    pub data: String,
}

impl ProviderSseEvent {
    /// Construct an event for adapters and public API consumers.
    #[must_use]
    pub fn new(event: Option<String>, id: Option<String>, data: String) -> Self {
        Self {
            event,
            id,
            retry: None,
            comment: None,
            data,
        }
    }
}

/// Selected response metadata safe to expose to the event consumer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderSseMetadata {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

impl ProviderHttpResponse {
    /// Construct a response fixture or an HTTP implementation result.
    #[must_use]
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
        }
    }

    /// Return the status code for adapter validation.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Borrow the raw body only while mapping it to tokenized chunks.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }
}

impl fmt::Debug for ProviderHttpResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderHttpResponse")
            .field("status", &self.status)
            .field("body", &"<redacted>")
            .finish()
    }
}

/// Small, injected HTTP seam used by the provider adapters and their tests.
pub trait ProviderHttpClient {
    /// Send one request without logging or retaining its content.
    ///
    /// # Errors
    /// Returns an error if the request cannot be sent or its response is invalid.
    fn send(
        &mut self,
        request: ProviderHttpRequest,
    ) -> Result<ProviderHttpResponse, TransportError>;

    /// Read a response body in bounded byte chunks. Implementations with a real
    /// network body should override this method; the default keeps fixture and
    /// legacy clients source-compatible.
    ///
    /// # Errors
    /// Returns an error if sending fails or either callback rejects the response.
    fn send_stream(
        &mut self,
        request: ProviderHttpRequest,
        on_response: &mut dyn FnMut(u16, Vec<(String, String)>),
        on_chunk: &mut dyn FnMut(&[u8]) -> Result<(), TransportError>,
    ) -> Result<(), TransportError> {
        let response = self.send(request)?;
        on_response(response.status(), Vec::new());
        on_chunk(response.body().as_bytes())?;
        Ok(())
    }
}

/// Provider endpoint and authentication configuration. `Debug` intentionally
/// redacts every value because endpoints may contain credentials in query strings.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderTransportConfig {
    endpoint: String,
    model: String,
    max_tokens: u32,
    credential: String,
}

impl ProviderTransportConfig {
    /// Construct transport configuration from transient application settings.
    #[must_use]
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        max_tokens: u32,
        credential: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            max_tokens,
            credential: credential.into(),
        }
    }

    /// Validate that this route terminates at a local or authenticated gateway.
    ///
    /// # Errors
    /// Returns an error if the URL, model, token limit, or credential is invalid.
    pub fn validate(&self) -> Result<(), TransportError> {
        let endpoint = reqwest::Url::parse(&self.endpoint).map_err(|_| TransportError::Failed)?;
        let host = endpoint.host_str().ok_or(TransportError::Failed)?;
        let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
        if endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback) {
            return Err(TransportError::Failed);
        }
        if self.model.trim().is_empty() || self.max_tokens == 0 || self.credential.trim().is_empty()
        {
            return Err(TransportError::Failed);
        }
        Ok(())
    }
}

impl fmt::Debug for ProviderTransportConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderTransportConfig")
            .field("endpoint", &"<opaque>")
            .field("model", &"<opaque>")
            .field("max_tokens", &self.max_tokens)
            .field("credential", &"<redacted>")
            .finish()
    }
}

/// Anthropic Messages adapter. It maps Anthropic SSE text deltas to token-only
/// chunks while preserving the identifiers supplied by the redaction seam.
pub struct AnthropicProviderTransport<H> {
    http: H,
    config: ProviderTransportConfig,
}

impl<H> AnthropicProviderTransport<H> {
    /// Construct an adapter over an injected HTTP client.
    #[must_use]
    pub fn new(http: H, config: ProviderTransportConfig) -> Self {
        Self { http, config }
    }

    /// Relay validated Claude SSE events as they arrive, redacting each logical
    /// text before any of its deltas are released.
    ///
    /// Claude Code consumes a streaming response event by event; buffering the
    /// whole body stalls the client and can trip its idle watchdog. The safety
    /// contract is *per logical text*, not per transport chunk: every redactable
    /// fragment of a content block is joined and passed through
    /// [`RedactionAdapter::redact_text`] once at `content_block_stop` (or at a
    /// terminal boundary), so PII split across deltas can never be observed in
    /// an earlier callback. Events without free-form text (message lifecycle,
    /// `ping` keepalives and safe SSE comments) are relayed immediately and in
    /// provider order. Unknown events, unsafe comments and malformed frames are
    /// refused without releasing buffered text.
    ///
    /// # Errors
    /// Returns an error if the provider, transport, event sequence, or redaction is invalid.
    pub fn collect_redacted_events<R, F>(
        &mut self,
        request: &RedactedProviderRequest,
        redactor: &R,
        mut callback: F,
    ) -> Result<ProviderSseMetadata, TransportError>
    where
        H: ProviderHttpClient,
        R: RedactionAdapter,
        F: FnMut(ProviderSseEvent),
    {
        if request.provider != ProviderKind::ClaudeDesktop {
            return Err(TransportError::Failed);
        }
        let body = json!({
            "model": self.config.model,
            "max_tokens": self.config.max_tokens,
            "stream": true,
            "messages": [{"role": "user", "content": provider_input(request)}],
            "metadata": {"conversation_id": request.conversation_id.as_str(), "context_id": request.context_id.as_str()}
        });
        let http_request = ProviderHttpRequest::new(
            "POST",
            self.config.endpoint.clone(),
            vec![
                ("content-type".into(), "application/json".into()),
                ("x-api-key".into(), self.config.credential.clone()),
                ("anthropic-version".into(), "2023-06-01".into()),
                (
                    "x-hacienda-conversation-id".into(),
                    request.conversation_id.as_str().into(),
                ),
                (
                    "x-hacienda-context-id".into(),
                    request.context_id.as_str().into(),
                ),
            ],
            serde_json::to_vec(&body).map_err(|_| TransportError::Failed)?,
        );
        let mut decoder = SseDecoder::default();
        let mut response_metadata = None;
        let metadata_invalid = std::cell::Cell::new(false);
        let mut relay = ClaudeSseRelay::new(redactor, &mut callback);
        self.http.send_stream(
            http_request,
            &mut |status, headers| {
                let metadata = ProviderSseMetadata { status, headers };
                let invalid = !valid_claude_response_metadata(&metadata);
                metadata_invalid.set(invalid);
                if !invalid {
                    response_metadata = Some(metadata);
                }
            },
            &mut |bytes| {
                if metadata_invalid.get() {
                    return Err(TransportError::Failed);
                }
                decoder.push_with(bytes, &mut |event| relay.accept(event))
            },
        )?;
        decoder.finish()?;
        relay.finish()?;
        response_metadata.ok_or(TransportError::Failed)
    }
}

impl<H> ProviderTransport for AnthropicProviderTransport<H>
where
    H: ProviderHttpClient,
{
    fn stream(
        &mut self,
        request: &RedactedProviderRequest,
    ) -> Result<ProviderResponse, TransportError> {
        if request.provider != ProviderKind::ClaudeDesktop {
            return Err(TransportError::Failed);
        }
        let body = json!({
            "model": self.config.model,
            "max_tokens": self.config.max_tokens,
            "stream": true,
            "messages": [{"role": "user", "content": provider_input(request)}],
            "metadata": {
                "conversation_id": request.conversation_id.as_str(),
                "context_id": request.context_id.as_str()
            }
        });
        let response = self.http.send(ProviderHttpRequest::new(
            "POST",
            self.config.endpoint.clone(),
            vec![
                ("content-type".to_owned(), "application/json".to_owned()),
                ("x-api-key".to_owned(), self.config.credential.clone()),
                ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
                (
                    "x-hacienda-conversation-id".to_owned(),
                    request.conversation_id.as_str().to_owned(),
                ),
                (
                    "x-hacienda-context-id".to_owned(),
                    request.context_id.as_str().to_owned(),
                ),
            ],
            serde_json::to_vec(&body).map_err(|_| TransportError::Failed)?,
        ))?;
        map_provider_response(
            request,
            &response,
            ProviderKind::ClaudeDesktop,
            anthropic_text,
        )
    }
}

/// `OpenAI` Chat Completions-compatible adapter for `Codex` transports.
pub struct OpenAiProviderTransport<H> {
    http: H,
    config: ProviderTransportConfig,
}

impl<H> OpenAiProviderTransport<H> {
    /// Construct an adapter over an injected HTTP client.
    #[must_use]
    pub fn new(http: H, config: ProviderTransportConfig) -> Self {
        Self { http, config }
    }
}

impl<H> ProviderTransport for OpenAiProviderTransport<H>
where
    H: ProviderHttpClient,
{
    fn stream(
        &mut self,
        request: &RedactedProviderRequest,
    ) -> Result<ProviderResponse, TransportError> {
        if request.provider != ProviderKind::Codex {
            return Err(TransportError::Failed);
        }
        let body = json!({
            "model": self.config.model,
            "stream": true,
            "messages": [{"role": "user", "content": provider_input(request)}],
            "metadata": {
                "conversation_id": request.conversation_id.as_str(),
                "context_id": request.context_id.as_str()
            }
        });
        let response = self.http.send(ProviderHttpRequest::new(
            "POST",
            self.config.endpoint.clone(),
            vec![
                ("content-type".to_owned(), "application/json".to_owned()),
                (
                    "authorization".to_owned(),
                    format!("Bearer {}", self.config.credential),
                ),
                (
                    "x-hacienda-conversation-id".to_owned(),
                    request.conversation_id.as_str().to_owned(),
                ),
                (
                    "x-hacienda-context-id".to_owned(),
                    request.context_id.as_str().to_owned(),
                ),
            ],
            serde_json::to_vec(&body).map_err(|_| TransportError::Failed)?,
        ))?;
        map_provider_response(request, &response, ProviderKind::Codex, openai_text)
    }
}

/// Real blocking HTTP implementation used by the desktop process.
#[derive(Clone)]
pub struct ReqwestHttpClient {
    client: reqwest::blocking::Client,
}

impl ReqwestHttpClient {
    /// Build the native HTTP client with a bounded request timeout.
    ///
    /// # Errors
    /// Returns an error if the native HTTP client cannot be built.
    pub fn new() -> Result<Self, TransportError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|_| TransportError::Failed)?;
        Ok(Self { client })
    }
}

impl ProviderHttpClient for ReqwestHttpClient {
    fn send(
        &mut self,
        request: ProviderHttpRequest,
    ) -> Result<ProviderHttpResponse, TransportError> {
        let mut builder = self.client.request(
            request.method.parse().map_err(|_| TransportError::Failed)?,
            &request.url,
        );
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .body(request.body)
            .send()
            .map_err(|_| TransportError::Failed)?;
        let status = response.status().as_u16();
        let body = response.text().map_err(|_| TransportError::Failed)?;
        Ok(ProviderHttpResponse::new(status, body))
    }

    fn send_stream(
        &mut self,
        request: ProviderHttpRequest,
        on_response: &mut dyn FnMut(u16, Vec<(String, String)>),
        on_chunk: &mut dyn FnMut(&[u8]) -> Result<(), TransportError>,
    ) -> Result<(), TransportError> {
        use std::io::Read;
        let mut builder = self.client.request(
            request.method.parse().map_err(|_| TransportError::Failed)?,
            &request.url,
        );
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        let mut response = builder
            .body(request.body)
            .send()
            .map_err(|_| TransportError::Failed)?;
        let status = response.status().as_u16();
        let headers = [
            "content-type",
            "cache-control",
            "retry-after",
            "x-request-id",
        ]
        .into_iter()
        .filter_map(|name| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(|value| (name.to_owned(), value.to_owned()))
        })
        .collect();
        on_response(status, headers);
        let mut buffer = [0_u8; 8192];
        loop {
            let read = response
                .read(&mut buffer)
                .map_err(|_| TransportError::Failed)?;
            if read == 0 {
                break;
            }
            on_chunk(&buffer[..read])?;
        }
        Ok(())
    }
}

/// Configured gateway transport that selects the provider wire adapter at runtime.
pub struct GatewayProviderTransport {
    http: ReqwestHttpClient,
    config: ProviderTransportConfig,
}

impl GatewayProviderTransport {
    /// Construct and validate a real gateway transport.
    ///
    /// # Errors
    /// Returns an error if the gateway configuration or native HTTP client is invalid.
    pub fn new(config: ProviderTransportConfig) -> Result<Self, TransportError> {
        config.validate()?;
        Ok(Self {
            http: ReqwestHttpClient::new()?,
            config,
        })
    }
}

impl ProviderTransport for GatewayProviderTransport {
    fn stream(
        &mut self,
        request: &RedactedProviderRequest,
    ) -> Result<ProviderResponse, TransportError> {
        match request.provider {
            ProviderKind::ClaudeDesktop => {
                AnthropicProviderTransport::new(self.http.clone(), self.config.clone())
                    .stream(request)
            }
            ProviderKind::Codex => {
                OpenAiProviderTransport::new(self.http.clone(), self.config.clone()).stream(request)
            }
        }
    }
}

fn provider_input(request: &RedactedProviderRequest) -> String {
    request
        .context
        .iter()
        .map(|item| item.content.text.as_str())
        .chain(std::iter::once(request.prompt.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn known_references(request: &RedactedProviderRequest) -> Vec<OpaqueTokenReference> {
    let mut references = Vec::new();
    let mut add = |text: &TokenizedText| {
        for reference in &text.references {
            if !references
                .iter()
                .any(|known: &OpaqueTokenReference| known == reference)
            {
                references.push(reference.clone());
            }
        }
    };
    add(&request.prompt);
    for context in &request.context {
        add(&context.content);
    }
    references
}

fn map_provider_response(
    request: &RedactedProviderRequest,
    response: &ProviderHttpResponse,
    provider: ProviderKind,
    extract_text: fn(&Value) -> Option<String>,
) -> Result<ProviderResponse, TransportError> {
    if !(200..300).contains(&response.status) {
        return Err(TransportError::Failed);
    }
    let references = known_references(request);
    let mut chunks = Vec::new();
    for data in sse_data(response.body()) {
        if data == "[DONE]" {
            continue;
        }
        let value: Value = serde_json::from_str(&data).map_err(|_| TransportError::Failed)?;
        if value.get("error").is_some() {
            return Err(TransportError::Failed);
        }
        if let Some(text) = extract_text(&value).filter(|text| !text.is_empty()) {
            chunks.push(ProviderStreamChunk {
                conversation_id: request.conversation_id.clone(),
                context_id: request.context_id.clone(),
                sequence: chunks.len() as u64,
                content: TokenizedText::new(text, references.clone()),
            });
        }
    }
    let conversation_id = request.conversation_id.clone();
    let context_id = request.context_id.clone();
    Ok(match provider {
        ProviderKind::ClaudeDesktop => ProviderResponse::ClaudeDesktop(ClaudeDesktopResponse {
            conversation_id,
            context_id,
            chunks,
        }),
        ProviderKind::Codex => ProviderResponse::Codex(CodexResponse {
            conversation_id,
            context_id,
            chunks,
        }),
    })
}

fn sse_data(body: &str) -> Vec<String> {
    let mut events = Vec::new();
    let mut current = Vec::new();
    let is_sse = body.lines().any(|line| line.starts_with("data:"));
    if !is_sse {
        return vec![body.to_owned()];
    }
    for line in body.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            current.push(data.trim_start());
        } else if line.trim().is_empty() && !current.is_empty() {
            events.push(current.join("\n"));
            current.clear();
        }
    }
    if !current.is_empty() {
        events.push(current.join("\n"));
    }
    events
}

const MAX_PROVIDER_SSE_FRAME: usize = 1024 * 1024;

#[derive(Default)]
struct SseDecoder {
    pending: Vec<u8>,
    data: Vec<String>,
    event: Option<String>,
    id: Option<String>,
    retry: Option<u64>,
    comment: Option<String>,
    frame_touched: bool,
    frame_bytes: usize,
}

impl SseDecoder {
    #[cfg(test)]
    fn push(
        &mut self,
        bytes: &[u8],
        events: &mut Vec<ProviderSseEvent>,
    ) -> Result<(), TransportError> {
        self.push_with(bytes, &mut |event| {
            events.push(event);
            Ok(())
        })
    }

    /// Decode complete frames and hand each one to `callback` immediately, so a
    /// relay can validate and release safe events while the body is still open.
    fn push_with(
        &mut self,
        bytes: &[u8],
        callback: &mut dyn FnMut(ProviderSseEvent) -> Result<(), TransportError>,
    ) -> Result<(), TransportError> {
        if self
            .pending
            .len()
            .saturating_add(self.frame_bytes)
            .saturating_add(bytes.len())
            > MAX_PROVIDER_SSE_FRAME
        {
            return Err(TransportError::Failed);
        }
        self.pending.extend_from_slice(bytes);
        if self.pending.len().saturating_add(self.frame_bytes) > MAX_PROVIDER_SSE_FRAME {
            return Err(TransportError::Failed);
        }
        while let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line = self.pending.drain(..=newline).collect::<Vec<_>>();
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
        let line = std::str::from_utf8(line).map_err(|_| TransportError::Failed)?;
        if line.len() > MAX_PROVIDER_SSE_FRAME {
            return Err(TransportError::Failed);
        }
        self.frame_bytes = self
            .frame_bytes
            .checked_add(line.len().saturating_add(1))
            .filter(|size| *size <= MAX_PROVIDER_SSE_FRAME)
            .ok_or(TransportError::Failed)?;
        if line.is_empty() {
            if !self.data.is_empty() || self.comment.is_some() {
                callback(ProviderSseEvent {
                    event: self.event.take(),
                    id: self.id.clone(),
                    retry: self.retry.take(),
                    comment: self.comment.take(),
                    data: self.data.join("\n"),
                })?;
            } else {
                self.event = None;
                self.retry = None;
            }
            self.data.clear();
            self.frame_touched = false;
            self.frame_bytes = 0;
            return Ok(());
        }
        self.frame_touched = true;
        if let Some(comment) = line.strip_prefix(':') {
            self.comment = Some(comment.strip_prefix(' ').unwrap_or(comment).to_owned());
            return Ok(());
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => {
                self.frame_bytes = self
                    .frame_bytes
                    .checked_add(std::mem::size_of::<String>())
                    .filter(|size| *size <= MAX_PROVIDER_SSE_FRAME)
                    .ok_or(TransportError::Failed)?;
                self.data.push(value.to_owned());
            }
            "event" => self.event = Some(value.to_owned()),
            "id" if !value.contains('\0') => self.id = Some(value.to_owned()),
            "retry" => self.retry = value.parse().ok(),
            _ => (), // SSE ignores extension fields and NUL-containing IDs.
        }
        Ok(())
    }

    fn finish(self) -> Result<(), TransportError> {
        // The protocol requires a blank-line terminator; incomplete UTF-8, lines,
        // and event frames are discarded as truncated input.
        if !self.pending.is_empty() || self.frame_touched {
            return Err(TransportError::Failed);
        }
        Ok(())
    }
}

#[cfg(test)]
fn decode_sse_chunks<I, S>(chunks: I) -> Result<Vec<ProviderSseEvent>, TransportError>
where
    I: IntoIterator<Item = Result<S, TransportError>>,
    S: AsRef<str>,
{
    let mut decoder = SseDecoder::default();
    let mut events = Vec::new();
    for chunk in chunks {
        decoder.push(chunk?.as_ref().as_bytes(), &mut events)?;
    }
    decoder.finish()?;
    if events.is_empty() {
        return Err(TransportError::Failed);
    }
    Ok(events)
}

fn valid_claude_response_metadata(metadata: &ProviderSseMetadata) -> bool {
    (200..300).contains(&metadata.status)
        && metadata.headers.iter().all(|(name, value)| {
            !name.eq_ignore_ascii_case("content-type")
                || value.to_ascii_lowercase().starts_with("text/event-stream")
        })
}

/// Only content-free SSE heartbeats may cross the relay. Anthropic uses
/// `: ping` / `: ping - <epoch>` frames; an empty comment is the axum keepalive.
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

#[cfg(test)]
fn validate_claude_sse_events(events: &[ProviderSseEvent]) -> Result<(), TransportError> {
    for event in events {
        validate_claude_sse_event(event)?;
    }
    Ok(())
}

// Keep the per-frame schema checks together so accepted wire shapes are easy to audit.
#[allow(clippy::too_many_lines)]
fn validate_claude_sse_event(event: &ProviderSseEvent) -> Result<(), TransportError> {
    if let Some(comment) = event.comment.as_deref() {
        if !is_safe_keepalive_comment(comment)
            || !event.data.is_empty()
            || event.event.is_some()
            || event.id.is_some()
            || event.retry.is_some()
        {
            return Err(TransportError::Failed);
        }
        return Ok(());
    }
    let kind = event.event.as_deref().ok_or(TransportError::Failed)?;
    if kind == "[DONE]" {
        return Err(TransportError::Failed);
    }
    let value: Value = serde_json::from_str(&event.data).map_err(|_| TransportError::Failed)?;
    if !value.is_object() || value.get("type").and_then(Value::as_str) != Some(kind) {
        return Err(TransportError::Failed);
    }
    match kind {
        "ping" | "message_stop" => require_keys(&value, &["type"])?,
        "content_block_delta" => {
            require_keys(&value, &["type", "index", "delta"])?;
            require_sse_index(&value)?;
            let delta = value.get("delta").ok_or(TransportError::Failed)?;
            match delta.get("type").and_then(Value::as_str) {
                Some("text_delta") => require_text_field(delta, "text")?,
                Some("input_json_delta") => require_text_field(delta, "partial_json")?,
                Some("thinking_delta") => require_text_field(delta, "thinking")?,
                Some("signature_delta") => require_text_field(delta, "signature")?,
                _ => return Err(TransportError::Failed),
            }
        }
        "content_block_start" => {
            require_keys(&value, &["type", "index", "content_block"])?;
            require_sse_index(&value)?;
            let block = value.get("content_block").ok_or(TransportError::Failed)?;
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    require_keys(block, &["type", "text"])?;
                    if block.get("text").and_then(Value::as_str) != Some("") {
                        return Err(TransportError::Failed);
                    }
                }
                Some("tool_use") => {
                    require_keys(block, &["type", "id", "name", "input"])?;
                    if !block.get("id").is_some_and(Value::is_string)
                        || !block.get("name").is_some_and(Value::is_string)
                        || !block
                            .get("input")
                            .and_then(Value::as_object)
                            .is_some_and(serde_json::Map::is_empty)
                    {
                        return Err(TransportError::Failed);
                    }
                }
                Some("thinking") => {
                    require_keys(block, &["type", "thinking", "signature"])?;
                    if block.get("thinking").and_then(Value::as_str) != Some("")
                        || !block.get("signature").is_some_and(Value::is_string)
                    {
                        return Err(TransportError::Failed);
                    }
                }
                _ => return Err(TransportError::Failed),
            }
        }
        "content_block_stop" => {
            require_keys(&value, &["type", "index"])?;
            require_sse_index(&value)?;
        }
        "message_start" => {
            require_keys(&value, &["type", "message"])?;
            let message = value.get("message").ok_or(TransportError::Failed)?;
            require_keys(
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
            if message
                .get("content")
                .and_then(Value::as_array)
                .is_none_or(|items| !items.is_empty())
                || message
                    .get("id")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                || message.get("type").and_then(Value::as_str) != Some("message")
                || message.get("role").and_then(Value::as_str) != Some("assistant")
                || message
                    .get("model")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                || !message.get("stop_reason").is_some_and(Value::is_null)
                || !message.get("stop_sequence").is_some_and(Value::is_null)
            {
                return Err(TransportError::Failed);
            }
            validate_usage(message.get("usage").ok_or(TransportError::Failed)?)?;
        }
        "message_delta" => {
            require_keys(&value, &["type", "delta", "usage"])?;
            let delta = value.get("delta").ok_or(TransportError::Failed)?;
            require_keys(delta, &["stop_reason", "stop_sequence"])?;
            if !delta
                .get("stop_reason")
                .is_some_and(|value| value.is_null() || value.is_string())
                || !delta
                    .get("stop_sequence")
                    .is_some_and(|value| value.is_null() || value.is_string())
            {
                return Err(TransportError::Failed);
            }
            validate_usage(value.get("usage").ok_or(TransportError::Failed)?)?;
        }
        "error" => {
            require_keys(&value, &["type", "error"])?;
            let detail = value.get("error").ok_or(TransportError::Failed)?;
            require_keys(detail, &["type", "message"])?;
            if !detail.get("message").is_some_and(Value::is_string) {
                return Err(TransportError::Failed);
            }
        }
        _ => return Err(TransportError::Failed),
    }
    Ok(())
}

fn require_sse_index(value: &Value) -> Result<u64, TransportError> {
    value
        .get("index")
        .and_then(Value::as_u64)
        .ok_or(TransportError::Failed)
}

fn require_text_field(value: &Value, field: &str) -> Result<(), TransportError> {
    require_keys(value, &["type", field])?;
    if value.get(field).is_some_and(Value::is_string) {
        Ok(())
    } else {
        Err(TransportError::Failed)
    }
}

fn validate_usage(value: &Value) -> Result<(), TransportError> {
    const USAGE_FIELDS: &[&str] = &[
        "input_tokens",
        "output_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "cache_creation",
        "server_tool_use",
        "service_tier",
    ];
    require_keys(value, USAGE_FIELDS)?;
    if value.as_object().is_none_or(|usage| {
        usage.values().any(|field| {
            !(field.is_number() || field.is_null() || field.is_object() || field.is_string())
        })
    }) {
        return Err(TransportError::Failed);
    }
    for key in ["cache_creation", "server_tool_use"] {
        if let Some(nested) = value.get(key) {
            require_keys(
                nested,
                &[
                    "ephemeral_5m_input_tokens",
                    "ephemeral_1h_input_tokens",
                    "web_search_requests",
                ],
            )?;
            if nested
                .as_object()
                .is_none_or(|fields| !fields.values().all(Value::is_number))
            {
                return Err(TransportError::Failed);
            }
        }
    }
    Ok(())
}

fn require_keys(value: &Value, allowed: &[&str]) -> Result<(), TransportError> {
    let object = value.as_object().ok_or(TransportError::Failed)?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(TransportError::Failed);
    }
    Ok(())
}

const MAX_REDACTION_BUFFER: usize = 1024 * 1024;
const MAX_REDACTION_DELTAS: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClaudeBlockKind {
    Text,
    ToolUse,
    Thinking,
}

struct PendingClaudeBlock {
    index: u64,
    kind: ClaudeBlockKind,
    joined: String,
    /// Delta frame plus the JSON pointer of its redactable fragment. A
    /// `signature_delta` has no redactable text; it is dropped if the signed
    /// thinking text changes during redaction.
    deltas: Vec<(ProviderSseEvent, Option<&'static str>)>,
    retained_bytes: usize,
}

/// Incremental redaction relay for Anthropic SSE frames.
///
/// The relay holds back only the *current* logical text. Every redactable
/// fragment of a content block is joined into `joined` and passed through the
/// adapter once before that block's deltas are released, so PII split across
/// provider deltas cannot escape in a partial callback. Structural frames are
/// forwarded as they arrive, keeping Claude's idle watchdog pacified.
struct ClaudeSseRelay<'a, R, F>
where
    R: RedactionAdapter,
    F: FnMut(ProviderSseEvent),
{
    redactor: &'a R,
    callback: &'a mut F,
    active: Option<PendingClaudeBlock>,
    message_started: bool,
    message_delta_seen: bool,
    terminal: bool,
}

impl<'a, R, F> ClaudeSseRelay<'a, R, F>
where
    R: RedactionAdapter,
    F: FnMut(ProviderSseEvent),
{
    fn new(redactor: &'a R, callback: &'a mut F) -> Self {
        Self {
            redactor,
            callback,
            active: None,
            message_started: false,
            message_delta_seen: false,
            terminal: false,
        }
    }

    fn emit(&mut self, event: ProviderSseEvent) {
        (self.callback)(event);
    }

    // This is the protocol state machine; splitting transitions across helpers would
    // make it harder to verify which frames can be emitted before buffered text is safe.
    #[allow(clippy::too_many_lines)]
    fn accept(&mut self, event: ProviderSseEvent) -> Result<(), TransportError> {
        if self.terminal {
            return Err(TransportError::Failed);
        }
        validate_claude_sse_event(&event)?;
        if event.comment.is_some() {
            self.emit(ProviderSseEvent {
                event: None,
                id: None,
                retry: None,
                comment: Some("ping".to_owned()),
                data: String::new(),
            });
            return Ok(());
        }
        let kind = event
            .event
            .as_deref()
            .ok_or(TransportError::Failed)?
            .to_owned();
        let value: Value = serde_json::from_str(&event.data).map_err(|_| TransportError::Failed)?;
        match kind.as_str() {
            "message_start" => {
                if self.message_started || self.message_delta_seen || self.active.is_some() {
                    return Err(TransportError::Failed);
                }
                self.message_started = true;
                self.emit(event);
            }
            "ping" => self.emit(event),
            "content_block_start" => {
                if !self.message_started || self.message_delta_seen || self.active.is_some() {
                    return Err(TransportError::Failed);
                }
                let index = require_sse_index(&value)?;
                let block = value.get("content_block").ok_or(TransportError::Failed)?;
                let block_kind = match block.get("type").and_then(Value::as_str) {
                    Some("text") => ClaudeBlockKind::Text,
                    Some("tool_use") => ClaudeBlockKind::ToolUse,
                    Some("thinking") => ClaudeBlockKind::Thinking,
                    _ => return Err(TransportError::Failed),
                };
                let mut safe = event;
                if block_kind == ClaudeBlockKind::ToolUse {
                    safe.data =
                        redact_json_string(&safe.data, "/content_block/name", self.redactor)?;
                }
                self.active = Some(PendingClaudeBlock {
                    index,
                    kind: block_kind,
                    joined: String::new(),
                    deltas: Vec::new(),
                    retained_bytes: 0,
                });
                self.emit(safe);
            }
            "content_block_delta" => {
                if !self.message_started || self.message_delta_seen {
                    return Err(TransportError::Failed);
                }
                let index = require_sse_index(&value)?;
                let delta = value.get("delta").ok_or(TransportError::Failed)?;
                let (pointer, block_kind) = match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => (Some("/delta/text"), ClaudeBlockKind::Text),
                    Some("input_json_delta") => {
                        (Some("/delta/partial_json"), ClaudeBlockKind::ToolUse)
                    }
                    Some("thinking_delta") => (Some("/delta/thinking"), ClaudeBlockKind::Thinking),
                    Some("signature_delta") => (None, ClaudeBlockKind::Thinking),
                    _ => return Err(TransportError::Failed),
                };
                let active = self.active.as_mut().ok_or(TransportError::Failed)?;
                if active.index != index || active.kind != block_kind {
                    return Err(TransportError::Failed);
                }
                if let Some(pointer) = pointer {
                    let fragment = value
                        .pointer(pointer)
                        .and_then(Value::as_str)
                        .ok_or(TransportError::Failed)?;
                    active.joined.push_str(fragment);
                    if active.joined.len() > MAX_REDACTION_BUFFER {
                        return Err(TransportError::Failed);
                    }
                }
                active.retained_bytes = active
                    .retained_bytes
                    .checked_add(event.data.len())
                    .filter(|bytes| *bytes <= MAX_REDACTION_BUFFER)
                    .ok_or(TransportError::Failed)?;
                if active.deltas.len() >= MAX_REDACTION_DELTAS {
                    return Err(TransportError::Failed);
                }
                active.deltas.push((event, pointer));
            }
            "content_block_stop" => {
                let index = require_sse_index(&value)?;
                if self
                    .active
                    .as_ref()
                    .is_none_or(|block| block.index != index)
                {
                    return Err(TransportError::Failed);
                }
                self.flush()?;
                self.emit(event);
            }
            "message_delta" => {
                if !self.message_started || self.active.is_some() || self.message_delta_seen {
                    return Err(TransportError::Failed);
                }
                self.message_delta_seen = true;
                let safe = if value
                    .pointer("/delta/stop_sequence")
                    .and_then(Value::as_str)
                    .is_some()
                {
                    ProviderSseEvent {
                        data: redact_json_string(
                            &event.data,
                            "/delta/stop_sequence",
                            self.redactor,
                        )?,
                        ..event
                    }
                } else {
                    event
                };
                self.emit(safe);
            }
            "message_stop" => {
                if !self.message_started || !self.message_delta_seen || self.active.is_some() {
                    return Err(TransportError::Failed);
                }
                self.terminal = true;
                self.emit(event);
            }
            "error" => {
                self.flush()?;
                let message = value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .ok_or(TransportError::Failed)?;
                let redacted = self
                    .redactor
                    .redact_text(message)
                    .map_err(|_| TransportError::Failed)?
                    .text;
                if redacted.len() > MAX_REDACTION_BUFFER {
                    return Err(TransportError::Failed);
                }
                let mut data = event.data;
                replace_json_string(&mut data, "/error/message", redacted)?;
                self.emit(ProviderSseEvent { data, ..event });
            }
            _ => return Err(TransportError::Failed),
        }
        Ok(())
    }

    /// Redact and release the buffered logical text, if any. This is the only
    /// place buffered provider text crosses the callback boundary.
    fn flush(&mut self) -> Result<(), TransportError> {
        let Some(block) = self.active.take() else {
            return Ok(());
        };
        if block.kind == ClaudeBlockKind::ToolUse && !is_json_object(&block.joined) {
            return Err(TransportError::Failed);
        }
        let safe_text = if block.joined.is_empty() {
            None
        } else if block.kind == ClaudeBlockKind::ToolUse {
            let mut value: Value =
                serde_json::from_str(&block.joined).map_err(|_| TransportError::Failed)?;
            if !value.is_object() {
                return Err(TransportError::Failed);
            }
            redact_json_value_strings(&mut value, self.redactor)?;
            let text = serde_json::to_string(&value).map_err(|_| TransportError::Failed)?;
            if text.len() > MAX_REDACTION_BUFFER || !is_json_object(&text) {
                return Err(TransportError::Failed);
            }
            Some(text)
        } else {
            let text = self
                .redactor
                .redact_text(&block.joined)
                .map_err(|_| TransportError::Failed)?
                .text;
            if text.len() > MAX_REDACTION_BUFFER {
                return Err(TransportError::Failed);
            }
            Some(text)
        };
        let invalidate_signature = block.kind == ClaudeBlockKind::Thinking
            && safe_text
                .as_deref()
                .is_some_and(|text| text != block.joined);
        let mut deltas = block.deltas;
        if invalidate_signature {
            deltas.retain(|(event, _)| {
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
        let last_redactable = deltas.iter().rposition(|(_, pointer)| pointer.is_some());
        for (position, (event, pointer)) in deltas.iter_mut().enumerate() {
            let Some(pointer) = pointer else {
                continue;
            };
            let text = if Some(position) == last_redactable {
                safe_text.clone().unwrap_or_default()
            } else {
                String::new()
            };
            replace_json_string(&mut event.data, pointer, text)?;
        }
        for (event, _) in deltas {
            self.emit(event);
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), TransportError> {
        if self.terminal && self.active.is_none() {
            Ok(())
        } else {
            Err(TransportError::Failed)
        }
    }
}

fn is_json_object(text: &str) -> bool {
    serde_json::from_str::<Value>(text).is_ok_and(|value| value.is_object())
}

fn redact_json_value_strings<R: RedactionAdapter>(
    value: &mut Value,
    redactor: &R,
) -> Result<(), TransportError> {
    match value {
        Value::String(text) => {
            if text.len() > MAX_REDACTION_BUFFER {
                return Err(TransportError::Failed);
            }
            *text = redactor
                .redact_text(text)
                .map_err(|_| TransportError::Failed)?
                .text;
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                redact_json_value_strings(item, redactor)?;
            }
            Ok(())
        }
        Value::Object(object) => {
            let keys: Vec<String> = object.keys().cloned().collect();
            for key in keys {
                let safe_key = redactor
                    .redact_text(&key)
                    .map_err(|_| TransportError::Failed)?
                    .text;
                if safe_key != key {
                    return Err(TransportError::Failed);
                }
                redact_json_value_strings(
                    object.get_mut(&key).ok_or(TransportError::Failed)?,
                    redactor,
                )?;
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}

fn replace_json_string(
    data: &mut String,
    pointer: &str,
    value: String,
) -> Result<(), TransportError> {
    let mut json: Value = serde_json::from_str(data).map_err(|_| TransportError::Failed)?;
    *json.pointer_mut(pointer).ok_or(TransportError::Failed)? = Value::String(value);
    *data = serde_json::to_string(&json).map_err(|_| TransportError::Failed)?;
    Ok(())
}

fn redact_json_string<R: RedactionAdapter>(
    data: &str,
    pointer: &str,
    redactor: &R,
) -> Result<String, TransportError> {
    let json: Value = serde_json::from_str(data).map_err(|_| TransportError::Failed)?;
    let text = json
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or(TransportError::Failed)?;
    let safe = redactor
        .redact_text(text)
        .map_err(|_| TransportError::Failed)?
        .text;
    let mut redacted = data.to_owned();
    replace_json_string(&mut redacted, pointer, safe)?;
    Ok(redacted)
}

/// Buffered variant of the relay, retained for adapters that already hold the
/// complete frame sequence. It follows the same fail-closed redaction path.
#[cfg(test)]
fn redact_claude_sse_events<R: RedactionAdapter>(
    redactor: &R,
    events: Vec<ProviderSseEvent>,
) -> Result<Vec<ProviderSseEvent>, TransportError> {
    validate_claude_sse_events(&events)?;
    let mut safe = Vec::with_capacity(events.len());
    {
        let mut collect = |event| safe.push(event);
        let mut relay = ClaudeSseRelay::new(redactor, &mut collect);
        for event in events {
            relay.accept(event)?;
        }
        relay.flush()?;
    }
    Ok(safe)
}

fn anthropic_text(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) == Some("content_block_delta") {
        return value
            .pointer("/delta/text")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
    if let Some(text) = value.get("completion").and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    value
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.iter().find_map(|item| item.get("text")))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn openai_text(value: &Value) -> Option<String> {
    if let Some(text) = value.get("output_text").and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    if let Some(choice) = value.get("choices").and_then(Value::as_array)?.first() {
        for pointer in ["/delta/content", "/message/content", "/text"] {
            if let Some(text) = choice.pointer(pointer).and_then(Value::as_str) {
                return Some(text.to_owned());
            }
        }
    }
    value
        .get("output")
        .and_then(Value::as_array)
        .and_then(|output| {
            output
                .iter()
                .find_map(|item| item.pointer("/content/0/text"))
        })
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Adapter seam for explicit, authorized human display rehydration.
pub trait RehydrationAdapter {
    /// Resolve token-only content for a viewer that supplied a capability.
    ///
    /// # Errors
    ///
    /// Returns an error when the capability is invalid or token resolution fails.
    fn rehydrate(
        &self,
        capability: &DisplayCapability,
        content: &TokenizedText,
    ) -> Result<String, RehydrationError>;
}

/// Local client module. It stores adapters only; raw prompts and display plaintext are
/// never retained in client state.
pub struct SecrecyClient<T, R, H> {
    transport: T,
    redactor: Option<R>,
    rehydrator: Option<H>,
}

impl<T, R, H> SecrecyClient<T, R, H>
where
    T: ProviderTransport,
    R: RedactionAdapter,
    H: RehydrationAdapter,
{
    /// Construct a client with explicit redaction and optional display adapters.
    #[must_use]
    pub fn new(transport: T, redactor: Option<R>, rehydrator: Option<H>) -> Self {
        Self {
            transport,
            redactor,
            rehydrator,
        }
    }

    /// Redact and stream a request through the only provider transport seam.
    ///
    /// # Errors
    ///
    /// Returns an error when redaction or the provider transport fails.
    pub fn stream(&mut self, request: &ProviderRequest) -> Result<ProviderResponse, ClientError> {
        let redactor = self
            .redactor
            .as_ref()
            .ok_or(ClientError::RedactionUnavailable)?;
        let safe_request = redactor.redact(request)?;
        let response = self
            .transport
            .stream(&safe_request)
            .map_err(ClientError::from)?;
        redact_provider_response(redactor, response).map_err(ClientError::from)
    }

    /// Rehydrate one response chunk only for an explicitly authorized display.
    ///
    /// # Errors
    ///
    /// Returns an error when display authorization or token resolution fails.
    pub fn rehydrate_for_display(
        &self,
        chunk: &ProviderStreamChunk,
        capability: Option<&DisplayCapability>,
    ) -> Result<DisplayChunk, ClientError> {
        let capability = capability.ok_or(ClientError::RehydrationUnavailable)?;
        let rehydrator = self
            .rehydrator
            .as_ref()
            .ok_or(ClientError::RehydrationUnavailable)?;
        let text = rehydrator.rehydrate(capability, &chunk.content)?;
        Ok(DisplayChunk {
            conversation_id: chunk.conversation_id.clone(),
            context_id: chunk.context_id.clone(),
            sequence: chunk.sequence,
            text,
        })
    }
}

fn redact_provider_response<R: RedactionAdapter>(
    redactor: &R,
    response: ProviderResponse,
) -> Result<ProviderResponse, RedactionError> {
    let redact =
        |chunks: Vec<ProviderStreamChunk>| -> Result<Vec<ProviderStreamChunk>, RedactionError> {
            let Some(first) = chunks.first() else {
                return Ok(chunks);
            };
            if chunks.iter().any(|chunk| {
                chunk.conversation_id != first.conversation_id
                    || chunk.context_id != first.context_id
            }) {
                return Err(RedactionError::Rejected);
            }
            let joined: String = chunks
                .iter()
                .map(|chunk| chunk.content.text.as_str())
                .collect();
            let mut content = redactor.redact_text(&joined)?;
            for reference in chunks.iter().flat_map(|chunk| &chunk.content.references) {
                if !content.references.contains(reference) {
                    content.references.push(reference.clone());
                }
            }
            Ok(vec![ProviderStreamChunk {
                conversation_id: first.conversation_id.clone(),
                context_id: first.context_id.clone(),
                sequence: first.sequence,
                content,
            }])
        };
    Ok(match response {
        ProviderResponse::ClaudeDesktop(mut response) => {
            response.chunks = redact(response.chunks)?;
            ProviderResponse::ClaudeDesktop(response)
        }
        ProviderResponse::Codex(mut response) => {
            response.chunks = redact(response.chunks)?;
            ProviderResponse::Codex(response)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    // Synthetic sentinels keep this test focused on the redaction boundary
    // without placing realistic PII in a provider fixture.
    const PERSON: &str = "<person-fixture>";
    const EMAIL: &str = "<email-fixture>";
    const PERSON_TOKEN: &str = "[PERSON:opaque-1]";

    #[derive(Default)]
    struct TestRedactor;

    impl RedactionAdapter for TestRedactor {
        fn redact(
            &self,
            request: &ProviderRequest,
        ) -> Result<RedactedProviderRequest, RedactionError> {
            let (prompt, context) = match request {
                ProviderRequest::ClaudeDesktop(request) => (&request.prompt, &request.context),
                ProviderRequest::Codex(request) => (&request.prompt, &request.context),
            };
            let references = vec![OpaqueTokenReference::new(PERSON_TOKEN, "person")];
            let tokenize = |value: &str| {
                TokenizedText::new(
                    value
                        .replace(PERSON, PERSON_TOKEN)
                        .replace(EMAIL, "[EMAIL:opaque-2]"),
                    references.clone(),
                )
            };
            Ok(RedactedProviderRequest {
                provider: request.provider(),
                conversation_id: request.conversation_id().clone(),
                context_id: request.context_id().clone(),
                prompt: tokenize(prompt),
                context: context
                    .iter()
                    .map(|item| RedactedContext {
                        label: item.label.clone(),
                        content: tokenize(&item.content),
                    })
                    .collect(),
            })
        }

        fn redact_text(&self, text: &str) -> Result<TokenizedText, RedactionError> {
            Ok(TokenizedText::new(
                text.replace(PERSON, PERSON_TOKEN)
                    .replace(EMAIL, "[EMAIL:opaque-2]"),
                vec![OpaqueTokenReference::new(PERSON_TOKEN, "person")],
            ))
        }
    }

    #[derive(Default)]
    struct RecordingTransport {
        request: Option<RedactedProviderRequest>,
    }

    impl ProviderTransport for RecordingTransport {
        fn stream(
            &mut self,
            request: &RedactedProviderRequest,
        ) -> Result<ProviderResponse, TransportError> {
            self.request = Some(request.clone());
            let chunk = ProviderStreamChunk {
                conversation_id: request.conversation_id.clone(),
                context_id: request.context_id.clone(),
                sequence: 0,
                content: request.prompt.clone(),
            };
            let response = match request.provider {
                ProviderKind::ClaudeDesktop => {
                    ProviderResponse::ClaudeDesktop(ClaudeDesktopResponse {
                        conversation_id: request.conversation_id.clone(),
                        context_id: request.context_id.clone(),
                        chunks: vec![chunk],
                    })
                }
                ProviderKind::Codex => ProviderResponse::Codex(CodexResponse {
                    conversation_id: request.conversation_id.clone(),
                    context_id: request.context_id.clone(),
                    chunks: vec![chunk],
                }),
            };
            Ok(response)
        }
    }

    struct LeakingTransport;

    impl ProviderTransport for LeakingTransport {
        fn stream(
            &mut self,
            request: &RedactedProviderRequest,
        ) -> Result<ProviderResponse, TransportError> {
            let chunk = ProviderStreamChunk {
                conversation_id: request.conversation_id.clone(),
                context_id: request.context_id.clone(),
                sequence: 0,
                content: TokenizedText::new(format!("Unexpected raw value: {PERSON}"), Vec::new()),
            };
            Ok(match request.provider {
                ProviderKind::ClaudeDesktop => {
                    ProviderResponse::ClaudeDesktop(ClaudeDesktopResponse {
                        conversation_id: request.conversation_id.clone(),
                        context_id: request.context_id.clone(),
                        chunks: vec![chunk],
                    })
                }
                ProviderKind::Codex => ProviderResponse::Codex(CodexResponse {
                    conversation_id: request.conversation_id.clone(),
                    context_id: request.context_id.clone(),
                    chunks: vec![chunk],
                }),
            })
        }
    }

    #[derive(Default)]
    struct TestRehydrator;

    impl RehydrationAdapter for TestRehydrator {
        fn rehydrate(
            &self,
            _capability: &DisplayCapability,
            content: &TokenizedText,
        ) -> Result<String, RehydrationError> {
            Ok(content.text.replace(PERSON_TOKEN, PERSON))
        }
    }

    fn request(provider: ProviderKind) -> ProviderRequest {
        let conversation_id = ConversationId::new("conversation-1");
        let context_id = ContextId::new("context-1");
        let context = vec![PromptContext::new("prior", format!("{PERSON} <{EMAIL}>"))];
        match provider {
            ProviderKind::ClaudeDesktop => ProviderRequest::ClaudeDesktop(
                ClaudeDesktopRequest::new(conversation_id, context_id, format!("Hello {PERSON}"))
                    .with_context(context),
            ),
            ProviderKind::Codex => ProviderRequest::Codex(
                CodexRequest::new(conversation_id, context_id, format!("Hello {PERSON}"))
                    .with_context(context),
            ),
        }
    }

    #[test]
    fn pii_is_absent_from_provider_payloads_for_both_provider_models() {
        for provider in [ProviderKind::ClaudeDesktop, ProviderKind::Codex] {
            let transport = RecordingTransport::default();
            let mut client =
                SecrecyClient::new(transport, Some(TestRedactor), None::<TestRehydrator>);
            client
                .stream(&request(provider))
                .expect("redacted transport succeeds");
            let payload = format!("{:?}", client.transport.request.expect("recorded payload"));
            assert!(!payload.contains(PERSON));
            assert!(!payload.contains(EMAIL));
        }
    }

    #[test]
    fn provider_response_with_raw_pii_is_redacted_before_return_for_both_providers() {
        for provider in [ProviderKind::ClaudeDesktop, ProviderKind::Codex] {
            let mut client =
                SecrecyClient::new(LeakingTransport, Some(TestRedactor), None::<TestRehydrator>);
            let response = client
                .stream(&request(provider))
                .expect("response is redacted");
            let text = match response {
                ProviderResponse::ClaudeDesktop(response) => {
                    response.chunks[0].content.text.clone()
                }
                ProviderResponse::Codex(response) => response.chunks[0].content.text.clone(),
            };
            assert!(!text.contains(PERSON));
            assert!(text.contains(PERSON_TOKEN));
        }
    }

    #[test]
    fn response_redaction_catches_pii_split_across_provider_chunks() {
        for provider in [ProviderKind::ClaudeDesktop, ProviderKind::Codex] {
            let conversation_id = ConversationId::new("conversation");
            let context_id = ContextId::new("context");
            let chunks: Vec<ProviderStreamChunk> = ["Unexpected <per", "son-fixture>"]
                .into_iter()
                .enumerate()
                .map(|(sequence, text)| ProviderStreamChunk {
                    conversation_id: conversation_id.clone(),
                    context_id: context_id.clone(),
                    sequence: sequence as u64,
                    content: TokenizedText::new(text, Vec::new()),
                })
                .collect();
            let response = match provider {
                ProviderKind::ClaudeDesktop => {
                    ProviderResponse::ClaudeDesktop(ClaudeDesktopResponse {
                        conversation_id: conversation_id.clone(),
                        context_id: context_id.clone(),
                        chunks,
                    })
                }
                ProviderKind::Codex => ProviderResponse::Codex(CodexResponse {
                    conversation_id: conversation_id.clone(),
                    context_id: context_id.clone(),
                    chunks,
                }),
            };
            let response = redact_provider_response(&TestRedactor, response).unwrap();
            let chunks = match response {
                ProviderResponse::ClaudeDesktop(response) => response.chunks,
                ProviderResponse::Codex(response) => response.chunks,
            };
            assert_eq!(chunks.len(), 1);
            assert_eq!(chunks[0].content.text, format!("Unexpected {PERSON_TOKEN}"));
            assert!(!chunks[0].content.text.contains(PERSON));
        }
    }

    #[test]
    fn response_redaction_default_rejects_unimplemented_adapters() {
        struct RequestOnlyRedactor;

        impl RedactionAdapter for RequestOnlyRedactor {
            fn redact(
                &self,
                request: &ProviderRequest,
            ) -> Result<RedactedProviderRequest, RedactionError> {
                TestRedactor.redact(request)
            }
        }

        assert_eq!(
            RequestOnlyRedactor.redact_text(PERSON),
            Err(RedactionError::Rejected)
        );
    }

    #[test]
    fn context_continuity_uses_stable_opaque_tokens_and_identifiers() {
        let transport = RecordingTransport::default();
        let mut client = SecrecyClient::new(transport, Some(TestRedactor), None::<TestRehydrator>);
        let response = client
            .stream(&request(ProviderKind::ClaudeDesktop))
            .expect("stream succeeds");
        let ProviderResponse::ClaudeDesktop(response) = response else {
            panic!("expected Claude response");
        };
        assert_eq!(response.conversation_id.as_str(), "conversation-1");
        assert_eq!(response.context_id.as_str(), "context-1");
        assert_eq!(response.chunks[0].content.references[0].token, PERSON_TOKEN);
    }

    #[test]
    fn unavailable_redaction_and_rehydration_fail_closed() {
        let transport = RecordingTransport::default();
        let mut client =
            SecrecyClient::new(transport, None::<TestRedactor>, None::<TestRehydrator>);
        assert_eq!(
            client.stream(&request(ProviderKind::Codex)),
            Err(ClientError::RedactionUnavailable)
        );

        let chunk = ProviderStreamChunk {
            conversation_id: ConversationId::new("conversation-1"),
            context_id: ContextId::new("context-1"),
            sequence: 0,
            content: TokenizedText::new(
                PERSON_TOKEN,
                vec![OpaqueTokenReference::new(PERSON_TOKEN, "person")],
            ),
        };
        assert_eq!(
            client.rehydrate_for_display(&chunk, None),
            Err(ClientError::RehydrationUnavailable)
        );
    }

    #[test]
    fn response_token_references_remain_stable_until_display_rehydration() {
        let transport = RecordingTransport::default();
        let mut client = SecrecyClient::new(transport, Some(TestRedactor), Some(TestRehydrator));
        let response = client
            .stream(&request(ProviderKind::Codex))
            .expect("stream succeeds");
        let ProviderResponse::Codex(response) = response else {
            panic!("expected Codex response");
        };
        let chunk = &response.chunks[0];
        assert_eq!(chunk.content.text, format!("Hello {PERSON_TOKEN}"));
        assert_eq!(chunk.content.references[0].token, PERSON_TOKEN);
        let display = client
            .rehydrate_for_display(chunk, Some(&DisplayCapability::new("viewer-capability")))
            .expect("authorized display succeeds");
        assert_eq!(display.text, format!("Hello {PERSON}"));
        assert_eq!(chunk.content.text, format!("Hello {PERSON_TOKEN}"));
    }

    #[derive(Clone)]
    struct HttpFixture {
        response: ProviderHttpResponse,
        seen: Arc<Mutex<Option<ProviderHttpRequest>>>,
    }

    impl ProviderHttpClient for HttpFixture {
        fn send(
            &mut self,
            request: ProviderHttpRequest,
        ) -> Result<ProviderHttpResponse, TransportError> {
            *self.seen.lock().expect("fixture lock") = Some(request);
            Ok(self.response.clone())
        }
    }

    fn redacted_request(provider: ProviderKind) -> RedactedProviderRequest {
        RedactedProviderRequest {
            provider,
            conversation_id: ConversationId::new("conversation-stable"),
            context_id: ContextId::new("context-stable"),
            prompt: TokenizedText::new(
                format!("Use {PERSON_TOKEN}"),
                vec![OpaqueTokenReference::new(PERSON_TOKEN, "person")],
            ),
            context: Vec::new(),
        }
    }

    #[test]
    fn anthropic_adapter_maps_sse_to_tokenized_chunks_without_raw_debug_values() {
        let seen = Arc::new(Mutex::new(None));
        let http = HttpFixture {
            response: ProviderHttpResponse::new(
                200,
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello [PERSON:opaque-1]\"}}\n\ndata: [DONE]\n\n",
            ),
            seen: Arc::clone(&seen),
        };
        let mut transport = AnthropicProviderTransport::new(
            http,
            ProviderTransportConfig::new(
                "https://provider.invalid/v1/messages?api_key=secret-url",
                "claude-test",
                32,
                "anthropic-secret",
            ),
        );
        let response = transport
            .stream(&redacted_request(ProviderKind::ClaudeDesktop))
            .expect("Anthropic fixture succeeds");
        let ProviderResponse::ClaudeDesktop(response) = response else {
            panic!("expected Claude response");
        };
        assert_eq!(response.conversation_id.as_str(), "conversation-stable");
        assert_eq!(response.context_id.as_str(), "context-stable");
        assert_eq!(response.chunks[0].content.text, "Hello [PERSON:opaque-1]");
        assert_eq!(response.chunks[0].content.references[0].token, PERSON_TOKEN);

        let request = seen.lock().expect("fixture lock").take().expect("request");
        let payload = String::from_utf8(request.body().to_vec()).expect("json body");
        assert!(payload.contains("[PERSON:opaque-1]"));
        assert!(!payload.contains(PERSON));
        let debug = format!("{request:?}");
        assert!(!debug.contains("anthropic-secret"));
        assert!(!debug.contains("[PERSON:opaque-1]"));
        assert!(
            !format!(
                "{:?}",
                ProviderHttpResponse::new(200, "raw-response-secret")
            )
            .contains("raw-response-secret")
        );
    }

    #[test]
    fn openai_adapter_maps_sse_and_keeps_response_token_references_stable() {
        let http = HttpFixture {
            response: ProviderHttpResponse::new(
                200,
                "data: {\"choices\":[{\"delta\":{\"content\":\"A [PERSON:opaque-1]\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\" continued\"}}]}\n\ndata: [DONE]\n\n",
            ),
            seen: Arc::new(Mutex::new(None)),
        };
        let mut transport = OpenAiProviderTransport::new(
            http,
            ProviderTransportConfig::new(
                "https://provider.invalid/v1/chat/completions",
                "codex-test",
                32,
                "openai-secret",
            ),
        );
        let response = transport
            .stream(&redacted_request(ProviderKind::Codex))
            .expect("OpenAI fixture succeeds");
        let ProviderResponse::Codex(response) = response else {
            panic!("expected Codex response");
        };
        assert_eq!(response.chunks.len(), 2);
        assert_eq!(response.chunks[0].sequence, 0);
        assert_eq!(response.chunks[1].sequence, 1);
        assert_eq!(response.chunks[1].content.references[0].token, PERSON_TOKEN);
        assert!(format!("{response:?}").contains("<token-only>"));
        assert!(!format!("{response:?}").contains("[PERSON:opaque-1]"));
    }

    #[test]
    fn gateway_transport_uses_loopback_http_for_claude_and_codex() {
        for provider in [ProviderKind::ClaudeDesktop, ProviderKind::Codex] {
            let response_body = match provider {
                ProviderKind::ClaudeDesktop => concat!(
                    "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Reply ",
                    "[PERSON:opaque-1]",
                    "\"}}\n\n",
                    "data: [DONE]\n\n"
                ),
                ProviderKind::Codex => concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"Reply ",
                    "[PERSON:opaque-1]",
                    "\"}}]}\n\n",
                    "data: [DONE]\n\n"
                ),
            };
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind local gateway fixture");
            let address = listener.local_addr().expect("fixture address");
            let response_body = response_body.to_owned();
            let server = thread::spawn(move || {
                let (socket, _) = listener.accept().expect("accept gateway request");
                let mut reader = BufReader::new(socket);
                let mut request_head = String::new();
                let mut line = String::new();
                reader.read_line(&mut line).expect("read request line");
                request_head.push_str(&line);
                let mut content_length = 0_usize;
                loop {
                    line.clear();
                    reader.read_line(&mut line).expect("read request headers");
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            content_length = value.trim().parse().expect("content length");
                        }
                    }
                    request_head.push_str(&line);
                }
                let mut body = vec![0; content_length];
                reader.read_exact(&mut body).expect("read request body");
                let request = format!(
                    "{request_head}{}",
                    String::from_utf8(body).expect("provider request JSON")
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                reader
                    .get_mut()
                    .write_all(response.as_bytes())
                    .expect("write gateway response");
                request
            });

            let config = ProviderTransportConfig::new(
                format!("http://{address}/v1/messages"),
                "local-fixture",
                64,
                "gateway-secret",
            );
            let mut transport = GatewayProviderTransport::new(config).expect("valid local gateway");
            let response = transport
                .stream(&redacted_request(provider))
                .expect("real reqwest loopback exchange succeeds");
            let request = server.join().expect("gateway fixture thread");
            let request_headers = request.to_ascii_lowercase();
            assert!(request.starts_with("POST /v1/messages HTTP/1.1\r\n"));
            assert!(request.contains("[PERSON:opaque-1]"), "{request}");
            assert!(!request.contains(PERSON), "{request}");
            match provider {
                ProviderKind::ClaudeDesktop => {
                    assert!(request_headers.contains("x-api-key: gateway-secret"));
                }
                ProviderKind::Codex => {
                    assert!(request_headers.contains("authorization: bearer gateway-secret"));
                }
            }
            match response {
                ProviderResponse::ClaudeDesktop(response)
                    if provider == ProviderKind::ClaudeDesktop =>
                {
                    assert_eq!(response.chunks[0].content.text, "Reply [PERSON:opaque-1]");
                }
                ProviderResponse::Codex(response) if provider == ProviderKind::Codex => {
                    assert_eq!(response.chunks[0].content.text, "Reply [PERSON:opaque-1]");
                }
                _ => panic!("gateway returned the wrong provider response variant"),
            }
        }
    }

    #[test]
    fn transport_configuration_accepts_loopback_and_https_gateways_only() {
        assert!(
            ProviderTransportConfig::new(
                "http://127.0.0.1:8787/provider",
                "local-model",
                32,
                "gateway-token",
            )
            .validate()
            .is_ok()
        );
        assert!(
            ProviderTransportConfig::new(
                "https://gateway.example.test/provider",
                "team-model",
                32,
                "gateway-token",
            )
            .validate()
            .is_ok()
        );
        assert!(
            ProviderTransportConfig::new("http://provider.example.test/v1", "model", 32, "token",)
                .validate()
                .is_err()
        );
    }

    #[test]
    fn sse_decoder_handles_network_chunk_boundaries_and_multiline_data() {
        let events = decode_sse_chunks([
            Ok("id: evt-1\nevent: content_block_delta\nda".to_owned()),
            Ok("ta: {\"type\":\"content_block_delta\",\n".to_owned()),
            Ok("data: \"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n".to_owned()),
        ])
        .expect("valid SSE");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id.as_deref(), Some("evt-1"));
        assert_eq!(events[0].event.as_deref(), Some("content_block_delta"));
        assert_eq!(
            events[0].data,
            "{\"type\":\"content_block_delta\",\n\"delta\":{".to_owned()
                + "\"type\":\"text_delta\",\"text\":\"hi\"}}"
        );
    }

    #[test]
    fn sse_decoder_bounds_unterminated_lines_and_complete_frames() {
        let oversized_line = "x".repeat(MAX_PROVIDER_SSE_FRAME + 1);
        assert_eq!(
            decode_sse_chunks([Ok(oversized_line)]),
            Err(TransportError::Failed)
        );

        let oversized_frame = format!("data: {}\n\n", "x".repeat(MAX_PROVIDER_SSE_FRAME));
        assert_eq!(
            decode_sse_chunks([Ok(oversized_frame)]),
            Err(TransportError::Failed)
        );
    }

    #[test]
    fn tool_block_start_rejects_nonempty_initial_input() {
        let event = ProviderSseEvent::new(
            Some("content_block_start".to_owned()),
            None,
            serde_json::json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": {
                    "type": "tool_use",
                    "id": "tool-1",
                    "name": "lookup",
                    "input": {"email": "bob@example.com"}
                }
            })
            .to_string(),
        );
        assert_eq!(
            validate_claude_sse_event(&event),
            Err(TransportError::Failed)
        );
    }

    #[test]
    fn sse_decoder_preserves_ping_and_fails_closed_on_error_event() {
        let events =
            decode_sse_chunks([Ok("event: ping\ndata: {\"type\":\"ping\"}\n\n".to_owned())])
                .expect("ping is valid");
        assert_eq!(events[0].event.as_deref(), Some("ping"));
        assert!(validate_claude_sse_events(&events).is_ok());

        let error =
            decode_sse_chunks([Ok("event: error\ndata: {\"type\":\"error\"}\n\n".to_owned())])
                .expect("frame parses");
        assert_eq!(
            validate_claude_sse_events(&error),
            Err(TransportError::Failed)
        );
    }

    #[test]
    fn sse_decoder_rejects_truncated_event_at_eof() {
        assert_eq!(
            decode_sse_chunks([Ok("event: ping\ndata: {\"type\":\"ping\"}".to_owned())]),
            Err(TransportError::Failed)
        );
    }

    #[test]
    fn claude_message_frames_reject_invalid_stop_sequence_types() {
        let mut start: Value = serde_json::from_str(
            message_start()
                .split_once("data: ")
                .expect("message start data")
                .1
                .trim(),
        )
        .expect("valid message start JSON");
        *start
            .pointer_mut("/message/stop_sequence")
            .expect("start stop_sequence") = Value::String("unexpected".to_owned());
        let start_event =
            ProviderSseEvent::new(Some("message_start".to_owned()), None, start.to_string());
        assert_eq!(
            validate_claude_sse_event(&start_event),
            Err(TransportError::Failed)
        );

        let delta_event = ProviderSseEvent::new(
            Some("message_delta".to_owned()),
            None,
            serde_json::json!({
                "type": "message_delta",
                "delta": {"stop_reason": "end_turn", "stop_sequence": 7},
                "usage": {"output_tokens": 2}
            })
            .to_string(),
        );
        assert_eq!(
            validate_claude_sse_event(&delta_event),
            Err(TransportError::Failed)
        );
    }

    #[test]
    fn response_redaction_buffers_deltas_so_split_pii_never_reaches_callback() {
        let mut events = decode_sse_chunks([Ok(message_start())]).expect("message start");
        events.push(ProviderSseEvent::new(
            Some("content_block_start".into()),
            None,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#
                .into(),
        ));
        events.extend(vec![
            ProviderSseEvent::new(Some("content_block_delta".into()), Some("1".into()),
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"<per"}}"#.into()),
            ProviderSseEvent::new(Some("content_block_delta".into()), Some("2".into()),
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"son-fixture>"}}"#.into()),
        ]);
        events.push(ProviderSseEvent::new(
            Some("content_block_stop".into()),
            None,
            r#"{"type":"content_block_stop","index":0}"#.into(),
        ));
        events.extend(decode_sse_chunks([Ok(message_end())]).expect("message end"));
        let safe = redact_claude_sse_events(&TestRedactor, events).expect("redacts joined text");
        let safe_deltas: Vec<_> = safe.iter().filter(|event| event.id.is_some()).collect();
        let values: Vec<Value> = safe_deltas
            .iter()
            .map(|event| serde_json::from_str(&event.data).unwrap())
            .collect();
        let joined = values
            .iter()
            .filter_map(|value| value.pointer("/delta/text").and_then(Value::as_str))
            .collect::<String>();
        assert!(!joined.contains(PERSON));
        assert!(joined.contains(PERSON_TOKEN));
        assert_eq!(safe_deltas[0].id.as_deref(), Some("1"));
        assert_eq!(safe_deltas[1].id.as_deref(), Some("2"));
    }

    #[test]
    fn public_claude_event_api_reads_network_chunks_and_delivers_only_redacted_events() {
        let wire = message_start()
            + "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n"
            + "event: content_block_delta\nid: first\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"<per\"}}\n\n"
            + "event: ping\nid: heartbeat\ndata: {\"type\":\"ping\"}\n\n"
            + "event: content_block_delta\nid: second\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"son-fixture>\"}}\n\n"
            + "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n"
            + "event: error\nid: failure\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"<person-fixture>\"}}\n\n"
            + &message_end();
        let chunks = wire.as_bytes().chunks(13).map(<[u8]>::to_vec).collect();
        let mut transport = AnthropicProviderTransport::new(
            ChunkedHttpFixture { chunks },
            ProviderTransportConfig::new(
                "https://gateway.invalid/v1/messages",
                "claude",
                32,
                "key",
            ),
        );
        let mut delivered = Vec::new();
        let metadata = transport
            .collect_redacted_events(
                &redacted_request(ProviderKind::ClaudeDesktop),
                &TestRedactor,
                |event| delivered.push(event),
            )
            .expect("chunked SSE response succeeds");
        assert_eq!(metadata.status, 200);
        assert_eq!(delivered.len(), 9);
        // A content-free keepalive is relayed while the block's text is still
        // buffered; the split-PII deltas are released together at the terminal
        // boundary, before the error and final stop frames.
        assert_eq!(delivered[2].event.as_deref(), Some("ping"));
        assert_eq!(delivered[2].id.as_deref(), Some("heartbeat"));
        assert_eq!(delivered[3].id.as_deref(), Some("first"));
        assert_eq!(delivered[4].id.as_deref(), Some("second"));
        assert_eq!(delivered[6].event.as_deref(), Some("error"));
        let joined = delivered
            .iter()
            .filter_map(|event| {
                let value = serde_json::from_str::<Value>(&event.data).ok()?;
                value
                    .pointer("/delta/text")
                    .or_else(|| value.pointer("/error/message"))?
                    .as_str()
                    .map(str::to_owned)
            })
            .collect::<String>();
        assert!(!joined.contains(PERSON));
        assert!(joined.contains(PERSON_TOKEN));
    }

    #[test]
    fn public_claude_event_api_rejects_clean_eof_without_terminal_event() {
        let wire = "event: ping\ndata: {\"type\":\"ping\"}\n\n";
        let mut transport = AnthropicProviderTransport::new(
            ChunkedHttpFixture {
                chunks: vec![wire.as_bytes().to_vec()],
            },
            ProviderTransportConfig::new("https://gateway.invalid/messages", "claude", 32, "key"),
        );
        let mut delivered = Vec::new();
        assert_eq!(
            transport.collect_redacted_events(
                &redacted_request(ProviderKind::ClaudeDesktop),
                &TestRedactor,
                |event| delivered.push(event),
            ),
            Err(TransportError::Failed)
        );
        // The content-free ping is safe to relay immediately; the absence of a
        // terminal frame is still refused rather than treated as a clean end.
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].event.as_deref(), Some("ping"));
    }

    /// Captures what the callback has observed after each provider chunk so a
    /// test can prove that text is withheld until its block is redacted.
    #[derive(Clone, Default)]
    struct ObservingFixture {
        chunks: Vec<Vec<u8>>,
        seen: Arc<Mutex<Vec<ProviderSseEvent>>>,
        snapshots: Arc<Mutex<Vec<usize>>>,
    }

    impl ProviderHttpClient for ObservingFixture {
        fn send(
            &mut self,
            _request: ProviderHttpRequest,
        ) -> Result<ProviderHttpResponse, TransportError> {
            Err(TransportError::Failed)
        }

        fn send_stream(
            &mut self,
            _request: ProviderHttpRequest,
            on_response: &mut dyn FnMut(u16, Vec<(String, String)>),
            on_chunk: &mut dyn FnMut(&[u8]) -> Result<(), TransportError>,
        ) -> Result<(), TransportError> {
            on_response(
                200,
                vec![("content-type".into(), "text/event-stream".into())],
            );
            for chunk in &self.chunks {
                on_chunk(chunk)?;
                self.snapshots
                    .lock()
                    .expect("snapshot lock")
                    .push(self.seen.lock().expect("seen lock").len());
            }
            Ok(())
        }
    }

    fn message_start() -> String {
        concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{",
            "\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",",
            "\"model\":\"claude\",\"content\":[],\"stop_reason\":null,",
            "\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n"
        )
        .to_owned()
    }

    fn message_end() -> String {
        concat!(
            "event: message_delta\ndata: {\"type\":\"message_delta\",",
            "\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},",
            "\"usage\":{\"output_tokens\":2}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        )
        .to_owned()
    }

    #[test]
    fn claude_events_stream_early_but_hold_split_pii_until_block_stop() {
        let first = message_start()
            + ": ping - 1763990000\n\n"
            + "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n"
            + "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello <per\"}}\n\n";
        let second = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"son-fixture>\"}}\n\n".to_owned()
            + "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n"
            + &message_end();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let snapshots = Arc::new(Mutex::new(Vec::new()));
        let mut transport = AnthropicProviderTransport::new(
            ObservingFixture {
                chunks: vec![first.into_bytes(), second.into_bytes()],
                seen: Arc::clone(&seen),
                snapshots: Arc::clone(&snapshots),
            },
            ProviderTransportConfig::new(
                "https://gateway.invalid/v1/messages",
                "claude",
                32,
                "key",
            ),
        );
        {
            let sink = Arc::clone(&seen);
            transport
                .collect_redacted_events(
                    &redacted_request(ProviderKind::ClaudeDesktop),
                    &TestRedactor,
                    |event| sink.lock().expect("callback lock").push(event),
                )
                .expect("streamed Claude response");
        }
        let snapshots = snapshots.lock().expect("snapshots lock").clone();
        // message_start, keepalive and block start are live; the first delta is
        // not released even though its bytes already arrived on the wire.
        assert_eq!(snapshots, vec![3, 8]);
        let delivered = seen.lock().expect("seen lock").clone();
        assert_eq!(delivered.len(), 8);
        assert_eq!(delivered[1].comment.as_deref(), Some("ping"));
        let joined: String = delivered
            .iter()
            .filter_map(|event| {
                let value = serde_json::from_str::<Value>(&event.data).ok()?;
                value
                    .pointer("/delta/text")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        assert!(!joined.contains(PERSON));
        assert!(joined.contains(PERSON_TOKEN));
        assert_eq!(delivered[6].event.as_deref(), Some("message_delta"));
        assert_eq!(delivered[7].event.as_deref(), Some("message_stop"));
    }

    #[test]
    fn claude_tool_and_thinking_streams_are_redacted_per_block() {
        let wire = message_start()
            + "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"tool-1\",\"name\":\"lookup\",\"input\":{}}}\n\n"
            + "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"email\\\":\\\"<per\"}}\n\n"
            + ": ping\n\n"
            + "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"son-fixture>\\\"}\"}}\n\n"
            + "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n"
            + "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\n"
            + "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"note <per\"}}\n\n"
            + "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"c2ln\"}}\n\n"
            + "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"son-fixture>\"}}\n\n"
            + "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n"
            + &message_end();
        let mut transport = AnthropicProviderTransport::new(
            ChunkedHttpFixture {
                chunks: wire.as_bytes().chunks(17).map(<[u8]>::to_vec).collect(),
            },
            ProviderTransportConfig::new(
                "https://gateway.invalid/v1/messages",
                "claude",
                32,
                "key",
            ),
        );
        let mut delivered = Vec::new();
        transport
            .collect_redacted_events(
                &redacted_request(ProviderKind::ClaudeDesktop),
                &TestRedactor,
                |event| delivered.push(event),
            )
            .expect("tool and thinking stream succeeds");
        let serialized = delivered
            .iter()
            .map(|event| event.data.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!serialized.contains(PERSON));
        let joined = |pointer: &str| -> String {
            delivered
                .iter()
                .filter_map(|event| {
                    let value = serde_json::from_str::<Value>(&event.data).ok()?;
                    value
                        .pointer(pointer)
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .collect()
        };
        let partial_json = joined("/delta/partial_json");
        assert!(is_json_object(&partial_json));
        assert!(partial_json.contains(PERSON_TOKEN));
        assert!(joined("/delta/thinking").contains(PERSON_TOKEN));
        assert!(!delivered.iter().any(|event| {
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

    #[test]
    fn claude_tool_arguments_redact_unicode_escaped_strings_after_json_decode() {
        let wire = message_start()
            + "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"tool-1\",\"name\":\"lookup\",\"input\":{}}}\n\n"
            + "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"email\\\":\\\"\\\\u003cemail-fixture\\\\u003e\\\"}\"}}\n\n"
            + "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n"
            + &message_end();
        let events = decode_sse_chunks([Ok(wire)]).expect("valid tool stream");
        let safe = redact_claude_sse_events(&TestRedactor, events).expect("redacted tool stream");
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
        let decoded: Value = serde_json::from_str(&arguments).expect("valid JSON arguments");
        assert!(!decoded.to_string().contains(EMAIL));
        assert!(decoded.to_string().contains("[EMAIL:opaque-2]"));
    }

    #[test]
    fn claude_unknown_event_fails_closed_before_buffered_text_is_released() {
        let wire = message_start()
            + "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n"
            + "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"<person-fixture>\"}}\n\n"
            + "event: future_debug\ndata: {\"type\":\"future_debug\",\"text\":\"<person-fixture>\"}\n\n"
            + "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n"
            + &message_end();
        let mut transport = AnthropicProviderTransport::new(
            ChunkedHttpFixture {
                chunks: vec![wire.into_bytes()],
            },
            ProviderTransportConfig::new(
                "https://gateway.invalid/v1/messages",
                "claude",
                32,
                "key",
            ),
        );
        let mut delivered = Vec::new();
        assert_eq!(
            transport.collect_redacted_events(
                &redacted_request(ProviderKind::ClaudeDesktop),
                &TestRedactor,
                |event| delivered.push(event),
            ),
            Err(TransportError::Failed)
        );
        assert!(delivered.iter().all(|event| !event.data.contains(PERSON)));
        assert!(
            delivered
                .iter()
                .all(|event| event.event.as_deref() != Some("content_block_delta"))
        );
    }

    #[derive(Clone)]
    struct ChunkedHttpFixture {
        chunks: Vec<Vec<u8>>,
    }

    impl ProviderHttpClient for ChunkedHttpFixture {
        fn send(
            &mut self,
            _request: ProviderHttpRequest,
        ) -> Result<ProviderHttpResponse, TransportError> {
            Err(TransportError::Failed)
        }

        fn send_stream(
            &mut self,
            _request: ProviderHttpRequest,
            on_response: &mut dyn FnMut(u16, Vec<(String, String)>),
            on_chunk: &mut dyn FnMut(&[u8]) -> Result<(), TransportError>,
        ) -> Result<(), TransportError> {
            on_response(
                200,
                vec![("content-type".into(), "text/event-stream".into())],
            );
            for chunk in &self.chunks {
                on_chunk(chunk)?;
            }
            Ok(())
        }
    }

    #[test]
    fn transport_configuration_fails_closed_when_required_values_are_missing() {
        assert!(
            ProviderTransportConfig::new("http://127.0.0.1:8787", "", 32, "token")
                .validate()
                .is_err()
        );
        assert!(
            ProviderTransportConfig::new("http://127.0.0.1:8787", "model", 0, "token")
                .validate()
                .is_err()
        );
        assert!(
            ProviderTransportConfig::new("http://127.0.0.1:8787", "model", 32, "")
                .validate()
                .is_err()
        );
    }
}
