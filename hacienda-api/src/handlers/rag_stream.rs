//! Handler for `POST /v1/rag/collections/{name}/answer` (Phase 12 Track 3).
//!
//! Streams a grounded answer over Server-Sent Events: retrieves chunks from the
//! configured `RagStore`, then hands the caller's prompt and the retrieved chunks to
//! `hacienda_rag::GuardedLlm` (`2026-08-13-P6-llm-call-enforcement-point.md`), which
//! redacts both — and the `system_prompt` override too (the É7 leak of wayfinder #131,
//! issue #148) — through `FacadeRedactor` below before ever reaching
//! `hacienda_rag::answer_stream` — the function that actually calls the LLM, and which is
//! deliberately not redaction-aware itself (see its `# Security` doc note in
//! `crates/hacienda-rag/src/stream.rs`). Before P6, this handler enforced that gate by
//! calling `redact_text_with_auth` by hand and trusting itself to remember; now the gate is
//! structural — `GuardedLlm::stream` cannot be called without a `Redactor`, and this is the
//! only call site in the workspace that constructs one.
//!
//! The same call site attaches the deferred P6 §5 egress audit (issue #148) via
//! `FacadeEgressAuditor`: every dispatch records an `llm_egress` audit entry set (one
//! summary + one entry per transmitted part, hash + reference only, never content) into
//! the caller tenant's chain through `HaciendaFacade::record_llm_egress_with_auth`, from
//! a spawned task so the audit write can neither slow the stream nor be lost when the
//! stream fails mid-flight.
//!
//! # Why SSE errors don't reuse `ApiError`
//!
//! Once the first byte of an SSE response is written, the HTTP status is fixed at
//! 200 — a mid-stream failure (a store error, an LLM connection failure) cannot
//! become a 4xx/5xx the way every other handler in this crate reports errors.
//! Failures that happen *before* the stream starts (no store configured, unknown
//! collection, invalid query, a redaction failure) still return `ApiError`
//! normally, via the `?` operator below. Failures that happen *after* the stream
//! starts are logged via `tracing::error!` and surfaced to the client as an
//! `error` SSE event carrying a generic, client-safe message — mirroring
//! `ApiError`'s own "never forward the internal message" rule (see `error.rs`'s
//! module doc).

use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::request::Parts,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::{Stream, StreamExt};
use hacienda_core::auth::{AuthContext, Caller};
use hacienda_core::{
    audit::{LlmEgressPart, LlmEgressRecord, LlmEgressRole},
    HaciendaError, HaciendaFacade,
};
use hacienda_rag::{
    AnswerEvent, EgressRole, GraphContext, GuardedLlm, LlmAnswerConfig, LlmEgressAuditor,
    LlmEgressEvent, RagError, RagResult, Redactor, RetrievedChunk, RetrievedContext,
};

use crate::{
    dto::AnswerRequest,
    error::ApiError,
    extract::Json as SafeJson,
    handlers::{
        caller_from_arc, extract_auth_context,
        graph::require_graph_store,
        rag::require_store,
        rag::{
            enforce_post_retrieval_tenant, merge_with_tenant_filter, scope_collection_name,
            tenant_authorization_filter, validate_collection_name,
        },
    },
    state::ApiState,
};

/// Adapts [`hacienda::HaciendaFacade::redact_text_with_auth`] to [`hacienda_rag::Redactor`]
/// — the seam `GuardedLlm` (P6) redacts through. `hacienda-rag` cannot depend on
/// `hacienda-core` (see that crate's dev-dependency comment on the edge), so this adapter
/// lives here, in the one crate that already depends on both.
///
/// `redact` boxes the `HaciendaError` it gets into `RagError::Backend` because
/// `hacienda_rag::Redactor`'s return type has no room for a richer error — but boxing loses
/// nothing: [`map_guarded_llm_error`] downcasts it back to `HaciendaError` at the one point
/// this crate turns a `GuardedLlm::stream` failure into an [`ApiError`], recovering the same
/// 403/400/500 classification `redact_text_with_auth`'s direct callers already get through
/// [`ApiError`]'s `From<HaciendaError>` impl. Without that downcast, every redaction failure
/// — including a missing `documents:process` capability, which should be 403 — would
/// collapse to the generic `RagError::Backend` → 500 mapping.
struct FacadeRedactor<'a> {
    facade: &'a HaciendaFacade,
    caller: Caller<'a>,
}

