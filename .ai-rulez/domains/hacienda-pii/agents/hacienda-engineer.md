---
name: hacienda-engineer
description: Hacienda PII detection, redaction, pseudonymization, and vertical NER training pipeline
model: sonnet
---

When working on the Hacienda PII subsystem:

1. Key source paths: `hacienda-core/src/pii/` (pipeline.rs, ner.rs, pseudonym.rs), `hacienda-api/src/` (lib.rs, routes.rs, dto.rs), `hacienda-cli/src/`, `hacienda/src/lib.rs` (facade), `crates/hacienda-wasm/src/lib.rs`
2. Pipeline order is fixed: regex + optional NER → merge non-overlapping spans → redaction → audit chain. Never reorder.
3. Only `pseudonymize` is reversible (AES-256-SIV, keyed by `HACIENDA_PSEUDONYM_ACTIVE_KEY`). Mask/hash/remove are one-way — treat them as such in tests and docs.
4. Vertical specialization is GLiNER2 base + LoRA adapter merged at load time. Training data flows: `labeling/taxonomy_gate.py` → `labeling/offset_resolver.py` → `labeling/consistency.py` (≥2/3 vote) → `dataset/assemble.py` (word-span assembly with round-trip assertion).
5. Taxonomy YAML (e.g. `apps/hacienda-studio/lib/verticals/business_law.yaml`) is shared source of truth between labeling pipeline and Studio frontend — changes must land in both consumers together.
6. `hacienda-api` is REST/Axum only, base64 inline document input, capability-guarded routes, in-memory job store (durable backend unbuilt).
7. `hacienda-cli` phases beyond extract/scan/config/serve (audit, review, compliance, glossary) are deliberately unbuilt — don't stub them to look functional.
8. `apps/hacienda-studio` calls `hacienda-wasm` directly, not `hacienda-api` — keep business logic in Rust, Studio stays a thin React client.
9. Check `superpowers/specs/` and `superpowers/plans/` for design intent, but verify against actual code and tests before trusting a spec as current — specs in this repo are known to drift ahead of implementation.
10. Always run the relevant crate's tests after changing pipeline or pseudonymization behavior — `pii_corpus.rs` and `lora_adapter_contract.rs` are the load-bearing regression tests.
