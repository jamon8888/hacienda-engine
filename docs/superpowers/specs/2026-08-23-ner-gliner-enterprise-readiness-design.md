# NER GLiNER2 Backend — Enterprise Readiness Design Specification

**Date**: 2026-08-23
**Status**: Proposed — ready for implementation planning
**Author**: investigation + spec by Claude, from a live code audit with the project owner
**Scope**: `hacienda-core`'s NER path (`src/pii/ner.rs`, `src/pii/pipeline.rs`), its consumers
(`crates/hacienda-wasm`, `apps/hacienda-studio`), and the evaluation/provenance machinery
around them. Does **not** propose changes to the GLiNER2 model architecture or retraining.

---

## 0. Summary

The GLiNER2 backend produces correct results on short inputs and silently incomplete results
on everything else. The headline defect: **any document longer than roughly 200–300 words has
its tail silently dropped before inference**, with no error, no warning, and no signal on the
result. Because the regex engine has no such limit, the output still looks thoroughly
redacted — regex-detectable PII (emails, IBANs, cards) is found throughout, while
model-detectable PII (names, organisations, addresses) vanishes after the first page.

For a compliance product, silent partial redaction is a worse failure than a hard error: the
operator has no way to know the document was only half-analysed.

This spec defines five workstreams to close that and the surrounding enterprise gaps:

| # | Workstream | Severity | Fixes |
|---|---|---|---|
| 1 | Windowed inference | **P0** | Silent truncation past the 512-token encoder limit |
| 2 | Truncation observability | **P0** | No signal that a document was partially analysed |
| 3 | Quality gate in CI | **P0** | No accuracy baseline, no regression detection |
| 4 | Model provenance & pinning | **P1** | Unpinned, unverified weights; no model identity in the audit chain |
| 5 | Threshold plumbing | **P1** | Recall below 0.5 structurally unobservable; no threshold surfaced in UI |

Workstream 1 is the only one that changes detection results. 2–5 make the system's behaviour
*legible and provable*, which is what "enterprise ready" actually requires for a tool whose
output is a compliance artefact.

---

## 1. Problem statement

### 1.1 The truncation chain (P0)

Verified end to end against the pinned `xberg` revision
`9bfbc10acca5c1648547a4d861153d2f5c363340`:

1. **The encoder truncates.** `run_pipeline` clamps the input to the encoder's position-embedding
   limit and never reports it:

   ```rust
   let max_seq = encoder.config.max_position_embeddings;
   let seq_len = encoded.input_ids.len().min(max_seq);
   ```

   (`crates/xberg-gliner/src/candle/pipeline.rs:31-33`). Words whose first token landed past
   `seq_len` are then filtered out of `text_positions` (`:44-48`). `decode_span_scores` is
   aware of this — its own comment reads *"when the input exceeds the encoder's
   position-embedding limit, `run_pipeline` truncates and the scores only cover the surviving
   words"* (`candle/decode.rs:60-63`) — but the fact never escapes the call.
2. **The limit is 512.** `max_position_embeddings: 512` in the deployed model's
   `encoder_config/config.json` (mDeBERTa-v2 base, 12 layers, hidden 768, vocab 250 112).
3. **The label prompt shares that budget.** `encode_v2` prepends the schema prompt into the
   same sequence (`v2/preprocess.rs:47-49`), so every added label costs document capacity.
4. **Nothing upstream chunks.** `NerDetector::detect` passes the whole document
   ([`ner.rs:151`](../../../hacienda-core/src/pii/ner.rs)); `hacienda-core` has no windowing;
   Studio's `chunkSize` config feeds xberg's *extraction* chunking only
   ([`worker/pipeline.ts:636`](../../../apps/hacienda-studio/worker/pipeline.ts)), not NER.
5. **A second, independent truncation exists.** `RegexSplitter::split` `break`s at its word
   limit (`crates/xberg-gliner/src/splitter.rs:28-30`), default 512
   (`config.rs:56`). This affects the v1 path; the v2/Candle path is bounded by (1).

### 1.2 Measured impact

Token costs computed by running SentencePiece Unigram Viterbi decoding against the deployed
`tokenizer.json` (`[P]`, `[E]`, `[SEP_TEXT]` confirmed as single added-vocab tokens — ids
250104/250106/250103 — so the prompt cost is exact, not estimated):

