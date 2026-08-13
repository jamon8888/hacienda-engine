# X2 — ONNX and native-backed enrichment features

**Date:** 2026-08-13
**Status:** Proposed
**Program:** `2026-08-13-hacienda-xberg-capability-parity-program.md` §6
**Depends on:** `2026-08-13-P7-structured-field-redaction-gap.md` for `transcription` and
`layout-detection`/`layout-tract` (extraction-time, same class of field-coverage risk as X1);
not blocked for `embeddings`/`reranker`/`sparse-embeddings`/`late-interaction` (retrieval-time,
governed by E4's guard, not P1/P7's)
**Blocks:** nothing

---

## 1. Problem

The remaining xberg extraction/retrieval capabilities need real infrastructure — ONNX
Runtime, a native system library, or both — not just a Cargo feature flip. This is a
provisioning decision as much as a code change, and each row below carries a different
infra cost:

| Feature | Infra needed | What it produces |
| --- | --- | --- |
| `embeddings` | ONNX Runtime + HF model download | Full-fidelity dense embeddings (vs. X1's `static-embeddings`) |
| `reranker` | ONNX Runtime + HF model | Cross-encoder reranking of retrieved chunks |
| `sparse-embeddings` | ONNX Runtime + HF model | SPLADE sparse vectors, hybrid dense+sparse retrieval |
| `late-interaction` | ONNX Runtime + HF model | ColBERT multi-vector embeddings, MaxSim retrieval |
| `layout-detection` | ONNX Runtime + HF model | Table/reading-order detection (YOLO/RT-DETR) |
| `layout-tract` | Pure-Rust `tract` engine + HF model, **no ORT** | Same, cheaper infra, but per xberg's own comment loses TATR/SLANeXT table-structure recognition and hits a `tract` op-translation bug on PP-DocLayout-V3 |
| `transcription` | ONNX Runtime (Whisper) + HF model + `symphonia`/`lofty` (pure-Rust audio decode) | Audio/video → text |
| `ocr` (Tesseract) | Native Tesseract binary | The heavier-infra sibling of X1's `candle-ocr`; only relevant if candle's backends miss a format/language Tesseract covers |

## 2. Objectives / Non-objectives

**Objectives**

- One explicit ORT-strategy decision (§4), made once, applied to every ONNX-backed row —
  not one ad hoc choice per feature.
- For `transcription` and `layout-detection`/`layout-tract` (extraction-time, feeding
  `ExtractedDocument`): confirmed PII-stage coverage, gated on P7(b) the same way X1's
  `keywords`/`summarization` are, since layout detection populates structured fields
  (`document`, the structure tree) of the same shape P7 exists to guard.
- For `embeddings`/`reranker`/`sparse-embeddings`/`late-interaction` (retrieval-time): confirmed
  they only ever run on already-redacted/pseudonymised text, via E4's existing vector-store
  guard — the same verification X1 Task 2 does for `static-embeddings`, extended to these.

**Non-objectives**

| Deferred | Reason |
| --- | --- |
| Model/backend selection as a product-facing config surface | Infra enablement only |
| GPU acceleration (`candle-cuda`/`candle-metal`/`candle-accelerate`/`candle-mkl`) | Deployment-target decision, orthogonal to whether the capability exists |
| `ocr` (Tesseract) unless a concrete gap in `candle-ocr`'s coverage is found | Prefer the pure-Rust path (X1) where it's sufficient; this row is a fallback, not a default |

## 3. Extraction-time vs. retrieval-time: two different guards apply

`transcription` and `layout-detection`/`layout-tract` feed `xberg::extract_batch` the same
way `pdf` does — their output becomes part of `ExtractedDocument`, which means P7's
structured-field guard governs them, not a new mechanism. Spoken PII in a transcript, or a
table structure lifted by layout detection, is exactly the shape of leak P7 exists to close;
these two features must not ship ahead of P7(b) landing, same rule as X1's `keywords`.

`embeddings`/`reranker`/`sparse-embeddings`/`late-interaction` are different in kind: they run
at retrieval time, over chunks a caller submits or the system has already redacted, not at
extraction time over raw document content. The relevant guard is E4's (`2026-08-01-hacienda-platform-parity-program.md`
§6, E4's "garde P1 — c'est ici qu'il compte le plus" clause): a vector index built from
unredacted text is retrievable by similarity, which is a worse leak than a single API
response, because it's queryable indefinitely. This spec's job for these four features is
the same as X1 Task 2's for `static-embeddings`: confirm the guard is generic across
embedders/rerankers, not written against one ONNX model in particular.

## 4. Decision to settle before implementation: ORT strategy

**`ort-bundled`** downloads a prebuilt ONNX Runtime at build/first-run time — needs glibc ≥
2.38 to run, per xberg's own feature comment. **`ort-dynamic`** loads a system-provided
runtime at runtime instead. xberg's own `publish.yaml` uses bundled for dev builds and
system-linked for release, explicitly to avoid shipping a glibc floor requirement into a
release artifact. hacienda should make the same choice deliberately rather than inheriting
whichever a feature happens to default to, since it affects every deployment target
(container base image, glibc version) this program has not otherwise had to think about.

**Recommendation:** bundled for development/CI (matches xberg's own dev-build choice, needs
no system provisioning), system-linked (`ort-dynamic`) for anything shipped as a release
artifact or container image — mirroring xberg's own precedent exactly rather than inventing a
different policy. Record the chosen `ORT_LIB_LOCATION`/build strategy in the same Cargo.toml
comment style the `xberg` dependency entry already uses for its other decisions.

## 5. Per-row notes

**`layout-tract` over `layout-detection`, provisionally.** No ORT dependency at all — cheaper
infra, one fewer moving part. Accept the documented limitation (no TATR/SLANeXT table
structure, a `tract` bug on PP-DocLayout-V3) unless a concrete document in hacienda's actual
target mix needs what only `layout-detection` provides. Don't enable both — xberg's own
comment warns against it ("never enable both in one build").

**`ocr` (Tesseract) stays off by default.** X1 already ships `candle-ocr` (pure Rust, no
native binary). Add Tesseract only if a specific format/language gap is found that candle's
backends (TrOCR, PaddleOCR-VL, GLM-OCR, DeepSeek-OCR) don't cover — evaluate empirically
against real target documents before adding a native system dependency.

## 6. Exit criteria

- One documented ORT-strategy decision (§4), applied consistently across every ONNX-backed
  feature enabled in this spec — not a per-feature ad hoc choice.
- `transcription`/`layout-detection`/`layout-tract`: pass the same control-corpus PII test
  class as X1 (a value in a test audio/document input is redacted in every transport's
  output, structured fields included) — blocked on P7(b), same as X1's `keywords`.
- `embeddings`/`reranker`/`sparse-embeddings`/`late-interaction`: E4's vector-store guard is
  confirmed (or extended, if it was written ONNX-`embeddings`-specific) to cover all four
  generically, verified with at least a second embedder/reranker to prove it isn't
  accidentally hardcoded to one model's call site.
- `cargo check` clean with the chosen ORT strategy for every row enabled in a given pass.

## 7. Sequencing

Independent of X1/X3/X4. Gate on §4's ORT-strategy decision first; extraction-time rows
(`transcription`, `layout-*`) additionally gate on P7(b); retrieval-time rows
(`embeddings`/`reranker`/`sparse-embeddings`/`late-interaction`) do not. Proceed per-row in
whatever order matches product priority once those gates clear — no row here blocks another.
