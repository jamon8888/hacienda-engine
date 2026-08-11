# Plan: enable real NER on wasm32 (revised — supersedes this file's original full-unification draft)

**Date:** 2026-08-09, revised same day
**Status:** Proposed — not started

## Revision note

The first version of this file proposed merging `hacienda-wasm` and `@xberg-io/xberg-wasm`
into one compiled artifact ("Option B"), justified by a devlog quote about Studio loading "two
`xberg` instances." That quote describes the state *before* PR #77 ("Option A"): back then,
`hacienda-wasm` linked its own duplicate copy of xberg's extraction/layout/candle-NER stack
even though it never called any of it.

PR #77 already removed that duplication — `hacienda-core`'s wasm32 `xberg` dependency is now
just `["ner"]` (types only, ~1.2 MB). Today there is genuinely **no duplication**: extraction
lives once, in `@xberg-io/xberg-wasm` (~48 MB); `hacienda-wasm` carries only PII/redaction/audit
(~1.2 MB). Merging them now would not meaningfully reduce what's shipped to the browser — going
from "48 MB + 1.2 MB in two modules" to "~49 MB in one module" is not a real size win, so the
size argument that motivated the original draft no longer applies.

What's still genuinely broken, independent of the merge question: **NER doesn't work on wasm32
at all today.** `hacienda-core`'s `load_detector` (`pii/pipeline.rs:266-273`) errors
unconditionally on wasm32 — `PiiPipeline::process`/`scan` only ever run the regex detector in
the browser, never the model. That's the real gap worth closing, and it doesn't require
pulling xberg's extraction/layout stack into `hacienda-wasm` at all — only its NER feature.

This revision replaces the original M1-M5 (full artifact merge) with a narrower plan that closes
the NER gap directly. The original full-unification plan is kept below as a deprioritized
option, not deleted, since the version-skew concern it was also chasing (see its own section)
may become worth revisiting later.

---

## Goal

Make `PiiPipeline::process`/`scan` actually run NER in the browser, using the same GLiNER2
weights Studio already fetches and caches today (`lib/asset-loader.ts`'s `loadNerModel` /
`MODEL_URL`) — without touching extraction, without removing `@xberg-io/xberg-wasm`, without
merging the two modules.

## Non-goals

- No change to extraction, layout detection, or OCR — `@xberg-io/xberg-wasm` keeps doing all of
  that exactly as today.
- Not attempting to eliminate the two-module split — see the revision note above for why that's
  no longer the active problem.

## Blocking prerequisite (unchanged from the original draft)

Same as before: `xberg`'s pinned git dependency (`tag = "v1.0.2"`) currently fails to resolve in
CI and in this session's sandbox because its `test_documents` submodule points at a commit that
no longer exists upstream (see PR #77's CI run and comment thread). Nothing here can be
compiled or verified until that's fixed upstream.

---

## N1 — Add a wasm32 NER feature to `hacienda-core`

- [ ] Add a new feature, mirroring the existing native one:
      ```toml
      [features]
      default = ["jobs"]
      ner-candle = ["xberg/ner-candle"]        # existing, native
      ner-candle-wasm = ["xberg/ner-candle-wasm"]  # new
      ```
      `xberg`'s wasm32 dependency (`hacienda-core/Cargo.toml`, currently `features = ["ner"]`
      per PR #77) doesn't need to change — Cargo lets a package's own feature turn on more of an
      already-declared dependency's features via `<dep>/<feature>`, so this is additive, not a
      second `xberg =` line.
- [ ] `crates/hacienda-wasm/Cargo.toml` enables `hacienda-core/ner-candle-wasm` (still
      `default-features = false` on everything else).
- *Check:* `cargo build -p hacienda-wasm --target wasm32-unknown-unknown --no-default-features
  --features hacienda-core/ner-candle-wasm` links clean — this is a much smaller compiled surface
  than the original plan's `wasm-extraction` (no `tract`, no `hf-hub`/hf downloader, no layout
  code), so it should also be faster to build and safer to land than the full-unification path.

## N2 — Wire a loaded model into `PiiPipeline` from `hacienda-wasm`

