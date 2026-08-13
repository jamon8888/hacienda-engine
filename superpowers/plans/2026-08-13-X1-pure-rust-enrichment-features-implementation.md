# X1 — Pure-Rust enrichment features — Implementation plan

**Spec:** `superpowers/specs/2026-08-13-X1-pure-rust-enrichment-features.md`
**Program:** `superpowers/specs/2026-08-13-hacienda-xberg-capability-parity-program.md` §5
**Hard prerequisite:** `superpowers/specs/2026-08-13-P7-structured-field-redaction-gap.md`
must ship (b) — recursive redaction of `ExtractedDocument`'s structured fields — before Task
3 (`keywords`) lands. Tasks 1, 2, 4, 5, 6 do not touch a field of that shape and are not
blocked on P7.

**Baseline:** `cargo test -p hacienda-core -p hacienda-api -p hacienda-cli -p hacienda
-p hacienda-mcp`, 2026-08-13: 366 passed, 5 pre-existing environmental failures (root-in-
container defeats `chmod`-based write-failure-injection tests; no live Postgres for
`connect_and_migrate`) — not regressions, do not attempt to fix them here.

---

## Ground truth, verified against the pinned xberg commit's source

| Fact | Location |
| --- | --- |
| Six features need no ONNX Runtime, no native library: `keywords` (`keywords-yake`+`keywords-rake`), `summarization`, `qr-codes`, `tree-sitter`, `static-embeddings`, `candle-ocr` | `crates/xberg/Cargo.toml` feature block, read directly |
| `hacienda-core`'s only extraction call site is `extract_all` → `xberg::extract_batch`/`xberg::extract` | `hacienda-core/src/facade.rs:1181-1188` |
| The PII stage rewrites exactly one field: `document.content = result.redacted_text.clone()` | `hacienda-core/src/facade.rs:702` |
| `ExtractedDocument.extracted_keywords: Option<Vec<Keyword>>` exists as a field **separate** from `content`, gated `#[cfg(any(feature = "keywords-yake", feature = "keywords-rake"))]` | `crates/xberg/src/types/extraction.rs:189-196` |
| `ExtractedDocument.summary: Option<DocumentSummary>` is likewise separate from `content` | `crates/xberg/src/types/extraction.rs:301-308` |
| hacienda already vendors candle (for `ner-candle`, `hacienda-core`'s own feature enabling `xberg/ner-candle`) | `hacienda-core/Cargo.toml:10-11` |
| `candle-ocr`'s four backend sub-features are independent aggregates: `candle-trocr`, `candle-paddleocr-vl`, `candle-glm-ocr` (needs `layout-detection` too), `candle-deepseek-ocr` | `crates/xberg/Cargo.toml`, `candle-*` feature block |
| `static-embeddings` needs `hf-hub`/`reqwest` (one-time model download) but not `ort`/`ndarray`-via-ONNX — pure `model2vec-rs` inference | `crates/xberg/Cargo.toml`, `static-embeddings` feature |
| E4's vector-store guard (redact-before-embed) is documented in the platform-parity program but not yet verified against a second embedder | `2026-08-01-hacienda-platform-parity-program.md` §6 E4 |

---

## Task 1 — `summarization`, `qr-codes`, `tree-sitter`: enable and verify PII-stage coverage

- [ ] Add `"summarization"`, `"qr-codes"`, `"tree-sitter"` to the workspace root `Cargo.toml`'s
      `xberg` dependency feature list, alongside the existing `pdf`/`office`/... entries.
      Extend that entry's comment with the same "confirmed pure Rust against xberg's own
      Cargo.toml" rationale already used for the extraction formats.
- [ ] `cargo check -p hacienda-core -p hacienda-api -p hacienda-cli -p hacienda-mcp`. Fix any
      compile fallout (expect none, matching the extraction-format precedent).
- [ ] Confirm `summarization`'s output lands in `ExtractedDocument.summary`, not `.content` —
      it is **not** rewritten by `facade.rs:702` today. If P7(b) has not shipped yet, do not
      enable `summarization` in production config; leave the Cargo feature compiled-in but
      unexercised (no `ExtractionConfig` flag turns it on) until P7(b) covers `.summary`.
      Record this explicitly in the PR description — do not silently defer it without saying
      so, matching the program's "degradation must be announced" precedent (D3, V2).
- [ ] `qr-codes`: a control-corpus test — encode a corpus email into a QR code image, extract
      it, assert the decoded payload appears redacted in the same fields `.content` already
      is (verify where xberg surfaces decoded QR payloads — likely `metadata.additional` or
      an `elements` entry; confirm against source before writing the test, don't assume).
- [ ] `tree-sitter`: confirm it's a passive capability (enables syntax-aware content
      formatting) with no new top-level field of its own; if it does add one, apply the same
      P7 gating decision as `summarization`.

## Task 2 — `static-embeddings`: enable, verify E4's guard covers it

- [ ] Add `"static-embeddings"` to the `xberg` feature list.
- [ ] Read `hacienda-rag`'s current embedding call site(s) (`crates/hacienda-rag/src/`) to
      confirm where an embedder is invoked relative to the redaction step E4 requires.
- [ ] If E4's guard is written generically (any embedder must receive already-redacted
      text) — confirm `static-embeddings` is reachable only through that path once wired.
      If E4's guard is written ONNX-`embeddings`-specific (or doesn't exist as enforced code
      yet) — this task's exit criterion becomes "extend the guard to be embedder-agnostic
      before wiring `static-embeddings` in," not "add a second copy of the check."
