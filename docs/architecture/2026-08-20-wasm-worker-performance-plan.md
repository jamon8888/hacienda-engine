# WASM/Worker Performance & Memory Plan — Hacienda Studio

Date: 2026-08-20
Status: Proposed

## Context

Hacienda Studio processes documents through a local, in-browser pipeline: a single
dedicated Web Worker (`apps/hacienda-studio/worker/pipeline.ts`) runs PII
detection and redaction using a **GLiNER2** NER model
(`gliner2-guardrails-pii-f16`, mdeberta-v3-base encoder) via the **Candle**
(Rust) ML framework compiled to `wasm32-unknown-unknown`. Files are processed
sequentially in a `for` loop inside that one worker — no cross-document
parallelism, no in-document threading.

Two investigations (codebase audit + external research) established the
current baseline:

- **Model**: GLiNER2, F16 weights, fetched at runtime from HuggingFace
  (`apps/hacienda-studio/lib/asset-loader.ts:9-22`). No local copy is vendored.
- **Duplicate inference**: the same Candle GLiNER2 model is loaded and run
  **twice per document** — once via this repo's own `hacienda-wasm` crate
  (`crates/hacienda-wasm`), once via the separately-published
  `@xberg-io/xberg-wasm` npm package used for the entity-glossary pass
  (`apps/hacienda-studio/lib/pii-engine.ts:75-82`,
  `apps/hacienda-studio/lib/asset-loader.ts:384-390`, already flagged as known
  duplication at `apps/hacienda-studio/worker/pipeline.ts:93-100`).
- **Build profile**: `crates/hacienda-wasm` release profile uses
  `opt-level = "z"` (size-optimized) with `codegen-units = 1` and no `lto`
  setting (`Cargo.toml:190-192`) — optimized for binary size, not inference
  speed.
- **SIMD**: not enabled. No `.cargo/config.toml` exists; no
  `target-feature=+simd128` anywhere in the build.
- **Threading**: deliberately disabled. `hacienda-core/Cargo.toml:108-113`
  documents that the wasm32 target is built **without** the `atomics`
  feature, using `SendWrapper` on the assumption of single-threaded
  execution.
- **wasmtime**: not applicable. Wasmtime is a standalone server/CLI runtime
  (Cranelift/Winch); browsers execute wasm through their own built-in engines
  (V8 / SpiderMonkey / JavaScriptCore), never wasmtime. Wasmtime would only
  become relevant if a server-side or CLI batch-redaction path were added
  outside the browser — and even there, `hacienda-core` already compiles
  natively for its `postgres`/`s3`/`jobs` server features, which outperforms
  a wasmtime-hosted wasm build of the same logic.

## Goals

1. Reduce peak RAM during document processing.
2. Reduce wall-clock time per document and across a batch.
3. Do this without weakening the in-browser, local-only processing model
   (no data leaves the client).

## Tier 1 — High impact, low effort, no architecture change

### 1.1 Eliminate the duplicate GLiNER2 inference pass

Today the entity-glossary pass (`@xberg-io/xberg-wasm`) and the PII pipeline
(`hacienda-wasm`) each instantiate their own copy of the same GLiNER2 model
and run inference independently. This means two wasm heaps holding the same
weights, and two forward passes per document for no functional benefit.

**Plan:**
- Consolidate on a single loaded model instance per worker. Either:
  - (a) Have `hacienda-wasm`'s PII pipeline consume the NER output already
    produced by the entity-glossary pass instead of re-running inference, or
  - (b) Drop one of the two wasm artifacts entirely and expose both
    "PII detection" and "entity glossary" as two consumers of one
    `NerDetector`/`NerModel` instance.
- Audit `apps/hacienda-studio/lib/pii-engine.ts` and
  `apps/hacienda-studio/lib/ner-bridge.ts` to identify the minimal shared
  interface both call sites need (spans, labels, scores).
- Expected impact: roughly halves peak RAM (one model in memory instead of
  two) and per-document inference time (one forward pass instead of two).

**Effort:** Medium — requires understanding both call sites well enough to
merge them safely; no new dependencies.

### 1.2 Fix the release build profile

`crates/hacienda-wasm`'s release profile
(root `Cargo.toml:190-192`) is tuned for binary size
(`opt-level = "z"`), which is the wrong tradeoff for a wasm module whose cost
is dominated by inference compute, not download size.

**Plan:**
- Change `opt-level` from `"z"` to `3`.
- Add `lto = "fat"` to the same profile block.
- Rebuild via `npm run build:wasm` and confirm the build still completes
  (expect a longer build, wasm-pack's bundled `wasm-opt` pass already runs
  automatically per `apps/hacienda-studio/scripts/build-wasm-if-needed.sh`).