- [ ] Add a `wasm-bindgen` entry point that takes the same three byte buffers Studio already
      fetches for xberg-wasm's own `NerModel.load()` (`weights`, `tokenizer`, `encoderConfig` —
      see `lib/asset-loader.ts:334-392`), constructs an `xberg::text::ner::candle::CandleBackend`
      through `NerDetector::from_candle_local`'s wasm32 equivalent (that constructor is currently
      native-only, `#[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]` in
      `pii/ner.rs:97` — needs a wasm32-compatible loader taking in-memory bytes instead of a
      filesystem path, since there's no disk to read `model_dir` from in a browser), and stores
      it for `PiiPipeline::with_detector`.
- [ ] Decide the exact JS-facing shape: either a `loadModel(weights, tokenizer, encoderConfig)`
      call before `process`/`scan`, or a constructor on a stateful pipeline object — match
      whichever is less disruptive to `lib/pii-engine.ts`'s existing `initPiiEngine()`/`scanForPii
      `/`redactPii` call shape.
- *Check:* a `wasm-bindgen-test` loads the real GLiNER2-guardrails-pii weights (or a small
  fixture model, if a full 600 MB download isn't practical in CI) and asserts `scan()` returns at
  least one model-sourced entity (`source` field ≠ `"regex"`) for a text a pure-regex pass
  wouldn't catch (e.g. a name with no structured pattern).

## N3 — Decide whether Studio should call this at all, and avoid a double inference cost

Studio's `worker/pipeline.ts` already runs xberg-wasm's own `NerModel` once per document
(feeding `xbergEntities` → the entity glossary / vault output, Track I). If `hacienda-wasm`'s PII
pipeline *also* runs its own NER pass on the same document for redaction purposes, that's the
same GLiNER2 model run twice per document — real, avoidable latency/CPU cost in the browser.

- [ ] Before wiring N2 into `worker/pipeline.ts`, check whether `hacienda-core`'s pipeline has
      (or should gain) a lower-level entry point that accepts *already-computed* model entities
      (i.e., reuse `xbergEntities` xberg-wasm already produced) instead of re-running its own
      detector — `pii/pipeline.rs`'s regex+model merge step (`merge_entities`, line ~241) may
      already be separable from the "load and run a detector" step in a way that lets Studio
      supply entities once and get both the glossary *and* the redaction pass from a single NER
      run. This needs a look at `pii/pipeline.rs`'s internals before committing to N2's API shape
      — don't build N2 as "run NER a second time" if a "merge already-computed entities" path is
      cheaper and just as correct.
- *Check:* whichever shape N2 lands on, confirm — with a real Playwright run, not just unit tests
  — that enabling PII redaction in Studio's config doesn't measurably double per-document
  processing time versus today's regex-only pass plus xberg-wasm's existing NER call.

---

## Deferred: full artifact unification (the original draft, kept for reference)

Not pursued now — see the revision note. The original motivation (removing duplicate xberg
copies) is stale post-#77. What's still a real, if smaller, argument for eventually doing this:

- **Version skew**: `hacienda-core` pins `xberg` to git tag `v1.0.2`; Studio's
  `@xberg-io/xberg-wasm` is pulled from npm independently (observed at `1.0.12` when this was
  checked) and could drift out of sync with the git-pinned version over time — worth confirming
  whether these two version numbers are even on comparable schemes before treating this as a
  real risk, not just assuming it.
- If that drift ever causes an observable behavioral difference (e.g. extraction output that
  differs between what CLI/API produce natively and what Studio produces in-browser), that's the
  trigger to revisit full unification, not size or "two modules" on principle alone.

If revisited, the original phases still apply as written: M1 (re-enable
`no-ort-target`/`excel-wasm`/`layout-tract`/`auto-rotate-tract`/`ner-candle-wasm` behind a
`wasm-extraction` feature) → M2 (expose `extract()` from `hacienda-wasm`) → M3 (this plan's N1-N3,
superset) → M4 (switch `worker/pipeline.ts`, drop `@xberg-io/xberg-wasm`) → M5 (measure). The
risks and rollback sections from that draft (binary-sharing misconception, WASI-import patch
parity, wasm-opt caveat) still apply unchanged if this is picked back up.
