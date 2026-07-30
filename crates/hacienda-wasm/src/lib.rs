//! wasm-bindgen entry points over `hacienda-core`'s `pii`/`redaction` pipeline.
//!
//! Deliberately thin: two functions that exercise the real API surface (regex/merge
//! detection and all four redaction modes, including the AES-SIV pseudonymisation path)
//! so a `wasm32-unknown-unknown` build actually reaches link time against
//! `hacienda-core`'s own code, per Track L2's check. Not yet the production binding set —
//! that lands once L3 (clock/uuid/timestamp fixes) and L5 (IndexedDB stores) are in.

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