- Benchmark before/after on a representative document to confirm the speed
  win outweighs any binary-size regression on cold load.

**Effort:** Low — a two-line Cargo.toml change plus a rebuild and benchmark.

### 1.3 Enable SIMD128

No `.cargo/config.toml` exists in the repo and no build path sets
`target-feature=+simd128`. For matmul-heavy Candle inference this is
typically a 2–4x speedup and is supported in all target browsers
(Chrome 91+, Firefox 89+, Safari 16.4+).

**Plan:**
- Add `.cargo/config.toml` (or set `RUSTFLAGS` in the `build:wasm` script)
  scoping `target-feature=+simd128` to the `wasm32-unknown-unknown` target.
- Rebuild both `hacienda-wasm` and, if feasible, request the same flag be
  applied to the `@xberg-io/xberg-wasm` build (or track its removal once
  Tier 1.1 lands and it's no longer needed).
- Benchmark before/after.

**Effort:** Low — config change plus rebuild and benchmark.

## Tier 2 — Moderate effort, after Tier 1 lands

### 2.1 Worker pool for cross-document parallelism

Replace the sequential `for` loop in `processFiles`
(`apps/hacienda-studio/worker/pipeline.ts:609-757`) with a small pool of
dedicated workers that round-robin files.

**Plan:**
- Size the pool by **available RAM ÷ per-worker model footprint**, not
  `navigator.hardwareConcurrency` — each worker holds a full model instance,
  so pooling before Tier 1.1 lands would double-count the duplication
  problem across N workers.
- Start with a conservative pool size (e.g. 3–4 workers) and make it
  configurable; measure peak RSS/heap under a realistic batch before tuning
  upward.
- Preserve the existing transcription RPC bridge
  (`apps/hacienda-studio/worker/transcribe-bridge.ts`) — it proxies to the
  main thread today because whisper-web can't run in a worker; this needs to
  keep working per-worker in the pool.

**Effort:** Medium — pool/dispatcher logic, plus care around shared UI state
(progress reporting, cancellation) that currently assumes one worker.

### 2.2 Cold-start caching

Streaming compilation (`instantiateStreaming`) is already the default via
wasm-bindgen's `--target web` output. Add explicit caching of the compiled
module (IndexedDB/Cache API, keyed by content hash) to skip recompilation on
repeat visits, reinforcing the browser's own compile cache.

**Effort:** Low–Medium.

## Tier 3 — Larger architecture change, only if Tier 1/2 aren't sufficient

### 3.1 In-document threading via wasm-bindgen-rayon

`hacienda-core` currently opts **out** of threading by design
(`hacienda-core/Cargo.toml:108-113`, `SendWrapper` usage assumes
single-threaded execution). Enabling `wasm-bindgen-rayon` would parallelize
within a single large document (as opposed to Tier 2's across-document
parallelism), which matters most for very large files.

**Plan (if pursued):**
- Requires a nightly Rust toolchain with `std` rebuilt for
  `wasm32-unknown-unknown` with `atomics`/`bulk-memory`.
- Requires COOP/COEP cross-origin isolation, which is already configured in
  `apps/hacienda-studio/vite.config.ts:5-8` for whisper transcription, so the
  hosting requirement is already met — but any other cross-origin resource
  the app loads must also comply.
- Requires reworking the `SendWrapper`-based single-threaded assumptions in
  `hacienda-core` before code can safely run across threads.
- Spawning a rayon thread pool from inside a dedicated Worker means a
  worker-of-workers topology; needs its own lifecycle/cleanup handling.

**Effort:** High — multi-day effort, changes a documented design assumption,
should be a deliberate decision made only if Tier 1/2 don't hit the target
RAM/latency numbers.

### 3.2 wasmtime — out of scope for the browser path

Confirmed not applicable to the in-browser SPA (see Context above). Only
revisit if a server-side/CLI batch-redaction service is added outside the
browser; even then, prefer running `hacienda-core` natively (it already
compiles for server use via the `postgres`/`s3`/`jobs` features) over hosting
the wasm build under wasmtime, unless WASI sandboxing or per-tenant
fuel-limited execution is a specific requirement.

## Suggested sequencing

1. Tier 1.1 (dedupe) — biggest win, no tradeoffs, do first.
2. Tier 1.2 + 1.3 (build profile + SIMD) — cheap, can be done in parallel
   with 1.1's development.
3. Benchmark after Tier 1 (RAM + wall-clock on a representative batch)
   before deciding whether Tier 2 or Tier 3 is warranted.
4. Tier 2.1 (worker pool) if batch throughput is still the bottleneck.
5. Tier 3.1 (threading) only if large single documents remain slow after
   Tier 1/2, given its cost and the design change it requires.

