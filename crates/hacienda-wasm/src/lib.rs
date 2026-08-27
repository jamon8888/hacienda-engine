//! wasm-bindgen entry points over `hacienda-core`'s `pii`/`redaction` pipeline.
//!
//! `process`/`scan`/`redact_empty` exercise the real API surface (regex/merge detection
//! and all four redaction modes, including the AES-SIV pseudonymisation path) so a
//! `wasm32-unknown-unknown` build actually reaches link time against `hacienda-core`'s
//! own code, per Track L2's check. `AuditHandle` (Track C3, wasm32-only) is the
//! production audit-chain binding L3 (clock/uuid/timestamp fixes) and L5 (IndexedDB
//! stores) were building toward — it wraps `IndexedDbAuditStore` directly rather than
//! re-implementing the chain in TypeScript.

use hacienda_core::pii::{ModelEntity, PiiPipeline, PipelineConfig};
use hacienda_core::redaction::pseudonym::{EnvKeyResolver, Pseudonymiser};
use hacienda_core::redaction::{RedactionConfig, RedactionEngine, RedactionMode};
use hacienda_core::tenancy::TenantId;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

fn to_js_err<E: std::fmt::Display>(err: E) -> JsValue {
    JsValue::from_str(&err.to_string())
}

/// Regex + NER (when a model is loaded, via [`ner_model::load_ner_model`]) detection and
/// redaction over `text`, using the default pipeline config. Returns the serialized
/// `PipelineResult`. `with_detector` (rather than `new`) is used unconditionally: it
/// ignores `PipelineConfig::model.enabled` and just uses whatever `Option<NerDetector>`
/// it's handed, so this is identical to today's regex-only behaviour whenever no model
/// is loaded (or the `ner-candle-wasm` feature is off) — no `#[cfg]` needed here.
#[wasm_bindgen]
pub async fn process(text: String) -> Result<JsValue, JsValue> {
    let pipeline = PiiPipeline::with_detector(PipelineConfig::default(), current_ner_detector())
        .map_err(to_js_err)?;
    let result = pipeline.process(&text).await.map_err(to_js_err)?;
    serde_wasm_bindgen::to_value(&result).map_err(to_js_err)
}

/// Detect without rewriting `text`. Returns the serialized `PipelineResult` with
/// `redacted_text` equal to the input.
#[wasm_bindgen]
pub async fn scan(text: String) -> Result<JsValue, JsValue> {
    let pipeline = PiiPipeline::with_detector(PipelineConfig::default(), current_ner_detector())
        .map_err(to_js_err)?;
    let result = pipeline.scan(&text).await.map_err(to_js_err)?;
    serde_wasm_bindgen::to_value(&result).map_err(to_js_err)
}

/// Detect and redact using pre-computed model entities (bypasses NER inference).
/// Takes pre-computed model entities as a JS array of objects with fields:
/// category, text, start, end, confidence.
#[wasm_bindgen]
pub async fn process_with_model_entities(
    text: String,
    model_entities: JsValue,
) -> Result<JsValue, JsValue> {
    let pipeline = PiiPipeline::with_detector(PipelineConfig::default(), current_ner_detector())
        .map_err(to_js_err)?;
    let model_entities: Vec<ModelEntity> =
        serde_wasm_bindgen::from_value(model_entities).map_err(to_js_err)?;
    let result = pipeline
        .process_with_model_entities(&text, model_entities)
        .await
        .map_err(to_js_err)?;
    serde_wasm_bindgen::to_value(&result).map_err(to_js_err)
}

