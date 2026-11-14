//! Provider gateway seam for the local secrecy client.
//!
//! The public interface deliberately has two kinds of values: raw request values are
//! accepted only at the redaction seam, while provider transport receives
//! [`RedactedProviderRequest`] and returns token-only chunks. The client owns neither
//! a log sink nor durable transcript storage, so raw values have no persistence path.

use std::fmt;

use serde::{Deserialize, Serialize};
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
#[derive(Clone, PartialEq, Eq)]
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
#[derive(Clone, PartialEq, Eq)]
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
#[derive(Clone, PartialEq, Eq)]
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
#[derive(Clone, PartialEq, Eq)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum RedactionError {
    #[error("redaction adapter rejected the request")]
    Rejected,
}

/// Provider transport failures are intentionally content-free and safe to log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum TransportError {
    #[error("provider transport failed")]
    Failed,
}

/// Human display rehydration failures are intentionally content-free and safe to log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum RehydrationError {
    #[error("display rehydration failed")]
    Failed,
}

/// Errors from the small client interface. No variant contains user content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
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
}

/// Adapter seam for provider streaming. It receives token-only data only.
pub trait ProviderTransport {
    /// Stream a complete set of chunks while preserving opaque references.
    fn stream(
        &mut self,
        request: &RedactedProviderRequest,
    ) -> Result<ProviderResponse, TransportError>;
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
        self.transport
            .stream(&safe_request)
            .map_err(ClientError::from)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