- [ ] Do not wire `static-embeddings` into `hacienda-rag`'s default embedding path in this
      task — enabling the Cargo feature and confirming the guard question is the scope here;
      making it the product's embedder is a follow-up product decision.

## Task 3 — `keywords`: blocked on P7(b)

- [ ] **Do not start this task until `superpowers/specs/2026-08-13-P7-structured-field-redaction-gap.md`'s
      exit criteria are met** — specifically, recursive redaction of `ExtractedDocument`'s
      structured fields, `extracted_keywords` included.
- [ ] Once unblocked: add `"keywords"` to the `xberg` feature list.
- [ ] Control-corpus test: a document whose most keyword-like phrase is a corpus PII value
      (e.g. a name used densely enough that YAKE/RAKE would surface it) — assert the value is
      redacted in `extracted_keywords`, not just absent from `.content`.

## Task 4 — `candle-ocr` (TrOCR backend only, for this pass)

- [ ] Add `"candle-trocr"` (not the full `candle-vlm-ocr` umbrella) to the `xberg` feature
      list — the smallest, best-understood backend, per the spec's §3 rationale.
- [ ] Confirm the model download path (`hf-hub`) works in the target deployment environment
      before treating this as done — this is the one feature in this plan with a runtime
      network dependency (a one-time HF fetch, cached thereafter), even though it needs no
      system library or ONNX Runtime.
- [ ] Control-corpus test: a scanned image (not a text layer) containing a corpus PII value,
      run through `hacienda extract` / `POST /v1/documents` / `hacienda-mcp`'s
      `documents_process` — assert the value is redacted in `.content` (OCR output lands
      there, same as any other extractor) in all three transports, reusing the shared test
      helper `2026-08-13-P7-structured-field-redaction-gap.md`'s exit criteria call for.

## Task 5 — Cross-transport verification

- [ ] Run the control-corpus tests from Tasks 1–4 against all three transports that share
      `HaciendaResult`: `hacienda extract` (CLI), `POST /v1/documents` (API), `documents_process`
      (hacienda-mcp) — one shared test fixture/helper, exercised three times, not three
      separate hand-written tests that can drift.

## Task 6 — Documentation

- [ ] `CHANGELOG.md` entry, matching the style of the existing extraction-format entry:
      which features were enabled, what stayed off and why (the ONNX/native/LLM-gated
      features, deferred to X2/X3), and the P7 dependency for `keywords`/`summarization`.
- [ ] Update `2026-08-13-hacienda-xberg-capability-parity-program.md`'s wave table if this
      task's actual delivery order diverged from the plan (e.g. `keywords` shipping in a
      later PR than the rest, once P7 lands).

---

## Exit criteria (recap from the spec)

- `cargo check` clean with all six features (candle-trocr only for `candle-ocr`).
- Every control-corpus test in Tasks 1–4 passes across all three transports (Task 5).
- `keywords` and `summarization` are either gated behind P7(b) shipping first, or explicitly
  left uncompiled/unconfigured with that reason stated in the PR — never shipped exposing an
  unredacted structured field.