#[async_trait::async_trait]
impl Redactor for FacadeRedactor<'_> {
    async fn redact(&self, text: &str) -> RagResult<String> {
        let result = self
            .facade
            .redact_text_with_auth(self.caller, text)
            .await
            .map_err(|error| RagError::Backend(Box::new(error)))?;
        Ok(result.redacted_text)
    }
}

/// Adapts [`HaciendaFacade::record_llm_egress_with_auth`] to
/// [`hacienda_rag::LlmEgressAuditor`] — the seam the deferred P6 "content sent to an LLM"
/// audit (issue #148) records through. Same boundary rationale as [`FacadeRedactor`]:
/// `hacienda-rag` cannot depend on `hacienda-core`, so the adapter lives here, in the one
/// crate that already depends on both.
///
/// The caller identity is owned (`AuthContext` is [`Clone`]) because the auditor is
/// [`Arc`]-held by `GuardedLlm` and invoked from a spawned task, i.e. it must be
/// `'static` — while the handler's `Caller<'_>` borrows the request's auth context.
/// Cloning the context preserves the principal and its tenant: the audit entries name
/// the same principal the redaction gate authenticated, and the capability check
/// (`documents:process`) is enforced again inside the facade call, not skipped.
///
/// The conversion is hash + reference only: [`LlmEgressEvent`] already carries no raw
/// content, and the facade persists exactly that — so no redacted span is ever written
/// to the audit chain in plain text.
struct FacadeEgressAuditor {
    facade: Arc<HaciendaFacade>,
    /// The authenticated principal's context, owned; `None` for an in-process
    /// (`Caller::Trusted`) call site.
    auth_ctx: Option<AuthContext>,
}

impl FacadeEgressAuditor {
    /// Rebuild the [`Caller`] the facade's `with_auth` seam expects from the owned
    /// identity this adapter holds.
    fn caller(&self) -> Caller<'_> {
        match &self.auth_ctx {
            Some(ctx) => Caller::Principal(ctx),
            None => Caller::Trusted,
        }
    }
}

#[async_trait::async_trait]
impl LlmEgressAuditor for FacadeEgressAuditor {
    async fn record_egress(&self, event: LlmEgressEvent) -> RagResult<()> {
        let record = LlmEgressRecord {
            purpose: event.purpose,
            provider_model: event.provider_model,
            provider_base_url: event.provider_base_url,
            request_hash: event.request_hash,
            request_length: event.request_length,
            parts: event
                .parts
                .into_iter()
                .map(|part| LlmEgressPart {
                    role: match part.role {
                        EgressRole::SystemPrompt => LlmEgressRole::SystemPrompt,
                        EgressRole::Prompt => LlmEgressRole::Prompt,
                        EgressRole::ContextChunk => LlmEgressRole::ContextChunk,
                    },
                    reference: part.reference,
                    hash: part.hash,
                    length: part.length,
                })
                .collect(),
        };
        self.facade
            .record_llm_egress_with_auth(self.caller(), record)
            .await
            .map(|_entries| ())
            .map_err(|error| RagError::Backend(Box::new(error)))
    }
}

/// Maps a [`GuardedLlm::stream`] failure to [`ApiError`], recovering the [`HaciendaError`]
/// [`FacadeRedactor::redact`] boxed instead of falling through to the generic
/// `RagError::Backend` → 500 mapping — see that struct's doc comment for why this matters.
/// A `RagError` that did not originate from `FacadeRedactor` (a genuine RAG-store error, a
/// filter/query problem) still goes through the ordinary `From<RagError> for ApiError`.
fn map_guarded_llm_error(error: RagError) -> ApiError {
    match error {
        RagError::Backend(inner) => match inner.downcast::<HaciendaError>() {
            Ok(hacienda_error) => ApiError::from(*hacienda_error),
            Err(inner) => ApiError::from(RagError::Backend(inner)),
        },
        other => ApiError::from(other),
    }
}

