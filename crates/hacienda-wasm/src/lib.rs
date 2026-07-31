//! wasm-bindgen entry points over `hacienda-core`'s `pii`/`redaction` pipeline.
//!
//! `process`/`scan`/`redact_empty` exercise the real API surface (regex/merge detection
//! and all four redaction modes, including the AES-SIV pseudonymisation path) so a
//! `wasm32-unknown-unknown` build actually reaches link time against `hacienda-core`'s
//! own code, per Track L2's check. `AuditHandle` (Track C3, wasm32-only) is the
//! production audit-chain binding L3 (clock/uuid/timestamp fixes) and L5 (IndexedDB
//! stores) were building toward — it wraps `IndexedDbAuditStore` directly rather than
//! re-implementing the chain in TypeScript.

use hacienda_core::pii::{PiiPipeline, PipelineConfig};
use hacienda_core::redaction::pseudonym::{EnvKeyResolver, Pseudonymiser};
use hacienda_core::redaction::{RedactionConfig, RedactionEngine, RedactionMode};
use std::sync::Arc;
use wasm_bindgen::prelude::*;

fn to_js_err<E: std::fmt::Display>(err: E) -> JsValue {
    JsValue::from_str(&err.to_string())
}

/// Regex + NER (when a model is loaded) detection and redaction over `text`, using the
/// default pipeline config. Returns the serialized `PipelineResult`.
#[wasm_bindgen]
pub async fn process(text: String) -> Result<JsValue, JsValue> {
    let pipeline = PiiPipeline::new(PipelineConfig::default()).map_err(to_js_err)?;
    let result = pipeline.process(&text).await.map_err(to_js_err)?;
    serde_wasm_bindgen::to_value(&result).map_err(to_js_err)
}

/// Detect without rewriting `text`. Returns the serialized `PipelineResult` with
/// `redacted_text` equal to the input.
#[wasm_bindgen]
pub async fn scan(text: String) -> Result<JsValue, JsValue> {
    let pipeline = PiiPipeline::new(PipelineConfig::default()).map_err(to_js_err)?;
    let result = pipeline.scan(&text).await.map_err(to_js_err)?;
    serde_wasm_bindgen::to_value(&result).map_err(to_js_err)
}

/// Redact `text` under `mode` ("mask", "hash", "pseudonymize", or "remove") with no
/// detected spans — exercises `RedactionEngine::redact`'s construction path, including
/// the AES-SIV `Pseudonymiser`, without depending on `PiiPipeline` detection.
#[wasm_bindgen]
pub fn redact_empty(text: String, mode: String) -> Result<JsValue, JsValue> {
    let mode: RedactionMode = mode.parse().map_err(to_js_err)?;
    let pseudonymiser = if mode == RedactionMode::Pseudonymize {
        let resolver = EnvKeyResolver::new();
        Some(Arc::new(
            Pseudonymiser::new(&resolver, &[]).map_err(to_js_err)?,
        ))
    } else {
        None
    };
    let config = RedactionConfig {
        mode,
        ..Default::default()
    };
    let engine = RedactionEngine::new(config, pseudonymiser).map_err(to_js_err)?;
    let result = engine.redact(&text, &[]).map_err(to_js_err)?;
    serde_wasm_bindgen::to_value(&result).map_err(to_js_err)
}

/// The browser-side tamper-evident audit chain (Track C3), backed by
/// [`hacienda_core::audit::IndexedDbAuditStore`] (Track L5) — not a second, parallel
/// TypeScript implementation. `wasm32`-only: `IndexedDbAuditStore` doesn't exist on
/// native targets, so this whole item is compiled out there rather than failing to link.
///
/// # What this does not decide
///
/// Persistence here is a browser IndexedDB database — it survives a reload but not a
/// cleared profile. Whether/how a chain gets exported into the vault (Track I2) so it
/// survives that is still open; Track I2 itself is unbuilt. See the plan's C3 and L5
/// entries.
#[cfg(target_arch = "wasm32")]
mod audit_handle {
    use super::to_js_err;
    use hacienda_core::audit::{AuditEntryInput, AuditStore, IndexedDbAuditStore, NodeId};
    use hacienda_core::pii::PipelineResult;
    use wasm_bindgen::prelude::*;

    /// Recorded on every entry this handle mints, mirroring
    /// `HaciendaFacade::PIPELINE_VERSION` — ties a browser-produced record to the code
    /// that made it.
    const PIPELINE_VERSION: &str = env!("CARGO_PKG_VERSION");

    /// One open [`IndexedDbAuditStore`] connection, exposed to JS.
    #[wasm_bindgen]
    pub struct AuditHandle {
        store: IndexedDbAuditStore,
    }

    #[wasm_bindgen]
    impl AuditHandle {
        /// Open (or resume, across a reload — Track L5's check) the IndexedDB database
        /// named `db_name`, scoped to `node_id` and `config_hash`.
        ///
        /// `wasm-bindgen` cannot expose an `async` constructor, so this is a static
        /// factory: `await AuditHandle.open(...)` from JS.
        pub async fn open(
            db_name: String,
            node_id: String,
            config_hash: String,
        ) -> Result<AuditHandle, JsValue> {
            let store = IndexedDbAuditStore::open(db_name, NodeId::new(node_id), config_hash)
                .await
                .map_err(to_js_err)?;
            Ok(AuditHandle { store })
        }

        /// Record one processed document's redactions as a single batch — one `append`
        /// per document, the same invariant `HaciendaFacade::record_audit` enforces, so
        /// a chain built here is structurally identical to one built server-side.
        ///
        /// `result` is the `JsValue` a caller already got back from [`super::process`].
        /// Entries with an empty `audit_log` (nothing was redacted) append nothing and
        /// return the chain's current tip unchanged.
        ///
        /// # Errors
        ///
        /// Rejects if `result` doesn't deserialize as a `PipelineResult`, or if the
        /// store rejects the batch (e.g. the chain was closed).
        #[wasm_bindgen(js_name = recordResult)]
        pub async fn record_result(&self, result: JsValue) -> Result<String, JsValue> {
            let result: PipelineResult =
                serde_wasm_bindgen::from_value(result).map_err(to_js_err)?;

            if !result.audit_log.is_empty() {
                let inputs: Vec<AuditEntryInput> = result
                    .audit_log
                    .into_iter()
                    .map(|entry| AuditEntryInput {
                        id: uuid::Uuid::new_v4().to_string(),
                        category: entry.category,
                        action: entry.action,
                        span_hash: entry.span_hash,
                        span_length: entry.span_length,
                        confidence: entry.confidence,
                        source: entry.source.into(),
                        pipeline_version: PIPELINE_VERSION.to_string(),
                        // The store owns config_hash — AuditChain::push overwrites this
                        // with the chain's own value (chain.rs). No local principal
                        // concept exists in Studio today, so this is always `None`.
                        config_hash: String::new(),
                        principal: None,
                    })
                    .collect();

                self.store.append(inputs).await.map_err(to_js_err)?;
            }

            self.store.tip().await.map_err(to_js_err)
        }

        /// The chain's current head — a client that records this alongside a result can
        /// later prove which chain state produced it.
        pub async fn tip(&self) -> Result<String, JsValue> {
            self.store.tip().await.map_err(to_js_err)
        }

        /// Recompute every chain hash from genesis. Throws (rejects) on the first
        /// mismatch rather than returning a boolean, so a caller can't accidentally
        /// ignore tampering by discarding a return value.
        pub async fn verify(&self) -> Result<(), JsValue> {
            self.store.verify().await.map_err(to_js_err)
        }
    }
}