| Configuration | Schema prompt | Document budget |
|---|---|---|
| 5 default categories | 21 tok | 491 of 512 tok |
| + "PII exhaustif" (34 labels) | 140 tok | 372 of 512 tok |

Converted to real document coverage at measured tokens-per-word rates:

| Configuration | English (1.50 t/w) | French (1.89 t/w) | German (2.18 t/w) |
|---|---|---|---|
| 5 categories | ~299 words | ~266 words | ~225 words |
| 34 labels | ~226 words | ~201 words | ~171 words |

Two consequences worth stating explicitly:

- **French — the primary client base — is ~12% worse than English**, and German worse again,
  because the multilingual SentencePiece vocabulary is less efficient on those languages.
- **Enabling "PII exhaustif" silently cuts document coverage by ~25%.** A user checking that
  box to detect *more* PII gets *less* document analysed. This is the most user-hostile
  behaviour in the current system.

A single A4 page of prose is ~500 words. Even one page is only partially analysed.

### 1.3 No quality gate (P0)

[`hacienda-core/tests/ner_eval.rs`](../../../hacienda-core/tests/ner_eval.rs) (939 lines) is a
genuinely good harness — exact/overlap P/R/F1, bootstrap confidence intervals, fixture
schema validation, and unusually candid about its own blind spots. But the half that scores a
real model is `#[ignore]`d and gated on `ner-candle`, and **no workflow runs it**:
`ci-rust.yaml:48` and `ci-e2e.yaml:32` run `cargo test --workspace --features "ner-candle"`,
which skips ignored tests. There is therefore no accuracy baseline and no regression
detection. Workstream 1 changes detection behaviour, so shipping it without this gate means
changing the thing we cannot measure.

### 1.4 Model provenance (P1)

- **Unpinned.** `asset-loader.ts:21` fetches from `.../resolve/main` — a moving branch, not a
  commit SHA. Two users on the same app build can receive different weights.
- **Unverified.** No checksum on the ~600MB `model.safetensors` download.
- **Not recorded.** `AuditEntry` carries `pipeline_version` (the crate version) and `vertical`,
  but no model identity. `EntitySource` is `Regex | Model`
  ([`audit/entry.rs:78-81`](../../../hacienda-core/src/audit/entry.rs)) — the chain can say
  "a model detected this" but not *which* model or which weights. For a tamper-evident log
  whose purpose is provenance, that is a material gap.

### 1.5 Threshold ceiling (P1)

`CandleBackend::detect` ignores the caller's threshold and hardcodes
`const DEFAULT_THRESHOLD: f32 = 0.5` (`xberg/src/text/ner/candle.rs:22,170`).
`NerDetector::with_threshold` therefore only *filters upward* — it can raise the effective
threshold but never lower it. Consequences: no entity scored below 0.5 is observable, every
recall figure is truncated-recall-at-0.5, and a precision/recall curve is unobtainable. The
eval harness documents this itself (`ner_eval.rs:25-40`) rather than reporting misleading
numbers, which is the right call, but the underlying limitation remains. Separately, **no NER
threshold is exposed anywhere in the Studio UI**.

### 1.6 Architectural limits (P2 — record, do not "fix")

These are properties of the trained GLiNER2 model, not defects in this port. They belong in
documentation and eval interpretation, not in a fix list:

- `MAX_WIDTH = 8` (`candle/heads/mod.rs:18`) — entities longer than 8 words are undetectable.
  Adequate for names, marginal for full postal addresses.
- `MAX_COUNT = 20` (`candle/heads/count_lstm.rs:37`) — the count-prediction head's ceiling.
  This matches the model's own `pos_embedding.weight.shape[0]` and `count_pred` output
  dimension, i.e. it is the trained architecture's limit. Windowing (workstream 1)
  incidentally relieves pressure on it by reducing entities per forward pass.
- **Inference is serialized.** `Mutex<Gliner2Candle>` (`ner/candle.rs:66`), one document per
  forward pass, no batching. Acceptable for browser-local use; a throughput ceiling for a
  server deployment.

### 1.7 Unverified observation