/// `POST /v1/rag/collections/{name}/answer`
#[utoipa::path(
    post,
    path = "/v1/rag/collections/{name}/answer",
    tag = "rag",
    operation_id = "answer",
    security(("bearerAuth" = [])),
    params(("name" = String, Path, description = "Collection name")),
    request_body = AnswerRequest,
    responses(
        (
            status = 200,
            description = "Server-Sent Events stream: token/citation/usage/done/error events, per this module's doc comment",
            content_type = "text/event-stream"
        ),
        (status = 400, description = "RAG is not enabled, or the retrieval query is invalid for this collection"),
        (status = 404, description = "No such collection")
    )
)]
pub async fn answer(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    parts: Parts,
    SafeJson(mut body): SafeJson<AnswerRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant_ctx = caller.tenant_ctx();
    validate_collection_name(&name)?;
    let scoped_name = scope_collection_name(&tenant_ctx, &name);

    let store = require_store(&state)?;
    let spec = store
        .get_collection(&scoped_name)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;

    // Answer synthesis has nothing to ground on without chunk text, regardless
    // of what the caller sent.
    if let Some(graph_request) = body.graph.as_ref() {
        graph_request.validate()?;
    }
    body.retrieve.include_content = true;
    body.retrieve.tenant_id = Some(tenant_ctx.tenant.as_str().to_owned());
    body.retrieve.filter = merge_with_tenant_filter(
        tenant_authorization_filter(&tenant_ctx),
        body.retrieve.filter.take(),
    );
    body.retrieve.validate(&spec).map_err(ApiError::from)?;

    let mut output = store
        .retrieve(&scoped_name, &body.retrieve)
        .await
        .map_err(ApiError::from)?;
    enforce_post_retrieval_tenant(&tenant_ctx, &mut output.chunks);

    // Mandatory PII redaction gate: `GuardedLlm` (P6) redacts the prompt and every
    // chunk's content through `FacadeRedactor` before `answer_stream` — which performs
    // no redaction of its own — ever sees them. This is now enforced by `GuardedLlm`'s
    // own construction rather than by this handler remembering to call
    // `redact_text_with_auth` twice by hand before reaching `answer_stream` directly.
    let mut chunks = output.chunks;
    if let Some(graph_request) = body.graph {
        let graph_store = require_graph_store(&state)?;
        let tenant = tenant_ctx.tenant.as_str();
        let graph = graph_store
            .neighborhood(
                &scoped_name,
                tenant,
                &graph_request.entity_ids,
                graph_request.hops,
                graph_request.limit,
                graph_request.min_confidence,
            )
            .await
            .map_err(ApiError::from)?;
        attach_evidence_backed_graph(&mut chunks, graph);
    }
    let context = RetrievedContext { chunks };
    let config = LlmAnswerConfig {
        llm: body.llm,
        system_prompt: body.system_prompt,
        // The API is always strict. An absent server policy becomes an empty
        // allowlist, never the permissive `None` used by lower-level callers.
        egress_allowlist: Some(state.rag_egress_allowlist.clone().unwrap_or_default()),
    };
    let redactor = FacadeRedactor {
        facade: &state.facade,
        caller,
    };
    // Deferred P6 egress audit (issue #148): every dispatch through `GuardedLlm` records
    // an `llm_egress` audit event (hash + reference of the redacted payload, provider,
    // purpose) from a spawned task — off the streaming hot path, and still emitted when
    // the stream itself later fails.
    let egress_auditor = Arc::new(FacadeEgressAuditor {
        facade: Arc::clone(&state.facade),
        auth_ctx: match caller {
            Caller::Trusted => None,
            Caller::Principal(ctx) => Some(ctx.clone()),
        },
    });
    let stream = GuardedLlm::with_auditor(&redactor, egress_auditor)
        .stream(context, body.prompt, config)
        .await
        .map_err(map_guarded_llm_error)?;

    let events = stream.map(to_sse_event);

    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

/// Add graph facts only to chunks retrieved and authorized by the vector path.
/// Keeping facts in `RetrievedChunk.content` makes the existing PII redaction,
/// citation, and egress-audit gates apply without a parallel prompt channel.
fn attach_evidence_backed_graph(chunks: &mut [RetrievedChunk], graph: GraphContext) {
    for chunk in chunks {
        let Some(content) = chunk.content.as_mut() else {
            continue;
        };
        let entities = graph
            .entities
            .iter()
            .filter(|entity| {
                entity.document_id == chunk.document_id
                    && entity.chunk_id.as_ref().is_none_or(|id| id == &chunk.id)
            })
            .collect::<Vec<_>>();
        let mut facts = Vec::new();
        for entity in &entities {
            facts.push(format!(
                "entity {:?}: {}",
                entity.label, entity.normalized_value
            ));
        }
        for relation in &graph.relations {
            let evidence = &relation.evidence;
            if evidence.document_id != chunk.document_id
                || evidence.chunk_id.as_ref().is_some_and(|id| id != &chunk.id)
            {
                continue;
            }
            let subject = graph.entities.iter().find(|e| e.id == relation.subject);
            let object = graph.entities.iter().find(|e| e.id == relation.object);
            if let (Some(subject), Some(object)) = (subject, object) {
                if subject.document_id == chunk.document_id
                    && object.document_id == chunk.document_id
                    // Evidence must ground both endpoints in *this* chunk. An
                    // endpoint from an unretrieved sibling chunk would otherwise
                    // be presented as grounded in this source's header.
                    && subject.chunk_id.as_ref().is_none_or(|id| id == &chunk.id)
                    && object.chunk_id.as_ref().is_none_or(|id| id == &chunk.id)
                {
                    facts.push(format!(
                        "relation: {} {:?} {}",
                        subject.normalized_value, relation.predicate, object.normalized_value
                    ));
                }
            }
        }
        facts.sort();
        facts.dedup();
        if !facts.is_empty() {
            content.push_str("\n\n[Graph context grounded in this source]\n");
            for fact in facts {
                content.push_str("- ");
                content.push_str(&fact);
                content.push('\n');
            }
        }
    }
}

/// Map one `hacienda_rag::stream` event to an outgoing SSE [`Event`].
///
/// `Err` is only reachable after the stream has already started — a store error
/// before that point is an [`ApiError`], handled in [`answer`] above via `?`. See
/// this module's doc comment for why a post-start failure becomes an `error` SSE
/// event instead of an HTTP status.
fn to_sse_event(item: RagResult<AnswerEvent>) -> Result<Event, Infallible> {
    let event = match item {
        Ok(AnswerEvent::Token(text)) => Event::default().event("token").data(text),
        Ok(AnswerEvent::Citation(citation)) => Event::default()
            .event("citation")
            .json_data(citation)
            .unwrap_or_else(|_| {
                Event::default()
                    .event("error")
                    .data("citation encoding failed")
            }),
        Ok(AnswerEvent::Usage(usage)) => Event::default()
            .event("usage")
            .json_data(usage)
            .unwrap_or_else(|_| {
                Event::default()
                    .event("error")
                    .data("usage encoding failed")
            }),
        Ok(AnswerEvent::Done) => Event::default().event("done").data(""),
        Ok(AnswerEvent::Error(message)) => Event::default().event("error").data(message),
        Err(err) => {
            tracing::error!(error = %err, "rag answer stream failed");
            Event::default()
                .event("error")
                .data("The answer stream failed. Check host logs for details.")
        }
    };
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacienda_core::{pii::PipelineConfig, HaciendaConfig};
    use hacienda_rag::{EgressPart, EntityId, GraphEntity};

    /// A witness value that must never reach the audit chain in plain text —
    /// deliberately unlike anything else in the codebase (same discipline as
    /// `handlers/audit.rs`'s control corpus).
    const WITNESS: &str = "MAGNOLIA-CARTHAGE-4471";

    fn sample_event() -> LlmEgressEvent {
        LlmEgressEvent {
            purpose: "rag-answer".to_string(),
            provider_model: "acme/test-model".to_string(),
            provider_base_url: Some("https://llm.test.invalid".to_string()),
            request_hash: "ee".repeat(32),
            request_length: 512,
            parts: vec![
                EgressPart {
                    role: EgressRole::SystemPrompt,
                    reference: "system_prompt".to_string(),
                    hash: "aa".repeat(32),
                    length: 96,
                },
                EgressPart {
                    role: EgressRole::Prompt,
                    reference: "prompt".to_string(),
                    hash: "bb".repeat(32),
                    length: 64,
                },
                EgressPart {
                    role: EgressRole::ContextChunk,
                    reference: "chunk:doc-1:c0:0".to_string(),
                    hash: "cc".repeat(32),
                    length: 352,
                },
            ],
        }
    }

    /// The adapter maps the rag event onto the facade seam and the entries land in the
    /// caller tenant's audit chain: one summary + one per transmitted part, with every
    /// role's category preserved across the `hacienda-rag`/`hacienda-core` enum
    /// boundary, `source = llm_egress`, hashes + references only.
    #[tokio::test]
    async fn facade_egress_auditor_records_hash_only_entries_in_the_chain() {
        let config = HaciendaConfig::default().with_pii(PipelineConfig::default());
        let facade = Arc::new(HaciendaFacade::new(config).expect("facade"));
        let auditor = FacadeEgressAuditor {
            facade: Arc::clone(&facade),
            auth_ctx: None, // in-process (Trusted) call site
        };

        auditor.record_egress(sample_event()).await.expect("record");

        let entries = facade
            .audit_entries_with_auth(Caller::Trusted)
            .await
            .expect("read entries");
        assert_eq!(entries.len(), 4, "summary + one entry per transmitted part");
        assert_eq!(entries[0].category, "llm_egress");
        assert_eq!(
            entries[0].source,
            hacienda_core::audit::EntitySource::LlmEgress
        );
        assert_eq!(
            entries[0].model.as_deref(),
            Some("acme/test-model @ https://llm.test.invalid")
        );
        assert!(entries[1].id.ends_with(":system_prompt"));
        assert_eq!(entries[1].category, "llm_egress.system_prompt");
        assert!(entries[2].id.ends_with(":prompt"));
        assert_eq!(entries[2].category, "llm_egress.prompt");
        // The entry id is the part's reference (`id = reference`, the same mapping
        // `LlmEgressRecord::to_entry_inputs` documents and the core-level
        // `to_entry_inputs_maps_summary_plus_one_entry_per_part` asserts) — never
        // reference-plus-content: raw span text has no place in the chain, and
        // `WITNESS` below is the canary that would catch it if it did.
        assert!(entries[3].id.ends_with(":chunk:doc-1:c0:0"));
        assert_eq!(entries[3].category, "llm_egress.context_chunk");

        // Hash + reference only: every span hash is a 64-hex blake3 digest, and the
        // serialized chain carries no raw span text — `WITNESS` stands in for the
        // content a real dispatch would digest, never store.
        for entry in &entries {
            assert_eq!(entry.span_hash.len(), 64, "blake3 hex digest: {entry:?}");
        }
        let json = serde_json::to_string(&entries).unwrap();
        assert!(
            !json.contains(WITNESS),
            "audit entries must never carry raw content: {json}"
        );

        facade
            .verify_audit()
            .await
            .expect("the chain still verifies");
    }

    fn chunk(doc: &str, id: &str) -> RetrievedChunk {
        RetrievedChunk {
            id: hacienda_rag::ChunkId(id.to_owned()),
            document_id: hacienda_rag::DocumentId(doc.to_owned()),
            ordinal: 0,
            external_id: None,
            content: Some("original chunk text".to_owned()),
            score: 1.0,
            primary_score: hacienda_rag::PrimaryScore::Vector { score: 1.0 },
            chunk_metadata: serde_json::Value::Null,
            document: None,
            tenant_id: None,
            acl: None,
        }
    }

    fn entity(id: &str, doc: &str, chunk_id: Option<&str>) -> GraphEntity {
        GraphEntity {
            id: EntityId(id.to_owned()),
            label: hacienda_rag::EntityLabel::Organization,
            normalized_value: format!("entity-{id}"),
            document_id: hacienda_rag::DocumentId(doc.to_owned()),
            tenant_id: None,
            chunk_id: chunk_id.map(|id| hacienda_rag::ChunkId(id.to_owned())),
            start: 0,
            end: 4,
            confidence: 0.99,
            model_version: "test".to_owned(),
            properties: serde_json::Value::Null,
        }
    }

    /// Facts must land only on the retrieved chunk whose (document, chunk)
    /// evidence matches — a same-document sibling chunk or another document
    /// must stay untouched even when it was retrieved in the same request.
    #[test]
    fn attach_evidence_backed_graph_uses_document_and_chunk_evidence() {
        let mut chunks = vec![
            chunk("doc-1", "c0"),
            chunk("doc-1", "c1"),
            chunk("doc-2", "c0"),
        ];
        let graph = GraphContext {
            entities: vec![
                entity("e0", "doc-1", Some("c0")),
                entity("e1", "doc-2", Some("c0")),
            ],
            relations: Vec::new(),
        };

        attach_evidence_backed_graph(&mut chunks, graph);

        assert!(chunks[0]
            .content
            .as_deref()
            .unwrap()
            .contains("[Graph context"));
        assert!(chunks[0].content.as_deref().unwrap().contains("entity-e0"));
        assert!(!chunks[1]
            .content
            .as_deref()
            .unwrap()
            .contains("[Graph context"));
        assert!(chunks[2]
            .content
            .as_deref()
            .unwrap()
            .contains("[Graph context"));
        assert!(chunks[2].content.as_deref().unwrap().contains("entity-e1"));
    }

    /// A relation whose evidence points at a different chunk of the same
    /// document must not be attached to the current chunk.
    #[test]
    fn attach_evidence_backed_graph_rejects_mismatched_relation_evidence() {
        let mut chunks = vec![chunk("doc-1", "c0")];
        let relation = hacienda_rag::GraphRelation {
            subject: EntityId("e0".to_owned()),
            predicate: hacienda_rag::RelationType::References,
            object: EntityId("e1".to_owned()),
            evidence: hacienda_rag::EvidenceSpan {
                document_id: hacienda_rag::DocumentId("doc-1".to_owned()),
                chunk_id: Some(hacienda_rag::ChunkId("c1".to_owned())),
                start: 0,
                end: 4,
                text_hash: "hash".to_owned(),
            },
            confidence: 0.95,
            ontology_version: "test-v1".to_owned(),
            tenant_id: None,
        };
        let graph = GraphContext {
            entities: vec![
                entity("e0", "doc-1", Some("c0")),
                entity("e1", "doc-1", Some("c1")),
            ],
            relations: vec![relation],
        };

        attach_evidence_backed_graph(&mut chunks, graph);

        let content = chunks[0].content.as_deref().unwrap();
        assert!(
            content.contains("entity-e0"),
            "matching entity should attach"
        );
        assert!(
            !content.contains("relation:"),
            "relation with mismatched evidence must not attach: {content}"
        );
    }

    /// Relation evidence in the current chunk does not make an endpoint from a
    /// different chunk grounded: the object's value must not be shown here.
    #[test]
    fn attach_evidence_backed_graph_requires_both_endpoints_in_chunk() {
        let mut chunks = vec![chunk("doc-1", "c0")];
        let relation = hacienda_rag::GraphRelation {
            subject: EntityId("e0".to_owned()),
            predicate: hacienda_rag::RelationType::References,
            object: EntityId("e1".to_owned()),
            evidence: hacienda_rag::EvidenceSpan {
                document_id: hacienda_rag::DocumentId("doc-1".to_owned()),
                chunk_id: Some(hacienda_rag::ChunkId("c0".to_owned())),
                start: 0,
                end: 4,
                text_hash: "hash".to_owned(),
            },
            confidence: 0.95,
            ontology_version: "test-v1".to_owned(),
            tenant_id: None,
        };
        let graph = GraphContext {
            entities: vec![
                entity("e0", "doc-1", Some("c0")),
                entity("e1", "doc-1", Some("c1")),
            ],
            relations: vec![relation],
        };

        attach_evidence_backed_graph(&mut chunks, graph);

        let content = chunks[0].content.as_deref().unwrap();
        assert!(content.contains("entity-e0"));
        assert!(
            !content.contains("relation:"),
            "an endpoint in c1 must not be shown as c0 evidence: {content}"
        );
    }

    #[test]
    fn answer_graph_request_validates_bounds() {
        use crate::dto::AnswerGraphRequest;

        let valid = || AnswerGraphRequest {
            entity_ids: vec![EntityId("e".to_owned())],
            hops: 1,
            limit: 30,
            min_confidence: 0.9,
        };
        assert!(valid().validate().is_ok());

        let mut empty = valid();
        empty.entity_ids.clear();
        assert!(empty.validate().is_err());

        let mut oversized = valid();
        oversized.limit = 501;
        assert!(oversized.validate().is_err());

        let mut too_many_ids = valid();
        too_many_ids.entity_ids = (0..=hacienda_rag::MAX_ENTITY_IDS)
            .map(|i| EntityId(format!("e-{i}")))
            .collect();
        assert!(too_many_ids.validate().is_err());

        let mut too_many_hops = valid();
        too_many_hops.hops = 3;
        assert!(too_many_hops.validate().is_err());

        let mut nan = valid();
        nan.min_confidence = f32::NAN;
        assert!(nan.validate().is_err());
    }
}
