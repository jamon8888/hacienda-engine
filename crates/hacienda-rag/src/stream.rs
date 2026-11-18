//! Streaming answer and progress primitives over `liter-llm`.
//!
//! Recovered from `77e2fd3d71^:crates/xberg-rag/src/stream.rs` (xberg, MIT) — this
//! module was deliberately left unrecovered when the rest of the crate was ported
//! (see `lib.rs`'s provenance doc comment) pending a design pass; Phase 12 Track 3
//! is that design pass. Adapted only for this crate's error type (`RagError`,
//! already carrying a `Backend(Box<dyn Error + Send + Sync>)` variant identical in
//! shape to the original) and its `RetrievedChunk`/`ChunkId`/`DocumentId` types,
//! which are unchanged from the original crate.
//!
//! ## Why a direct `liter-llm` dependency, not xberg's `llm` module
//!
//! xberg's own `llm` module (`xberg::llm::text_completion::complete_text` and
//! `complete_with_json_schema`) is whole-response only — neither function exposes
//! a token stream, and the module's `create_client` is `pub(crate)`, so a
//! streaming call cannot be reached through xberg at all (verified directly
//! against xberg v1.0.2's source: `crates/xberg/src/llm/{client,text_completion,structured}.rs`).
//! `liter-llm` itself does expose real token streaming via
//! [`liter_llm::LlmClient::chat_stream`], which is what this module calls
//! directly — the same approach the original `xberg-rag` crate took. This
//! crate's `streaming` feature therefore depends on `liter-llm` directly and does
//! **not** enable xberg's own `liter-llm` feature (see the `liter-llm` workspace
//! dependency's doc comment in the workspace root `Cargo.toml`): that feature
//! additionally pulls in `ocr-pipeline`, for no benefit here. `xberg::LlmConfig`
//! is plain configuration data with no feature gate, so it's used as this
//! module's config type without pulling in any of xberg's own LLM plumbing.
//!
//! The core entry-point is [`answer_stream`], which:
//! 1. Emits [`AnswerEvent::Citation`] events synchronously for every
//!    [`RetrievedChunk`](crate::types::RetrievedChunk) that carries content.
//! 2. Connects to the LLM via liter-llm and maps token chunks to
//!    [`AnswerEvent::Token`] events.
//! 3. Emits a final [`AnswerEvent::Usage`] event when the provider reports
//!    token counts.
//! 4. Closes with [`AnswerEvent::Done`].
//!
//! [`map_tokens`] is exposed publicly so callers can test the chunk-to-event
//! mapping with a synthetic source without a live network connection.

use std::pin::Pin;
use std::sync::Arc;

use futures::{stream, Stream, StreamExt, TryStreamExt};
use serde::Serialize;

use crate::error::{RagError, RagResult};
use crate::types::{ChunkId, DocumentId, RetrievedChunk};

/// A single event emitted by the answer stream.
#[derive(Debug, Clone, PartialEq)]
pub enum AnswerEvent {
    /// A token fragment from the LLM.
    Token(String),
    /// A source citation from the retrieved context.
    Citation(Citation),
    /// Token usage reported by the provider (typically in the final chunk).
    Usage(TokenUsage),
    /// The stream has finished successfully.
    Done,
    /// A non-fatal error was encountered and surfaced as an event.
    Error(String),
}

/// A source citation, linking back to the chunk that contributed context.
///
/// `Serialize` (only, no `Deserialize`) — this type crosses the process boundary
/// as an outgoing SSE `citation` event (`hacienda-api`'s `rag_stream::answer`
/// handler), never as request input.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Citation {
    /// Parent document identifier.
    pub document_id: DocumentId,
    /// Chunk identifier.
    pub chunk_id: ChunkId,
    /// Position of the chunk within its document.
    pub ordinal: u32,
}

/// Token usage reported by the provider.
///
/// `Serialize` for the same reason as [`Citation`]: it crosses the process
/// boundary as an outgoing SSE `usage` event.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TokenUsage {
    /// Tokens consumed by the prompt (including context).
    pub prompt_tokens: u32,
    /// Tokens generated in the completion.
    pub completion_tokens: u32,
}

/// The context passed to the LLM: an ordered list of retrieved chunks.
#[derive(Debug, Clone)]
pub struct RetrievedContext {
    /// Chunks in descending relevance order.
    pub chunks: Vec<RetrievedChunk>,
}

/// Configuration for the LLM used by [`answer_stream`].
#[derive(Debug, Clone, Default)]
pub struct LlmAnswerConfig {
    /// LLM client configuration (model, API key, temperature, …).
    pub llm: xberg::LlmConfig,
    /// Optional system-message override. When `Some`, it is used verbatim as the
    /// system prompt and the engine performs no context injection — the caller
    /// owns prompt composition entirely (this is the seam the product layer uses
    /// for its curated prompts, e.g. injecting an already-redacted context block
    /// under its own framing). When `None`, a minimal generic system framing
    /// that injects the retrieved context is used.
    pub system_prompt: Option<String>,
    /// Optional egress allowlist for the LLM base URL.
    ///
    /// When `Some` (strict egress mode), [`build_token_stream`] refuses to open
    /// the transport unless `config.llm.base_url` is present **and** appears in
    /// this list — fail-closed: an absent or unauthorized destination is
    /// rejected before any connection is attempted. When `None`, no egress
    /// restriction is applied (the caller's existing behaviour).
    pub egress_allowlist: Option<Vec<String>>,
}

/// Redacts text before it is sent to an LLM.
///
/// Implemented by the caller — this crate has no PII-detection code of its own and no
/// dependency on `hacienda-core` (`hacienda-core` cannot depend on `hacienda-rag`: see
/// `hacienda-rag/Cargo.toml`'s dev-dependency comment on that edge, and `liter-llm` lives
/// only here, not in `hacienda-core`, so the guard has to live on this side of that
/// boundary). `hacienda-api` implements this over `HaciendaFacade::redact_text_with_auth`.
///
/// This is `2026-08-13-P6-llm-call-enforcement-point.md`'s guard, applied at the one seam
/// this crate controls: [`GuardedLlm`] is the only public way in this crate to reach
/// [`answer_stream`] with unredacted input, the same way [`answer_stream`] itself is the
/// only public way to reach `liter_llm::LlmClient::chat_stream` (`build_token_stream` below
/// is private). A future caller that skips `GuardedLlm` and calls `answer_stream` directly
/// still has to redact by hand — same as today — but every consumer that has no reason to
/// bypass it gets the guarantee by construction instead of by remembering to copy
/// `rag_stream.rs`'s pattern.
#[async_trait::async_trait]
pub trait Redactor: Send + Sync {
    /// Detect and redact PII in `text`, returning the redacted string.
    async fn redact(&self, text: &str) -> RagResult<String>;
}