In a live Studio run, all five detected entities returned confidence **exactly**
`0.8999999761581421`. Distinct entities across distinct categories should not share a score.
The Candle path does propagate real span scores (`ner/candle.rs:141`, `v2/decode.rs:46`),
which suggests Studio's entity-glossary pass runs through the compiled `xberg-wasm` `NerModel`
rather than the Candle path, and that this path may emit a constant. **If confirmed,
confidence carries no discriminative signal in the Studio path and no threshold could be
tuned on it** — which would make workstream 5 a prerequisite for any calibration work.
Resolving this is the first task in §9.

---

## 2. Goals

1. **No silent data loss.** Every word of every document reaches the model, or the caller is
   told, in a machine-readable way, that it did not.
2. **Language-neutral correctness.** French and German documents must not be more truncated
   than English ones.
3. **Measurable quality.** A committed accuracy baseline that CI enforces, so a change to
   preprocessing, windowing, or the model produces a visible delta.
4. **Reproducible detection.** The same document + same app version + same config yields the
   same result, and the audit chain records which model produced it.
5. **No regression in the browser.** Windowing must not make Studio's in-tab processing
   unusably slow or exceed its memory envelope on the low device tier.

---

## 3. Non-goals

- **Retraining or replacing GLiNER2.** `MAX_WIDTH`/`MAX_COUNT`/512 are accepted as given.
- **Long-context encoders.** Swapping mDeBERTa for a 4k-context model is a separate
  evaluation, not this spec.
- **Batched/GPU server inference.** Recorded in §1.6; out of scope until there is a server
  deployment to optimise for.
- **Patching the vendored `xberg` fork by default.** Workstream 2 specifies an upstream-shaped
  change; if upstream is unavailable, §7.2 gives a fallback that needs no fork.
- **Changing the regex engine.** Its precision work landed separately (checksum validators and
  context boosting, `pii/validators.rs`, `pii/context.rs`).

---

## 4. Architecture

### 4.1 Windowed inference (workstream 1)

A new module, `hacienda-core/src/pii/window.rs`, owns splitting and reassembly.
`NerDetector::detect` becomes: split → per-window inference → rebase offsets → deduplicate.

```rust
/// Splitting policy for one detector configuration.
pub struct WindowPlan {
    /// Max words per window. Derived from the token budget; see `WindowPlan::for_labels`.
    pub max_words: usize,
    /// Words of overlap between consecutive windows. Must exceed the model's
    /// `MAX_WIDTH` (8) so no entity can straddle a boundary without appearing
    /// whole in at least one window.
    pub overlap_words: usize,
}
```

**Budget derivation.** The detector knows its own label set, so the prompt cost is computable
rather than guessed:

```text
schema_words = 4 + Σ(1 + words_in_label) + 2 + 1        // ( [P] entities ( … ) ) [SEP_TEXT]
token_budget = 512 − ceil(schema_words × TOKENS_PER_WORD) − SAFETY_MARGIN
max_words    = token_budget / TOKENS_PER_WORD
```

`TOKENS_PER_WORD = 3.0`. Justification — measured against the deployed tokenizer:

| Content | tokens/word |
|---|---|
| English prose | 1.50 |
| French prose | 1.89 |
| German prose | 2.18 |
| **Dense PII** (names, addresses, IBAN, SIRET) | **2.53** |

Dense PII is simultaneously the worst case *and* precisely the content we exist to detect, so
the constant is set above it with headroom rather than at an average. `SAFETY_MARGIN = 16`
tokens absorbs tokenizer edge cases (NFC normalisation, unusual scripts).

This yields ~150 words/window for 5 categories and ~110 for 34 labels — deliberately below
the ~196/~148 a 2.5 constant would allow. The cost is more windows; the benefit is that
correctness does not depend on the estimate being tight. Windowing is a **linear** cost in
document length, so a conservative constant costs throughput, never correctness.

Both constants live in `PipelineConfig` so they are tunable without a rebuild, and so §7.1's
eval can sweep them.

**Split-point rules**, in priority order — a window boundary must land on:

1. a paragraph break (`\n\n`), else
2. a sentence boundary (`.`/`!`/`?` followed by whitespace), else
3. any whitespace, else
4. a UTF-8 character boundary (hard fallback for CJK or unbroken runs).

