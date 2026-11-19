//! P6 §5 "content sent to an LLM" audit records (issue #148).
//!
//! When redacted content is dispatched to an external LLM provider, the audit chain
//! must answer "did this content leave the process, to which provider, for what
//! purpose, on whose behalf" — without storing the content itself. This module is the
//! structured record of one dispatch; [`LlmEgressRecord::to_entry_inputs`] maps it onto
//! the existing [`AuditEntryInput`] schema (no migration: `FileAuditStore` persists
//! entries as JSON, `PostgresAuditStore` stores the `Display` string forms, and
//! [`EntitySource::LlmEgress`] round-trips through both).
//!
//! Mapping, one `AuditStore::append` batch per dispatch (the D3 "one append per event"
//! invariant):
//!
//! - one **summary** entry: `category = "llm_egress"`, `span_hash = blake3(full outgoing
//!   request payload)`, `span_length` = its byte length, `action = Custom(purpose)`,
//!   `model` = provider (`model @ base_url`), `source = LlmEgress`, `principal` = caller;
//! - one entry **per transmitted part** (system prompt / user prompt / each context
//!   chunk): `category = "llm_egress.<role>"`, `id` = the part's *reference*
//!   (`system_prompt`, `prompt`, `chunk:<document_id>:<chunk_id>:<ordinal>`),
//!   `span_hash = blake3(redacted span)` — hash + reference, never raw content.

use serde::{Deserialize, Serialize};

use super::entry::{AuditEntryInput, EntitySource, RedactionAction};

/// Category of the summary entry minted once per LLM dispatch.
pub const LLM_EGRESS_CATEGORY: &str = "llm_egress";

/// Which part of an outgoing LLM request an [`LlmEgressPart`] describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmEgressRole {
    /// The (redacted) `system_prompt` override.
    SystemPrompt,
    /// The (redacted) user prompt.
    Prompt,
    /// One retrieved chunk's (redacted) content.
    ContextChunk,
}

impl LlmEgressRole {
    /// The audit `category` used for parts of this role.
    pub fn category(&self) -> String {
        match self {
            Self::SystemPrompt => format!("{LLM_EGRESS_CATEGORY}.system_prompt"),
            Self::Prompt => format!("{LLM_EGRESS_CATEGORY}.prompt"),
            Self::ContextChunk => format!("{LLM_EGRESS_CATEGORY}.context_chunk"),
        }
    }
}

/// One transmitted span of an LLM request, as hash + reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmEgressPart {
    pub role: LlmEgressRole,
    /// Reference identifying the part: `system_prompt`, `prompt`, or
    /// `chunk:<document_id>:<chunk_id>:<ordinal>`. Becomes the audit entry's `id`.
    pub reference: String,
    /// blake3 hex digest of the exact (redacted) bytes transmitted.
    pub hash: String,
    /// Byte length of the transmitted span.
    pub length: u32,
}

/// Structured record of one LLM dispatch. Carries no raw content, so it is safe to
/// persist and to log. Produced at the egress gate (`hacienda-rag`'s `GuardedLlm`,
/// issue #148) and recorded via `HaciendaFacade::record_llm_egress_with_auth`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmEgressRecord {
    /// What the call was for, e.g. `"rag-answer"` — recorded as the entries' action.
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
    pub parts: Vec<LlmEgressPart>,
}

impl LlmEgressRecord {
    /// Map the record onto audit-chain inputs: one summary entry for the whole request
    /// plus one entry per transmitted part. `config_hash` is left empty on purpose —
    /// the store owns that field (`AuditChain::push` overwrites it), same as
    /// `HaciendaFacade::record_audit_entries`.
    pub fn to_entry_inputs(
        &self,
        principal: Option<String>,
        pipeline_version: &str,
        vertical: Option<String>,
    ) -> Vec<AuditEntryInput> {
        let provider = match &self.provider_base_url {
            Some(url) => format!("{} @ {}", self.provider_model, url),
            None => self.provider_model.clone(),
        };

        // One UUID per dispatch: `audit_entries.id` is a primary key in Postgres, so a
        // bare part reference (`prompt`, `chunk:...`) would collide on the second
        // dispatch (and across tenants). Prefixing keeps the reference readable.
        let dispatch_id = uuid::Uuid::new_v4();
        let mut inputs = Vec::with_capacity(self.parts.len() + 1);
        inputs.push(AuditEntryInput {
            id: dispatch_id.to_string(),
            category: LLM_EGRESS_CATEGORY.to_string(),
            action: RedactionAction::Custom(self.purpose.clone()),
            span_hash: self.request_hash.clone(),
            span_length: self.request_length,
            confidence: None,
            source: EntitySource::LlmEgress,
            pipeline_version: pipeline_version.to_string(),
            config_hash: String::new(),
            principal: principal.clone(),
            vertical: vertical.clone(),
            model: Some(provider),
        });
        for part in &self.parts {
            inputs.push(AuditEntryInput {
                id: format!("{dispatch_id}:{}", part.reference),
                category: part.role.category(),
                action: RedactionAction::Custom(self.purpose.clone()),
                span_hash: part.hash.clone(),
                span_length: part.length,
                confidence: None,
                source: EntitySource::LlmEgress,
                pipeline_version: pipeline_version.to_string(),
                config_hash: String::new(),
                principal: principal.clone(),
                vertical: vertical.clone(),
                model: None,
            });
        }
        inputs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditStore, InMemoryAuditStore};
    use crate::tenancy::TenantId;

