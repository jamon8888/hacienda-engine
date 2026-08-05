---
description: PII detection, redaction, and reversible pseudonymization for legal documents
---

- Pipeline: regex engine + optional NER detection → merge into non-overlapping spans → redaction engine → audit chain (hacienda-core/src/pii/pipeline.rs)
- Redaction modes: mask, hash, pseudonymize, remove — pseudonymize is the only reversible mode
- NER backend: `Arc<dyn NerBackend>` abstraction over Xberg's Candle GLiNER2 backend, with optional per-vertical LoRA adapter loaded at model-load time
- Pseudonymization: AES-256-SIV (authenticated, deterministic) keyed by `HACIENDA_PSEUDONYM_ACTIVE_KEY` + retired keys; tokens are `[CATEGORY:key_id:base32_ciphertext]`
- Deterministic co-reference: values are NFKC-normalized (case, whitespace, digit-only for phone numbers) before pseudonymizing so equivalent mentions get the same token
- Crate layout: `hacienda` is a distribution facade re-exporting `hacienda-core` + `xberg`; `hacienda-core` holds the pipeline; `hacienda-api` and `hacienda-cli` are thin consumers; `crates/hacienda-wasm` exposes the same pipeline to the browser
- API surface: Axum REST only (no gRPC) — `POST /v1/documents(/async)`, `GET /v1/jobs/{id}`, `POST /v1/pii/scan`, `POST /v1/pii/redact`, `GET /v1/pii/config`; document input is base64 inline bytes only, never a wire-supplied path/URL (SSRF prevention)
- CLI surface: `extract` (mask/hash/pseudonymize), `scan` (detect-only, no rewrite), `config show`, `serve` (loopback-only unless auth enabled); audit/review/compliance/glossary commands are deliberately unbuilt, not stubbed
- Browser delivery: `apps/hacienda-studio` (React + Vite + shadcn + CodeMirror) calls `hacienda-wasm` directly, not the REST API; audit trail persists via IndexedDB (`idb`) and does not yet export into the vault
- Training data pipeline: `labeling/` (taxonomy gate, offset resolver, 3x self-consistency voting) and `dataset/assemble.py` (char→word-span conversion with round-trip assertion) produce GLiNER2 LoRA training data per vertical taxonomy (e.g. `business_law`)