Rule 4 is non-negotiable: `PiiPipeline::validate_model_entity` already rejects spans that are
not on UTF-8 boundaries, and `str` slicing panics on them. Rules 1–3 exist because splitting
mid-sentence degrades NER quality — the model uses local context, and a name split from its
title ("Dr." | "Dupont") is measurably harder.

**Overlap.** `overlap_words = 16` (2 × `MAX_WIDTH`). Any entity is at most 8 words, so an
entity crossing a boundary is fully contained in the following window. Cost is ~10% extra
inference at 150-word windows.

### 4.2 Offset rebasing and deduplication

Each window carries its byte offset in the source. Entity offsets are rebased by adding it —
`ModelEntity.start`/`.end` are `u32` UTF-8 byte offsets, and windows split on char boundaries,
so rebasing is addition with no re-encoding.

Overlap regions produce duplicate detections. Dedup rule, applied after rebasing:

- Group by `(start, end, category)`; keep the highest `confidence`.
- Then, for spans that *partially* overlap and share a category, keep the longer span — this
  mirrors `merge::merge_entities`' existing `prefer_stronger` tie-break so windowing does not
  introduce a second, inconsistent overlap policy.

Dedup happens **inside** `NerDetector::detect`, before results reach `merge_entities`, so the
regex/model merge sees exactly the shape it sees today. This keeps the change invisible to
`pipeline.rs`, `facade.rs`, and every downstream consumer.

### 4.3 Truncation observability (workstream 2)

Windowing makes truncation *unlikely*; observability makes it *impossible to miss*.

`NerDetector::detect` gains a sibling returning detection plus provenance:

```rust
pub struct NerOutcome {
    pub entities: Vec<ModelEntity>,
    /// Windows the document was split into. 1 = single pass.
    pub windows: u32,
    /// Words the model never saw. Must be 0; any other value is a defect
    /// in the budget estimate and is surfaced, not swallowed.
    pub words_dropped: u32,
}
```

`detect()` keeps its current signature and delegates, so no caller breaks.

`words_dropped` requires knowing what the backend actually consumed. Preferred: `xberg`'s
`run_pipeline` returns whether it clamped — a small, upstream-shaped change (one bool through
`extract_ner` → `Span` output metadata). **Fallback if upstream is unavailable:** treat a
window whose last detected entity ends implausibly short as suspect *only* in the eval
harness, and rely on the conservative budget plus §7.1's window-overflow test in production.
The fallback is weaker; §11 tracks the upstream ask.

`PipelineMetrics` gains `ner_windows: u32` and `ner_words_dropped: u32` alongside the existing
timing fields, so every consumer that already surfaces metrics gets this free.

**Studio surfacing.** `words_dropped > 0` raises a per-document warning in the file list and
blocks the "verified complete" affordance — the operator must be able to distinguish
"no PII found" from "not fully analysed".

### 4.4 Model provenance and pinning (workstream 4)

**Pin.** `MODEL_BASE` moves from `resolve/main` to `resolve/<commit-sha>`. Hugging Face serves
commit-pinned paths identically, so this is a one-line change with no infrastructure cost.

**Verify.** Ship expected SHA-256 digests for `model.safetensors`, `tokenizer.json`, and
`encoder_config/config.json` as build-time constants; verify after download, before the
IndexedDB write, in `lib/asset-loader.ts`. A mismatch is a hard failure with a distinct error
— not a silent fallback to regex-only. Reuses the digest machinery already present for
`lib/content-hash.ts`.

**Record.** A `ModelProvenance { id, revision, digest_prefix }` threads from
`NerDetector` into `AuditEntryInput`, mirroring how `vertical` already carries
`"<id>@<digest>"` rather than a bare id — the existing field's doc comment explains the
reasoning ("an id alone is a false provenance claim"), and model identity has exactly the same
property. `AuditEntry` gains `model: Option<String>`, `#[serde(default)]`, hashed into
`compute_chain_hash` **after** `vertical` with the same tagged-length framing
(`VERTICAL_PRESENT_TAG`), so chains written before the field existed still verify byte for
byte. That back-compat requirement is already established and tested by
`should_verify_a_chain_written_before_the_vertical_field_existed`.

