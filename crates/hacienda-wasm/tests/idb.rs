//! wasm32 regression guard for Track L5's check: append two audit entries, "reload the
//! page", verify the blake3 chain still verifies across the reload.
//!
//! There's no real browser page-navigation available in this test harness, so "reload"
//! is modeled the way the ecosystem models it for exactly this class of test: drop the
//! store instance and open a fresh one against the same IndexedDB database name. What's
//! actually being verified — that the chain survives past the wasm module's in-memory
//! lifetime, not merely a JS variable — is identical either way; a real browser reload
//! adds a page-navigation event this harness can't produce, not a different persistence
//! guarantee.
//!
//! Requires an IndexedDB implementation in the Node process running this test (Node has
//! none natively) — run with `fake-indexeddb/auto` preloaded, e.g.
//! `NODE_OPTIONS="--require /path/to/node_modules/fake-indexeddb/auto" cargo test
//! --target wasm32-unknown-unknown -p hacienda-wasm --test idb`.
//!
//! wasm32-only: `IndexedDbAuditStore` doesn't exist on native targets (see
//! `hacienda-core/src/audit/mod.rs`), so this whole file is a no-op there rather than a
//! compile error.
#![cfg(target_arch = "wasm32")]

use hacienda_core::audit::{
    AuditEntryInput, AuditStore, EntitySource, IndexedDbAuditStore, NodeId, RedactionAction,
};
use wasm_bindgen_test::*;

fn entry(id: &str, category: &str) -> AuditEntryInput {
    AuditEntryInput {
        id: id.to_string(),
        category: category.to_string(),
        action: RedactionAction::Mask,
        span_hash: format!("blake3-of-{category}"),
        span_length: 12,
        confidence: Some(0.99),
        source: EntitySource::Regex,
        pipeline_version: "test".to_string(),
        config_hash: "l5-reload-guard".to_string(),
        principal: None,
    }
}

#[wasm_bindgen_test]
async fn audit_chain_survives_a_reload_via_indexeddb() {
    let db_name = "hacienda-l5-reload-guard";

    // Session 1: open, append two entries, capture the tip. No `close()` — an
    // unclean tab close is exactly the case durability has to cover, not just the
    // orderly shutdown path.
    let tip_before = {
        let store = IndexedDbAuditStore::open(db_name, NodeId::new("test-node"), "l5-reload-guard")
            .await
            .expect("opening a fresh IndexedDB-backed store must succeed");

        store
            .append(vec![entry("entry-1", "EMAIL"), entry("entry-2", "PHONE")])
            .await
            .expect("appending two entries must succeed");

        let entries = store.entries().await.expect("entries() must succeed");
        assert_eq!(
            entries.len(),
            2,
            "both appended entries must be visible pre-reload"
        );

        store
            .verify()
            .await
            .expect("chain must verify before the reload");
        store.tip().await.expect("tip() must succeed")
    }; // `store` dropped here — nothing more references the IndexedDB connection or the
       // in-memory state; only what was persisted to IndexedDB survives.

    // Session 2 ("reload"): a fresh store, same database name, no memory of session 1's
    // Rust-side state.
    let store = IndexedDbAuditStore::open(db_name, NodeId::new("test-node"), "l5-reload-guard")
        .await
        .expect("reopening the same IndexedDB database must succeed");

    let entries = store
        .entries()
        .await
        .expect("entries() must succeed after reload");
    assert_eq!(
        entries.len(),
        2,
        "both entries appended in session 1 must have survived the reload"
    );
    assert_eq!(entries[0].category, "EMAIL");
    assert_eq!(entries[1].category, "PHONE");

    let tip_after = store.tip().await.expect("tip() must succeed after reload");
    assert_eq!(
        tip_before, tip_after,
        "the chain head must be unchanged by the reload"
    );

    // This is Track L5's actual check: the blake3 chain still verifies after a reload,
    // which for `IndexedDbAuditStore` specifically confirms the replay-through-`Segment
    // ::append` rehydration path (`store_idb.rs`'s `rehydrate`) reconstructs a chain
    // that is bit-for-bit what a continuously-running store would have produced.
    store
        .verify()
        .await
        .expect("blake3 chain must still verify after the reload");
}