    fn sample_record() -> LlmEgressRecord {
        LlmEgressRecord {
            purpose: "rag-answer".to_string(),
            provider_model: "acme/gateway-model".to_string(),
            provider_base_url: Some("https://llm.internal.example.com".to_string()),
            request_hash: "aa".repeat(32),
            request_length: 4096,
            parts: vec![
                LlmEgressPart {
                    role: LlmEgressRole::SystemPrompt,
                    reference: "system_prompt".to_string(),
                    hash: "bb".repeat(32),
                    length: 128,
                },
                LlmEgressPart {
                    role: LlmEgressRole::Prompt,
                    reference: "prompt".to_string(),
                    hash: "cc".repeat(32),
                    length: 64,
                },
                LlmEgressPart {
                    role: LlmEgressRole::ContextChunk,
                    reference: "chunk:doc-1:c0:0".to_string(),
                    hash: "dd".repeat(32),
                    length: 3904,
                },
            ],
        }
    }

    #[test]
    fn to_entry_inputs_maps_summary_plus_one_entry_per_part() {
        let record = sample_record();
        let inputs = record.to_entry_inputs(
            Some("avocat-7".to_string()),
            "0.1.0",
            Some("finance@3f9a1c02".to_string()),
        );

        assert_eq!(inputs.len(), 4);

        let summary = &inputs[0];
        assert_eq!(summary.category, "llm_egress");
        assert_eq!(
            summary.action,
            RedactionAction::Custom("rag-answer".to_string())
        );
        assert_eq!(summary.span_hash, "aa".repeat(32));
        assert_eq!(summary.span_length, 4096);
        assert_eq!(summary.source, EntitySource::LlmEgress);
        assert_eq!(
            summary.model,
            Some("acme/gateway-model @ https://llm.internal.example.com".to_string())
        );
        assert_eq!(summary.principal, Some("avocat-7".to_string()));
        // The store owns config_hash — the input leaves it empty.
        assert!(summary.config_hash.is_empty());

        assert!(inputs[1].id.ends_with(":system_prompt"));
        assert_eq!(inputs[1].category, "llm_egress.system_prompt");
        assert_eq!(inputs[1].span_hash, "bb".repeat(32));
        assert!(inputs[2].id.ends_with(":prompt"));
        assert_eq!(inputs[2].category, "llm_egress.prompt");
        assert!(inputs[3].id.ends_with(":chunk:doc-1:c0:0"));
        assert_eq!(inputs[3].category, "llm_egress.context_chunk");
        for input in &inputs {
            assert_eq!(input.source, EntitySource::LlmEgress);
            assert_eq!(input.pipeline_version, "0.1.0");
        }
    }

    /// Two dispatches of the same record must never reuse an audit entry ID:
    /// `audit_entries.id` is a primary key in Postgres, so a bare part reference
    /// (`prompt`, `chunk:...`) would make the second dispatch fail its whole batch.
    #[test]
    fn repeated_dispatch_ids_are_unique() {
        let record = sample_record();
        let first = record.to_entry_inputs(Some("avocat-7".to_string()), "0.1.0", None);
        let second = record.to_entry_inputs(Some("avocat-7".to_string()), "0.1.0", None);
        let ids_first: std::collections::HashSet<_> = first.iter().map(|i| i.id.as_str()).collect();
        let ids_second: std::collections::HashSet<_> = second.iter().map(|i| i.id.as_str()).collect();
        assert!(ids_first.is_disjoint(&ids_second), "two dispatches reused audit entry IDs");
        assert_eq!(ids_first.len(), first.len());
        assert_eq!(ids_second.len(), second.len());
    }

    /// The record must persist through the existing store seam unchanged: one append
    /// batch, entries readable back with the egress source and reference ids intact,
    /// and the chain still verifies.
    #[tokio::test]
    async fn egress_record_round_trips_through_the_audit_store() {
        let store = InMemoryAuditStore::new("test-config");
        let tenant = TenantId::default_tenant();
        let record = sample_record();
        let inputs = record.to_entry_inputs(Some("avocat-7".to_string()), "0.1.0", None);

        let entries = store.append(&tenant, inputs).await.unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].category, "llm_egress");
        assert_eq!(entries[0].source, EntitySource::LlmEgress);
        assert!(entries[3].id.ends_with(":chunk:doc-1:c0:0"));
        assert_eq!(entries[3].category, "llm_egress.context_chunk");
        // config_hash is owned by the store/chain, not by the caller.
        assert_eq!(entries[0].config_hash, "test-config");

        let read_back = store.entries(&tenant).await.unwrap();
        assert_eq!(read_back.len(), 4);
        store.verify(&tenant).await.unwrap();

        // The entries serialize without any raw content field — hash + reference only.
        let json = serde_json::to_string(&read_back).unwrap();
        assert!(!json.contains("rag-answer content"));
        assert!(json.contains("llm_egress"));
    }
}