### 4.5 Threshold plumbing (workstream 5)

Thread a real threshold from `NerDetector` to `extract_ner`, replacing the hardcoded
`DEFAULT_THRESHOLD`. This is an upstream-shaped change to `NerBackend::detect`'s contract; it
unblocks the precision/recall curve the eval harness cannot currently produce, and is a
prerequisite for calibrating thresholds per category.

Studio surfaces a single "detection sensitivity" control (low/balanced/high → 0.3/0.5/0.7)
rather than a raw float — `AppConfig` already reserves a `sensitivity` shape in
`pages/Settings.tsx` (currently an `as any` cast against a field that does not exist on
`AppConfig`; this workstream is the opportunity to make it real).

Gated on resolving §1.7: if Studio's path emits constant confidence, no threshold is
meaningful there and that must be fixed first.

---

## 5. Public API changes

| Item | Change | Breaking? |
|---|---|---|
| `NerDetector::detect` | unchanged signature | No |
| `NerDetector::detect_with_outcome` | new, returns `NerOutcome` | No (additive) |
| `NerDetector::with_window_plan` | new builder | No (additive) |
| `PipelineMetrics` | `+ ner_windows`, `+ ner_words_dropped` | No (struct is constructed internally) |
| `AuditEntry` | `+ model: Option<String>` (`#[serde(default)]`) | No — old chains verify |
| `PipelineConfig` | `+ window` section | No (serde default) |
| `NerBackend::detect` | threshold honoured, not ignored | Behavioural — see §10 |

---

## 6. Files touched

### 6.1 New

- `hacienda-core/src/pii/window.rs` — split policy, rebasing, dedup
- `hacienda-core/tests/ner_window.rs` — window integration tests
- `.github/workflows/ci-ner-eval.yaml` — the quality gate
- `hacienda-core/fixtures/ner-eval/baseline.json` — committed metrics baseline

### 6.2 Modified

- `hacienda-core/src/pii/ner.rs` — windowing in `detect`, `NerOutcome`, provenance
- `hacienda-core/src/pii/pipeline.rs` — metrics plumbing (both `detect` and
  `detect_with_model_entities`)
- `hacienda-core/src/pii/config.rs` — `window` config section
- `hacienda-core/src/audit/entry.rs` — `model` field + chain hashing
- `crates/hacienda-wasm/src/lib.rs` — surface `NerOutcome` to JS
- `apps/hacienda-studio/lib/asset-loader.ts` — pin + digest verification
- `apps/hacienda-studio/worker/pipeline.ts` — consume/propagate truncation warning
- `apps/hacienda-studio/pages/Settings.tsx` — sensitivity control (workstream 5)

---

## 7. Testing strategy

### 7.1 Unit and integration (must pass in default `cargo test`, no model)

| Test | Asserts |
|---|---|
| `should_split_a_long_document_into_overlapping_windows` | window count, overlap size |
| `should_split_only_on_utf8_char_boundaries` | fuzz over CJK/emoji/combining marks |
| `should_prefer_paragraph_then_sentence_then_whitespace_boundaries` | rule priority §4.1 |
| `should_rebase_entity_offsets_onto_the_source_document` | `text[start..end]` equals mention |
| `should_deduplicate_entities_found_in_an_overlap_region` | one span, highest confidence |
| `should_detect_an_entity_straddling_a_window_boundary` | the regression this exists to prevent |
| `should_shrink_the_window_budget_as_labels_are_added` | 34 labels ⇒ smaller `max_words` |
| `should_report_zero_dropped_words_for_any_input` | property test, generated lengths |
| `should_verify_a_chain_written_before_the_model_field_existed` | audit back-compat |

Window tests use a stub `NerBackend` (the pattern `ner.rs`'s existing `StubBackend` already
establishes), so they need no weights and run in CI today.

### 7.2 Quality gate (workstream 3)

New workflow `ci-ner-eval.yaml`:

1. Fetch the **pinned** model (§4.4) — cached by digest across runs.
2. Run the currently-`#[ignore]`d runner: `cargo test -p hacienda-core --features ner-candle
   -- --ignored ner_eval`.