/// Which part of an outgoing LLM request an [`EgressPart`] describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressRole {
    /// The `system_prompt` override (redacted before dispatch — the É7 leak of
    /// wayfinder #131, closed by redacting it in [`GuardedLlm::stream_with_purpose`]).
    SystemPrompt,
    /// The user prompt (redacted before dispatch).
    Prompt,
    /// One retrieved chunk's content (redacted before dispatch).
    ContextChunk,
}

/// One transmitted span of an LLM request, recorded as **hash + reference only** —
/// never raw content (issue #148, P6 §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EgressPart {
    pub role: EgressRole,
    /// Stable reference identifying the part: `system_prompt`, `prompt`, or
    /// `chunk:<document_id>:<chunk_id>:<ordinal>`.
    pub reference: String,
    /// blake3 hex digest of the exact (redacted) bytes transmitted.
    pub hash: String,
    /// Byte length of the transmitted span.
    pub length: u32,
}

/// Structured record of one LLM dispatch: what was sent (hashes + references), to
/// which provider, for which purpose. Carries no raw content, so it is safe to
/// persist in the audit chain and to log.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LlmEgressEvent {
    /// What the call was for, e.g. `"rag-answer"`.
    pub purpose: String,
    /// Provider model that received the payload.
    pub provider_model: String,
    /// Provider base URL, when configured.
    pub provider_base_url: Option<String>,
    /// blake3 hex digest of the full outgoing request payload (system + user messages).
    pub request_hash: String,
    /// Byte length of that payload.
    pub request_length: u32,
    /// Per-part hashes + references.
    pub parts: Vec<EgressPart>,
}

/// Deferred auditor for the P6 "content sent to an LLM" record (issue #148).
///
/// This crate has no audit store of its own (and no dependency on `hacienda-core`,
/// same boundary as [`Redactor`]), so the audit sink is injected the same way the
/// redactor is: `hacienda-api` implements this over
/// `HaciendaFacade::record_llm_egress_with_auth`. [`GuardedLlm`] invokes it from a
/// spawned task — never inline — so the audit write cannot slow the stream down,
/// and the event fires even when the stream itself later fails.
#[async_trait::async_trait]
pub trait LlmEgressAuditor: Send + Sync {
    /// Record one LLM dispatch. Called off the streaming hot path; an error is
    /// logged by the caller, never propagated into the answer stream.
    async fn record_egress(&self, event: LlmEgressEvent) -> RagResult<()>;
}

/// blake3 hex digest of `bytes` — the digest scheme the audit chain already uses
/// for `span_hash` (`hacienda-core`'s `audit` module), so an egress entry can be
/// joined to a redaction entry by digest where the two cover the same text.
fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn byte_len(text: &str) -> u32 {
    u32::try_from(text.len()).unwrap_or(u32::MAX)
}

/// P6's LLM-call enforcement point: redacts `prompt` and every chunk in `context` through
/// an injected [`Redactor`] *before* handing either to [`answer_stream`].
///
/// Unlike [`answer_stream`], which documents "callers must redact... this module performs
/// no redaction itself," `GuardedLlm` performs the redaction itself — there is no discipline
/// left for a caller to get wrong beyond constructing this type with a real `Redactor`
/// instead of skipping it.
pub struct GuardedLlm<'a> {
    redactor: &'a (dyn Redactor + Sync),
    auditor: Option<Arc<dyn LlmEgressAuditor>>,
}

impl<'a> GuardedLlm<'a> {
    /// Wrap `redactor` as the mandatory redaction step for every call this guard makes.
    ///
    /// No egress auditor is attached: calls are redacted but not audited. Prefer
    /// [`GuardedLlm::with_auditor`] for any production call site.
    pub fn new(redactor: &'a (dyn Redactor + Sync)) -> Self {
        Self {
            redactor,
            auditor: None,
        }
    }

    /// Wrap `redactor` as the mandatory redaction step **and** attach `auditor` as the
    /// deferred P6 egress audit (issue #148): every call this guard makes records one
    /// [`LlmEgressEvent`] (hashes + references of the redacted payload, provider, purpose)
    /// through `auditor`, from a spawned task so the audit never blocks the stream.
    pub fn with_auditor(
        redactor: &'a (dyn Redactor + Sync),
        auditor: Arc<dyn LlmEgressAuditor>,
    ) -> Self {
        Self {
            redactor,
            auditor: Some(auditor),
        }
    }

