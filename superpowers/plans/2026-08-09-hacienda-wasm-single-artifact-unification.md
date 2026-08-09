# Plan: hacienda-wasm as Studio's single compiled artifact (Track L-unify, "Option B")

**Date:** 2026-08-09
**Status:** Proposed — not started
**Depends on:** PR #77 ("Option A" — trim `hacienda-core`'s wasm32 `xberg` dependency to the
`ner` feature it actually uses). This plan reverses part of that trim, deliberately and behind
a new feature flag, once the reasons below justify paying the cost again.

---

## Problem this solves

`apps/hacienda-studio` currently loads **two independent WASM modules** in the same worker:

| Module | Source | Size (last measured) | Does |
|---|---|---|---|
| `@xberg-io/xberg-wasm` | npm package, built by the xberg team | ~48 MB (`xberg_wasm_bg.wasm`) | Document extraction (97+ formats), layout detection, Candle GLiNER2 NER |
| `hacienda-wasm` | Built locally from `crates/hacienda-wasm` | ~1.2 MB after PR #77 (was 17.8 MB before it) | Regex PII detection/redaction, IndexedDB audit chain |

Before PR #77, `hacienda-wasm` statically linked its **own, independent copy** of xberg's
extraction/layout/candle-NER stack (`no-ort-target`, `excel-wasm`, `layout-tract`,
`auto-rotate-tract`, `ner-candle-wasm`) — the same code already inside `xberg_wasm_bg.wasm` —
even though nothing in `crates/hacienda-wasm/src/lib.rs` ever called it. This was flagged back
in the original wasm port (see the L2 note in
`docs/superpowers/plans/2026-07-30-hacienda-program-plan.md`):

> Track L's actual integration (Studio embeds one compiled artifact, not two `xberg`
> instances) would not pay for that duplication — the true marginal cost of adding
> hacienda-core's own code (`pii`/`redaction`/`audit`) on top of Studio's existing bundle is
> closer to L1's 1,227,264-byte, narrower-`xberg`-feature measurement than to \[17.8 MB].

That "actual integration" was never built. PR #77 removes today's dead weight (Option A), but
the two-module split itself remains. This plan is the follow-through: make `hacienda-wasm` the
**one** WASM artifact Studio loads — extraction, layout, NER, PII, and audit all behind a single
`wasm-bindgen` surface — and delete the separate `@xberg-io/xberg-wasm` import from Studio
entirely.

## Goals

- Studio loads one compiled `xberg` (inside `hacienda-wasm`), not two.
- No functional regression: every format/mode Studio supports today keeps working.
- The `hacienda-wasm` crate stays wasm32's only consumer of xberg's extraction — no change to
  native builds (`hacienda-cli`/`hacienda-api` already link `xberg` natively, unaffected).
- Every phase below is independently landable and independently revertable — no big-bang PR.

## Non-goals

- OCR is **out of scope**. Studio's OCR already runs through the standalone `tesseract-wasm`
  npm package, not xberg's `ocr`/`ocr-wasm` feature — confirm this in M0 and leave it alone.
  xberg's own `ocr-wasm` stays excluded from `hacienda-core`'s wasm32 features regardless of
  this plan (still blocked on WASI SDK provisioning in CI — see the existing comment this plan
  is replacing in `hacienda-core/Cargo.toml`).
- Not attempting to make hacienda-wasm and the published `@xberg-io/xberg-wasm` package share
  one *binary* — that's not how Cargo/wasm-bindgen works. "One compiled artifact" means
  hacienda-wasm's own build links xberg as a source dependency and Studio stops loading the
  separately-published package — see the Risks section on why this distinction matters.

## Blocking prerequisite (must land before M1, not just be checked before merge)

`xberg`'s pinned git dependency (`tag = "v1.0.2"`, `hacienda-core/Cargo.toml`) is currently
broken in CI and in every sandbox this work was investigated in: its `test_documents` submodule
points at a commit (`4ca54bb0...`) that no longer exists in `xberg-io/test_documents`, so
`cargo`'s dependency resolution fails before reaching any of this repo's own code (see PR #77's
CI run and comment thread for the full trace — every job that touches `cargo` currently fails
here, unconditionally of feature selection).

This has to be fixed upstream (re-pin `xberg`'s tag to a commit with a valid submodule
reference, or a fix in `xberg-io/test_documents` itself) before M1 can even compile, let alone
be verified. Track this as a blocking dependency, not a footnote.