3. Compare against `fixtures/ner-eval/baseline.json`.
4. **Fail** if per-label F1 drops more than the bootstrap CI half-width below baseline. Using
   the harness's existing bootstrap interval as the tolerance avoids a hand-tuned epsilon and
   makes the gate statistically meaningful rather than arbitrary.
5. Post the metrics delta as a PR comment.

Baseline is generated once from `main` **before** workstream 1 lands, so the windowing change
is measured against pre-windowing behaviour. That comparison is the actual proof this spec
works: recall on documents >300 words should rise substantially; precision should not move.

Runs on PRs touching `hacienda-core/src/pii/**`, the model pin, or the fixtures — not every
PR, since it needs a 600MB artefact.

### 7.3 Performance budget

Windowing multiplies forward passes by `ceil(words / 150)`. Measure and gate:

| Document | Windows | Budget (browser, mid tier) |
|---|---|---|
| 1 page (~500 w) | 4 | < 2.5 s |
| 10 pages (~5 000 w) | 34 | < 20 s |
| 50 pages (~25 000 w) | 167 | < 100 s, must not OOM |

The 50-page case is the one that decides whether Studio needs progress reporting per window
(likely yes) and whether the low device tier must cap document length. Measured before
rollout, not assumed.

---

## 8. Risks and mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| Windowing degrades quality at boundaries (lost cross-sentence context) | Medium | 16-word overlap; §7.2 measures it directly — if F1 drops, raise overlap and re-measure |
| N× slower on long documents | **High** (inherent) | §7.3 budget; per-window progress in Studio; consider capping length on low tier |
| `TOKENS_PER_WORD = 3.0` still too low for an unseen script | Low | `SAFETY_MARGIN`; `words_dropped` surfaces it rather than hiding it; constant is config-tunable |
| Upstream `xberg` change (truncation flag, threshold) not accepted | Medium | §4.3/§4.5 fallbacks; vendoring precedent exists (`vendor/xberg-wasm-ner-fix`) but is a last resort |
| Audit chain change breaks existing chains | Low | Tagged-length framing + back-compat test, mirroring the `vertical` field precedent exactly |
| Model pin blocks a needed model update | Low | Pin is a one-line change; updating it is a reviewable PR that re-runs §7.2 — which is the point |

---

## 9. Rollout

**Phase 0 — Make it measurable (blocks everything).**
Resolve §1.7 (constant-confidence question). Wire `ci-ner-eval.yaml` and commit a baseline
from current `main`. Nothing here changes behaviour; it makes the next phase provable.

**Phase 1 — Windowing (the P0 fix).**
`window.rs` + `NerDetector` integration + §7.1 tests. Ship behind
`PipelineConfig.window.enabled`, default **on**, with an escape hatch for a fast rollback.
Gate on §7.2 showing recall up and precision flat.

**Phase 2 — Observability.**
`NerOutcome`, metrics plumbing, Studio warning surface. Upstream truncation-flag ask.

**Phase 3 — Provenance.**
Model pin + digest verification, then the `AuditEntry.model` field.

**Phase 4 — Threshold.**
Upstream threshold plumbing, then the Studio sensitivity control. Sequenced last because it
depends on Phase 0's answer and delivers least if confidence is constant.

---

## 10. Open questions

1. **Is Studio's NER confidence constant?** (§1.7) Decides whether Phase 4 is worth doing and
   whether any threshold work is meaningful in the browser. Cheapest possible experiment;
   should be answered before planning Phase 1.
2. **Is upstream `xberg` ours to change?** The repo pins a git revision and already vendors
   `xberg-wasm-ner-fix`, so patching has precedent — but §4.3 and §4.5 are cleaner as upstream
   contributions. Answer determines whether Phases 2 and 4 need a fork.
3. **Should the low device tier cap document length?** §7.3's 50-page measurement decides.
   Capping is a product decision (refuse vs. degrade), not purely technical.
4. **Per-category thresholds?** The harness can produce per-label curves once Phase 4 lands.
   Worth it only if per-label optimal thresholds diverge materially — measure before building.
5. **Does `MAX_WIDTH = 8` actually cost recall on French postal addresses?** Testable today
   against the existing fixtures; if yes, it is an argument for a regex/gazetteer address
   pattern rather than a model change.