    /// Redact `prompt` and every chunk's content in `context`, then stream an answer
    /// exactly as [`answer_stream`] would from the redacted versions.
    ///
    /// Equivalent to [`GuardedLlm::stream_with_purpose`] with a generic `"llm-call"`
    /// audit purpose.
    pub async fn stream(
        &self,
        context: RetrievedContext,
        prompt: String,
        config: LlmAnswerConfig,
    ) -> RagResult<impl Stream<Item = RagResult<AnswerEvent>> + Send + 'static> {
        self.stream_with_purpose(context, prompt, config, "llm-call")
            .await
    }

    /// Redact `prompt` and every chunk's content in `context`, then stream an answer
    /// exactly as [`answer_stream`] would from the redacted versions.
    ///
    /// Redaction happens here, before the returned stream is even constructed — a
    /// redaction failure is a `RagResult::Err` returned directly from this `async fn`, not
    /// an error surfacing mid-stream. This matters to callers that report pre-stream
    /// failures differently from in-stream ones (`hacienda-api`'s `rag_stream::answer`
    /// handler is exactly such a caller: see its module doc on why SSE errors can't reuse
    /// its normal error type once the stream has started).
    ///
    /// # Scope
    ///
    /// Only `prompt` and `RetrievedChunk::content` are redacted. Every other chunk field
    /// (`chunk_metadata`, `external_id`, `document`) is forwarded unchanged, because
    /// [`answer_stream`] builds the LLM's context from `content` alone — this is safe today
    /// only because of that fact about a *different* function. If `answer_stream` ever
    /// starts injecting another field into the prompt or system message, this method must
    /// redact that field too, or the guarantee this type exists for quietly stops holding.
    ///
    /// The `system_prompt` override, when present, is redacted through the same injected
    /// [`Redactor`] before it ever reaches the LLM transport — a redaction failure fails
    /// the whole call (fail-closed): the unredacted system prompt must never be sent to
    /// the provider.
    ///
    /// Egress is fail-closed too: when `config.egress_allowlist` is `Some` (strict mode),
    /// `config.llm.base_url` must be present and listed, or the call fails before any
    /// connection is attempted.
    ///
    /// # Deferred audit (P6, issue #148)
    ///
    /// When an auditor was attached via [`GuardedLlm::with_auditor`], one
    /// [`LlmEgressEvent`] is built from the **redacted** values — the exact strings handed
    /// to [`answer_stream`] — and recorded through the auditor from a spawned task. The
    /// spawn happens before the stream is returned, so the audit fires even if the stream
    /// later fails, and the audit write never blocks stream construction or token
    /// delivery. `purpose` names the calling capability (e.g. `"rag-answer"`) in the
    /// audit record.
    pub async fn stream_with_purpose(
        &self,
        context: RetrievedContext,
        prompt: String,
        mut config: LlmAnswerConfig,
        purpose: impl Into<String>,
    ) -> RagResult<impl Stream<Item = RagResult<AnswerEvent>> + Send + 'static> {
        // Redact the system prompt override, if any, before it reaches the transport.
        // A redaction failure is fail-closed: the unredacted system prompt never reaches
        // the LLM, and no stream is built.
        if let Some(system) = config.system_prompt.take() {
            config.system_prompt = Some(self.redactor.redact(&system).await?);
        }

        // Fail-closed egress policy: an absent or unauthorized base_url is rejected
        // before the transport is ever constructed.
        validate_egress(&config)?;

        let redacted_prompt = self.redactor.redact(&prompt).await?;
        let mut redacted_chunks = Vec::with_capacity(context.chunks.len());
        for mut chunk in context.chunks {
            if let Some(content) = chunk.content.take() {
                chunk.content = Some(self.redactor.redact(&content).await?);
            }
            redacted_chunks.push(chunk);
        }
        let redacted_context = RetrievedContext {
            chunks: redacted_chunks,
        };

        // Deferred P6 egress audit (issue #148): hash the exact redacted payload that
        // is about to be dispatched, and record it from a spawned task so the audit
        // write never blocks the streaming hot path — and still fires if the stream
        // below later fails.
        if let Some(auditor) = &self.auditor {
            let event = build_egress_event(&config, &redacted_prompt, &redacted_context, purpose);
            let auditor = Arc::clone(auditor);
            tokio::spawn(async move {
                if let Err(err) = auditor.record_egress(event).await {
                    tracing::warn!(error = %err, "llm egress audit write failed");
                }
            });
        }

        Ok(answer_stream(redacted_context, redacted_prompt, config))
    }
}

/// Build the [`LlmEgressEvent`] for one dispatch from the **redacted** values that are
/// about to be handed to [`answer_stream`] — the same strings [`build_token_stream`]
/// will place into the outgoing request, hashed here via [`context_text_of`] /
/// [`system_content_of`] so the digests cannot drift from what is actually sent.
fn build_egress_event(
    config: &LlmAnswerConfig,
    prompt: &str,
    context: &RetrievedContext,
    purpose: impl Into<String>,
) -> LlmEgressEvent {
    let context_text = context_text_of(&context.chunks);
    let system_content = system_content_of(config, &context_text);

    let mut parts = Vec::with_capacity(context.chunks.len() + 2);
    if let Some(system) = &config.system_prompt {
        parts.push(EgressPart {
            role: EgressRole::SystemPrompt,
            reference: "system_prompt".to_string(),
            hash: blake3_hex(system.as_bytes()),
            length: byte_len(system),
        });
    }
    parts.push(EgressPart {
        role: EgressRole::Prompt,
        reference: "prompt".to_string(),
        hash: blake3_hex(prompt.as_bytes()),
        length: byte_len(prompt),
    });
    for chunk in &context.chunks {
        if let Some(content) = &chunk.content {
            parts.push(EgressPart {
                role: EgressRole::ContextChunk,
                reference: format!(
                    "chunk:{}:{}:{}",
                    chunk.document_id.0, chunk.id.0, chunk.ordinal
                ),
                hash: blake3_hex(content.as_bytes()),
                length: byte_len(content),
            });
        }
    }

    let payload = serde_json::json!({ "system": system_content, "user": prompt });
    let payload_bytes = serde_json::to_vec(&payload).unwrap_or_default();

    LlmEgressEvent {
        purpose: purpose.into(),
        provider_model: config.llm.model.clone(),
        provider_base_url: config.llm.base_url.clone(),
        request_hash: blake3_hex(&payload_bytes),
        request_length: u32::try_from(payload_bytes.len()).unwrap_or(u32::MAX),
        parts,
    }
}

/// Fail-closed egress policy for the LLM transport.
///
/// When `config.egress_allowlist` is `Some` (strict mode), `config.llm.base_url` must be
/// present and appear in the list. Returns an [`RagError::InvalidQuery`] otherwise — the
/// transport is never built with an absent or unauthorized destination.
fn validate_egress(config: &LlmAnswerConfig) -> RagResult<()> {
    let Some(allowed) = &config.egress_allowlist else {
        return Ok(()); // egress not restricted
    };
    let Some(url) = &config.llm.base_url else {
        return Err(RagError::InvalidQuery(
            "strict egress mode requires a base_url".to_string(),
        ));
    };
    if !allowed.iter().any(|a| a == url) {
        return Err(RagError::InvalidQuery(format!(
            "base_url not in egress allowlist: {url}"
        )));
    }
    Ok(())
}