/// Detect without rewriting `text`, using pre-computed model entities (bypasses NER inference).
#[wasm_bindgen]
pub async fn scan_with_model_entities(
    text: String,
    model_entities: JsValue,
) -> Result<JsValue, JsValue> {
    let pipeline = PiiPipeline::with_detector(PipelineConfig::default(), current_ner_detector())
        .map_err(to_js_err)?;
    let model_entities: Vec<ModelEntity> =
        serde_wasm_bindgen::from_value(model_entities).map_err(to_js_err)?;
    let result = pipeline
        .scan_with_model_entities(&text, model_entities)
        .await
        .map_err(to_js_err)?;
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
        // A browser-side SDK instance has no multi-tenant server context of its own —
        // the default tenant is the only one that makes sense here, and resolves
        // identically to the pre-P3a, untenanted lookup (see `EnvKeyResolver`'s doc).
        Some(Arc::new(
            Pseudonymiser::new(&resolver, &TenantId::default_tenant(), &[]).map_err(to_js_err)?,
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

/// Real (Candle GLiNER2) NER for `process`/`scan`, loaded from in-memory bytes rather
/// than a filesystem model directory — see `hacienda_core::pii::NerDetector::
/// from_candle_bytes`. `wasm32` + `ner-candle-wasm`-only: without the feature, `process`/
/// `scan` stay regex-only exactly as before this was added (see `current_ner_detector`'s
/// fallback below).
#[cfg(all(target_arch = "wasm32", feature = "ner-candle-wasm"))]
mod ner_model {
    use super::to_js_err;
    use hacienda_core::pii::NerDetector;
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;

    thread_local! {
        static DETECTOR: RefCell<Option<NerDetector>> = RefCell::new(None);
    }

    /// Load the model `process`/`scan` will use from now on, replacing any previously
    /// loaded one. Bytes are already fully in memory (Studio fetches and IndexedDB-caches
    /// them once, shared with its own separate entity-glossary NER pass) — this is
    /// synchronous, no I/O happens here.
    ///
    /// # Errors
    ///
    /// Throws if the model bytes cannot be loaded — this function is synchronous (per its
    /// own doc above), so a JS caller sees a thrown exception, not a rejected Promise.
    #[wasm_bindgen(js_name = loadNerModel)]
    pub fn load_ner_model(
        weights: Vec<u8>,
        tokenizer: Vec<u8>,
        encoder_config: Vec<u8>,
    ) -> Result<(), JsValue> {
        let detector = NerDetector::from_candle_bytes(&weights, &tokenizer, &encoder_config)
            .map_err(to_js_err)?;
        DETECTOR.with(|d| *d.borrow_mut() = Some(detector));
        Ok(())
    }

    #[wasm_bindgen(js_name = isNerModelLoaded)]
    pub fn is_ner_model_loaded() -> bool {
        DETECTOR.with(|d| d.borrow().is_some())
    }

    pub(crate) fn current() -> Option<NerDetector> {
        DETECTOR.with(|d| d.borrow().clone())
    }
}

#[cfg(all(target_arch = "wasm32", feature = "ner-candle-wasm"))]
use ner_model::current as current_ner_detector;

/// `process`/`scan`'s regex-only fallback: no model, on any build without both
/// `target_arch = "wasm32"` and the `ner-candle-wasm` feature enabled.
#[cfg(not(all(target_arch = "wasm32", feature = "ner-candle-wasm")))]
fn current_ner_detector() -> Option<hacienda_core::pii::NerDetector> {
    None
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
    use hacienda_core::audit::{
        AuditEntryInput, AuditStore, EntitySource, IndexedDbAuditStore, NodeId, RedactionAction,
    };
    use hacienda_core::pii::PipelineResult;
    use hacienda_core::tenancy::TenantId;
    use std::str::FromStr;
    use wasm_bindgen::prelude::*;

    /// Recorded on every entry this handle mints, mirroring
    /// `HaciendaFacade::PIPELINE_VERSION` — ties a browser-produced record to the code
    /// that made it.
    const PIPELINE_VERSION: &str = env!("CARGO_PKG_VERSION");

    /// Page size [`AuditHandle::list_entries`] pulls `AuditStore::history` in. Only a
    /// round-trip granularity, not a cap: `list_entries` pages to exhaustion.
    const HISTORY_PAGE_SIZE: usize = 512;

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
                        // Studio hardcodes `PipelineConfig::default()` (no config
                        // surface exists in the browser today), so no vertical is ever
                        // configured here either.
                        vertical: None,
                    })
                    .collect();

                self.store
                    .append(&TenantId::default_tenant(), inputs)
                    .await
                    .map_err(to_js_err)?;
            }

            self.store
                .tip(&TenantId::default_tenant())
                .await
                .map_err(to_js_err)
        }

        /// Record that `revealed_text` was shown to the user in plaintext — the wasm
        /// counterpart of `RedactionAction::Reveal` (see that variant's doc: an audit
        /// chain that omits "who accessed the unredacted span text" is not credible for a
        /// compliance product). Only the blake3 digest of `revealed_text` is ever hashed
        /// into the chain — the plaintext itself is never stored, matching
        /// `RedactionEngine::redact`'s own `span_hash` convention (`redaction/engine.rs`).
        ///
        /// `source` is `"regex"` or `"model"`, matching `PiiEntity.source` on the JS side.
        ///
        /// # `principal: None`, not a caller-supplied identity
        ///
        /// `AuditEntry::principal` exists precisely to answer "who did this" (see its own
        /// doc comment), and a reveal is the one action here where that question matters
        /// most. It is `None` below anyway, consistently with `record_result` above and
        /// with every entry this module ever writes: Studio has no account system to
        /// authenticate against at all — "Pas de compte, pas de stockage serveur" is the
        /// product's own stated design, not an omission (`App.tsx`'s landing copy).
        ///
        /// Accepting an unauthenticated, UI-supplied string here (a name typed into a
        /// field, say) would not close that gap — it would launder it: the chain would
        /// *look* attributed while recording whatever the same browser tab that revealed
        /// the plaintext also chose to claim, which reduces to no attribution at all
        /// wearing a costume. Real attribution needs a session-authenticated `Caller`
        /// (`hacienda-core`'s auth module already has that concept server-side) threaded
        /// through from an identity Studio does not currently have. That is a product
        /// decision — whether Studio grows accounts — not a wasm-binding fix, so it is
        /// left as `None` deliberately rather than filled with something that only looks
        /// like an answer.
        #[wasm_bindgen(js_name = recordReveal)]
        pub async fn record_reveal(
            &self,
            revealed_text: String,
            category: String,
            source: String,
        ) -> Result<String, JsValue> {
            let source = EntitySource::from_str(&source).map_err(to_js_err)?;
            let span_hash = blake3::hash(revealed_text.as_bytes()).to_hex().to_string();
            let input = AuditEntryInput {
                id: uuid::Uuid::new_v4().to_string(),
                category,
                action: RedactionAction::Reveal,
                span_hash,
                span_length: revealed_text.len() as u32,
                confidence: None,
                source,
                pipeline_version: PIPELINE_VERSION.to_string(),
                config_hash: String::new(),
                principal: None,
                vertical: None,
            };

            self.store
                .append(&TenantId::default_tenant(), vec![input])
                .await
                .map_err(to_js_err)?;

            self.store
                .tip(&TenantId::default_tenant())
                .await
                .map_err(to_js_err)
        }

        /// Every entry recorded so far for the default tenant, oldest first — backs
        /// `DocumentDetail.tsx`'s Audit tab entry list.
        ///
        /// Pages through [`AuditStore::history`], **not** `AuditStore::entries`. The two
        /// are not interchangeable: `entries` reports only the currently-open segment, so
        /// once a rotation has happened it answers "what was recorded since the last
        /// rotation" while looking like it answers "what was recorded". `history`'s own
        /// doc comment names the consequence — an auditor concluding that events which
        /// exist never happened — and that is exactly what an Audit tab built on `entries`
        /// would show.
        ///
        /// Paged to exhaustion here rather than exposing a cursor to JS: the caller is one
        /// browser-local tab rendering its own chain, and `IndexedDbAuditStore` already
        /// holds the segment in memory, so there is no page the UI could usefully defer.
        /// A JS-side cursor API is the right shape if this ever backs a server-side chain.
        #[wasm_bindgen(js_name = listEntries)]
        pub async fn list_entries(&self) -> Result<JsValue, JsValue> {
            let tenant = TenantId::default_tenant();
            let mut all: Vec<hacienda_core::audit::AuditEntry> = Vec::new();
            let mut cursor: Option<hacienda_core::audit::AuditCursor> = None;

            loop {
                let page = self
                    .store
                    .history(&tenant, cursor.as_ref(), HISTORY_PAGE_SIZE)
                    .await
                    .map_err(to_js_err)?;
                // `history` never reports "finished" — a non-empty page always yields a
                // cursor (see `AuditPage::next`), so an empty page is the only stop
                // condition, and `next: None` on a non-empty page would be a store bug
                // rather than the end of the chain. Break on either instead of looping
                // forever if one ever occurs.
                if page.entries.is_empty() {
                    break;
                }
                all.extend(page.entries);
                match page.next {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }

            serde_wasm_bindgen::to_value(&all).map_err(to_js_err)
        }

        /// The chain's current head — a client that records this alongside a result can
        /// later prove which chain state produced it.
        pub async fn tip(&self) -> Result<String, JsValue> {
            self.store
                .tip(&TenantId::default_tenant())
                .await
                .map_err(to_js_err)
        }

        /// Recompute every chain hash from genesis. Throws (rejects) on the first
        /// mismatch rather than returning a boolean, so a caller can't accidentally
        /// ignore tampering by discarding a return value.
        pub async fn verify(&self) -> Result<(), JsValue> {
            self.store
                .verify(&TenantId::default_tenant())
                .await
                .map_err(to_js_err)
        }
    }
}