---

## M0 — Confirm scope before touching anything

- [ ] Confirm OCR really doesn't route through xberg's `ocr`/`ocr-wasm` in Studio today. Trace
      `worker/pipeline.ts`'s OCR call sites and `lib/asset-loader.ts`'s `loadTessdata` to verify
      they're fully independent of `@xberg-io/xberg-wasm`'s extraction call — if extraction
      *internally* invokes OCR for image-heavy PDFs via a registered post-processor, that
      changes M2's scope.
- [ ] Confirm the xberg pin/submodule issue (above) is fixed and `cargo build -p hacienda-wasm
      --target wasm32-unknown-unknown --features wasm-extraction` (M1's new feature, once it
      exists) can actually run to completion somewhere — locally or in CI. Nothing else in this
      plan can be verified until this is true.

## M1 — Re-enable xberg's extraction features on wasm32, opt-in

Don't just revert PR #77 — that would make every consumer of `hacienda-core` pay for extraction
again, including ones that (like `hacienda-wasm` today) never call it. Gate it behind a new
Cargo feature on `hacienda-core` instead:

- [ ] Add a `wasm-extraction` feature to `hacienda-core/Cargo.toml` that, on `wasm32`, upgrades
      the `xberg` dependency's feature set from `["ner"]` (PR #77's baseline) to
      `["no-ort-target", "excel-wasm", "layout-tract", "auto-rotate-tract", "ner-candle-wasm"]`
      (the pre-#77 set — everything `wasm-target` had minus `ocr-wasm`, per M0/non-goals).
      Cargo doesn't let a feature conditionally change a dependency's own feature list directly;
      express this via `xberg`'s optional dependency features
      (`xberg/no-ort-target`, `xberg/excel-wasm`, etc.) gated behind `wasm-extraction` in
      `hacienda-core`'s `[features]` table, with a small always-on base (currently `["ner"]`)
      kept unconditional.
- [ ] `crates/hacienda-wasm/Cargo.toml` enables `wasm-extraction` on its `hacienda-core`
      dependency (still `default-features = false` otherwise).
- *Check:* `cargo build -p hacienda-wasm --target wasm32-unknown-unknown --no-default-features
  --features hacienda-core/wasm-extraction` reaches a **link** error about `hacienda-wasm`'s own
  code (i.e., xberg's side compiles clean), matching the bar the original wasm port used.

## M2 — Expose extraction through `hacienda-wasm`'s `wasm-bindgen` surface

- [ ] Add an `extract` entry point to `crates/hacienda-wasm/src/lib.rs` wrapping
      `xberg::extract`/`extract_batch`, gated behind `wasm-extraction`. Match
      `@xberg-io/xberg-wasm`'s JS-facing shape closely enough that `worker/pipeline.ts`'s call
      sites need minimal changes — same input (bytes + filename [+ config]), same output shape
      (markdown + entities), not a redesigned API.
- [ ] Decide the async/threading model: Studio already runs the pipeline inside a Worker, and
      `hacienda-wasm`'s existing `process`/`scan` are already `async fn` via
      `wasm-bindgen-futures` — extend the same pattern rather than introducing a second one.
- *Check:* a `wasm-bindgen-test` (or `vitest` against the real compiled `.wasm`, matching how
  `pii-engine.test.ts`/`audit-handle.test.ts` already test `hacienda-wasm`) round-trips a
  fixture document through the new `extract()` and compares markdown/entity output against
  `@xberg-io/xberg-wasm`'s output for the *same* fixture — a parity check, not just "it
  compiles and returns something."

## M3 — NER: decide the model-loading seam, then wire it

`hacienda-core`'s wasm32 `load_detector` (`pii/pipeline.rs:266-273`) currently just errors and
expects a detector supplied via `PiiPipeline::with_detector`. Once `ner-candle-wasm` is enabled
(M1), a real `CandleBackend` becomes buildable — decide whether to use it:

- [ ] **Recommended:** expose a `NerModel`-shaped class from `hacienda-wasm` (same
      `load({weights, tokenizer, encoderConfig})` shape `@xberg-io/xberg-wasm`'s `NerModel`
      already has), backed internally by `xberg::text::ner::candle::CandleBackend` now that it's
      linked. This lets `lib/asset-loader.ts`'s `createNerBackend()` change only its import
      source (`@xberg-io/xberg-wasm` → `hacienda-wasm`), not its call shape or the GLiNER2
      weight-fetching logic already built in `lib/asset-loader.ts` (HF download, IndexedDB
      cache, range-parallel fetch) — none of that needs to move into Rust.
- [ ] Wire the loaded `NerModel` into `PiiPipeline` via `with_detector` so `process`/`scan`
      actually run NER, not just regex — this is currently unreachable on wasm32 regardless of
      M1/M2, and is real missing functionality worth closing out as part of this track.
- *Check:* `lib/ner-bridge.test.ts`'s existing 9 assertions still pass against the new backend
  source, unchanged in shape.

## M4 — Switch `worker/pipeline.ts` to the single module

- [ ] Replace the `@xberg-io/xberg-wasm` import (`initWasm`, extraction calls,
      `xberg_wasm_bg.wasm?url`) with `hacienda-wasm`'s M2/M3 equivalents.
- [ ] Collapse the two `Promise.all([initWasm(...), initPiiEngine()])` init paths into one.
- [ ] Remove `@xberg-io/xberg-wasm` from `apps/hacienda-studio/package.json` once nothing
      imports it.
- [ ] Update `vite.config.ts`: drop the `"xberg-wasm": ["@xberg-io/xberg-wasm"]` `manualChunks`
      entry, keep `hacienda-wasm`'s.
- *Check:* production `vite build` succeeds with exactly one wasm chunk, not two;
  `npm run test:e2e` (Playwright) green end-to-end including a real document upload → zip
  download round trip (matching the verification bar Track I/L already held themselves to).

## M5 — Measure and record the actual win

- [ ] Release build size before/after, same methodology as the original L1/L2 measurements
      (`cargo build --release`, no `wasm-opt` caveat noted explicitly if it's still unavailable
      wherever this is measured).
- [ ] Record the result in this file's own "Result" section once measured for real — this repo's
      convention (see L1/L2 in `docs/superpowers/plans/2026-07-30-hacienda-program-plan.md`) is
      verified numbers, not estimates.

---

## Risks

- **"One compiled artifact" ≠ literal binary sharing.** `hacienda-wasm` links `xberg` as a
  *source* dependency compiled by this repo's own toolchain; `@xberg-io/xberg-wasm` is a
  separately published package built by the xberg team's own pipeline, possibly with
  different optimization flags or post-processing. `crates/xberg-wasm/scripts/fix-wasi-imports.mjs`
  (referenced in `scripts/ci/wasm/run-crate-tests.sh`) patches WASI imports out of the published
  package — confirm whether `hacienda-wasm`'s own build needs an equivalent step once
  `ner-candle-wasm`/`layout-tract` pull in anything with the same WASI-import problem, or it may
  fail to load in a browser the same way the published package would without that patch.
- **The blocking prerequisite (xberg pin/submodule) is external.** This plan cannot start, not
  just "cannot be verified," until that's fixed. Don't schedule M1 work assuming it'll resolve
  itself.
- **Feature-parity drift.** `@xberg-io/xberg-wasm` may be ahead of the `v1.0.2` tag
  `hacienda-core` pins — M2's parity check (fixture round-trip) should catch behavioral
  differences, not just compilation success.
- **wasm-opt.** The 17.8 MB pre-#77 measurement was taken without a `wasm-opt` pass
  ("unavailable in this sandbox" per the original devlog). M5 needs to confirm `wasm-opt`
  actually runs in the real CI/publish pipeline before trusting a size comparison — otherwise
  the de-duplication win could be partly offset by a missing optimization pass on one side of
  the comparison and not the other.

## Rollback

Every milestone here is additive behind the new `wasm-extraction` feature (M1) until M4 actually
flips `worker/pipeline.ts` over. Up through M3, `hacienda-core`'s and `hacienda-wasm`'s default
builds are untouched — Option A's lean baseline from PR #77 stays the default, and Studio keeps
working exactly as it does today. If M4 surfaces a blocking regression, reverting it alone (keep
`@xberg-io/xberg-wasm` in `package.json`, keep the old `worker/pipeline.ts` call sites) fully
restores today's behavior without touching M1-M3.

## Effort shape

This is a multi-PR track, not a single change — M1 is blocked on an external repo fix outside
this codebase's control, M2/M3 are real new Rust API surface with their own test bar, and M4 is
a real behavior change to Studio's runtime path that needs the same manual-Playwright-run
verification bar the original Track I/L work held itself to. Land and verify each milestone
independently; don't batch them into one PR.