/// Stream an answer grounded in `context` for the given `prompt`.
///
/// Events arrive in this order:
/// 1. [`AnswerEvent::Citation`] — one per chunk in `context` that has content,
///    emitted synchronously before the LLM connection is opened.
/// 2. [`AnswerEvent::Token`] — one per non-empty content fragment.
/// 3. [`AnswerEvent::Usage`] — token usage if reported by the provider.
/// 4. [`AnswerEvent::Done`] — always the last event.
///
/// The function is **not** `async`; it returns a lazy stream that performs I/O
/// only when polled.
///
/// # Security
///
/// Callers **must** redact `prompt` and every chunk's `content` in `context`
/// for PII before calling this function — this module performs no redaction
/// itself. See `hacienda-api`'s `rag_stream` handler for the mandatory
/// redaction gate applied before this function is reached.
pub fn answer_stream(
    context: RetrievedContext,
    prompt: String,
    config: LlmAnswerConfig,
) -> impl Stream<Item = RagResult<AnswerEvent>> + Send + 'static {
    let citations: Vec<RagResult<AnswerEvent>> = context
        .chunks
        .iter()
        .filter(|c| c.content.is_some())
        .map(|c| {
            Ok(AnswerEvent::Citation(Citation {
                document_id: c.document_id.clone(),
                chunk_id: c.id.clone(),
                ordinal: c.ordinal,
            }))
        })
        .collect();

    let setup = async move {
        let context_text = context_text_of(&context.chunks);

        let raw = build_token_stream(prompt, context_text, config).await?;
        let tokens = map_tokens(raw);
        let done = stream::once(async { Ok(AnswerEvent::Done) });
        let combined: Pin<Box<dyn Stream<Item = RagResult<AnswerEvent>> + Send + 'static>> =
            Box::pin(tokens.chain(done));
        Ok::<_, RagError>(combined)
    };

    let llm_stream = stream::once(setup).try_flatten();

    stream::iter(citations).chain(llm_stream)
}

/// Map a raw liter-llm chunk stream to [`AnswerEvent`]s.
///
/// - Non-empty `delta.content` → [`AnswerEvent::Token`]
/// - `usage` present → [`AnswerEvent::Usage`]
/// - Empty content and no usage → filtered out
/// - Error → propagated as [`RagError::Backend`]
///
/// This function is exposed publicly so callers can unit-test the mapping
/// without a live network connection.
pub fn map_tokens(
    raw: impl Stream<Item = liter_llm::Result<liter_llm::ChatCompletionChunk>> + Send + 'static,
) -> impl Stream<Item = RagResult<AnswerEvent>> + Send + 'static {
    raw.filter_map(|result| async move {
        match result {
            Err(e) => Some(Err(RagError::Backend(Box::new(e)))),
            Ok(chunk) => {
                if let Some(usage) = chunk.usage {
                    return Some(Ok(AnswerEvent::Usage(TokenUsage {
                        prompt_tokens: usage.prompt_tokens as u32,
                        completion_tokens: usage.completion_tokens as u32,
                    })));
                }
                chunk
                    .choices
                    .into_iter()
                    .next()
                    .and_then(|c| c.delta.content)
                    .filter(|s| !s.is_empty())
                    .map(|s| Ok(AnswerEvent::Token(s)))
            }
        }
    })
}

