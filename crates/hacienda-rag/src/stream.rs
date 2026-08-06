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
        let context_text: String = context
            .chunks
            .iter()
            .filter_map(|c| c.content.as_deref())
            .collect::<Vec<_>>()
            .join("\n\n");

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

    let client = liter_llm::create_client(
        config.llm.api_key.clone().unwrap_or_default(),
        config.llm.base_url.clone(),
        config.llm.timeout_secs,
        config.llm.max_retries,
        Some(config.llm.model.clone()),
    )
    .map_err(|e| RagError::Backend(Box::new(e)))?;

    let system_content = match config.system_prompt {
        Some(system) => system,
        None if context_text.is_empty() => "You are a helpful assistant.".to_string(),
        None => format!("Use the following context to answer the question:\n\n{context_text}"),
    };

    let req = liter_llm::ChatCompletionRequest {
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
    };

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
}
