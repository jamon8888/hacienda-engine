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
    /// OpenAI Codex-compatible transport.
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
    fn redact(&self, request: &ProviderRequest) -> Result<RedactedProviderRequest, RedactionError>;

    /// Redact provider output before it is exposed to the renderer.
    ///
    /// Implementations must use the same local Hacienda policy as [`Self::redact`].
    /// The default rejects the operation so an adapter that has not implemented the
    /// response boundary cannot accidentally return raw provider text.
    fn redact_text(&self, _text: &str) -> Result<TokenizedText, RedactionError> {
        Err(RedactionError::Rejected)
    }
}

/// Adapter seam for provider streaming. It receives token-only data only.
pub trait ProviderTransport {
    /// Stream a complete set of chunks while preserving opaque references.
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
    fn send(
        &mut self,
        request: ProviderHttpRequest,
    ) -> Result<ProviderHttpResponse, TransportError>;
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
            response,
            ProviderKind::ClaudeDesktop,
            anthropic_text,
        )
    }
}

/// OpenAI Chat Completions-compatible adapter for Codex transports.
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
        map_provider_response(request, response, ProviderKind::Codex, openai_text)
    }
}

/// Real blocking HTTP implementation used by the desktop process.
#[derive(Clone)]
pub struct ReqwestHttpClient {
    client: reqwest::blocking::Client,
}

impl ReqwestHttpClient {
    /// Build the native HTTP client with a bounded request timeout.
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
}

/// Configured gateway transport that selects the provider wire adapter at runtime.
pub struct GatewayProviderTransport {
    http: ReqwestHttpClient,
    config: ProviderTransportConfig,
}

impl GatewayProviderTransport {
    /// Construct and validate a real gateway transport.
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
    response: ProviderHttpResponse,
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
    let redact = |mut chunk: ProviderStreamChunk| {
        let original_references = chunk.content.references.clone();
        let mut content = redactor.redact_text(&chunk.content.text)?;
        for reference in original_references {
            if !content.references.contains(&reference) {
                content.references.push(reference);
            }
        }
        chunk.content = content;
        Ok(chunk)
    };
    Ok(match response {
        ProviderResponse::ClaudeDesktop(mut response) => {
            response.chunks = response
                .chunks
                .into_iter()
                .map(redact)
                .collect::<Result<_, _>>()?;
            ProviderResponse::ClaudeDesktop(response)
        }
        ProviderResponse::Codex(mut response) => {
            response.chunks = response
                .chunks
                .into_iter()
                .map(redact)
                .collect::<Result<_, _>>()?;
            ProviderResponse::Codex(response)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

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
