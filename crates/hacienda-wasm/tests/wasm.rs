//! wasm32 regression guard for Track L3: `Instant::now()` panics on
//! `wasm32-unknown-unknown` without a clock seam, `Uuid::new_v4()` fails to compile
//! without a randomness source, and `chrono::Utc::now()` silently returns the epoch if
//! `wasmbind` is ever dropped. All three are exercised through real `hacienda-core` code
//! paths — not mocks — so a regression here is a regression there.
//!
//! Run with `wasm-pack test --node` (or `cargo test --target wasm32-unknown-unknown`
//! with `wasm-bindgen-test-runner` configured as the target runner).

use hacienda_core::audit::{AuditEntryInput, AuditStore, InMemoryAuditStore};
#[cfg(feature = "ner-candle-wasm")]
use hacienda_core::pii::NerDetector;
use hacienda_core::pii::{PiiPipeline, PipelineConfig};
use hacienda_core::tenancy::TenantId;
use uuid::Uuid;
use wasm_bindgen_test::*;

// No `wasm_bindgen_test_configure!` call: node.js is the default execution target for
// `#[wasm_bindgen_test]`, which is exactly what this suite needs.

/// Anything before this is clearly a clock that silently reset to the epoch rather than
/// reading real time (2001-09-09, comfortably after `chrono`/`web-time` could plausibly
/// be broken and still assert non-zero for other reasons).
const NOT_EPOCH: u64 = 1_000_000_000;

#[wasm_bindgen_test]
async fn redaction_round_trip_has_a_real_clock_and_uuid_on_wasm32() {
    // (a)/(c): a real redaction round trip — regex detection then masking — must
    // produce a `RedactionAuditEntry` timestamp from `web_time::SystemTime::now()`
    // (`redaction/engine.rs`), not a panic and not 1970.
    let pipeline =
        PiiPipeline::new(PipelineConfig::default()).expect("default pipeline config must build");
    let result = pipeline
        .process("Contact Jane Doe at jane.doe@example.com for details.")
        .await
        .expect("regex-only processing must not fail");

    assert!(
        !result.audit_log.is_empty(),
        "the builtin email pattern must have matched, producing at least one audit entry"
    );
    for entry in &result.audit_log {
        assert!(
            entry.timestamp > NOT_EPOCH,
            "redaction audit entry timestamp {} looks like it came from a broken wasm32 \
             clock, not `SystemTime::now()`",
            entry.timestamp
        );
    }
    assert_ne!(
        result.redacted_text, "Contact Jane Doe at jane.doe@example.com for details.",
        "the detected email should have been masked"
    );

    // (b): a real in-memory audit store mints a segment id via `Uuid::new_v4()`
    // (`audit/segment.rs`) and an `opened_at` timestamp via `chrono::Utc::now()`
    // (also `audit/segment.rs`) as soon as it opens — both must work on wasm32.
    let store = InMemoryAuditStore::new("wasm32-l3-regression-guard".to_string());
    let inputs: Vec<AuditEntryInput> = result
        .audit_log
        .iter()
        .enumerate()
        .map(|(i, entry)| AuditEntryInput {
            id: format!("entry-{i}"),
            category: entry.category.clone(),
            action: entry.action.clone(),
            span_hash: entry.span_hash.clone(),
            span_length: entry.span_length,
            confidence: entry.confidence,
            source: entry.source.into(),
            pipeline_version: "test".to_string(),
            config_hash: "wasm32-l3-regression-guard".to_string(),
            principal: None,
            vertical: None,
        })
        .collect();
    let tenant = TenantId::default_tenant();
    store
        .append(&tenant, inputs)
        .await
        .expect("appending the redaction's own audit entries must succeed");

    let seal = store
        .close(&tenant)
        .await
        .expect("closing the store must seal its segment");

    let segment_uuid =
        Uuid::parse_str(&seal.segment_id).expect("segment_id must be a real uuid v4 string");
    assert!(
        !segment_uuid.is_nil(),
        "segment id came back nil — uuid's wasm32 randomness source produced no entropy"
    );

    let opened_at = chrono::DateTime::parse_from_rfc3339(&seal.opened_at)
        .expect("opened_at must be a valid RFC3339 timestamp");
    assert!(
        opened_at.timestamp() as u64 > NOT_EPOCH,
        "segment opened_at {} looks like the wasm32 epoch fallback, not `Utc::now()`",
        seal.opened_at
    );
}

/// `NerDetector::from_candle_bytes` (the wasm32 sibling of `from_candle_local`, loading
/// the Candle GLiNER2 backend from in-memory bytes instead of a filesystem model
/// directory) must at least *link and run* on wasm32 and reject cleanly on bad input.
/// Real GLiNER2 weights are ~600MB — infeasible as a CI fixture — so this only exercises
/// the error path; model-accuracy verification with real weights is manual (see the
/// 2026-08-09 wasm NER plan).
#[cfg(feature = "ner-candle-wasm")]
#[wasm_bindgen_test]
fn from_candle_bytes_rejects_garbage_input_on_wasm32() {
    let result = NerDetector::from_candle_bytes(&[], &[], &[]);
    assert!(
        result.is_err(),
        "empty/garbage model bytes must fail to load, not silently produce a usable detector"
    );
}