/// The context text injected into the LLM request: every chunk's `content`, joined —
/// extracted from [`answer_stream`] so the egress audit (issue #148) hashes exactly the
/// string that is transmitted, with no chance of the two drifting apart.
fn context_text_of(chunks: &[RetrievedChunk]) -> String {
    chunks
        .iter()
        .filter_map(|c| c.content.as_deref())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The system message actually sent for `config`, given the composed `context_text` —
/// extracted from [`build_token_stream`] for the same reason: the egress audit must
/// hash the same string the transport receives.
fn system_content_of(config: &LlmAnswerConfig, context_text: &str) -> String {
    match &config.system_prompt {
        Some(system) => system.clone(),
        None if context_text.is_empty() => "You are a helpful assistant.".to_string(),
        None => format!("Use the following context to answer the question:\n\n{context_text}"),
    }
}

/// Build the outgoing chat request from `prompt` and `context_text` — extracted from
/// [`build_token_stream`] so the exact messages handed to the provider are
/// unit-testable without a live connection (the redaction guarantee is only as strong
/// as "the request carries exactly the strings it was given").
fn build_chat_request(
    prompt: String,
    context_text: String,
    config: LlmAnswerConfig,
) -> liter_llm::ChatCompletionRequest {
    let system_content = system_content_of(&config, &context_text);

    liter_llm::ChatCompletionRequest {
        model: config.llm.model,
        messages: vec![
            liter_llm::Message::System(liter_llm::SystemMessage {
                content: liter_llm::UserContent::Text(system_content),
                name: None,
            }),
            liter_llm::Message::User(liter_llm::UserMessage {
                content: liter_llm::UserContent::Text(prompt),
                name: None,
            }),
        ],
        temperature: config.llm.temperature,
        max_tokens: config.llm.max_tokens,
        stream_options: Some(liter_llm::StreamOptions {
            include_usage: Some(true),
        }),
        ..Default::default()
    }
}

/// Open the LLM connection and return a raw streaming chunk iterator.
///
/// Constructs the request from `prompt` and `context_text`, creates a
/// liter-llm client from `config.llm`, and calls `chat_stream`.
async fn build_token_stream(
    prompt: String,
    context_text: String,
    config: LlmAnswerConfig,
) -> RagResult<liter_llm::BoxStream<'static, liter_llm::Result<liter_llm::ChatCompletionChunk>>> {
    use liter_llm::LlmClient as _;

    // Fail-closed egress: never open the transport to an absent or unauthorized
    // destination, even when `answer_stream` is called directly (bypassing
    // `GuardedLlm`).
    validate_egress(&config)?;

    let client = liter_llm::create_client(
        config.llm.api_key.clone().unwrap_or_default(),
        config.llm.base_url.clone(),
        config.llm.timeout_secs,
        config.llm.max_retries,
        Some(config.llm.model.clone()),
    )
    .map_err(|e| RagError::Backend(Box::new(e)))?;

    let req = build_chat_request(prompt, context_text, config);

    client
        .chat_stream(req)
        .await
        .map_err(|e| RagError::Backend(Box::new(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn token_chunk(content: &str) -> liter_llm::ChatCompletionChunk {
        liter_llm::ChatCompletionChunk {
            id: "t".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: "m".to_string(),
            choices: vec![liter_llm::StreamChoice {
                index: 0,
                delta: liter_llm::StreamDelta {
                    content: Some(content.to_string()),
                    ..Default::default()
                },
                finish_reason: None,
            }],
            usage: None,
            system_fingerprint: None,
            service_tier: None,
        }
    }

    fn usage_chunk(prompt: u64, completion: u64) -> liter_llm::ChatCompletionChunk {
        liter_llm::ChatCompletionChunk {
            id: "u".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: "m".to_string(),
            choices: vec![],
            usage: Some(liter_llm::Usage {
                prompt_tokens: prompt,
                completion_tokens: completion,
                total_tokens: prompt + completion,
                ..Default::default()
            }),
            system_fingerprint: None,
            service_tier: None,
        }
    }

    #[tokio::test]
    async fn map_tokens_emits_token_events() {
        let raw = stream::iter(vec![
            Ok::<_, liter_llm::LiterLlmError>(token_chunk("Hello")),
            Ok(token_chunk(" world")),
        ]);
        let events: Vec<_> = map_tokens(raw).collect().await;
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], Ok(AnswerEvent::Token(t)) if t == "Hello"));
        assert!(matches!(&events[1], Ok(AnswerEvent::Token(t)) if t == " world"));
    }

    #[tokio::test]
    async fn map_tokens_emits_usage_event() {
        let raw = stream::iter(vec![Ok::<_, liter_llm::LiterLlmError>(usage_chunk(
            100, 50,
        ))]);
        let events: Vec<_> = map_tokens(raw).collect().await;
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], Ok(AnswerEvent::Usage(u)) if u.prompt_tokens == 100 && u.completion_tokens == 50)
        );
    }

    #[tokio::test]
    async fn map_tokens_filters_empty_content() {
        let raw = stream::iter(vec![
            Ok::<_, liter_llm::LiterLlmError>(token_chunk("")),
            Ok(token_chunk("word")),
        ]);
        let events: Vec<_> = map_tokens(raw).collect().await;
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], Ok(AnswerEvent::Token(t)) if t == "word"));
    }

    #[tokio::test]
    async fn map_tokens_propagates_errors() {
        let raw = stream::iter(vec![Err::<liter_llm::ChatCompletionChunk, _>(
            liter_llm::LiterLlmError::BadRequest {
                message: "upstream error".to_string(),
                status: 400,
            },
        )]);
        let events: Vec<_> = map_tokens(raw).collect().await;
        assert_eq!(events.len(), 1);
        assert!(events[0].is_err());
    }

    #[tokio::test]
    async fn map_tokens_handles_empty_stream() {
        let raw = stream::iter(Vec::<liter_llm::Result<liter_llm::ChatCompletionChunk>>::new());
        let events: Vec<_> = map_tokens(raw).collect().await;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn answer_stream_emits_citations_before_llm_error() {
        // No live network: the LLM connection attempt fails (empty api_key /
        // unroutable model), so the stream must still emit citations first,
        // then surface the failure as an `Err` item rather than panicking or
        // silently dropping the citations.
        use crate::types::PrimaryScore;

        let chunk = RetrievedChunk {
            id: ChunkId("c1".to_string()),
            document_id: DocumentId("d1".to_string()),
            ordinal: 0,
            external_id: None,
            content: Some("hello world".to_string()),
            score: 1.0,
            primary_score: PrimaryScore::Vector { score: 1.0 },
            chunk_metadata: serde_json::Value::Null,
            document: None,
        };
        let context = RetrievedContext {
            chunks: vec![chunk],
        };
        let config = LlmAnswerConfig {
            llm: xberg::LlmConfig {
                model: "not-a-real-provider/does-not-exist".to_string(),
                ..Default::default()
            },
            system_prompt: None,
            egress_allowlist: None,
        };

        let events: Vec<_> = answer_stream(context, "what is this?".to_string(), config)
            .collect()
            .await;

        assert!(!events.is_empty());
        assert!(
            matches!(&events[0], Ok(AnswerEvent::Citation(c)) if c.chunk_id == ChunkId("c1".to_string()))
        );
        assert!(events.iter().skip(1).any(|e| e.is_err()));
    }

    // ── GuardedLlm (P6) ────────────────────────────────────────────────────────

    use std::sync::{Arc, Mutex};

    /// Records every string it was asked to redact, and either echoes it back
    /// unchanged or fails, depending on `fail_on`.
    struct SpyRedactor {
        seen: Arc<Mutex<Vec<String>>>,
        fail_on: Option<&'static str>,
    }

    impl SpyRedactor {
        fn new() -> Self {
            Self {
                seen: Arc::new(Mutex::new(Vec::new())),
                fail_on: None,
            }
        }

        fn failing_on(marker: &'static str) -> Self {
            Self {
                seen: Arc::new(Mutex::new(Vec::new())),
                fail_on: Some(marker),
            }
        }
    }

    #[async_trait::async_trait]
    impl Redactor for SpyRedactor {
        async fn redact(&self, text: &str) -> RagResult<String> {
            self.seen.lock().unwrap().push(text.to_string());
            if self.fail_on == Some(text) {
                return Err(RagError::InvalidQuery(
                    "control-corpus value present — redaction refused".to_string(),
                ));
            }
            Ok(text.to_string())
        }
    }

    fn guarded_llm_test_chunk(content: &str) -> RetrievedChunk {
        guarded_llm_test_chunk_with(0, Some(content))
    }

    fn guarded_llm_test_chunk_with(ordinal: u32, content: Option<&str>) -> RetrievedChunk {
        use crate::types::PrimaryScore;
        RetrievedChunk {
            id: ChunkId(format!("c{ordinal}")),
            document_id: DocumentId("d1".to_string()),
            ordinal,
            external_id: None,
            content: content.map(str::to_string),
            score: 1.0,
            primary_score: PrimaryScore::Vector { score: 1.0 },
            chunk_metadata: serde_json::Value::Null,
            document: None,
        }
    }

    fn guarded_llm_test_config() -> LlmAnswerConfig {
        LlmAnswerConfig {
            llm: xberg::LlmConfig {
                model: "not-a-real-provider/does-not-exist".to_string(),
                ..Default::default()
            },
            system_prompt: None,
            egress_allowlist: None,
        }
    }

    /// The redaction step must see the prompt and every chunk's raw content — proof
    /// nothing reaches an LLM without first passing through the injected [`Redactor`].
    #[tokio::test]
    async fn guarded_llm_calls_the_redactor_on_the_prompt_and_every_chunk() {
        let redactor = SpyRedactor::new();
        let context = RetrievedContext {
            chunks: vec![guarded_llm_test_chunk("chunk with a PII value")],
        };

        let result = GuardedLlm::new(&redactor)
            .stream(
                context,
                "prompt with a PII value".to_string(),
                guarded_llm_test_config(),
            )
            .await;
        assert!(result.is_ok(), "redaction succeeded, stream must build");

        let seen = redactor.seen.lock().unwrap();
        assert!(seen.contains(&"prompt with a PII value".to_string()));
        assert!(seen.contains(&"chunk with a PII value".to_string()));
    }

    /// A redactor that refuses the prompt must stop `GuardedLlm` before any stream is
    /// built — the LLM connection is never attempted with an un-redactable prompt.
    #[tokio::test]
    async fn guarded_llm_refuses_to_stream_when_the_redactor_rejects_the_prompt() {
        let redactor = SpyRedactor::failing_on("control-corpus prompt");
        let context = RetrievedContext {
            chunks: vec![guarded_llm_test_chunk("harmless chunk")],
        };

        let result = GuardedLlm::new(&redactor)
            .stream(
                context,
                "control-corpus prompt".to_string(),
                guarded_llm_test_config(),
            )
            .await;

        assert!(
            result.is_err(),
            "a rejected prompt must not reach answer_stream"
        );
    }

    /// Same guarantee, for chunk content instead of the prompt: a single un-redactable
    /// chunk stops the whole call, not just that chunk.
    #[tokio::test]
    async fn guarded_llm_refuses_to_stream_when_the_redactor_rejects_a_chunk() {
        let redactor = SpyRedactor::failing_on("control-corpus chunk");
        let context = RetrievedContext {
            chunks: vec![guarded_llm_test_chunk("control-corpus chunk")],
        };

        let result = GuardedLlm::new(&redactor)
            .stream(
                context,
                "harmless prompt".to_string(),
                guarded_llm_test_config(),
            )
            .await;

        assert!(
            result.is_err(),
            "a rejected chunk must not reach answer_stream"
        );
    }

    /// The loop in `stream` processes chunks one at a time; a rejection partway through
    /// must still see every earlier chunk (proving the loop doesn't short-circuit before
    /// reaching the redactor) and must still fail the whole call (proving a later
    /// rejection isn't silently dropped once earlier chunks succeeded). A chunk with no
    /// content (`content: None`) must be skipped without ever reaching the redactor —
    /// there is nothing to redact.
    #[tokio::test]
    async fn guarded_llm_processes_every_chunk_in_order_and_fails_on_the_first_rejection() {
        let redactor = SpyRedactor::failing_on("control-corpus chunk");
        let context = RetrievedContext {
            chunks: vec![
                guarded_llm_test_chunk_with(0, Some("clean chunk")),
                guarded_llm_test_chunk_with(1, None),
                guarded_llm_test_chunk_with(2, Some("control-corpus chunk")),
            ],
        };

        let result = GuardedLlm::new(&redactor)
            .stream(
                context,
                "harmless prompt".to_string(),
                guarded_llm_test_config(),
            )
            .await;

        assert!(
            result.is_err(),
            "the rejected chunk must fail the whole call, not just itself"
        );
        let seen = redactor.seen.lock().unwrap();
        assert!(
            seen.contains(&"clean chunk".to_string()),
            "the clean chunk before the rejected one must still have reached the redactor"
        );
        assert!(
            !seen.iter().any(|s| s.is_empty()),
            "a chunk with content: None must never reach the redactor — there is nothing \
             to redact, and calling redact(\"\") would misrepresent it as an empty string \
             that was actually redacted"
        );
    }

    /// The system prompt override is redacted through the injected [`Redactor`] before the
    /// stream is built — proof the unredacted system prompt never reaches the transport.
    #[tokio::test]
    async fn guarded_llm_redacts_the_system_prompt_before_building_the_stream() {
        let redactor = SpyRedactor::new();
        let system = "system prompt with a PII value";
        let context = RetrievedContext { chunks: vec![] };

        let mut config = guarded_llm_test_config();
        config.system_prompt = Some(system.to_string());

        let result = GuardedLlm::new(&redactor)
            .stream(context, "harmless prompt".to_string(), config)
            .await;
        assert!(result.is_ok(), "redaction succeeded, stream must build");

        let seen = redactor.seen.lock().unwrap();
        assert!(
            seen.contains(&system.to_string()),
            "the system prompt must have reached the redactor"
        );
    }

    /// A redactor that refuses the system prompt must stop `GuardedLlm` before any stream
    /// is built (fail-closed): the unredacted system prompt never reaches the transport.
    #[tokio::test]
    async fn guarded_llm_refuses_to_stream_when_the_redactor_rejects_the_system_prompt() {
        let redactor = SpyRedactor::failing_on("control-corpus system");
        let context = RetrievedContext { chunks: vec![] };

        let mut config = guarded_llm_test_config();
        config.system_prompt = Some("control-corpus system".to_string());

        let result = GuardedLlm::new(&redactor)
            .stream(context, "harmless prompt".to_string(), config)
            .await;

        assert!(
            result.is_err(),
            "a rejected system prompt must not reach answer_stream"
        );
    }

    fn strict_egress_config(base_url: Option<&str>) -> LlmAnswerConfig {
        LlmAnswerConfig {
            llm: xberg::LlmConfig {
                model: "not-a-real-provider/does-not-exist".to_string(),
                base_url: base_url.map(str::to_string),
                ..Default::default()
            },
            system_prompt: None,
            egress_allowlist: Some(vec!["https://allowed.example.com".to_string()]),
        }
    }

    /// Strict egress mode: an absent `base_url` is rejected before the transport is built.
    #[tokio::test]
    async fn guarded_llm_rejects_missing_base_url_in_strict_mode() {
        let redactor = SpyRedactor::new();
        let context = RetrievedContext { chunks: vec![] };

        let result = GuardedLlm::new(&redactor)
            .stream(
                context,
                "harmless prompt".to_string(),
                strict_egress_config(None),
            )
            .await;

        assert!(
            result.is_err(),
            "a missing base_url must be rejected in strict egress mode"
        );
    }

    /// Strict egress mode: a `base_url` that is not on the allowlist is rejected before the
    /// transport is built.
    #[tokio::test]
    async fn guarded_llm_rejects_unauthorized_base_url() {
        let redactor = SpyRedactor::new();
        let context = RetrievedContext { chunks: vec![] };

        let result = GuardedLlm::new(&redactor)
            .stream(
                context,
                "harmless prompt".to_string(),
                strict_egress_config(Some("https://evil.example.com")),
            )
            .await;

        assert!(
            result.is_err(),
            "an unauthorized base_url must be rejected in strict egress mode"
        );
    }

    /// Strict egress mode: an authorized `base_url` is accepted (the stream is built; any
    /// later LLM failure is unrelated to egress).
    #[tokio::test]
    async fn guarded_llm_allows_authorized_base_url() {
        let redactor = SpyRedactor::new();
        let context = RetrievedContext { chunks: vec![] };

        let result = GuardedLlm::new(&redactor)
            .stream(
                context,
                "harmless prompt".to_string(),
                strict_egress_config(Some("https://allowed.example.com")),
            )
            .await;

        assert!(result.is_ok(), "an authorized base_url must be accepted");
    }

    // ── Deferred P6 egress audit (issue #148) ───────────────────────────────────

    /// A redactor that actually redacts a witness value (replacing it with a token)
    /// and records both what it saw and what it produced.
    struct RedactingSpy {
        produced: Arc<Mutex<Vec<String>>>,
    }

    impl RedactingSpy {
        fn new() -> Self {
            Self {
                produced: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn redacted(text: &str) -> String {
            text.replace("CONTROL_CORPUS_SECRET", "[REDACTED]")
        }
    }

    #[async_trait::async_trait]
    impl Redactor for RedactingSpy {
        async fn redact(&self, text: &str) -> RagResult<String> {
            let out = Self::redacted(text);
            self.produced.lock().unwrap().push(out.clone());
            Ok(out)
        }
    }

    /// Auditor spy: records every event it is asked to record.
    struct SpyAuditor {
        events: Arc<Mutex<Vec<LlmEgressEvent>>>,
    }

    impl SpyAuditor {
        fn new() -> (Arc<Self>, Arc<Mutex<Vec<LlmEgressEvent>>>) {
            let events = Arc::new(Mutex::new(Vec::new()));
            (
                Arc::new(Self {
                    events: Arc::clone(&events),
                }),
                events,
            )
        }
    }

    #[async_trait::async_trait]
    impl LlmEgressAuditor for SpyAuditor {
        async fn record_egress(&self, event: LlmEgressEvent) -> RagResult<()> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    /// Give the spawned audit task a bounded number of runtime turns to run.
    async fn drain_spawned_audit(events: &Arc<Mutex<Vec<LlmEgressEvent>>>) {
        for _ in 0..200 {
            if !events.lock().unwrap().is_empty() {
                return;
            }
            tokio::task::yield_now().await;
        }
    }

    /// The egress event must hash the **redacted** payload only: a witness value present
    /// in the raw prompt / system prompt / chunk must not appear anywhere in the event
    /// (which carries hashes + references, never content), and each part's hash must be
    /// the digest of the redacted string — provably different from the digest of the raw
    /// string that contains the witness.
    #[tokio::test]
    async fn guarded_llm_egress_event_hashes_only_redacted_content() {
        let redactor = RedactingSpy::new();
        let produced = Arc::clone(&redactor.produced);
        let (auditor, events) = SpyAuditor::new();

        let mut config = guarded_llm_test_config();
        config.system_prompt = Some("system with CONTROL_CORPUS_SECRET inside".to_string());
        let context = RetrievedContext {
            chunks: vec![guarded_llm_test_chunk("chunk with CONTROL_CORPUS_SECRET")],
        };

        let result = GuardedLlm::with_auditor(&redactor, auditor)
            .stream_with_purpose(
                context,
                "prompt with CONTROL_CORPUS_SECRET".to_string(),
                config,
                "rag-answer",
            )
            .await;
        assert!(result.is_ok(), "redaction succeeded, stream must build");

        drain_spawned_audit(&events).await;
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1, "exactly one egress event per dispatch");
        let event = &events[0];
        assert_eq!(event.purpose, "rag-answer");
        assert_eq!(event.provider_model, "not-a-real-provider/does-not-exist");

        // Hash + reference only: the serialized event must not contain the witness.
        let serialized = serde_json::to_string(event).unwrap();
        assert!(
            !serialized.contains("CONTROL_CORPUS_SECRET"),
            "egress event must carry hashes, not raw content: {serialized}"
        );

        // The redacted outputs of the redactor must not contain the witness either —
        // this is the string GuardedLlm forwards to answer_stream.
        let produced = produced.lock().unwrap();
        assert_eq!(produced.len(), 3, "system prompt + prompt + one chunk");
        for text in produced.iter() {
            assert!(
                !text.contains("CONTROL_CORPUS_SECRET"),
                "redacted text must not contain the witness: {text}"
            );
        }

        // Each part's hash is the digest of the redacted string, not the raw one.
        let sys_part = event
            .parts
            .iter()
            .find(|p| p.role == EgressRole::SystemPrompt)
            .expect("system prompt part");
        assert_eq!(sys_part.reference, "system_prompt");
        assert_eq!(
            sys_part.hash,
            blake3::hash("system with [REDACTED] inside".as_bytes())
                .to_hex()
                .to_string()
        );

        let prompt_part = event
            .parts
            .iter()
            .find(|p| p.role == EgressRole::Prompt)
            .expect("prompt part");
        assert_eq!(prompt_part.reference, "prompt");
        assert_eq!(
            prompt_part.hash,
            blake3::hash("prompt with [REDACTED]".as_bytes())
                .to_hex()
                .to_string()
        );
        assert_ne!(
            prompt_part.hash,
            blake3::hash("prompt with CONTROL_CORPUS_SECRET".as_bytes())
                .to_hex()
                .to_string(),
            "the hashed prompt must be the redacted one, not the raw one"
        );

        // With a system-prompt override the retrieved chunks are not transmitted, so
        // they must not appear in the egress audit (CodeRabbit #158). The redactor did
        // still process the chunk (see `produced` above) — but no audit part claims it
        // was sent to the provider.
        assert!(
            !event
                .parts
                .iter()
                .any(|p| p.role == EgressRole::ContextChunk),
            "undelivered chunks must not be recorded as transmitted"
        );
    }

    /// The egress audit fires even when the stream itself fails: the dispatch is
    /// audited at the guard (before the stream is returned), so an unroutable model
    /// surfacing as an `Err` item mid-stream must not lose the audit record.
    #[tokio::test]
    async fn guarded_llm_emits_egress_audit_even_when_the_stream_fails() {
        let redactor = SpyRedactor::new();
        let (auditor, events) = SpyAuditor::new();
        let context = RetrievedContext {
            chunks: vec![guarded_llm_test_chunk("hello world")],
        };

        // Unroutable model: the stream builds, then fails when polled.
        let stream = GuardedLlm::with_auditor(&redactor, auditor)
            .stream(
                context,
                "what is this?".to_string(),
                guarded_llm_test_config(),
            )
            .await
            .expect("stream must build");
        let collected: Vec<_> = stream.collect().await;
        assert!(
            collected.iter().any(|e| e.is_err()),
            "the unroutable model must surface as an Err item"
        );

        drain_spawned_audit(&events).await;
        let events = events.lock().unwrap();
        assert_eq!(
            events.len(),
            1,
            "the egress audit must be recorded even though the stream failed"
        );
        assert_eq!(
            events[0].purpose, "llm-call",
            "stream() uses the default purpose"
        );
    }

    /// The audit is deferred: a `record_egress` that never completes must not prevent
    /// `stream` from building the answer stream (the audit runs in a spawned task, off
    /// the streaming hot path).
    #[tokio::test]
    async fn guarded_llm_audit_never_blocks_stream_construction() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct PendingAuditor {
            calls: Arc<AtomicUsize>,
        }

        #[async_trait::async_trait]
        impl LlmEgressAuditor for PendingAuditor {
            async fn record_egress(&self, _event: LlmEgressEvent) -> RagResult<()> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                futures::future::pending::<()>().await;
                Ok(())
            }
        }

        let redactor = SpyRedactor::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let auditor: Arc<dyn LlmEgressAuditor> = Arc::new(PendingAuditor {
            calls: Arc::clone(&calls),
        });
        let context = RetrievedContext { chunks: vec![] };

        // If the audit were awaited inline, this would hang forever on the pending
        // auditor; the timeout turns a regression into a clean failure.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            GuardedLlm::with_auditor(&redactor, auditor).stream(
                context,
                "harmless prompt".to_string(),
                guarded_llm_test_config(),
            ),
        )
        .await;
        assert!(
            result.is_ok(),
            "stream construction must not wait on the auditor"
        );
        assert!(result.unwrap().is_ok(), "stream must build");

        // The auditor was still invoked — from the spawned task, not inline.
        for _ in 0..200 {
            if calls.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            calls.load(Ordering::SeqCst) > 0,
            "the auditor must be invoked"
        );
    }

    /// No auditor attached (plain `GuardedLlm::new`): nothing is recorded, the stream
    /// still builds — the audit is opt-in per call site.
    #[tokio::test]
    async fn guarded_llm_without_auditor_streams_normally() {
        let redactor = SpyRedactor::new();
        let context = RetrievedContext { chunks: vec![] };
        let result = GuardedLlm::new(&redactor)
            .stream(
                context,
                "harmless prompt".to_string(),
                guarded_llm_test_config(),
            )
            .await;
        assert!(result.is_ok());
    }

    /// The outgoing request carries exactly the strings it was given — system message
    /// first, user prompt second. Combined with the redacted-only event hashes above,
    /// this closes the chain: GuardedLlm redacts → answer_stream forwards verbatim →
    /// the transport sends what build_chat_request built.
    #[test]
    fn build_chat_request_places_system_then_user_verbatim() {
        let mut config = guarded_llm_test_config();
        config.system_prompt = Some("SYS".to_string());
        let req = build_chat_request("USER".to_string(), "CTX".to_string(), config);
        assert_eq!(req.messages.len(), 2);
        match &req.messages[0] {
            liter_llm::Message::System(m) => {
                assert!(matches!(&m.content, liter_llm::UserContent::Text(t) if t == "SYS"))
            }
            other => panic!("first message must be system, got {other:?}"),
        }
        match &req.messages[1] {
            liter_llm::Message::User(m) => {
                assert!(matches!(&m.content, liter_llm::UserContent::Text(t) if t == "USER"))
            }
            other => panic!("second message must be user, got {other:?}"),
        }

        // Without an override, the default framing injects the context into the system
        // message — the same string system_content_of hashes for the egress event.
        let config = guarded_llm_test_config();
        let req = build_chat_request("USER".to_string(), "CTX".to_string(), config.clone());
        let system = match &req.messages[0] {
            liter_llm::Message::System(m) => m.content.clone(),
            other => panic!("first message must be system, got {other:?}"),
        };
        let expected = system_content_of(&config, "CTX");
        assert!(matches!(&system, liter_llm::UserContent::Text(t) if t == &expected));
        assert!(expected.contains("CTX"));
    }
}
