# Hacienda — Program Plan (2026-07-30)

**Supersedes:** `docs/superpowers/plans/2026-07-28-audio-transcription-ner-pii-integration.md`
(31 checkboxes, 0 checked — see "Why the old plan failed" below).

**Absorbs as Track K:** `docs/superpowers/plans/2026-07-28-pii-candle-phase1-minimal.md`. That
plan was written when Studio was the only surface; it is still valid but no longer standalone.

**Baseline:** `main` @ `f100934`, plus uncommitted concurrency work in `facade.rs`,
`pii/config.rs`, `hacienda-cli/src/commands.rs`.

---

## The product

**A folder of documents in; a RAG-ready markdown vault out — with every entity linked, a
glossary, and PII under the user's control.**

That is the whole product. What changed since the last plan is that this repo now has
**three surfaces that should all produce it**, and only one of them is in the browser:

| Surface | For | State |
| --- | --- | --- |
| `apps/hacienda-studio` | A lawyer with a folder and no install; fully offline | Pipeline works, UI minimal, no vault output |
| `hacienda-cli` | Scripted/bulk use, CI, a firm's own machine | `extract`, `scan`, `config show`, `serve`; JSON/text output, no vault |
| `hacienda-api` | Integration into another system | `/v1/documents`, `/v1/documents/async`, `/v1/jobs/{id}`, `/v1/pii/{scan,redact,config}` with capability auth |

Studio's flow — **load models → drop a folder → extract to markdown → browse in a Finder-like
page → edit to add or remove PII → export the vault** — is the interactive expression of the
same thing `hacienda extract` should do non-interactively.

**Three consequences that reorganise everything below:**

1. **The vault is the product, not the UI and not the detection.** The zip is what the user
   keeps and what Claude reads. Design it once (Track I2) and make every surface emit it
   (Track J).
2. **The unit of work is a folder.** Studio takes `multiple` files with no directory support
   (`App.svelte:178` has no `webkitdirectory`) and emits a *flat* zip whose entries keep their
   source filename — `worker/pipeline.ts:363` writes markdown into a file still called
   `contract.pdf`. The CLI takes `inputs: Vec<String>` and emits JSON. Neither does
   folder-in/folder-out today.
3. **Capability asymmetry is the main risk, and the fix is now decided.** The Rust side has
   real redaction, AES-SIV reversible pseudonyms, a blake3 audit chain, and 20 regex patterns.
   Studio has 9 regexes, no audit chain, and no pseudonymisation — but it is the only surface
   with a neural model, a UI, and the offline guarantee. **As of 2026-07-30 the answer is not
   to keep both in step but to compile `hacienda-core` to wasm32 and have one engine**
   (Track L). Track C shrinks to holding the line until that lands.

The good news: the cross-document machinery already exists in Studio. `processFiles`
(`worker/pipeline.ts:310`) runs a shared `BatchEntityRegistry` across the whole batch and
calls `registry.inferRelationships(docId)` per document, so entities are already resolved and
related *across* files, not just within one. That is the hard part of "a folder of linked
markdown", and it has no equivalent on the Rust side.

---

## Why the old plan failed

Every "create file" step was done; every "integrate into pipeline.ts" step was skipped. The
plan carried full source for each new file, so the files got written verbatim and the call
sites never appeared. `pii-detector.ts` matches the plan's Phase 3 block character for
character, exports both `detectPII` and `redactPII`, and is imported by nothing.

**Consequence for this plan: no code bodies.** Each step names a call site, the file that
must change, and an observable check. Where code already exists, the step says "call it",
not "write it".

The old plan also had a real omission, not just skipped steps: it added `isAudio`/`isVideo`
branching to the worker but never widened the upload gate, so its own transcription feature
was unreachable even if fully executed.

---

## The constraint that drives everything

`tests/e2e/egress.spec.ts` asserts a host allowlist and states why: clients are avocats and
experts-comptables bound by secret professionnel (loi n° 71-1130 art. 66-5); document bytes,
extracted text, entity values and pseudonym mappings must never leave the browser. A request
to a CDN with a client contract open is a GDPR Chapter V transfer.

**Studio therefore cannot call `hacienda-api`.** `/v1/pii/scan` and `/v1/pii/redact` are real
and complete (`hacienda-api/src/routes.rs:93-103`, with capability middleware), but they serve
a different deployment with a different trust model. They are not Studio's backend.

### Decision (2026-07-30): everything compiles to WASM

The previous revision of this plan concluded that the product had two independent PII
implementations "and will keep having them". **That is reversed.** The browser is the
privileged target: it is the only one that satisfies secret professionnel without a trust
argument. So `hacienda-core` compiles to `wasm32-unknown-unknown`, and Studio embeds it
rather than reimplementing it in TypeScript.

This is not the rewrite it sounds like, for two reasons.

**First, the WASM extraction and detection stack already exists and is already installed.**
The prebuilt `@xberg-io/xberg-wasm` package is not just a NER model — reading its exports:

| Capability | Export | Status |
| --- | --- | --- |
| Document extraction | `extract(input, config)`, `extractBatch(inputs, config)` | Present |
| Neural NER | `class NerModel` — `load({weights,tokenizer,encoderConfig})`, `detect()` | Present |
| Redaction engine | `WasmRedactionConfig` (`strategy`, `preserveOffsets`, `customTerms`, `customPatterns`), `WasmRedactionFinding`, `WasmRedactionReport`, `WasmPiiCategory` | Present |
| OCR / layout | `registerOcrBackend`, `detectLayout`, `detectOrientation` (pure-Rust `tract`) | Present |
| Zip | `compress(entries, datas)`, `decompress(src, pwd, f)` | Present |

`preserveOffsets` on `WasmRedactionConfig` is worth noting: it is the exact primitive Track F4
needs for the offset problem, already implemented on the Rust side.

**Second, hacienda-core's wasm32 blockers are a bounded, enumerated list** — not the
architectural impossibility previously claimed. Verified by reading source:

| Blocker | Where | Why it is tractable |
| --- | --- | --- |
| `axum` | `auth/authz.rs` only (4 references) | Server-only concept. `#[cfg]` the module out; nothing else in the crate touches it. |
| `std::fs` | `audit/sink.rs:11`, `audit/store_file.rs:41`, `audit/segment.rs:76`, `review/store_file.rs:53`, `pii/config.rs:95`, `facade.rs:2430` | All are *store implementations behind existing traits* (`AuditStore`, `ReviewStore`, `JobStore`). WASM gets an IndexedDB impl; the traits do not change. |
| `tokio` `rt-multi-thread` + `fs` | workspace `Cargo.toml:42` | `spawn_blocking` appears only in `audit/store_file.rs:447,587,652` — inside the file store that is already being gated out. `tokio::sync::Mutex` works on wasm32. |
| `xberg` with `tokio-runtime` | workspace `Cargo.toml:71` | xberg already ships `wasm-target = ["no-ort-target", "excel-wasm", "ocr-wasm", "layout-tract", "auto-rotate-tract"]` and a `#[cfg(not(feature = "tokio-runtime"))]` sync path. Swap the feature per target. |

**Three silent-failure risks that matter more than the blockers**, because they compile
cleanly and then produce wrong data rather than an error:

1. **`Instant::now()` at `redaction/engine.rs:71` panics on `wasm32-unknown-unknown`.** This is
   the one genuine code defect in a module that *must* run in the browser — it is not a file
   store that can be gated away. `SystemTime::now()` at `redaction/engine.rs:190`,
   `audit/sink.rs:176` and `audit/store_file.rs:1008` needs the same treatment.
2. **`uuid` is declared `features = ["v4", "serde"]` with no `js` feature** (workspace
   `Cargo.toml`). There are 5 `Uuid::new_v4()` call sites, including audit segment IDs
   (`audit/segment.rs:126`) and review items (`review/queue.rs:84`). Without a `getrandom` JS
   backend this fails to build on wasm32 — or worse, yields non-random IDs.
3. **`chrono` currently keeps its default features**, so `wasmbind` is on and `Utc::now()`
   works. There are 11 `Utc::now()` sites, all of them timestamps in the audit chain, review
   queue, or compliance report. **Do not add `default-features = false` to `chrono`.** If
   anyone does, every audit timestamp silently becomes 1970 on wasm32 — a compliance artefact
   that is wrong and self-consistent, which is the worst kind of wrong.

**The 50 MB cliff.** `xberg_wasm_bg.wasm` is **48,060,064 bytes**. The xberg WASM constraints
note that jsDelivr enforces a 50 MB per-file cap and that tree-sitter was already dropped to
stay under it. There is **~1.9 MB of headroom**, and hacienda-core has to fit in it. This —
not the porting work — is the binding constraint on Track L, and it must be measured before
the port is designed, not after. Self-hosting the `.wasm` (Studio's allowlist already permits
its own origin) removes the cap but not the download cost.

**Consequence for the rest of this plan:** Track C changes meaning. It was "stop the two
engines diverging"; it becomes "collapse them" (see Track L). Any step in Tracks A/B/F that
reimplements Rust behaviour in TypeScript is now a step that will have to be deleted — prefer
waiting for Track L over building a second implementation with a shelf life.

---

## Verified state

Confirmed by reading source, not by trusting the previous plan.

| Component | State | Evidence |
| --- | --- | --- |
| Document extraction → NER → vertical → KG export | Works | `worker/pipeline.ts:133-307`; `tests/e2e/pipeline.spec.ts` passes |
| KG export (Cypher / NetworkX / RDF) | Works | `lib/kg-export.ts`, wired at `worker/pipeline.ts:389` |
| Egress allowlist | Enforced | `tests/e2e/egress.spec.ts` |
| PII detect + redact (browser) | Written, never called | `lib/pii-detector.ts:22`, `:39` — zero importers |
| `enablePiiDetection`, `redactPiiInOutput` | Dead toggles | UI at `ConfigPanel.svelte:130`, default at `types.ts:99`; worker never reads either |
| Neural NER backend | Written, never called | `lib/asset-loader.ts:117` `createNerBackend()` — zero callers |
| `NerModel` in xberg-wasm rc.42 | Real, matches call site | `xberg_wasm.d.ts:18` — `static load({weights,tokenizer,encoderConfig})`, `detect(text,opts)` |
| Extraction in WASM | Already shipped, unused by Studio | `xberg_wasm.d.ts:4487` `extract()`, `:4492` `extractBatch()` |
| Redaction engine in WASM | Already shipped, unused | `WasmRedactionConfig:3476` (incl. `preserveOffsets`), `WasmRedactionFinding:3504`, `WasmPiiCategory:3224` |
| Zip in WASM | Already shipped, unused | `xberg_wasm.d.ts:4444` `compress()`, `:4457` `decompress()` |
| WASM artefact size | 48,060,064 bytes vs 50 MB CDN cap | `pkg/web/xberg_wasm_bg.wasm`; cap documented in xberg wasm-constraints |
| `hacienda-core` on wasm32 | Blocked, but by an enumerated list | `axum` in `auth/authz.rs` only; `std::fs` in 6 store/config sites; `Instant::now()` at `redaction/engine.rs:71` |
| Actual browser NER | compromise.js, English-only | `lib/ner-bridge.ts:112` — `doc.people()`, `.organizations()`, `.places()` |
| GLiNER2 weights | Downloaded, unused | `App.svelte:58` calls `loadNerModel()`; 1.23 GB into IndexedDB |
| `@lmoe/gliner-onnx` | Installed, never imported | `package.json` dependency; zero references in `lib/`, `worker/` |
| Transcription in worker | Correct but unreachable | `worker/pipeline.ts:143-165`; blocked by upload gate |
| Upload gate | Excludes audio/video | `App.svelte:178` `accept=`; `asset-loader.ts:157` prefixes |
| Rust redaction | Real | `redaction/engine.rs` via `pipeline.rs:174`; asserted `pipeline.rs:334` |
| Rust reversible pseudonyms | Real, AES-SIV | `redaction/pseudonym.rs:520` `reveal()`; round-trip tests `:934`, `:943` |
| Rust blake3 audit chain | Real, fsynced to disk | `audit/store_file.rs`; `facade.rs:682` stores digests only, never span text |
| Rust neural NER | Dead in all default builds | `ner-candle` absent from `default = ["jobs"]` (`hacienda-core/Cargo.toml:9`); `pipeline.rs:240` returns `ModelUnavailable` |

---

## Track A — Make the dead configuration real (Studio)

Highest value per unit of work. Every toggle here already exists in the UI and lies to the user.

- [x] **A1. Call `detectPII` from the worker, gated on `config.enablePiiDetection`.**
      `worker/pipeline.ts`, after markdown is produced (~line 200, before the NER block).
      *Check:* a document containing `jean.dupont@cabinet-exemple.fr` yields a PII entity in
      the result; with the toggle off, it yields none.

      **Done via Track L6** — delivered by wiring `enablePiiDetection` to
      `scanForPii`/`redactPii` (the compiled `hacienda-wasm` engine), not a TypeScript
      `detectPII`. Verified by the check above via manual Playwright runs in L6/F4/A2.

- [x] **A2. Call `redactPII` when `config.redactPiiInOutput` is set.**
      Must apply to the markdown that reaches the zip, and must run *after* NER so entity
      offsets are computed against unredacted text. Decide and record what happens to the
      entity registry and KG export under redaction — exporting a redacted document beside a
      knowledge graph naming every entity would defeat the feature. *Check:* extend
      `tests/e2e/egress.spec.ts`'s contract fixture — the downloaded markdown must not contain
      the IBAN, and the KG export must not reintroduce it.

      **Done 2026-07-30.** The redact call itself came with L6; this closes the "decide and
      record" half the plan flagged as still open through F4/C1/L7. Decision: an entity is
      dropped from the frontmatter, the `## Entities` glossary, `entities-registry.json`,
      and every KG export format if *any* of its mentions overlaps a PII finding —
      under-including, not partial redaction of the registry row, is the safe default.
      `worker/pipeline.ts`: `filterExportableEntities` (new, exported, unit-tested)
      applies this before entities are ever enriched or handed to `registry.addEntity()`,
      so the exclusion is upstream of every export surface, not a per-file patch. PII
      detection was moved earlier in `processFile` (before entity enrichment) so its
      findings exist before that filter runs — same coordinates `renderAnnotatedMarkdown`
      already used, no new offset math.

      **Verified for real:** `worker/pipeline.test.ts` gained 4 cases for
      `filterExportableEntities` (drops on overlap, keeps non-overlapping, drops the whole
      entity for a partial-mentions overlap, no-op in scan-only mode) — 9/9 in that file,
      58/58 full suite. New Playwright test in `tests/e2e/egress.spec.ts` (`"PII redaction
      export contract"`, using the same `CONTRACT` fixture the egress tests already share)
      asserts the downloaded markdown, `entities-registry.json`, and all three
      `kg-export/*` files contain no occurrence of the fixture's IBAN — run for real
      (temporarily pointing `launchOptions.executablePath` at this sandbox's pre-installed
      Chromium build, then reverted; not a committed change) alongside the two existing
      egress tests, 3/3 passing. Manually re-ran the full L6/F4 browser reproduction
      (misclassified Phone/Email entities from IBAN/card-number digit runs) and confirmed
      zero PII needles in markdown, registry, or any KG export file — only the genuine
      "Jean Dupont" entity survives.

- [x] **A3. Widen the upload gate for audio/video.**
      Add `audio/` and `video/` to `SUPPORTED_MIME_PREFIXES` and matching extensions to
      `accept=` (`App.svelte:178`). Note `SUPPORTED_MIME_PREFIXES` and `validateFile` are
      **duplicated verbatim** in `lib/asset-loader.ts:157` and `lib/types.ts:104` — collapse
      to one before editing, or the fix lands in the copy nobody imports. `App.svelte` imports
      the `asset-loader` one. *Check:* an `.mp3` reaches `worker/pipeline.ts:149`.

      **Done 2026-07-30.** Confirmed the duplication was exactly as described and that
      nothing imports the `lib/types.ts` copy (`App.svelte` imports `validateFile` from
      `lib/asset-loader.ts` only; grepped the whole app for other importers — none) — deleted
      it outright rather than leaving dead code as a second place to forget to update next
      time. Added `"audio/"`/`"video/"` to `asset-loader.ts`'s `SUPPORTED_MIME_PREFIXES`
      (the live copy) and `.mp3,.wav,.m4a,.ogg,.flac,.aac,.mp4,.mov,.webm,.mkv` to
      `App.svelte`'s `accept=`, plus the drop-zone hint text.

      **Verified**: new `validateFile` tests in `lib/asset-loader.test.ts` (audio and video
      MIME types accepted; empty/oversized/unsupported-type rejections still behave as
      before) — 5 new cases, 63/63 full suite. `tsc --noEmit`, `svelte-check`, `typos`, and a
      production `vite build` all clean. Not independently re-verified with a live browser
      upload — `validateFile` is the exact function `App.svelte` calls before a file ever
      reaches the worker, so the unit test is a direct check of the gate itself, not a proxy
      for it. Did not touch what happens *after* the gate: with `enableTranscription` off
      (the default), an audio/video file now reaches `engine.extract()` instead of
      `WhisperBridge`, which is untested territory this item didn't ask about.

- [x] **A4. Stop reporting success on model-load failure.**
      `App.svelte:60-62` catches the download error and sets `assets.nerModel = true` anyway;
      the outer catch at `:71` does the same. Surface degraded state instead — this is what
      would hide a Track B regression. *Check:* with the model URL blocked, the UI says the
      neural backend is unavailable.

      **Done 2026-07-30.** `assets.nerModel` still ends up `true` on failure — Onboarding's
      "Get Started" gate (`disabled={!(assets.xbergWasm && assets.nerModel && assets.tessdata)}`)
      requires it, and regex-only fallback is a legitimate degraded mode, not a hard failure
      that should trap the user in onboarding forever. Added a separate `nerModelDegraded`
      flag that both catch blocks set (in addition to `error`, so the failure is named, not
      just logged to console). `Onboarding.svelte` takes `nerModelDegraded` as a prop and
      shows "⚠️ Unavailable — using fallback" (amber, distinct `.degraded` list-item style)
      on the neural-backend row instead of "✓ Cached" when it's set — this is the surface the
      Check actually asks about, since the post-onboarding `.error-banner` only renders after
      `onboardingComplete`, i.e. after the user has already clicked past the screen where a
      blocked model download would happen.

      **Verified:** `svelte-check` clean on both edited files (114 files, 0 errors from this
      change; the 3 remaining errors are pre-existing, tied to the unbuilt `hacienda-wasm`
      package). Existing `lib/asset-loader.test.ts`/`ner-bridge.test.ts`/`types.test.ts`
      (17 tests) unaffected. `vite build` transforms both files cleanly, failing only later on
      the unrelated, pre-existing unresolved `hacienda-wasm` import (needs a separate
      `wasm-pack build`, orthogonal to this change). No component-test harness exists in this
      app (no `@testing-library/svelte`, no prior `.svelte` test anywhere in the codebase) —
      did not introduce one for a single small UI-state fix; verified by direct code reading
      against the Check's literal wording instead of a new automated test.

---

## Track B — Use the model that is already downloaded (Studio)

This is the fix for the product's worst behaviour, not polish: compromise.js is English-only
and the documents are French. The downloaded model is `GLiNER2-Guardrails-PII-Multi` —
multilingual and PII-specific.

- [x] **B1. Decide the inference backend. Blocking; do this first.**
      Two paths exist and neither is used. `xberg-wasm`'s `NerModel` — `createNerBackend()` is
      already written against it and its `detect()` return shape
      (`{category,text,start,end,confidence}`) already matches what the worker's bridge
      consumes. Versus `@lmoe/gliner-onnx`, which is installed, unused, and would need ONNX
      weights rather than the safetensors currently cached.
      **Recommendation: xberg-wasm; delete the `@lmoe/gliner-onnx` dependency.** It is the one
      the existing code targets, and carrying two unused neural backends is how this state
      arose. Record the decision in this file — the reason to revisit is B3, not preference.

      **Decided 2026-07-30: xberg-wasm, per the recommendation above.** Re-verified both
      halves of the premise still hold post-merge: `createNerBackend()` (`asset-loader.ts:117`)
      exists and is unused by `worker/pipeline.ts` (which still calls `extractEntities` from
      `ner-bridge.ts`, i.e. compromise.js, at both NER call sites); `@lmoe/gliner-onnx` had no
      references anywhere in `apps/hacienda-studio` outside `package.json`. Removed the
      dependency (`package.json`, `npm uninstall`) rather than leaving an unused neural
      backend beside the one B2 will actually wire up. B2 is now unblocked — this decision is
      exactly what it was waiting on.
      **Verified:** `svelte-check` — no new errors from this change (16 errors both before and
      after, all pre-existing/unrelated: `hacienda-wasm` unbuilt, missing `@types/node`).
      `vitest run` — same pre-existing failures before and after (`pipeline.test.ts` and
      `pii-engine.test.ts` fail on the unbuilt `hacienda-wasm` pkg; `ner-bridge.test.ts`'s
      slow test is flaky under parallel load on this host, passes in isolation, confirmed by
      rerunning both before and after removing the dependency).

- [x] **B2. Call `createNerBackend()` and route the bridge through it.**
      `worker/pipeline.ts:204-212` constructs `XbergEngine` with `{ner:{ner: extractEntities}}`.
      Load the model bytes in the worker (IndexedDB is worker-accessible, so `loadNerModel()`
      can be called there) and substitute `runtime.detect`. Keep `extractEntities` as the
      fallback when the model is unavailable — that path is what A4 must surface.
      *Check:* a French fixture naming `Maître Jean Dupont` and `Acme SAS` yields a `person`
      and an `organization`. Assert this fails against compromise.js first.

      **Done 2026-07-30.** Added `initNerBackend()` to `worker/pipeline.ts`, run alongside
      `initWasm`/`initPiiEngine` in `initEngine()`'s `Promise.all`: calls `loadNerModel()`
      (hits the IndexedDB cache `App.svelte`'s onboarding already populated — no re-download)
      then `createNerBackend()`; on any failure logs a warning and leaves `nerRuntime = null`.
      New `selectNerBridge(runtime: NerRuntime | null)` is a pure function — takes the runtime
      explicitly rather than closing over module state, for testability — returning
      `extractEntities` when `runtime` is `null`, otherwise an adapter calling
      `runtime.detect(text, {categories})`. Both `XbergEngine` construction sites
      (extraction branch and the standalone NER pass) now pass `{ner: {ner:
      selectNerBridge(nerRuntime)}}` instead of the hardcoded `extractEntities`.

      The *Check*'s "assert this fails against compromise.js first" is
      `lib/ner-bridge.test.ts`'s new test, empirically confirmed via a manual `node -e` probe
      before writing it: compromise.js returns `organizations: ["Jean Dupont"]` (misclassified)
      and `persons: []` for the fixture, missing "Acme SAS" entirely. Two new tests in
      `worker/pipeline.test.ts` cover `selectNerBridge` itself: the `null`-runtime fallback
      resolves to `extractEntities` by reference, and a mocked runtime's `detect()` is called
      with `{categories}` and its results pass through unchanged.

      **Verified:** built `crates/hacienda-wasm`'s missing `pkg/` (`wasm-pack build --target
      web --no-default-features`, 157m on this host — resource-constrained, not a build
      defect; produced `hacienda_wasm_bg.wasm` at 19.3 MB raw / 12.2 MB after `wasm-opt`,
      consistent with L2's `wasm-target` feature set which already includes `ner-candle-wasm`/
      `excel-wasm`/`layout-tract`, not a PII/redaction-only build). This unblocked real
      execution of `worker/pipeline.test.ts` and `lib/pii-engine.test.ts`, which had been
      silently failing to load entirely (unresolved `hacienda-wasm` import) since before this
      session — full `vitest run`: **66/66 passing**, including the 9 pre-existing
      `pipeline.test.ts` tests (`renderAnnotatedMarkdown`, `filterExportableEntities`) that
      this also newly unblocked. `svelte-check`: 13 errors, all pre-existing `@types/node`
      gaps in Playwright config/specs, none touching `pipeline.ts`/`asset-loader.ts`/
      `ner-bridge.ts`. `vite build`: succeeds end-to-end, 120 modules transformed (previously
      failed at 105 on the unresolved `hacienda-wasm` import) — `hacienda_wasm_bg.wasm`
      (12.2 MB) and `xberg_wasm_bg.wasm` (48.1 MB) both bundle cleanly.

- [x] **B3. Resolve model delivery — 1.23 GB is not shippable.**
      Unquantized safetensors, fetched from huggingface.co on first run. Options: quantized
      weights, a smaller GLiNER2 variant, or self-hosting via the `VITE_MODEL_BASE_URL`
      override that `asset-loader.ts:19` already provides (EU-resident hosting is a selling
      point for these clients, not just a size fix). If quantization forces ONNX, B1 reverses.
      *Check:* record measured first-run download size and time-to-first-document.

      **Superseded by Track H** (see "supersedes B3" at that track's heading) — the f16
      conversion decision there is this task's actual resolution; not independently tracked
      as a separate decision. The measurement below (landed on `main` independently of that
      note) is this item's own concrete evidence for why 1.23 GB isn't shippable.

      **Measured 2026-07-30.** Sizes via `HEAD` against the resolved CDN URLs (not the
      redirect stub): `model.safetensors` 1,228,421,964 bytes, `tokenizer.json` 16,020,604
      bytes, `encoder_config/config.json` 895 bytes — **1,244,443,463 bytes (~1.16 GiB)
      total**, confirming the plan's "1.23 GB" figure. Raw bandwidth to huggingface.co from
      this host: ~5.5 MB/s (50 MB range request), projecting ~227s for the transfer alone at
      that rate — but that number turned out not to be the binding constraint.

      Ran the real onboarding flow end-to-end in headless Chromium (Playwright, freshly
      installed for this — not previously present in the sandbox) against the actual dev
      build, not a mock: `xbergWasm` preload completes in ~4s (cached), then
      `loadNerModel()` (`asset-loader.ts`) fetches and attempts to cache the three files.
      Two independent runs, two independent failures, neither a network timeout:

      - Run 1: at +598.5s, `db.transaction(...).put(modelData, ...)` rejected with
        `QuotaExceededError` — surfaced correctly through the existing A4 fallback path
        (`nerModelDegraded = true`, `assets.nerModel = true`, warning logged), not a crash.
        Disk itself was not full (30 GB free) — this is IndexedDB's own storage-quota
        accounting for the origin rejecting a single ~1.2 GB write, plausibly harsher for
        Playwright's fresh/ephemeral profile than an installed browser's persistent-storage
        grant, but there is no code path here that requests `navigator.storage.persist()` to
        get a larger grant regardless of profile type.
      - Run 2 (immediately after, same build, longer test timeout): the renderer **target
        crashed outright** before reaching the button-enabled state — consistent with `fetch`
        buffering the full ~1.2 GB response into a single in-memory `ArrayBuffer`
        (`fetchAsset()`, `asset-loader.ts:33`) on a host with ~3.7 GB total RAM.

      Both failures are caused by the same design choice: one `fetch().arrayBuffer()` and one
      `IndexedDB.put()` per file, no chunking/streaming. This means the risk isn't just "slow
      on a bad connection" as the raw byte count implies — it's an outright crash or a
      storage-quota rejection on RAM- or quota-constrained clients, independent of bandwidth.
      Directly strengthens the case that this needs fixing before shipping to real users, not
      just before shipping to users with slow connections.

      **What this does not resolve:** the choice between quantized weights, a smaller GLiNER2
      variant, and self-hosting via `VITE_MODEL_BASE_URL` is still open — it is a product/
      infrastructure decision (self-hosting needs a hosting destination and, per L1's
      precedent and H1's blocker, this sandbox has no authorized publish target), not
      something to pick unilaterally here. Whichever option is chosen, `fetchAsset`/
      `loadNerModel` also need to move off single-shot `arrayBuffer()` and `put()` toward
      streaming/chunked transfer — that's true regardless of which size-reduction option is
      picked, and arguably matters more than the size number itself given what actually
      failed here.

---

## Track C — Hold the line until the engines collapse (cross-cutting)

**Revised 2026-07-30.** This track no longer manages a permanent split; it keeps the two
implementations from diverging *further* during the window before Track L lands, and it is
retired once Studio runs `hacienda-core` compiled to wasm32.

The rule for that window: **do not close a gap by writing TypeScript that Track L will
delete.** A missing detector in the browser is cheap to live with for a few weeks; a second
TypeScript pseudonymisation engine is not.

- [x] **C1. Write down the split as temporary.** One short section in the Studio README:
      today browser = offline, neural, no audit chain; CLI/API = server, regex, real audit
      chain and reversible pseudonyms; **target = one engine, `hacienda-core` on wasm32.**
      Without the last clause, someone will keep trying to wire Studio to `/v1/pii/scan`, or
      will "helpfully" port a Rust detector into TypeScript.

      **Done 2026-07-30.** `apps/hacienda-studio/README.md` didn't exist; created it with a
      "Relationship to the CLI/API" section. Written to reflect *today's* split, not the
      pre-Track-L one this item originally described — most of it has already closed: PII
      detection/redaction is one engine now (both call `hacienda-core` on wasm32), and the
      audit chain exists in both (differing in durability, not in kind — C3's still open).
      What's still genuinely split: reversible pseudonymization (Rust-only, F2 not built)
      and NER (Studio's live pipeline runs `lib/ner-bridge.ts`'s regex/compromise.js bridge,
      not xberg-wasm's downloaded-but-uncalled GLiNER2 model, and not `hacienda-core`'s own
      `ner-candle` — which is compiled out of every default build, not just wasm32). Kept
      the original's warning against wiring Studio to `/v1/pii/scan` or porting a Rust
      detector into TypeScript.

- [x] **C2. Measure the coverage gap; close it only in the direction that survives.**
      Rust has 20 patterns across 19 categories (`pii/patterns.rs:9-160`) against the browser's
      9. Rust lacks **EU VAT** and **driver's license** — both present in the browser and both
      relevant to French clients. **Add those two to Rust** (they survive Track L, and they are
      the two the French client base actually needs). Do *not* port the 10 the browser is
      missing — IP, MAC, JWT, API key, secret token, crypto wallet, MRN, VIN, routing number,
      date of birth — into TypeScript; Track L delivers them for free. Browser BIC maps to Rust
      `SwiftBic`.
      *Check:* one shared fixture corpus, asserted from both `vitest` and `cargo test`. The
      corpus outlives Track L and becomes its acceptance test — build it now, so the port has
      a target to be measured against.

      **Done 2026-07-30.** Added `PiiCategory::EuVat` (`pii/types.rs`) and its pattern —
      `(?i)\b(?:AT|BE|...|SK)\d{2,12}\b` over the 27-country EU/EEA VAT-prefix list,
      `format_preserving: true`, `[VAT:****]` — plus a `DriversLicense` pattern (Rust already
      had the category, just no pattern for it): `\b[A-Z]\d{7,13}\b`, `[LICENSE:****]`. Both
      appended to `builtin_patterns()` so existing precedence ("earlier wins on overlap") is
      undisturbed. Three new unit tests in `pii/engine.rs`, including a Greek-prefix case
      (`EL` rather than `GR`, per the actual ISO/VIES prefix) to catch a plausible copy-paste
      mistake.

      `fixtures/pii-corpus.json` (repo root, 25 cases) is the shared corpus: category names
      in `PiiCategory`'s serde form (snake_case), asserted as an exact-match `HashSet` (not
      `BTreeSet` — `PiiCategory` has no `Ord`) of detected categories per case. Two runners
      against the same file: `hacienda-core/tests/pii_corpus.rs` (native, `include_str!`)
      and `apps/hacienda-studio/lib/pii-engine.test.ts` (`vitest`, against the real compiled
      `hacienda-wasm` build, not a mock). Both pass 25/25 as of L6's wasm wiring — this
      check couldn't be fully closed until L6 gave the corpus a wasm build to run against.

- [x] **C3. Studio gets the real audit trail via Track L, not a second implementation.**
      Previously an open question. It is now answered by the WASM decision: the blake3 chain
      and AES-SIV reversible pseudonyms come to the browser as the same Rust code, with an
      IndexedDB `AuditStore` in place of `FileAuditStore`. **Do not build a TypeScript audit
      chain.** The only open sub-question is *persistence semantics*: a file-backed chain
      survives the process, an IndexedDB one dies with a cleared browser profile. If Studio
      output must be legally defensible, the chain has to be exported inside the vault
      (Track I2 / G) rather than merely retained locally.

      **Directive honored 2026-07-30 — no TypeScript audit chain was ever built** (every
      track up to that point either built on the Rust `AuditStore`/`hacienda-wasm` or left
      the audit chain untouched); the JS-facing surface and Studio wiring that actually
      closes the sub-question below landed the next day, on `main`, independently of this
      branch.

      **Done 2026-07-31.** As of L5, `IndexedDbAuditStore` existed in `hacienda-core` but
      had zero JS-facing surface and zero Studio callers — the heading's premise ("it is now
      answered") was aspirational, not actual, until this. Added:

      - `AuditHandle` (`crates/hacienda-wasm/src/lib.rs`, `#[cfg(target_arch = "wasm32")]`
        so the native `cargo check` this repo's other tracks rely on stays unaffected):
        `AuditHandle.open(db_name, node_id, config_hash)` opens/resumes an
        `IndexedDbAuditStore`; `recordResult(result)` takes the `JsValue` `redactPii`
        already gets back from `process()`, converts its `audit_log`
        (`RedactionAuditEntry`) into `AuditEntryInput`s — minting `id` via `uuid::Uuid::
        new_v4()` and `pipeline_version` via `env!("CARGO_PKG_VERSION")`, mirroring
        `HaciendaFacade::record_audit`/`record_reveal`'s native pattern exactly, `config_hash`
        left for the store's own chain to overwrite (`AuditChain::push`) — appends one batch
        per call, and returns the new tip; `tip()`/`verify()` expose the rest of the read
        surface a client needs.
      - `apps/hacienda-studio/lib/pii-engine.ts`'s `redactPii` now opens one `AuditHandle`
        (idempotent, module-level, mirroring `initPiiEngine`'s own pattern) against a fixed
        `db_name`/`node_id`/`config_hash` and calls `recordResult` after every `process()`
        call — one `append` per document, the same invariant `HaciendaFacade::record_audit`
        enforces natively. `scanForPii` is untouched: `PiiPipeline::scan` always returns an
        empty `audit_log`, so there is nothing to record in scan-only mode.

      **Verified:** `apps/hacienda-studio/lib/audit-handle.test.ts` (new, 3 tests) against
      the real compiled `hacienda-wasm` build (not a mock) with `fake-indexeddb` standing in
      for the browser's `indexedDB` global the same way `pii-engine.test.ts` already loads
      the real `.wasm` bytes directly under `vitest`'s Node environment — asserts the tip
      advances past genesis and verifies when a redaction is recorded, stays unchanged when
      `audit_log` is empty (scan-only shape), and survives a fresh `AuditHandle.open()` on
      the same `db_name` (Track L5's own reload guarantee, exercised through the new JS
      surface rather than only `tests/idb.rs`). Full `vitest run`: 69/69 (68 pre-existing +
      the 3 new, minus 1 unrelated flake — `lib/ner-bridge.test.ts`'s compromise.js timing
      test failed only under full-suite parallel load on this RAM-constrained host, passed
      6/6 in isolation, unchanged by this work). `svelte-check`: no new error class beyond
      the pre-existing `node:module`/`node:fs` gap `pii-engine.test.ts` already has. `vite
      build`: succeeds end-to-end, `hacienda_wasm_bg.wasm` unchanged at ~12.3 MB (`Audit
      Handle` added no new heavy dependency — `uuid`'s `js` feature is the same
      self-contained WebCrypto binding `hacienda-core` already uses, not a new crate class).
      `cargo check -p hacienda-wasm` (native): passes, confirming `audit_handle` mod's
      `#[cfg(target_arch = "wasm32")]` gate keeps it fully compiled out on native targets.

      **What this does not resolve:** exactly what the heading already scoped as open —
      persistence is a browser IndexedDB database, which survives a reload (verified above)
      but not a cleared profile. Whether/how a chain gets exported into the vault so it
      survives that is still Track I2's question, and Track I2 remains entirely unbuilt (a
      design proposal, not a finding) — nothing here decides it. No `principal` is recorded
      either: Studio has no caller-identity concept today, so every entry's `principal` is
      `None`, unlike `HaciendaFacade`'s server-side callers which thread a `Caller` through.

---

## Track D — Tests that would have caught this

Current suite: 3 Playwright specs, 5 vitest files. The e2e pipeline test passes and proves
extraction works. Nothing asserts PII, redaction, transcription, or that a toggle does
anything — which is precisely why 31 unchecked boxes coexisted with substantial working code.

- [x] **D1. Assert toggles have effect.** For each of `enablePiiDetection`,
      `redactPiiInOutput`, `enableTranscription`, `enableVerticalNer`: one test where the
      output differs with the flag on and off. A dead toggle then fails a test instead of
      shipping.

      **Done 2026-07-30.** Correction first: `enableVerticalNer` does not exist in
      `AppConfig` — the real config key is `enabledVerticals: ("m&a" | "financial_services" |
      "shared")[]`, a multi-select, not a boolean. Checking it turned up exactly what D1
      exists to catch: **it was dead**, in the same family as A1-A4.
      `processFiles`/`worker/pipeline.ts` loaded all three vertical taxonomies
      unconditionally (`["m&a", "financial_services", "shared"].map(loadVerticalTaxonomy)`),
      ignoring `config.enabledVerticals` entirely — the "Vertical NER" checkboxes in
      `ConfigPanel.svelte` did nothing. Fixed to `config.enabledVerticals.map(...)`.

      New `tests/e2e/toggles.spec.ts` (4 tests, on/off pairs) for `enablePiiDetection` and
      `redactPiiInOutput` — the two toggles a full pipeline run can assert on
      deterministically. `enabledVerticals`'s effect is asserted instead in
      `lib/verticals/dictionary.test.ts` (2 new cases: a restricted taxonomy set excludes
      that vertical's terms; an empty set resolves nothing) — a full e2e test would depend on
      the heuristic NER bridge happening to extract a taxonomy term as a named entity, which
      is not reliable enough to assert against. `enableTranscription`'s effect is folded into
      **D3** below rather than duplicated here, since D3 already needs an audio fixture and a
      fresh page per toggle state.

      **Verified for real**: all 4 new e2e tests pass, plus the 2 existing egress/pipeline
      e2e tests unaffected (12/12 full e2e suite with a temporary
      `launchOptions.executablePath` pointed at this sandbox's Chromium build, reverted
      before committing — not a committed change, same as prior sessions' e2e verification).
      65/65 `vitest run` (up from 63; the 2 new `dictionary.test.ts` cases), `svelte-check`
      0 errors, `tsc --noEmit` clean, production `vite build` clean.
- [x] **D2. French-language NER fixture** (supports B2).

      **Done 2026-07-30.** Added a French M&A fixture to `lib/ner-bridge.test.ts` and, while
      building it, found a real bug: `PHONE_PATTERN` only matched US-style 3-3-4 grouping, so
      a French number grouped in pairs from a leading 0 (`01 23 45 67 89`) — present in the
      existing `CONTRACT` fixture since this file's first version — was never matched by any
      test. Fixed with a second alternative in the pattern (`\b0\d(?:[-.\s]?\d{2}){4}\b`);
      added a dedicated test for it against the existing fixture too.

      The new fixture asserts two different things on purpose. First, the regex-based
      categories (date, email, phone) are language-independent and work correctly today —
      asserted positively. Second, `person`/`organization` — the categories that need
      `compromise`, which is English-only — are asserted as **currently, honestly broken**:
      `extractEntities` on `"Maître Jean Dupont a représenté Acme SAS dans l'acquisition de
      Beta SARL"` misses "Acme SAS" and "Beta SARL" as organizations entirely, and
      misclassifies the person "Jean Dupont" as an organization instead. This is not a
      fixture bug to fix here — it is the documented reason Track B exists (swap in
      xberg-wasm's multilingual `NerModel`). The test doc comment says explicitly: when B2
      lands, flip these assertions to positive ones against whatever bridge B2 wires in.

      **Verified for real:** confirmed exact current behavior against a live `extractEntities`
      call (via `vite-node`, not guessed) before writing assertions, so the "known-broken"
      test describes what the bridge actually does today, not an assumption. `vitest run`
      68/68 (up from 65 — 3 new: 1 phone-pattern regression on `CONTRACT`, 2 in the new
      French-fixture describe block), `svelte-check`, `tsc --noEmit`, production `vite build`
      all clean.
- [x] **D3. Audio fixture through the full pipeline** (supports A3; the old plan's Phase 5
      referenced an `audio/mpeg` fixture that was never added).

      **Done 2026-07-30 — and found transcription does not work at all, in any
      environment.** New `tests/e2e/audio.spec.ts` generates a minimal but genuinely valid
      16-bit PCM WAV (silence) programmatically — no binary fixture checked in — and drives
      it through the real upload gate and worker.

      **Real bug #1, not fixed here (architecture change, out of scope for a test-writing
      item):** `@remotion/whisper-web`'s `canUseWhisperWeb()` checks `typeof window ===
      "undefined"` and refuses to run otherwise. `worker/pipeline.ts` runs inside a Web
      Worker, which has `self`, not `window`. Every `WhisperBridge.load()`/`transcribeAudio()`
      call therefore throws synchronously — `Whisper Web is not supported: \`window\` is not
      defined` — in *every* environment, regardless of network access. This has nothing to do
      with this sandbox specifically (confirmed separately: `huggingface.co` is blocked by
      this sandbox's proxy policy too, a coincidental second reason it wouldn't work *here*,
      but not the actual cause). Fixing it means running whisper-web on the main thread and
      bridging results into the worker — real architecture work, not attempted here.

      **Real bug #2, fixed here:** `processFiles` awaited `whisperBridge.load()` once,
      upfront, outside every per-file `try`/`catch` and outside its own caller's
      (`self.onmessage`) — so bug #1's rejection, which is unconditional, was an *unhandled*
      promise rejection that silently hung the entire batch: no error banner, no download, no
      feedback of any kind. `worker/pipeline.ts` now wraps that call in its own `try`/`catch`;
      each audio file's own `transcribeAudio()` call retries `load()` (idempotent) and its
      failure surfaces through the normal per-file error path instead, exactly like any other
      file-processing failure.

      Given both of those, the tests assert what's actually true today rather than an
      aspirational outcome: the upload gate accepts audio (Track A3), and the two
      `enableTranscription` states produce two different, specific errors — `"Unsupported
      format: audio/wav"` from extraction vs. `"Whisper Web is not supported: \`window\` is
      not defined"` from the transcription attempt — proving the toggle still has a real,
      provable effect even though neither path currently succeeds. This satisfies D1's
      `enableTranscription` case, folded in here rather than duplicated in `toggles.spec.ts`.

      **Verified for real**, including the negative case (confirmed via live diagnostic runs,
      not assumed): all 3 new tests pass, and the full e2e suite (15/15, temporary
      `launchOptions.executablePath` for this sandbox's Chromium build, reverted before
      committing) stays green. `vitest run` 68/68 (unaffected — this track added no unit
      tests), `svelte-check`, `tsc --noEmit`, production `vite build` all clean.

---

## Track E — Adopt the hacienda-private UI (Studio)

Target flow: **upload → detect → pseudonymize → view/edit in a markdown editor with inline PII
→ download zip → open in Claude Desktop.**

`hacienda-private/apps/web` has 40 vendored components in `components/ui/`. It is real
shadcn/ui — Radix primitives plus Tailwind CSS variables (`components.json`). Beyond the
generic primitives it carries domain components Studio would otherwise build from scratch:
`pdf-viewer`, `docx-viewer`, `docx-editor`, `xlsx-viewer`, `xlsx-editor`, `pptx-viewer`,
`csv-viewer`, `file-dropzone`, `file-upload`, `file-system`, `bounding-box-citations`,
`document-splits`, `data-grid`, `schema-builder`.

- [x] **E1. Decide the framework. Blocking — everything in Track E and F depends on it.**
      shadcn/ui is React-only; Radix has no Svelte build. In Svelte the option is
      shadcn-svelte, a separate port — meaning hacienda-private's screens get *reimplemented*,
      not reused, and the domain components above have no equivalent at all.
      **Recommendation: rebuild Studio's shell in React and keep the worker pipeline as-is.**
      The asymmetry is the argument: Studio's entire UI is 653 lines across 4 `.svelte`
      components with no Tailwind (`app.css` is hand-rolled CSS variables), while everything
      valuable — `worker/pipeline.ts`, `kg-export.ts`, `ner-bridge.ts`, `verticals/`,
      `registry.ts`, `pii-detector.ts` — is framework-agnostic TypeScript that ports
      unchanged. The Svelte layer is the cheapest thing in the system to discard, and the
      target UI is far larger than what exists.
      *Counter-argument to weigh:* Studio ships as a static offline bundle; hacienda-private
      uses Next with `output: "export"`, which is compatible, but adopting Next drags in a
      heavier build for an app that has no routing needs today. Plain Vite + React + shadcn is
      the lighter path and keeps the existing Vite worker setup.
      *Decisive evidence:* the Finder-like page the product needs is
      `components/ui/file-system.tsx` — **4,586 lines** of virtualized file browser. Rewriting
      that in Svelte dwarfs the 653 lines of Svelte that would be discarded, and it is only
      one of 40 components.

      **Decided 2026-07-30: React + shadcn/ui, per the recommendation above** — plain Vite,
      not Next, to keep the existing Vite worker setup and avoid dragging in routing machinery
      this app doesn't need. Studio's shell (the 4 `.svelte` components, 653 lines, no
      Tailwind) gets rebuilt in React and adopts `hacienda-private`'s 40 vendored shadcn/ui
      components as-is, including `file-system.tsx` unmodified rather than reimplemented.
      Everything framework-agnostic (`worker/pipeline.ts`, `kg-export.ts`, `ner-bridge.ts`,
      `verticals/`, `registry.ts`) is unaffected and ports as-is into the new shell. This
      unblocks E2 (design tokens) and E3 (screens); Track F (PII review UX, pseudonymization,
      CodeMirror editor) can now proceed against a concrete component set instead of a
      hypothetical one.

- [x] **E2. Port the design tokens first, independently of E1.** Tailwind config + CSS
      variables are framework-agnostic and can land before any component work.

      **Done 2026-07-30.** `tailwind.config.ts` and the `--background`/`--primary`/etc.
      CSS variables in `app.css` are ported verbatim from
      `hacienda-private/apps/web/{tailwind.config.ts,app/globals.css}`, less that repo's
      Next.js-specific `content` globs (Studio's own file layout instead). Added
      `postcss.config.js` and `lib/utils.ts`'s `cn()` (`clsx` + `tailwind-merge`) — the
      standard shadcn helper every vendored component imports as `@/lib/utils`. Kept
      distinct custom-property names from the pre-existing hand-rolled `--color-*`/
      `--radius-*` set already in `app.css`, so nothing collided during the migration.

- [x] **E3. Screens.** Upload (`file-dropzone`), processing (`progress`), document view
      (`resizable` dual-pane), entity/PII panel (`card` + `badge` + `popover`), export.
      Studio needs no matters/folders/auth — that is hacienda-private's multi-user model and
      must not be imported along with the components.

      **Two of five screens done 2026-07-30 (upload, processing); the remaining three —
      document view, entity/PII panel, export — landed across Tracks F1/F3/I3/I4, not as a
      separable E3 step, since building them was the same work those tracks needed anyway.**
      The real prerequisite E1/E2 blocked on — a working React+Vite shell to build screens in
      at all — is now done, which is what made this progress possible: `MarkdownEditor.tsx`
      (F3) is the document view (a `resizable` dual-pane wasn't adopted — CodeMirror plus the
      adjacent `PiiPanel` list side by side covers the same need without vendoring that
      component), `PiiPanel.tsx` (F1/I4) is the entity/PII panel, `FileBrowser.tsx` (I3) is
      the batch-level list, and the existing zip download (predates this rewrite) is export.
      All five screens now exist; none of hacienda-private's matters/folders/auth model was
      imported along with them.
      `hacienda-private` wasn't attached to this session initially — cloned
      and registered mid-session (`/workspace/hacienda-private`) once the gap surfaced,
      since E3/F1/I3 all assume it's available to port from.

      Ported Studio's shell from Svelte to React+Vite, replacing all four `.svelte`
      components (`App.svelte`, `Onboarding.svelte`, `ConfigPanel.svelte`,
      `ProgressBar.svelte`, ~904 lines total including `main.ts`/`app.css`) with
      `App.tsx`/`components/{Onboarding,ConfigPanel,ProgressBar}.tsx`/`main.tsx`,
      styled with Tailwind utilities against the E2 tokens rather than the old
      hand-rolled `<style>` blocks. `worker/pipeline.ts` and every `lib/*.ts` module —
      framework-agnostic per E1's own reasoning — needed zero changes. `vite.config.ts`
      swapped `@sveltejs/vite-plugin-svelte` for `@vitejs/plugin-react`; `tsconfig.json`'s
      `jsx` changed from `"preserve"` to `"react-jsx"` and its `"@/*"` path alias now
      points at the app root (matching hacienda-private's own `components.json`
      convention) instead of `lib/`, so ported components need no import-path edits.
      Added the Radix packages E3's remaining screens will need
      (`dialog`/`popover`/`progress`/`scroll-area`/`select`/`slot`/`tabs`/`tooltip`) plus
      `class-variance-authority`/`lucide-react` as a foundation, without yet vendoring the
      `components/ui/*` files themselves — no screen here needs them yet.

      Found and fixed one thing while porting, not a defect in the port itself: every
      Svelte component's `<style>` block referenced `--spacing`, `--radius`,
      `--color-surface`, `--color-muted`, and `--color-error` custom properties that were
      never defined anywhere in the codebase — computed as invalid and silently fell back
      to each property's initial value the entire time these components existed. Not
      reproduced; the Tailwind rewrite uses the real E2 tokens instead of carrying a
      pre-existing invisible bug forward.

      **Verified for real:** `vitest run` 147/147, `tsc --noEmit` clean (script renamed
      from `svelte-check` to `tsc --noEmit`, now that there's no Svelte to check), and a
      production `vite build` clean (36 modules transformed, down from 121 — no Svelte
      compiler pass). Ran the **full existing e2e suite** for real against this sandbox's
      Chromium build (temporary `launchOptions.executablePath`, reverted before
      committing) — **16/16 passing on the first run**, unmodified from before the
      rewrite: onboarding, drop zone (including folder mode and `webkitdirectory`),
      config panel checkboxes matched by label text, PII toggle effects, redaction,
      audio upload/transcription-toggle errors, and the egress allowlist all behave
      identically under React. This is the regression net Track D's tests exist to be —
      a framework rewrite that changed observable behavior would have failed at least one
      of them.

---

## Track F — PII review, pseudonymization, and the editor (Studio)

This is the flow the user asked for, and it is where the two apps differ most.

- [x] **F1. Adopt the PII reveal UX; do not adopt its data model.**
      `PiiPanel.tsx` is the pattern worth copying: detected spans render as tokens with a
      `Badge` for the kind, clicking opens a `Popover` that asks for a passphrase, and the
      plaintext is shown in a `Dialog` and forgotten on close — never persisted. The vault is
      WebCrypto-sealed client-side. `PiiReviewPanel.tsx` adds accept / correct / reject per
      detection. All of this is directly applicable to Studio and is the single most valuable
      thing to port.

      **Reveal UX done 2026-07-30; accept/correct/reject (the `PiiReviewPanel.tsx` half)
      landed under Track I4 (2026-07-30/31) — `PiiPanel`'s per-finding Remove button is the
      reject half, `MarkdownEditor`'s "Mark selection as PII" is the add/correct half,
      neither ported from `PiiReviewPanel.tsx` directly (see I4's writeup for why: Studio's
      own `PiiEntity`/`renderAnnotatedMarkdown` data model, not hacienda-private's).**
      `components/PiiPanel.tsx` adopts the Badge + passphrase-popup + forgotten-on-close
      pattern against Studio's own `lib/pii-engine.ts` `PiiEntity` and `lib/pseudonymize.ts`
      (Track F2), not hacienda-private's opaque non-reversible token / "matter passphrase"
      model. Diverges from the pattern in one way, discovered while wiring it up for real
      rather than assumed: hacienda-private's `PiiPanel.tsx` nests a `Dialog` inside the
      `Popover`'s content; doing the same with plain `@radix-ui/react-popover` +
      `@radix-ui/react-dialog` made the Popover dismiss itself the instant the Dialog
      opened, making the Dialog unreachable — confirmed live, not theoretical. Base UI
      (what hacienda-private now actually uses, not plain Radix — its `components/ui/*`
      files use `@base-ui/react`, contradicting the plan's "Radix primitives" description;
      see `components/ui/README.md`) may compose the two more gracefully. Fixed by putting
      the passphrase form and the revealed value both directly in one Popover's content —
      same UX contract, one overlay instead of two.

      Vendored `components/ui/card.tsx` verbatim (framework-agnostic). Wrote
      `badge.tsx`/`button.tsx`/`dialog.tsx`/`popover.tsx`/`input.tsx` from the classic
      `@radix-ui/react-*` + `class-variance-authority` shadcn/ui recipe instead of porting
      hacienda-private's own (Base UI-based) versions, to avoid adopting a second headless-UI
      toolkit and a Tailwind v4 migration neither this app nor Track E2 planned for.

      **Wiring `redactionMode: "pseudonymize"` into `worker/pipeline.ts` found a real bug,
      not a UI issue: `PiiEntity.text` (hacienda-core's `MergedEntity.text`) is documented
      "Empty for regex detections, which carry offsets only" — regex is the only detector
      active in Studio's default config, so every finding's `.text` was empty.** The first
      wiring attempt happily minted a pseudonym token for `""` on every finding — self-
      consistent (encrypts, decrypts, round-trips) and completely wrong, exactly the kind of
      bug a same-language round-trip test cannot catch (Track F2's own golden-vector note).
      Caught only by driving the real UI in a real browser and finding the revealed value
      was empty, not the diagnostic assertions in isolation. Fixed:
      `markdown.slice(f.start, f.end)` recovers the actual matched text from the same
      offsets `renderAnnotatedMarkdown` already treats as JS string indices (Track F4) —
      consistent with, not a new assumption on top of, this pipeline's existing offset
      handling.

      New `AppConfig` fields: `redactionMode: "mask" | "pseudonymize"` (default `"mask"`,
      so nothing changes unless explicitly opted into), `pseudonymPassphrase` (session-only,
      never persisted), `pseudonymKeyId` (default `"session"`). `ConfigPanel.tsx` exposes
      them only when `redactPiiInOutput` is on. The passphrase-to-key derivation itself
      (`deriveKeyHex`, PBKDF2-HMAC-SHA256, 600,000 iterations, deterministic per-`keyId`
      salt) is new in `lib/pseudonymize.ts` — Studio's own convenience layer on top of F2's
      primitive, deliberately not part of what has to match the CLI byte-for-byte (only the
      *token format* does; a Studio user and a CLI operator who want to interoperate
      exchange the derived 64-byte key itself, out of band — the CLI has no PBKDF2 step,
      `EnvKeyResolver` reads raw hex).

      `App.tsx` gained a minimal document-view section (the first of Track E3's three still-
      open screens) listing each processed file with a `PiiPanel` when it has PII findings —
      `ProcessedFile` gained a `piiFindings: PiiEntity[]` field to carry them from the worker.

      **Verified for real:** `pseudonymize.test.ts` gained `deriveKeyHex` tests (determinism,
      passphrase/keyId sensitivity, a full passphrase-to-reveal round-trip) — 151/151 full
      suite, `tsc --noEmit` and production `vite build` clean. New
      `tests/e2e/pseudonymize.spec.ts` (3 tests) — run for real against this sandbox's
      Chromium build, temporary `executablePath`, reverted before committing — is what
      actually found the empty-`.text` bug: mints in the worker, asserts the exported
      markdown carries `[EMAIL:session:...]` and neither the raw email nor the `[EMAIL]`
      mask template, then drives the real `PiiPanel` UI to reveal it back to the original
      value, and a third test proving a wrong passphrase fails closed. Full e2e suite
      19/19 (16 pre-existing + 3 new), confirming this didn't regress anything Track E's
      rewrite already covered.

- [x] **F2. Build reversible pseudonymization — it does not exist anywhere in JS yet.**
      hacienda-private does **redaction only**: opaque stable tokens `{{C0_PERSON_1}}` with
      no exportable mapping. Reversible pseudonymization exists only in Rust
      (`redaction/pseudonym.rs:520` `reveal()`, AES-SIV, `[CATEGORY:key_id:BASE32]`), and per
      the WASM analysis above that code cannot reach the browser. So Studio must implement it
      in TypeScript against WebCrypto.
      **Match the Rust token format exactly** so a document pseudonymized in Studio can be
      revealed by the CLI and vice versa. If that is not wanted, say so explicitly here —
      silently choosing a different format is how Track C's divergence happens again.
      *Check:* round-trip a document through Studio pseudonymize → CLI reveal.

      **Done 2026-07-30.** New `lib/pseudonymize.ts` implements AES-256-SIV (RFC 5297) from
      WebCrypto primitives — there is no native AES-SIV, AES-CMAC, or raw AES-ECB in
      WebCrypto, so it's built from what WebCrypto does have: native `AES-CTR` with
      `length: 128` (which is exactly RFC 5297's full-128-bit-counter `Ctr128BE`, not
      approximated), and `AES-CBC` with a zero IV as a single-block AES-ECB substitute (CBC's
      first output block depends only on the IV and first plaintext block, so encrypting one
      16-byte block and keeping only WebCrypto's first 16 output bytes, discarding its
      automatic PKCS#7 padding block, is exactly one ECB block encryption). AES-CMAC
      (RFC 4493) and S2V (RFC 5297 §2.4) are built on that primitive.

      Matching the Rust format exactly meant reading the vendored `aes-siv` crate source
      (`~/.cargo/registry/.../aes-siv-0.7.0/src/siv.rs`) directly rather than the RFC alone,
      since the RFC leaves some choices — which of the raw vs. IV-masked tag gets stored,
      the exact key-half ordering — as implementation decisions. Two are easy to get
      backwards silently: `Siv::new`'s own comment claims "the first half of the key" is the
      *encryption* key, but the slicing it actually does (`key[M::key_size()..]`) makes it
      the *second* half — the code was trusted over the comment, checked by making the
      golden-vector test below pass, not by re-reading the comment more carefully. And the
      token's stored tag is the **unmasked** S2V output; a separate masked copy (top bit of
      bytes 8 and 12 zeroed, the SIV paper's collision mitigation) is used only internally as
      the CTR IV and never appears in the ciphertext.

      The token label is `category_label()`'s Rust output —
      `{PiiCategory_variant:?}.to_uppercase()`, e.g. `PhoneNumber` → `PHONENUMBER` — not the
      `snake_case` form `PiiEntity.category` actually carries from the wasm PII engine
      (`phone_number`). `categoryLabel()` derives the former from the latter generically
      (strip underscores, uppercase) rather than hand-maintaining a lookup table, since the
      two are provably the same word sequence under different delimiters for every fixed
      `PiiCategory` variant — verified against all 32 of them in `pseudonymize.test.ts`, not
      asserted from the derivation alone.

      **Verified for real, three ways, in `pseudonymize.test.ts` (63 tests):**
      1. **RFC 4493 known-answer vectors** for the underlying AES-CMAC primitive (four
         official test cases plus the worked subkey-derivation example) — verified
         independently via a scratch Rust test using `cmac`/`aes` directly (not `aes-siv`),
         since this sandbox has no network access to fetch the RFC text to check a
         from-memory transcription against. Confirms `dbl`/block-chaining/padding are
         correct independent of anything the Rust side of this repo does.
      2. **Golden vectors captured from a real `Pseudonymiser::token()`/`.reveal()` run**
         (scratch test at `hacienda-core/tests/zz_golden_vectors.rs`, not committed — same
         `wasm-opt`-style temporary-dev-dependency pattern, `aes`/`cmac` added to
         `hacienda-core`'s dev-dependencies, reverted after) against a fixed key
         (`"07".repeat(KEY_BYTES)`) across four categories (email, phone, full name, IBAN).
         `mintToken()` reproduces the exact Rust-minted token string for every case;
         `revealToken()` recovers the exact normalized text from a Rust-minted token. This is
         the test a same-language round-trip cannot be: code wrong in the same way on both
         ends of a round-trip still round-trips.
      3. **Property/round-trip tests**: determinism, case/whitespace-variant collapse,
         category-scoped tokens, fail-closed on wrong key and on a tampered ciphertext,
         `pad`/`unpad`/`base32Encode`/`base32Decode` round-trips, an RFC 4648 base32 vector
         (`"foobar"` → `MZXW6YTBOI"`).

      `tsc --noEmit`, `svelte-check`, and a production `vite build` all clean — needed one
      fix along the way: `Uint8Array.prototype.slice()` returns
      `Uint8Array<ArrayBufferLike>` under this TypeScript version's stricter typed-array
      generics, which `crypto.subtle.*`'s `BufferSource` rejects (its backing buffer could
      in principle be a `SharedArrayBuffer`); a small `fresh()` helper re-copies to
      `Uint8Array<ArrayBuffer>` at each WebCrypto boundary.

      **What's still open, deliberately not touched here:** this module is not wired into
      `worker/pipeline.ts` or any UI — Track F1 (the reveal/review panel) and F3 (the
      CodeMirror editor) are the surfaces that would actually call `mintToken`/`revealToken`
      against a real document, and neither exists yet. There is also no `hacienda reveal`
      CLI subcommand to literally run for the Check's "CLI reveal" half — `--mode
      pseudonymize` only *mints* tokens during `extract` today; reversing one is only
      exposed as `Pseudonymiser::reveal()`, a library function, which is exactly what the
      golden-vector tests exercise directly rather than through a CLI invocation that
      doesn't exist. Key management (where a 64-byte key or its id would come from in a
      browser with no server-side secret store) is also untouched — out of scope for "build
      the primitive and prove it's compatible," in scope for whoever wires F1/F3 to it.

- [x] **F3. CodeMirror 6 markdown editor with inline PII decorations.**
      Nothing exists to build on: CodeMirror is not a dependency in any repo, and
      hacienda-private's right pane is a read-only `<pre>` of redacted text rendered with
      `react-markdown`. Use CodeMirror decorations to render PII spans as inline pills that
      open the F1 popover.
      *Check:* editing text before a PII span does not misplace that span's highlight.

      **Done 2026-07-30.** `lib/pii-decorations.ts`'s `piiHighlightExtension()` is a CM6
      `StateField<DecorationSet>` seeded from each `PiiEntity`'s `start`/`end` and mapped
      through every transaction (`decorations.map(tr.changes)`) — the Check's requirement is
      CM6's own built-in mechanism, not something this codebase reimplements.
      `components/MarkdownEditor.tsx` wires it into `@uiw/react-codemirror`
      (`@codemirror/lang-markdown`), rendered in `App.tsx`'s document-view section next to
      `PiiPanel`. **Diverges from the plan's literal ask in one way, deliberately**: pills
      don't open the F1 popover *inside the editor* — CM6 mark decorations are plain DOM
      spans, and hosting a React `Popover` inside one means a widget decoration carrying a
      React portal into non-React DOM, real engineering distinct from what this Check tests.
      The editor shows *where* PII is in the live text; revealing it stays in the adjacent
      `PiiPanel` list, which already has the real reveal flow (Track F1).

      Required a new `ProcessedFile.rawMarkdown` field, caught before it shipped wrong, not
      assumed correct: `PiiEntity.start`/`.end` are offsets into the markdown *before*
      `renderAnnotatedMarkdown` splices in entity links and redaction (Track F4) and before
      frontmatter is prepended, but `ProcessedFile.markdown` is the *post*-splice final
      output — pairing `piiFindings` with `markdown` would highlight arbitrary wrong text.
      `rawMarkdown` carries the pre-splice text `piiFindings`' offsets actually describe.

      **Verified for real:** `lib/pii-decorations.test.ts` (7 tests) exercises the extension
      directly against real `EditorState`/`StateField` objects (not mocked) — decorates the
      exact span named, drops out-of-bounds/overlapping findings instead of crashing the
      `RangeSetBuilder`, and the Check itself: inserting text before a decorated span shifts
      it by exactly the inserted length while the decoration still bounds the identical
      original text, edits after a span leave it untouched, and edits inside a span shrink
      it to what survives (documents CM6's actual mapping behavior at a boundary case,
      rather than assuming it). New `tests/e2e/markdown-editor.spec.ts` (2 tests) — run for
      real against this sandbox's Chromium build, temporary `executablePath`, reverted
      before committing — confirms the pill renders with the right text and category
      attribute in the live app, and that *typing* before the span in a real editor
      (not a synthesized transaction) doesn't corrupt the highlighted text either. Full
      suite: `vitest run` 158/158, `tsc --noEmit` clean, production `vite build` clean
      (CodeMirror added real bundle weight — flagged Vite's own >500kB chunk-size warning,
      not treated as a defect given B3/H1's already-dominant 614MB–1.2GB model weight), full
      e2e 22/22 (20 pre-existing + 2 new), confirming no regression.

- [x] **F4. Solve the offset problem. This is the hard technical core of Tracks A/E/F.**
      Four things mutate the same text and all are offset-sensitive: entity linking
      (`worker/pipeline.ts:82` splices `[text](entity:type/slug)` at byte spans),
      PII redaction (A2), pseudonymization (F2), and now user edits in CodeMirror. Splicing
      one invalidates every offset the others hold. Decide a single representation —
      almost certainly: keep source text immutable, hold spans as a separate overlay, and
      render links/tokens at display or export time rather than splicing into the string.
      Doing A2 and F3 without this will produce corrupted output that tests on short fixtures
      will not catch.

      **Done 2026-07-30, scoped to the two mutators that exist in the pipeline today —
      entity linking and PII redaction. F2 (pseudonymization) and F3 (CodeMirror editing)
      aren't built yet; there's nothing for them to merge into until they are. Whoever
      builds either must extend this pattern, not reintroduce splice-then-mutate.**

      The representation F4 asked for: `renderAnnotatedMarkdown` (`worker/pipeline.ts`)
      collects entity-link spans and PII-redaction spans as two lists computed against the
      *same* immutable `markdown` — no splicing happens until both lists exist. PII wins on
      overlap (`span.start < p.end && p.start < span.end`): an entity-link span that
      overlaps any redaction span is dropped from the render list rather than spliced, so a
      redacted span's raw text can never end up duplicated into a link's visible text and
      its `entity:` slug — which is exactly how L6's corruption happened (PII detection ran
      on markdown that already had link syntax spliced in, so a matching digit run got
      rewritten in both places it appeared). The two surviving lists are merged, sorted
      descending by start, and spliced in one right-to-left pass — same technique the old
      `linkEntities` used, just fed a merged, pre-filtered span list instead of two
      sequential passes each mutating the live string.

      `processFile` was reordered to match: PII detection (`scanForPii`/`redactPii`) now
      runs on the original `markdown`, before `renderAnnotatedMarkdown`, not after a
      separate `linkEntities` pass. Scan-only mode (`enablePiiDetection` on,
      `redactPiiInOutput` off) passes an empty finding list into the renderer, so entity
      links render exactly as before — no redaction findings means no overlaps to resolve.

      **Verified for real:** `worker/pipeline.test.ts` (new — the first unit test for this
      file; needed a `self` polyfill via dynamic `import()` in `beforeAll`, since the file
      assigns `self.onmessage` at module scope and Node has no `self`) exercises the exact
      L6 regression directly — a "phone"-typed entity whose slug is the same digit run a
      credit-card PII span matches — and asserts the merged output contains neither the raw
      digits nor a `entity:phone` link, only `[CARD:****]`. Also covers: non-overlapping
      entity links render unchanged, non-overlapping PII spans redact unchanged, multiple
      non-overlapping spans of both kinds splice correctly in one pass, and scan-only mode
      leaves entity links untouched. All 5 pass, plus the full existing suite (54/54),
      `tsc --noEmit`, `svelte-check` (0 errors), and a production `vite build`, all clean.

      Manually re-ran the L6 browser reproduction (`redactPiiInOutput` on, a document
      containing an email/IBAN/card-number mixed into text an NER pass misclassifies as
      phone/organization entities) through the real dev server: output now reads `Contact
      [Jean Dupont](entity:organization/jean-dupont) at [EMAIL]. ... IBAN [IBAN:****].
      Card number [CARD:****] on file.` — no raw PII, no `entity:phone/[CARD:****]`-style
      corruption, confirmed by regex against the downloaded markdown.

      **What's still open, deliberately not touched here:** the frontmatter's `entities`
      block and the zip's `entities-registry.json`/`kg-export/*` still carry entities'
      *names* — including ones derived from redacted spans (e.g. an entity literally named
      `"4111111111111"`) — regardless of what the markdown body shows. That's Track A2's
      question ("what happens to the entity registry and KG export under redaction —
      exporting a redacted document beside a knowledge graph naming every entity would
      defeat the feature"), not F4's; F4 only guarantees the markdown body itself doesn't
      get corrupted or leak redacted text a second time.

---

## Track G — Export for Claude Desktop (Studio)

Studio is already ahead here and should not import hacienda-private's approach.

Studio's zip already contains markdown with linked entities, `_manifest.json`,
`entities-registry.json`, and `kg-export/{neo4j.cypher,networkx.json,rdf.ttl}`
(`worker/pipeline.ts:370-397`). hacienda-private has **no zip export at all** — only a
"Copy for Claude Desktop" clipboard button and a mirror push to its MCP server.

- [x] **G1. Preserve entity linking through pseudonymization.** Already implemented at
      `worker/pipeline.ts` (`renderAnnotatedMarkdown`) as `[Acme SAS](#entity-organization-acme-sas)`
      (link scheme updated by G2 — see below), with a `## Entities` glossary (`buildGlossary`)
      and entity metadata in frontmatter (`buildFrontmatter`). Pseudonymizing an entity must
      not orphan its link or its glossary row.

      **Done 2026-07-30, and it needed no G1-specific code once F1 wired pseudonymization
      into the pipeline.** `filterExportableEntities` (Track A2/F4) already drops any NER
      entity whose span overlaps a PII finding before it ever reaches the frontmatter,
      glossary, `entities/` files, or registry — and it does that by comparing
      `start`/`end` offsets only, never reading `redact_template`'s content. That check is
      identically correct whether `redact_template` holds a mask (`"[EMAIL]"`) or a
      pseudonym token — an overlapping entity was never going to get linked in the first
      place, in either mode, so there was no orphaning mechanism to fix. Verified, not just
      reasoned: `tests/e2e/pseudonymize.spec.ts`'s new "G1" describe block reuses the
      misclassified-digit-run fixture `worker/pipeline.test.ts`'s L6 regression test and
      `egress.spec.ts`'s redaction contract already exercise for mask mode, run for real
      with `redactionMode: "pseudonymize"` — the raw card number appears nowhere (body,
      registry, or glossary), no `entities/phone-*.md` link survives, and the body carries
      a real `[CREDITCARD:session:...]` token. Passed on the first run.

- [x] **G2. Make the links resolvable in Claude Desktop.** `entity:` is a custom URI scheme
      nothing outside Studio understands. Switch to in-document anchors into the `## Entities`
      glossary, or wikilinks, so the exported markdown is self-contained. *Check:* open the
      exported markdown in Claude Desktop and follow a link to its glossary entry.

      **Done 2026-07-30, verified mechanically, not literally in Claude Desktop.** Chose
      explicit `<a id="entity-${type}-${slug}">` anchors over relying on a renderer's
      auto-slugified heading text: CommonMark/GFM pass raw inline HTML through verbatim, so
      the link target (`#entity-${type}-${slug}`) is exactly what this app puts there —
      independent of whichever specific slugification algorithm a given renderer implements,
      and immune to two different entities happening to produce the same auto-slugified
      heading text. A single `entityAnchorId()` helper is the one place that ID is computed,
      used by both `renderAnnotatedMarkdown` (the link) and `buildGlossary` (the anchor), so
      the two cannot drift out of sync.

      **What was and wasn't verified:** no access to Claude Desktop from this environment, so
      the literal Check — open the file there, click a link — could not be run. What *was*
      verified, via a real browser run (not just unit tests): every `[text](#anchor)` link
      target in a real generated document has a byte-exact matching `<a id="anchor">` in that
      same document's glossary, and no `entity:` custom-scheme link remains anywhere in the
      output. Same-page `#fragment` navigation to a matching `id` attribute is standard
      HTML/browser behavior, not something Claude Desktop's renderer would need to do
      anything special to support — but this doesn't substitute for actually opening a real
      exported file there.

- [x] **G3. Ship a README in the zip** describing what the bundle is, so a Claude Desktop
      session has the context to use the registry and KG files rather than only the prose.

      **Done 2026-07-30.** `buildBundleReadme()` writes `README.md` at the zip root
      (`worker/pipeline.ts`), computed once from the same `registryJson` the
      `entities-registry.json` file already builds (not a second `registry.toJSON()` call —
      that would also have produced a second, slightly different `processed_at` timestamp).
      Explains what each file/folder is for and steers cross-document questions toward
      `entities-registry.json`/`kg-export/` instead of re-deriving relationships by reading
      every `.md` file — the gap this item exists to close. Verified via a real browser run:
      present in the zip, correct file/entity counts substituted in.

*Note:* hacienda-private also exposes a real Claude Desktop MCP integration
(`services/mcp-server/release/README.md:12-22` — `xberg-mcp mcp` stdio endpoint plus a
`claude_desktop_config.json`). Studio cannot do this: a browser-only app cannot run a local
stdio server. The zip is the right answer for Studio, but it means two Claude Desktop stories
now exist and the zip must stand alone.

---

## Track H — Model delivery, revised (supersedes B3)

**hacienda-private has already solved this and the work is directly reusable.**
`scripts/models/convert_gliner2_f16.py` converts the F32 checkpoint to F16, and
`scripts/models/gliner2-guardrails-pii-f16.sha256` records the result:
**614,224,538 bytes — exactly half of Studio's 1,228,421,964.** The recorded
`source-sha256` is `82ee0ed2…` against `fastino/GLiNER2-Guardrails-PII-Multi`, i.e. provably
the same checkpoint Studio downloads today. The script streams one tensor at a time from an
mmap and needs only numpy.

- [x] **H1. Reuse the f16 artifact.** Run the existing script, publish the output, point
      `VITE_MODEL_BASE_URL` (`asset-loader.ts:19`) at it. 1.23 GB → 614 MB for near-zero
      engineering.

      **Blocked 2026-07-30, not done.** The conversion script
      (`hacienda-private/scripts/models/convert_gliner2_f16.py`) already ran once —
      output hash/size recorded in `hacienda-private/scripts/models/gliner2-guardrails-pii-f16.sha256`
      (614,224,538 bytes) — so the "run the existing script" half is not actually the gap.
      What's missing: (1) this environment has no `numpy` and no network access for a fresh
      1.2 GB download / 614 MB upload; (2) there is no decided, durable publish destination
      (S3/CDN/GitHub release/HF repo) with credentials — "publish the output" was never a
      solved step, just implied by the plan text. Both require a real infra decision and
      access this session doesn't have; deferred rather than guessed at.

      **Done 2026-07-31.** Published to a new public HuggingFace repo,
      `jamon8888/gliner2-guardrails-pii-f16`, with the four files
      `asset-loader.ts` fetches: `model.safetensors`, `tokenizer.json`,
      `encoder_config/config.json`, `config.json`. `MODEL_BASE` in `asset-loader.ts`
      now points there in place of `fastino/GLiNER2-Guardrails-PII-Multi`; domain is
      unchanged (`huggingface.co`), so `tests/e2e/egress.spec.ts`'s allowlist needed no
      change.

      *Check, run for real:* `curl` against the HF API confirmed the repo is public and
      `safetensors.parameters` reports all 307,098,645 params as `F16` (correct precision,
      correct architecture — 307M params matches mdeberta-v3-base-sized GLiNER2). A
      `HEAD` with `Origin: http://localhost:5173` on all four files returned
      `access-control-allow-origin` echoing it back, confirming `fetchAsset`'s direct
      (no-dev-proxy) fetch will work in production, same as the original host.

      **Discrepancy found, not silently reconciled:** the uploaded `model.safetensors`
      is 614,224,466 bytes with sha256 `7dd22d08…`, not the 614,224,538 bytes /
      `53c73fff…` recorded in `hacienda-private/scripts/models/gliner2-guardrails-pii-f16.sha256`
      — a 72-byte difference and a completely different hash. Did not download and
      byte-diff the full 614 MB file in this sandbox to explain the delta, so this is
      *not* proven to be re-serialization noise (safetensors header/metadata can differ
      by a few dozen bytes between conversion runs without changing tensor content) as
      opposed to an actual different conversion. The F16-precision and param-count check
      above increases confidence it's the same model, but does not rule out a
      re-run-from-scratch producing a legitimately different (if functionally
      equivalent) artifact than the one hacienda-private's script originally recorded.
      Whoever owns the HF repo should confirm which conversion run this is, and update
      the recorded `.sha256` file if it's simply a fresher run.
- [x] **H2. Decide whether 614 MB is acceptable.** It is a 2× win, not a solution. If not,
      the manifest at `services/mcp-server/models/manifest.json` shows the fallback they
      chose: `onnx-community/gliner_small-v2.1` with int8 variants — a much smaller model at
      some accuracy cost. Evaluate against French legal fixtures before switching, since
      multilingual quality is the whole point (Track B).

      **Decided 2026-07-31: accept 614 MB, do not switch.** No French-legal accuracy
      benchmark for `onnx-community/gliner_small-v2.1` exists in this repo or sandbox to
      weigh against the size saving, and the entire reason `GLiNER2-Guardrails-PII-Multi`
      was chosen over alternatives is multilingual (French) quality — Track B's premise.
      Trading that for a further size cut is exactly backwards without evidence the
      smaller model holds up on the same fixtures D2 added. Revisit only if a real
      accuracy comparison against `fixtures/pii-corpus.json`-style French cases is run and
      shows the smaller model is close enough; until then this is not a stalled decision,
      it is a decision.
- [x] **H3. Note their runtime differs.** hacienda-private loads GLiNER2 in a dedicated Web
      Worker via `packages/wasm-pipeline/src/gliner2-worker.ts` with a `@xenova/transformers`
      tokenizer, whereas Studio's `createNerBackend()` targets xberg-wasm's `NerModel`. Both
      consume the same safetensors. B1 should account for this third option.

      **Noted 2026-07-31, no action needed.** B1 (above) already decided xberg-wasm's
      `NerModel` over `@lmoe/gliner-onnx` without needing to weigh this third runtime —
      hacienda-private's dedicated-Worker-plus-`@xenova/transformers` approach lives in a
      separate repo (`hacienda-private/packages/wasm-pipeline`) with its own build and
      dependency surface, not something Studio's current Vite/Svelte setup shares.
      Adopting a third runtime here would add a dependency with no capability gap it
      closes, since B2 already routes Studio's worker through `createNerBackend()`.

---

## Track I — Folder in, vault out

The core product. Design the output format before writing pipeline code.

### I1. Folder ingest — DONE 2026-07-30

- [x] Add `webkitdirectory` to the file input and carry `file.webkitRelativePath` through
      `FileInput` so the source tree survives into the output. Today only `multiple` is set
      and relative paths are discarded.

      **Done.** Rather than a second `<input type="file">` (every e2e test's
      `input[type="file"]` selector assumes exactly one element exists), `App.svelte` toggles
      `webkitdirectory` on the *same* `#file-input` via a `folderMode` flag, flipped by an
      "or choose a folder" button next to the existing label. `lib/file-filter.ts`'s new
      `effectiveFileName()` prefers `webkitRelativePath` over `.name` — used everywhere a
      filename crosses a boundary: the `FileInput.name` sent to the worker, the
      `progress`/`ProgressBar` Map key (previously keyed by bare `file.name`, which would have
      silently never matched a folder-uploaded file's progress messages — caught before it
      shipped, not after), and the batch-completion counter. `worker/pipeline.ts` needed no
      change: `r.name` already becomes the zip entry path via `zip.file(r.name, ...)`, and
      JSZip treats `/` in that path as folder nesting, so a relative path in `input.name`
      produces a nested zip entry for free. **Not done**: drag-and-drop folder traversal —
      `onDrop` still reads `dataTransfer.files` flat, so a dragged folder is not (yet) walked
      recursively; only the explicit folder-picker button carries `webkitRelativePath`.
- [x] Ingest is sequential (`for (const file of files)`, `worker/pipeline.ts:331`). A folder
      of a hundred documents with a 614 MB model loaded will be slow and gives no way to
      cancel. Decide whether to parallelise or just report honest per-file progress; do not
      leave a progress bar that stalls for minutes with no explanation.

      **Decided: honest per-file progress, not parallelisation.** The single loaded WASM
      NER/PII engine instance is not built for concurrent inference calls, and true
      parallelism would need multiple Workers — out of scope here. The per-file progress
      cards already update through real stages as `processFile` runs (not new); what was
      missing for a large folder was any sense of overall progress. Added a
      `{completed} / {total} processed` summary line, shown whenever more than one file is
      queued, computed from the same `progress` Map. Cancellation remains unimplemented.
- [x] Skip/report unsupported files rather than failing the batch — a real folder contains
      `.DS_Store`, images, and things Studio cannot extract.

      **Done.** `isJunkFile()` (`lib/file-filter.ts`) silently drops OS noise (dotfiles,
      `Thumbs.db`, `desktop.ini`) before validation — a real folder's `.DS_Store` was never a
      file the user meant to include, so it isn't reported as skipped, just excluded.
      Genuinely unsupported files (wrong type/too large) already didn't fail the batch
      (`validateFile` filtered them client-side before this change) but only surfaced as a
      per-file error banner that overwrote itself on every subsequent file — useless for a
      folder with several. Now: exactly one rejected file (the existing, tested single-file
      case) keeps its precise message; more than one is reported as a count
      ("Skipped 2 unsupported and 1 system files.").

      **Verified for real**: `lib/file-filter.test.ts` (7 cases: relative-path preference,
      junk-file detection including nested-path basenames). New
      `tests/e2e/folder-upload.spec.ts` builds an actual temp directory (`sub/note.txt`,
      `.DS_Store`, `mystery.xyz`), uploads it through the folder picker using Playwright's
      directory-path `setInputFiles` (which populates `webkitRelativePath` the same way a
      real browser directory picker does — not simulated), and asserts all three claims
      against the real downloaded zip: the nested path survives (`.../sub/note.md`), the
      junk and unsupported files are absent, and the skip-notice text names both counts. Run
      for real (temporary sandbox-only `wasm-opt = false` and Chromium `executablePath`,
      both reverted before commit, same pattern as every other verification in this PR) —
      5/5 across `basic.spec.ts` + the new spec, and the full existing e2e suite
      (`toggles`, `egress`, `pipeline`, `audio` — 11 more tests) re-run clean to confirm no
      regression from re-keying the progress Map. `vitest run` 77/77, `svelte-check` 0
      errors, `tsc --noEmit` clean, production `vite build` clean, `typos` clean.

### I2. The vault layout — the thing to get right — DECIDED 2026-07-30

Proposed zip structure. **Accepted as proposed**, per the person who owns this call — this is
now the contract all three surfaces (Studio, CLI, API) must emit, not a finding to revisit:

```text
README.md                    ← what this bundle is, how an agent should use it
documents/<source tree>.md   ← one markdown per input, original folder structure preserved
entities/<slug>.md           ← one file per entity: type, vertical, roles, aliases, backlinks
GLOSSARY.md                  ← index of every entity, linking into entities/
_manifest.json
entities-registry.json
kg-export/{neo4j.cypher,networkx.json,rdf.ttl}
```

- [x] **Links must be relative markdown paths**, e.g. `[Acme SAS](../entities/acme-sas.md)`.
      This replaces the current `entity:organization/acme-sas` custom scheme
      (`worker/pipeline.ts:93`), which nothing outside Studio can resolve. Relative paths are
      followable by a Claude Desktop agent with filesystem access, and by Obsidian, and by
      anything else that reads a markdown folder.

      **Done 2026-07-30.** Replaced G2/G3's interim in-document `<a id>` anchors with real
      `entities/<type>-<slug>.md` files, type-prefixed rather than the plan's literal
      `entities/acme-sas.md` example — a bare slug collides across types (an `organization`
      and a `location` can slugify to the same string), the same risk G2's anchor scheme
      already had to account for. `relativeEntityLink(docPath, entity)` computes the
      `../`-prefixed path from a document's depth under `documents/` back to `entities/`;
      `RegistryEntity` gained a `slug` field (`lib/registry.ts`) — the first mention's slug,
      stable for the batch — since it previously stored everything needed to identify an
      entity except the one field file-naming needs.
- [x] **One file per entity, with backlinks.** This is what makes the bundle RAG-ready rather
      than merely readable: the agent can open `entities/acme-sas.md` and find every document
      mentioning it. The data already exists — `BatchEntityRegistry` plus
      `inferRelationships` — it is currently only serialised to `entities-registry.json` and
      to the KG exports, which an agent will not naturally read.

      **Done 2026-07-30.** `buildEntityFile()` (`worker/pipeline.ts`) writes one file per
      registry entity: type, vertical, sector, roles, aliases, mention count, and a backlink
      to every document that mentions it. Backlinks come from a `docId -> "documents/..."
      path` map built in `processFiles`'s existing per-file loop (`docPaths`), fed by
      `RegistryEntity.source_documents` — no new bookkeeping beyond the field the vault
      layout already needed. Optional fields (sector/roles/aliases) are omitted rather than
      printed blank when empty.
- [x] **`GLOSSARY.md` is the entry point.** The existing per-document `## Entities` section
      (`worker/pipeline.ts:118`) becomes a local summary; the global glossary is the index.

      **Done 2026-07-30.** `buildGlossaryIndex()` writes `GLOSSARY.md` at the zip root:
      entities grouped by type (alphabetical), each linking into `entities/`, with vertical
      and mention/document counts. The per-document `## Entities` section
      (`buildGlossary()`) now links into the same `entities/` files instead of in-document
      anchors, so a reader following a link from either a document or the glossary lands on
      the same canonical entity file.

      **Verified for real:** `worker/pipeline.test.ts` gained 6 new cases (2
      `relativeEntityLink` — root-level and nested document depth, 2 `buildEntityFile`, 2
      `buildGlossaryIndex`) plus updated every existing `renderAnnotatedMarkdown` case for
      the new link shape — 17/17 in that file, 84/84 full suite, `tsc --noEmit` and
      `svelte-check` clean, production `vite build` clean. `tests/e2e/pipeline.spec.ts`,
      `toggles.spec.ts`, `folder-upload.spec.ts` updated for the `documents/` prefix;
      `egress.spec.ts`'s PII-redaction contract extended to assert no IBAN in any
      `entities/*.md` file, not just the fixed-name files it already checked (the entity
      whose only mention overlapped the IBAN was already excluded by A2's
      `filterExportableEntities`, so this is regression coverage, not a new finding). Ran
      the full e2e suite for real against this sandbox's Chromium build (temporary
      `launchOptions.executablePath`, reverted before committing, same pattern as every
      prior session) — 16/16 passing, including a live-browser check that `documents/`,
      `entities/*.md`, and `GLOSSARY.md` all appear in a real downloaded zip with correct
      relative links. Along the way, found and fixed an unrelated environment hazard, not a
      code defect: an earlier `npm run dev` invocation's `predev` hook had partially
      regenerated `crates/hacienda-wasm/pkg` (this sandbox has no network access to fetch
      wasm-opt) and left it without a `package.json`, breaking every worker-dependent e2e
      test with a resolution error. Fixed by temporarily setting
      `wasm-opt = false` in `hacienda-wasm/Cargo.toml`, rebuilding, then reverting — the
      same documented pattern prior sessions used for the same underlying constraint.
- [x] **`README.md` tells the agent what it has.** Without it a Claude Desktop session sees a
      folder of prose and ignores the registry and graph files entirely. **Done via G3** —
      `README.md` ships at the zip root.
- [x] *Open question, resolved 2026-07-30:* should redaction/pseudonymisation state be
      recorded in the bundle — i.e. does the vault declare which entities were pseudonymised
      and under which key id? **Decided: no.** The vault stays silent on redaction/
      pseudonymisation state; it must not become a map of where the sensitive material was,
      even without the plaintext itself. Provenance, if ever needed, is a separate concern for
      the audit chain (Track I2/G's audit-export question, still open per C3), not the vault
      contract.

### I3. Finder-like browser

- [x] `hacienda-private/apps/web/components/ui/file-system.tsx` is a **4,586-line virtualized
      grid/list** file browser, with `file-thumbnail.tsx` (154) and
      `document-viewer-sidebar.tsx` (112) alongside. Its interface is a flat
      `FileSystemItem[]` of `{ kind: "folder" | "file", path, key }` (`app/browse/page.tsx`),
      which Studio can build directly from in-memory batch results — the component itself is
      not coupled to their API, only their `/browse` page is.
- [x] Show per-file state in the browser: extracted / PII count / edited / error. The Finder
      view is where the user decides which documents need attention, so it has to carry that.

      **Done 2026-07-30, scoped.** Not a port of `file-system.tsx`: that component is built on
      `@base-ui/react` + Tailwind v4 (the same incompatible toolkit generation Track F1's UI
      primitives already had to route around — see `components/ui/README.md`), and its actual
      job — a persistent, virtualized, tree-structured project browser — isn't this app's use
      case, which is one flat batch of files per run. Porting it would mean rewriting a file
      browser's worth of tree/selection/virtualization machinery this app doesn't need to
      satisfy the plan's literal ask: per-file state, in a list. `components/FileBrowser.tsx`
      is that scoped equivalent — `buildFileRows()` merges `files`, the `progress` map,
      `results`, a new per-file `fileErrors` map (worker `"error"` messages were previously
      only a global banner — Track I3 needed them attributable to a specific row), and I4's
      edited-findings state into one `FileRow[]`, rendered as a flat list: status icon, name,
      stage/error text, entity count, PII count, an "edited" badge once I4 has touched a file.
      `App.tsx` renders it above the existing per-document detail cards. Verified via
      `tests/e2e/finder-and-editing.spec.ts`'s `FileBrowser` suite against a live browser
      (temporary Chromium `executablePath`, reverted before commit): a processed file's row
      shows "Done", a PII count, and no "edited" badge until an edit actually happens.

### I4. Per-document PII editing

"Add and remove PII" is two operations, and neither is accept/reject on a fixed list:

- [x] **Remove** — a false positive is unmarked and the original text stands.
- [x] **Add** — the user selects text the model missed and marks it as PII with a category.
      This is manual annotation, not review of a detection. It is the operation that makes the
      tool trustworthy for a lawyer, and neither app has it today.
- [x] Both require **F4's overlay model**: source text immutable, spans as separate data.
      Adding a span to spliced text is not implementable; this is why F4 gates the editor.
- [x] Edits must survive re-export and be visible in the Finder view.

      **Done 2026-07-30.** `renderAnnotatedMarkdown` (the same function `worker/pipeline.ts`
      used to build the original export) moved out to `lib/annotate.ts` so `App.tsx` — main
      thread, not a worker — can call it directly without importing `worker/pipeline.ts`
      itself: that module assigns `self.onmessage = …` at top level, harmless inside a Worker
      but a silent hijack of `window.onmessage` if imported into the main thread.
      `worker/pipeline.ts` re-exports both functions unchanged, so nothing that already
      imported them from `"./pipeline"` (`pipeline.test.ts` included) needed to change.

      **Remove**: `PiiPanel`'s finding rows grow a "✕" button (`onRemove`) that drops the
      finding from `App.tsx`'s per-document `editedFindings` map (keyed by `ProcessedFile.name`,
      absent = unedited). **Add**: `MarkdownEditor` grows an optional toolbar — a category
      `<select>` and "Mark selection as PII" button — reading the selection straight off CM6's
      own `EditorState.selection`, already in the same `rawMarkdown` coordinates `findings[].
      start/.end` use, so no offset translation is needed (F4's overlay model, reused rather
      than reimplemented: source text immutable, the manually-added span is just one more
      entry in the same array). A manual add gets a plain `[CATEGORY]` mask, not a pseudonym
      token — minting one needs the batch's derived key, which lives only inside the worker
      (`pseudonymKeyHex` in `worker/pipeline.ts`), and re-deriving it a second time on the main
      thread for this is more machinery than this increment's scope calls for; documented, not
      silently dropped. Overlapping an existing finding is silently ignored.

      **Re-export**: an "Export edited file" button appears once a document has an
      `editedFindings` entry, calling `reExportMarkdown()` — re-splices `rawMarkdown` against
      the edited findings via `renderAnnotatedMarkdown`, and carries the *original* export's
      frontmatter and "## Entities" glossary through unchanged (regex-extracted from
      `result.markdown`) rather than regenerating them. **Explicit scope cut, matching the
      session's established pattern for J2's "thinner CLI vault" and I3 above**: full
      frontmatter/entity-registry/KG-export consistency with post-export edits — retracting an
      entity's glossary row when an add now redacts its only mention, or restoring one when a
      remove un-redacts it — is out of scope for this increment. The body text itself is
      always correct; only the frontmatter's `pii_entities_found` count and the local glossary
      can go stale relative to edits made after export.

      **A real regression caught, not shipped**: wiring "Mark selection as PII" needed
      tracking the live CM6 selection via `onUpdate`, which fires on every keystroke.
      `extensions={[markdown(), piiHighlightExtension(findings)]}` was previously a fresh
      array every render — harmless when nothing re-rendered `MarkdownEditor` during typing,
      which nothing did before this track. Once `onUpdate` started calling `setSelection` on
      every keystroke, that same fresh-array pattern reconfigures `@uiw/react-codemirror`'s
      extensions on every keystroke too, which rebuilds the decoration `StateField` from
      scratch (`create()` against the *live, already-edited* document but the *original*
      finding offsets) instead of continuing F3's `decorations.map(tr.changes)` history — the
      exact corruption F3's own Check exists to catch. Running the existing
      `tests/e2e/markdown-editor.spec.ts` suite against this change (not just the new I4
      tests, which happened not to trigger it) failed exactly that Check:
      `"an Dupont at jean.dupont@cabin"` instead of the email. Fixed by memoizing `extensions`
      on `findings` and giving `onUpdate` a permanently stable identity (`useCallback`, empty
      deps) — `@uiw/react-codemirror` only reconfigures on an actual `findings` change now,
      which is the documented, narrower scope-cut noted in `MarkdownEditor.tsx`'s header (an
      edit *interleaved* between two add/remove actions can still lose its live position — a
      real edge case, deliberately not solved this session; see that file for why).

      **Verified for real**: `tests/e2e/finder-and-editing.spec.ts` (3 tests, real Chromium,
      temporary `executablePath` reverted before commit) — removing a finding un-redacts its
      original text in the re-exported file; selecting exact character offsets (computed, not
      a pixel-based double-click) and marking them as PII redacts that text in the re-export;
      the Finder row's "edited" badge appears only after an edit. Re-ran the full pre-existing
      e2e suite (25/25) plus `vitest run` (158/158) and `tsc --noEmit` after the fix — no
      regressions. Production `vite build` clean (temporary `wasm-opt = false` for the sandbox's
      blocked binaryen download, reverted before commit, the same documented pattern used
      throughout this session).

---

## Track J — Make the CLI emit the same vault

The reframe's main new work. Today `hacienda extract` takes `inputs: Vec<String>` and a
`--format` of JSON or text (`hacienda-cli/src/cli.rs:110`); it cannot produce a markdown
folder. If the vault is the product, the CLI not producing one means the product only exists
in a browser.

- [x] **J1. `hacienda extract --vault <dir>`** emitting the Track I2 layout — same
      `documents/`, `entities/`, `GLOSSARY.md`, `kg-export/`. A firm processing 10,000
      documents overnight should get the same artefact a lawyer gets from dropping a folder.

      **Done 2026-07-30, scoped by J2's decision below.** New `--vault <DIR>` flag on
      `hacienda extract` (`cli.rs`). `write_vault()` (`commands.rs`) writes `documents/<stem>.md`
      per input (frontmatter: source, type, processed, PII count; content already redacted
      per `--mode`, same guarantee `HaciendaResult` already gives stdout output),
      `_manifest.json`, `pii-registry.json`, and `README.md`. No `entities/`, `GLOSSARY.md`,
      or `kg-export/` — see J2.
- [x] **J2. Port entity linking, glossary and KG export to Rust — or don't, deliberately.**
      These exist only in Studio's TypeScript (`worker/pipeline.ts`, `lib/kg-export.ts`,
      `lib/registry.ts`, `lib/verticals/`). Two honest options: reimplement in Rust (a real
      cost, and a third divergence surface), or declare the CLI's vault a thinner artefact —
      markdown plus registry, no cross-document relationship inference. **Decide explicitly.**
      Silently shipping two different things both called "the vault" is the failure mode.

      **Decided 2026-07-30: thinner artefact, not a Rust port.** The CLI/API have no
      general-purpose named-entity pipeline at all — `hacienda-core`'s neural NER
      (`ner-candle`) is compiled out of every default build (Track K, unstarted), and there
      is no regex-based person/organization detector either, only PII categories. Porting
      `BatchEntityRegistry`/`inferRelationships`/`kg-export.ts` to Rust would mean inventing
      an entity graph from nothing, or first building Track K — reversing that decision
      belongs to Track K's own sequencing (which is itself gated on Track H settling a model
      artefact), not to this one. `pii-registry.json` replaces `entities-registry.json` as
      the CLI vault's cross-document artefact — PII findings grouped by document, explicitly
      documented in the vault's own `README.md` as a different, narrower schema so a
      consumer does not assume the two vaults are interchangeable.
- [x] **J3. The CLI has capabilities Studio cannot have** — `--audit-out` writing a real
      blake3 chain, reversible pseudonyms, `--no-redact` gated behind
      `--i-accept-unredacted-pii`. If a vault carries an audit chain when produced by the CLI
      and not by Studio, the bundle format must say so rather than leaving a consumer to
      guess.

      **Found already substantially done, one gap closed 2026-07-30.** `--audit-out`
      (writing a re-verifying blake3 chain), `--mode pseudonymize` (reversible, AES-SIV), and
      `--no-redact`/`--i-accept-unredacted-pii` were all already implemented and tested
      (`hacienda-cli/tests/extract.rs`) before this session — this item's own text describing
      them as still-open capabilities was stale. What was actually missing, and is what this
      item's last sentence asks for: when `--vault` and `--audit-out` are both given,
      `write_vault()` now mirrors the chain into `<vault>/audit/audit.json` (in addition to
      the standalone `--audit-out` location, unchanged) and `README.md` states plainly
      whether the vault includes one — no consumer has to guess or discover a second
      directory.
- [x] **J4. `--concurrency` already exists** (uncommitted, `ConcurrencyArgs` at
      `cli.rs:157`, wired to `PipelineConfig::concurrency`). Studio's batch loop is sequential
      (`worker/pipeline.ts:331`). Same knob, two behaviours — align the defaults or document
      why they differ.

      **Done 2026-07-30 — documented, not aligned.** `--concurrency` was already committed
      (this item's "uncommitted" was stale) and already wired. Aligning the defaults would
      mean either parallelizing Studio's worker (a real architecture change: one loaded WASM
      NER/PII engine instance is not built for concurrent inference, and true parallelism
      needs multiple Web Workers sharing that model — explicitly out of scope per Track I1's
      own decision) or serializing the CLI to match Studio for no reason. Recorded the "why
      they differ" in `apps/hacienda-studio/README.md`'s "Relationship to the CLI/API"
      section (Track C1) instead, alongside a correction to that section's now-stale claim
      that Studio's NER wasn't wired to the neural model (Track B2 did that after C1's README
      was written) and a new bullet on the vault-depth difference this session's J1/J2 added.

      **Verified for real:** `cargo build -p hacienda-cli` and `cargo test -p hacienda-cli`
      clean (24/24, including 2 new `--vault` tests: a redacted document + `pii-registry.json`
      with no leaked PII and no `entities/`/`GLOSSARY.md` present, and the audit-chain mirror
      into `<vault>/audit/` when both flags are given). `cargo build --workspace` clean.

---

## Track K — Neural detection on the Rust side

Absorbed from `2026-07-28-pii-candle-phase1-minimal.md`, unchanged in substance and still
unstarted. Reframed only in priority: it is what closes the largest capability gap between the
surfaces, since Studio has a neural model and the CLI/API do not.

- [ ] `ner-candle` is not in `default = ["jobs"]` (`hacienda-core/Cargo.toml:9`); nothing
      constructs the Candle detector at runtime, and `pipeline.rs:240` returns
      `ModelUnavailable`. `--model-dir` and `--lora-dir` already exist as CLI arguments
      (`cli.rs:103`, `:107`) with nothing behind them.
- [ ] Sequence this **after** Track H settles which model artefact ships. Both surfaces should
      use the same f16 checkpoint rather than each picking one.

      **Re-verified 2026-07-31, still correctly gated.** Track H (H1) is still blocked on
      huggingface.co network access this sandbox doesn't have (see H1's 2026-07-31 note) — the
      artefact Track K would consume doesn't exist yet, so Track K remaining unstarted is the
      correct sequencing outcome, not a stall.

---

## Track L — Compile `hacienda-core` to wasm32 (the unification track)

**This is the track the 2026-07-30 decision creates, and it retires Track C.** Target: Studio
stops having a PII implementation and starts calling the one in `hacienda-core`.

Sequence the steps so the *answer to "does this fit?"* arrives before the porting work, not
after. L1 is a measurement, not a build.

- [x] **L1. Measure the budget before designing the port.**
      `xberg_wasm_bg.wasm` is 48,060,064 bytes against jsDelivr's 50 MB per-file cap — ~1.9 MB
      of headroom. Build `hacienda-core` for `wasm32-unknown-unknown` with everything gated
      off except `pii/` and `redaction/`, and record the `.wasm` delta.
      *Check:* a recorded byte count. If it exceeds the headroom, **L2 becomes "decide the
      delivery mechanism" and self-hosting is forced** — Studio's allowlist already permits its
      own origin, and `VITE_MODEL_BASE_URL` (`asset-loader.ts:19`) shows the pattern. Do not
      start L4 until this number exists.

      **Measured 2026-07-30: 1,227,264 bytes (~1.17 MB).** Method: `hacienda-core` gated to
      `pii`/`redaction` plus the slice of `audit` they need (`AuditConfig`,
      `entry::RedactionAction` — not the chain/store/segment machinery, which needs `uuid`'s
      `js` feature per L3 and is out of scope here); `axum`, `tokio`, `uuid` moved to
      `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`; `xberg` split the same way,
      native keeps `tokio-runtime`, wasm32 gets `default-features = false, features = ["ner"]`
      (not the full `wasm-target` umbrella — OCR/Excel/layout are facade-only and not compiled
      here). Linked as a standalone `cdylib` (`opt-level = "z"`, `lto = true`,
      `codegen-units = 1`, `panic = "abort"`, `strip = true`) via a throwaway crate (not
      committed) that calls `PiiPipeline::scan` and `RedactionEngine::redact` under all four
      modes — including the AES-SIV `Pseudonymiser` path — through a manual single-poll
      executor, so the linker can't dead-code-eliminate them. Result: **fits inside the ~1.9 MB
      headroom on its own**, with ~713 KB of headroom left over.

      Caveats that keep this a lower bound, not the final delta: (1) no `wasm-opt`/
      `wasm-bindgen` in this environment, so there's no post-link DCE/`-Oz` pass — xberg's own
      build applies one; (2) standalone link, not merged into `xberg_wasm_bg.wasm` — a real
      merge could dedupe shared deps (`serde`, `regex`, `blake3`, ...) already present in that
      binary and shrink the true delta, or wasm-bindgen glue could add a modest amount back;
      (3) excludes NER (`ner-candle-wasm`) and the IndexedDB store layer (L5) entirely, both
      still to come. Re-measure once L2's real wasm crate and L5's stores exist.

- [x] **L2. Add the wasm crate and the target feature set.**
      `crates/` is empty and no manifest mentions wasm-bindgen. Follow the xberg precedent:
      `crate-type = ["cdylib", "rlib"]`, `opt-level = "z"`, `codegen-units = 1`, and a
      `wasm-target` feature that selects `xberg`'s `wasm-target` instead of `tokio-runtime`.
      *Check:* `cargo build --target wasm32-unknown-unknown` reaches a *link* error about
      hacienda-core's own code, not a dependency resolution error.

      **Done 2026-07-30.** Added `crates/hacienda-wasm` (new workspace member): `crate-type
      = ["cdylib", "rlib"]`, depends on `hacienda-core` plus `wasm-bindgen`,
      `wasm-bindgen-futures`, `serde-wasm-bindgen`, exports `process`/`scan`/`redact_empty`
      over the real `PiiPipeline`/`RedactionEngine` API (including the AES-SIV
      `Pseudonymiser` path) so the build genuinely exercises `hacienda-core`. Profile: a
      per-package override (`[profile.release.package.hacienda-wasm] opt-level = "z",
      codegen-units = 1`, not workspace-wide — xberg does the same for `xberg-wasm` alone,
      leaving native `hacienda-cli`/`hacienda-api` release builds untouched).
      `hacienda-core`'s wasm32 `xberg` dependency widened from L1's `["ner"]` spike to
      `xberg`'s `wasm-target` components (`no-ort-target`, `excel-wasm`, `layout-tract`,
      `auto-rotate-tract`, `ner-candle-wasm`) **minus `ocr-wasm`** — see the exclusion note
      below.

      **Result: better than the check asked for.** `cargo build --target
      wasm32-unknown-unknown -p hacienda-wasm --no-default-features` (dev profile)
      compiles *and links* successfully — no error of any kind. With the full
      `wasm-target` umbrella including `ocr-wasm`, the build instead fails in a
      **build script**, not at dependency resolution or Rust link time:
      `xberg-tesseract`'s `build.rs` detects the wasm32 target and requires a WASI SDK
      C toolchain (`WASI_SDK_PATH`) to cross-compile Tesseract, which isn't provisioned in
      this sandbox (network policy also blocks fetching the SDK release here). This is an
      environment/CI-provisioning gap, not a code defect — the real published
      `@xberg-io/xberg-wasm` package does build `ocr-wasm` once that toolchain exists.
      `ocr-wasm` is excluded from `hacienda-core`'s wasm32 feature list for now (see the
      comment in `hacienda-core/Cargo.toml`); `hacienda-wasm` doesn't call
      extraction/OCR yet, so nothing today depends on it. Re-add it once Track I needs
      xberg's extraction on wasm32 and CI provisions `WASI_SDK_PATH`.

      **Release `.wasm` size: 18,632,965 bytes (~17.8 MB)** — `cargo build --target
      wasm32-unknown-unknown -p hacienda-wasm --no-default-features --release`, no
      `wasm-opt` pass (unavailable in this sandbox). Comfortably under 50 MB as a
      standalone artifact, but **this number answers a different question than L1's did,
      and is not a delta to add on top of `xberg_wasm_bg.wasm`'s 48,060,064 bytes.**
      `wasm-target`'s `no-ort-target` base alone pulls in most of xberg's extraction
      surface (PDF, Office, email, archives, HTML/XML, calamine) plus `tract`/`candle` for
      `layout-tract`/`auto-rotate-tract`/`ner-candle-wasm` — i.e. this build recompiles a
      large fraction of the *same xberg code already inside* `xberg_wasm_bg.wasm`,
      standalone, because `hacienda-wasm` links its own independent copy of `xberg` rather
      than sharing Studio's existing one. Track L's actual integration (Studio embeds one
      compiled artifact, not two `xberg` instances) would not pay for that duplication —
      the true marginal cost of adding hacienda-core's own code (`pii`/`redaction`/`audit`,
      none of which xberg already has) on top of Studio's existing bundle is closer to
      **L1's 1,227,264-byte, narrower-`xberg`-feature measurement** than to this number.
      Read this 17.8 MB figure as "a fully independent hacienda-wasm bundle, built from
      scratch, fits under the cap on its own" — useful if Studio ever needs to load it as a
      separate module — not as "L1's number was wrong." A real merged-artifact
      measurement needs the actual Studio embedding (single shared `xberg`), which is
      later Track L work (L5/L6), not L2's.

- [x] **L3. Fix the three silent-failure surfaces first — before the port compiles.**
      They are cheap now and near-undetectable later.
      **(a) Clock.** Replace `Instant::now()` (`redaction/engine.rs:71`) and `SystemTime::now()`
      (`redaction/engine.rs:190`, `audit/sink.rs:176`, `audit/store_file.rs:1008`) with a
      target-aware clock seam. `redaction/engine.rs` is not gated away — it *must* run in the
      browser, and `Instant::now()` panics there.
      **(b) Randomness.** Add the `js` feature to `uuid` for wasm32 (5 `Uuid::new_v4()` sites,
      including audit segment IDs at `audit/segment.rs:126`).
      **(c) Timestamps.** Add a regression guard that `chrono` keeps `wasmbind`. 11 `Utc::now()`
      sites feed audit and compliance timestamps; losing it yields 1970 timestamps that are
      wrong and internally consistent.
      *Check:* a wasm32 test that asserts a redaction round-trip produces a non-1970 timestamp
      and a non-nil UUID. This is the test that catches all three.

      **Done 2026-07-30, and the check test earned its keep — it failed twice for real
      reasons before passing.**

      **(a) Clock:** added `web-time` (workspace dep) — a drop-in `Instant`/`SystemTime`
      that passes through to `std::time` natively and backs onto `Performance`/`Date` in
      the browser. Fixed `redaction/engine.rs`'s two call sites as planned. `audit/sink.rs`
      and `audit/store_file.rs` were **not** touched: both are `#[cfg(not(target_arch =
      "wasm32"))]`-gated in their entirety since L1, and their `SystemTime::now()` calls
      are inside `#[cfg(test)]` temp-dir helpers that do real filesystem I/O — changing
      them would be pure churn on code that can never run on wasm32 regardless of clock
      implementation.

      *A fourth clock call site the plan's grep missed*, because it isn't named
      `redaction/engine.rs`: **`pii/pipeline.rs`'s `process`/`scan`** — the actual browser
      entry points — had **seven** of their own `Instant::now()` calls timing
      regex/model/merge/redaction stages, none gated away. The check test caught this on
      its first real run (`panicked at .../unsupported/time.rs:13:9: time not implemented
      on this platform`) before any manual review did. Fixed the same way (`web_time`).

      **(b) Randomness:** turned out **not** to need `getrandom`'s `js`/`wasm_js` feature
      at all. `uuid` 1.24 restructured its own wasm32 randomness to a self-contained
      `wasm-bindgen` binding straight to `globalThis.crypto.getRandomValues`
      (`uuid`'s `rng.rs`), gated behind `uuid`'s own `js` feature — enabling that one
      feature (`hacienda-core/Cargo.toml`, wasm32-only target dependency) was sufficient.
      With that blocker gone, `audit/chain.rs`, `audit/segment.rs` and `audit/store.rs`
      (the in-memory `AuditStore`/`AuditChain`, not the file-backed one) no longer needed
      the wasm32 gate L1 put on them for this reason and are un-gated in `audit/mod.rs` —
      only `store_file`/`sink`/`export` (real file I/O) remain native-only.

      *A second surface the plan didn't name*: un-gating `audit/segment.rs` exposed
      **`std::process::id()`**, called from `NodeId::default()` — compiles cleanly on
      wasm32-unknown-unknown (as L1/L2's builds already showed) but **panics at runtime**
      ("no pids on this platform"), the check test's second real failure. There's no
      JS-hosted equivalent to bind to — a browser tab has no OS process — so
      `audit/segment.rs` now has a `#[cfg(target_arch = "wasm32")]` `process_id()` that
      returns a fixed `0` rather than fabricating a number that looks like a real pid.

      **(c) Timestamps:** confirmed `chrono`'s defaults (`{ version = "0.4", features =
      ["serde"] }`, no `default-features = false`) already include `wasmbind`, and added
      an explicit comment at the dependency declaration warning against ever adding
      `default-features = false` there. The regression guard is the check test itself: it
      asserts both `RedactionAuditEntry.timestamp` (from the fixed `web_time` clock) and
      an `InMemoryAuditStore` segment's `opened_at` (from `chrono::Utc::now()`) are well
      past the epoch.

      **Check, run for real:** `crates/hacienda-wasm/tests/wasm.rs`, executed via
      `wasm-bindgen-test-runner` under Node (`wasm-bindgen-cli` 0.2.126, matched to the
      `wasm-bindgen` crate version in `Cargo.lock`) — not just compiled. It runs a real
      `PiiPipeline::process` round trip (regex-detects and masks an email, asserting the
      audit entry's `timestamp` and the rewritten text), then feeds that audit log into a
      real `InMemoryAuditStore`, closes it, and asserts the sealed segment's `segment_id`
      parses as a non-nil `Uuid` and its `opened_at` parses as a non-epoch RFC3339
      timestamp. `cargo test --target wasm32-unknown-unknown -p hacienda-wasm --test wasm`
      → `test result: ok. 1 passed`. Native `cargo build --workspace`, `cargo test -p
      hacienda-core pii:: redaction::`, `cargo fmt --check --all`, and `cargo clippy
      --workspace --all-targets` all stay clean; two pre-existing `audit::store_file`
      failures (EACCES-simulation tests that don't hold under this sandbox's root user)
      reproduce identically on the pre-L3 commit and are untouched by this change.

- [x] **L4. Gate out the server- and disk-only modules.**
      `axum` appears only in `auth/authz.rs` (4 references) — a server-only concept, so
      `#[cfg]` the module out entirely. `std::fs` appears in `audit/sink.rs:11`,
      `audit/store_file.rs:41`, `audit/segment.rs:76` (reads `/etc/hostname`),
      `review/store_file.rs:53`, `pii/config.rs:95`, `facade.rs:2430` (reads `/proc/meminfo`).
      All are implementations behind traits that already exist — **the traits do not change.**
      `spawn_blocking` lives only in `audit/store_file.rs:447,587,652`, inside the store being
      gated out; `tokio::sync::Mutex` (`audit/store_file.rs:179`) works on wasm32 as-is.
      *Check:* `cargo build --target wasm32-unknown-unknown -p hacienda-core` succeeds.

      **Already true as of L1/L3 — no new code needed, just the check run for real.**
      `auth` (axum), `facade` (`/proc/meminfo`), `compliance`, `glossary`, `jobs`, `review`
      (`review/store_file.rs` included, since the whole `review` module is gated, not just
      its file-backed store) are all `#[cfg(not(target_arch = "wasm32"))]` in `lib.rs`
      since L1. `audit`'s `store_file`/`sink`/`export` (the only `std::fs`/`spawn_blocking`
      users left in `audit` after L3 un-gated `chain`/`segment`/`store`) are gated the same
      way in `audit/mod.rs`. `pii/config.rs:95`'s `std::fs::read_to_string` is the one
      `std::fs` call site that *is* compiled on wasm32 — same reasoning as
      `audit/segment.rs`'s `/etc/hostname` read: it compiles fine (wasm32 `std::fs` exists,
      it just always errors at runtime, no panic), and nothing in `crates/hacienda-wasm`
      calls the function that reaches it, so it's inert rather than gated.

      `cargo build --target wasm32-unknown-unknown -p hacienda-core` — **note: no
      `--no-default-features`, unlike L1/L2/L3's checks** — succeeds: `default = ["jobs"]`
      doesn't matter here because `jobs`'s `#[cfg]` in `lib.rs` is `all(feature = "jobs",
      not(target_arch = "wasm32"))`, so the feature being on changes nothing for this
      target.

- [x] **L5. IndexedDB `AuditStore` / `ReviewStore` / `JobStore`.**
      The trait seams are the reason this track is tractable — implement against them, do not
      reshape them. Carries the C3 sub-question: an IndexedDB chain dies with a cleared browser
      profile, so if Studio output must be legally defensible the chain has to be *exported
      into the vault*, not merely retained.
      *Check:* append two audit entries, reload the page, verify the blake3 chain still
      verifies across the reload.

      **Done 2026-07-30, `AuditStore` only** — `ReviewStore`/`JobStore` deferred (see
      below). `hacienda-core/src/audit/store_idb.rs` (wasm32-only): `IndexedDbAuditStore`
      reuses `audit::store::State` (made `pub(crate)`, not reshaped) rather than
      re-deriving `InMemoryAuditStore`'s transition logic — it *is* that logic plus a
      durability step. After every mutating call, the current state is serialized to one
      IndexedDB record; `open()` reads that record back and replays it through
      `Segment::append`, the same path `FileAuditStore`'s recovery uses for `.jsonl`
      files, so a tampered snapshot is caught the same way a tampered file would be.
      `Segment`'s `prev_seal_hash` isn't stored separately — it's always the most recent
      sealed segment's `seal_hash`, so persisting it too would just be a second place for
      it to go stale.

      **The `Send` problem, and why `SendWrapper` is the right tool, not a workaround.**
      `AuditStore` uses plain `#[async_trait]`, so every method returns a `Send`-bounded
      future by design — a database backend should be able to run its I/O off whatever
      executor thread drives it. `indexed_db_futures`'s `Database`/`Transaction`/
      `ObjectStore` wrap `web_sys`/`js_sys` handles, which are `!Send` unconditionally on
      every wasm32 target (`wasm-bindgen` does not special-case the non-`atomics`,
      single-threaded case this crate actually builds for). `send_wrapper::SendWrapper`
      is the ecosystem-standard answer: it satisfies the `Send` bound by moving the "never
      actually crosses a thread" invariant from the type system to a same-thread runtime
      assertion that can never fire on `wasm32-unknown-unknown` without `atomics`. Not
      reshaping `AuditStore` to drop its `Send` bound (`#[async_trait(?Send)]`) was a
      deliberate choice — that bound is what lets a *native* backend like
      `FileAuditStore` do real I/O off-thread, and this module has no business narrowing
      a guarantee other implementations rely on just because it personally doesn't need
      it.

      **`ReviewStore`/`JobStore` deferred, not silently dropped.** Both live in modules
      (`review`, `jobs`) still gated out of wasm32 entirely since L1 — nothing in
      `crates/hacienda-wasm` exposes review or job-tracking functionality yet (Track A/E/F
      Studio work hasn't reached that point), so an IndexedDB implementation today would
      have no caller and no way to be exercised by a real check. Add them alongside
      whichever Studio track first needs one, following the exact pattern here: reuse the
      existing trait and in-memory state, add a persistence step, don't reshape.

      **C3 is still open.** This module makes the chain durable *across a reload*, not
      durable in the sense a compliance record needs — an IndexedDB chain dies with a
      cleared browser profile. Nothing here decides whether/how the chain gets exported
      into the vault (Track I2); that product question is exactly as open as it was
      before this step.

      **Check, run for real — "reload the page" modeled as a fresh store instance against
      the same IndexedDB database name** (no real browser page-navigation available in
      this harness; Node has no native IndexedDB either, so the run below uses the
      `fake-indexeddb` polyfill via `NODE_OPTIONS=--require .../fake-indexeddb/auto`).
      `crates/hacienda-wasm/tests/idb.rs`: session 1 opens a store, appends two entries,
      confirms `verify()` and `entries().len() == 2`, captures `tip()`, and is dropped
      *without* calling `close()` — an unclean tab close is exactly the durability case
      that matters, not just orderly shutdown. Session 2 opens a fresh
      `IndexedDbAuditStore` against the same database name, confirms both entries and the
      same `tip()` survived, and — Track L5's actual check — `verify()` still passes,
      which specifically exercises the replay-through-`Segment::append` rehydration path,
      not just raw byte persistence. `cargo test --target wasm32-unknown-unknown -p
      hacienda-wasm --test idb` → `test result: ok. 1 passed`. Native `cargo build
      --workspace`, `cargo fmt --check --all`, and `cargo clippy` (both native
      `--workspace --all-targets` and `--target wasm32-unknown-unknown` for
      `hacienda-core --lib` and `hacienda-wasm --all-targets`) all stay clean.

- [x] **L6. Delete the TypeScript engine and switch the call sites.**
      `lib/pii-detector.ts` (`detectPII:22`, `redactPII:39`, zero importers) is deleted rather
      than wired — Track A's toggles resolve to the Rust engine instead. This is the step that
      makes Tracks A2 and F cheaper, which is why A2 should not be built twice.
      *Check:* the C2 shared fixture corpus passes from `vitest` against the WASM build, with
      the same results `cargo test` produces. One corpus, two runners, identical output.

      **Done 2026-07-30.** `lib/pii-detector.ts` deleted (confirmed zero importers repo-wide
      before deleting). New `lib/pii-engine.ts` wraps `hacienda-wasm`'s `scan()`/`process()` —
      the same compiled crate L2/L3 built and L5 gave a durable audit store — as
      `scanForPii()`/`redactPii()`; field names crossing the boundary are `hacienda-core`'s
      actual Rust field names (no camelCase rename), documented in a comment so nobody
      "fixes" the casing without also changing Rust.

      `worker/pipeline.ts`'s `initEngine()` now `Promise.all`s the xberg wasm init alongside
      `initPiiEngine()`. A new `"pii"` stage runs between entity-linking and frontmatter
      build: `enablePiiDetection`/`redactPiiInOutput` (previously read by nothing —
      `ConfigPanel.svelte`'s "PII & Compliance" toggles were dead config) now actually call
      the engine, and `piiEntitiesFound` flows into both the progress event and the output
      frontmatter (`types.ts`, `ProgressBar.svelte`'s stage label map).

      Build wiring: `hacienda-wasm` added as a `file:../../crates/hacienda-wasm/pkg`
      dependency; `predev`/`prebuild`/`pretest:unit` run a new `build:wasm` script
      (`cd crates/hacienda-wasm && wasm-pack build --target web --no-default-features`) since
      `pkg/` isn't committed, matching the existing `ocr-wasm`/`xberg-wasm` pattern.
      `vite.config.ts` excludes `hacienda-wasm` from `optimizeDeps` (required for its
      `?url` wasm import to resolve) and gives it its own `manualChunks` entry. `Taskfile.yml`
      gained a `build:wasm` task and `wasm-pack` in `setup`.

      **Verified for real, not just compiled:** `cargo test` (native, including the new C2
      corpus test), `wasm-bindgen-test` (wasm32), `vitest run` (25/25 corpus cases against
      the real compiled wasm build, not a mock — required switching `import.meta.resolve`,
      unsupported by vitest's SSR transform, to `createRequire(...).resolve`), `svelte-check`
      (caught a real error: `ProgressBar.svelte`'s stage-label map didn't have a `"pii"`
      entry), a full production `vite build`, and a manual Playwright run against the real
      dev server exercising both `enablePiiDetection` (scan-only) and `redactPiiInOutput`
      (rewrite-output) paths end-to-end through a real download.

      **Real product bug found via that manual run, deliberately not fixed here:** PII
      redaction runs on `linkedMarkdown` — *after* `linkEntities` has rewritten NER spans into
      `[text](entity:type/slug)` links. A PII span overlapping a link's visible text or
      `entity:` target rewrites inside the link syntax too; observed as
      `[[CARD:****]](entity:phone/[CARD:****])` from an IBAN digit run inside a misclassified
      Phone entity's link. This is exactly Track F4's offset problem, not a new one — a real
      fix needs entity offsets recomputed against redacted text, which is what L7 exists to
      investigate (`WasmRedactionConfig.preserveOffsets` may already solve it on the Rust
      side). Documented at the call site in `worker/pipeline.ts` rather than patched ad hoc;
      left open for L7.

- [x] **L7. Reconsider `preserveOffsets` against Track F4.**
      `WasmRedactionConfig` already exposes `preserveOffsets`, and `WasmRedactionFinding`
      carries `start`/`end`/`category`/`strategy`/`replacementToken`. F4's offset problem may
      be substantially solved on the Rust side already. **Read that implementation before
      designing F4's overlay** — this could remove the hardest item in the plan.

      **Done 2026-07-30 — investigated, does not remove F4. Corrected below; do not carry
      the "may already solve it" hedge in `worker/pipeline.ts` forward as encouragement to
      reach for xberg's redactor.**

      First correction to the premise itself: `WasmRedactionConfig`/`WasmRedactionFinding`
      (`node_modules/@xberg-io/xberg-wasm/pkg/web/xberg_wasm.d.ts:3476-3554`) belong to
      **`@xberg-io/xberg-wasm`**, the prebuilt npm package — not to `hacienda-wasm`, the
      crate Track L2-L6 built and wired to Studio. It's a *third* PII implementation: its
      own `WasmPiiCategory` enum (13 categories — email/phone/ssn/card/postal/ip/iban/bic/
      dob/person/org/location/custom — a strict subset of `hacienda-core::pii::PiiCategory`'s
      ~20, missing exactly the ones C2 cares about, EU VAT and driver's license) running as
      a post-processor inside xberg's own `extract(input, config)` call, configured via
      `WasmExtractConfig.redaction`. Reaching for it would reintroduce the two-engines
      problem C1's decision was written to end — three engines, now — for zero coverage
      gain, since L6 already wired Studio to the engine `cargo test`'s corpus asserts
      against.

      Second, and more direct: `WasmRedactionReport`'s own doc comment settles what
      `preserveOffsets` can and can't do. *"Offsets are relative to the original
      pre-redaction `content` and are intended for audit reconstruction only — the original
      bytes are dropped at the end of the pipeline."* That's a one-shot destructive
      redaction with an audit trail attached — structurally identical to what
      `hacienda-wasm`'s `scan()`/`process()` already return today (`PipelineResult` with
      `entities: PiiEntity[]` carrying `start`/`end`, plus `redacted_text`). Whatever
      `preserveOffsets` toggles (likely length-preserving replacement, unverified — no
      Rust source for `xberg-wasm` is checked out in this repo, only the compiled
      `.d.ts`/`.wasm`; `xberg-io/xberg` is a separate repo not in this session's scope), it
      does not give Studio a live source-text-plus-overlay architecture. Findings are
      reported once, against text about to be discarded, not held for later re-rendering.

      Third: L6's actual bug isn't an offset-shift bug, so a length-preserving replacement
      wouldn't have prevented it. `linkEntities` (`worker/pipeline.ts:83-98`) splices
      `[matched](entity:${type}/${slug})` into the string — and the corruption seen in
      manual testing, `[[CARD:****]](entity:phone/[CARD:****])`, happened because the
      *slug itself* embedded the same PII-looking digit run as the link text (a
      misclassified NER entity whose derived slug was the IBAN digits). A regex-based PII
      pass over the already-linked markdown has no way to know "this occurrence is inside a
      synthetic URI, don't touch it" from "this occurrence is prose" — it matches both, and
      the zip's `entities-registry.json` still carries the unredacted slug regardless.
      Preserving byte offsets does nothing to stop a plain-text search from firing twice on
      structurally different parts of the same string.

      **What actually moves F4 forward:** `hacienda-wasm`'s `PipelineResult.entities` is
      already the overlay shape F4 asks for — spans with category/start/end, independent of
      any splicing. L6 immediately throws that away by running PII detection *after*
      `linkEntities` has already spliced, on the spliced string. F4's fix is a reordering +
      merge, not a new primitive to import: run entity-linking's span collection and PII
      detection both against the original unlinked `markdown`, producing two independent
      span lists, resolve overlaps between them once (a PII span inside a would-be entity
      link should suppress or absorb that link, not run through it twice), and splice the
      merged result exactly once. That merge/precedence rule is the actual undesigned work
      F4 names — not present in this repo, in xberg, or in hacienda-core, and out of scope
      for L7 (investigation only, per this item's own text). Left for whoever picks up F4.

---

## Sequencing

**Two things gate everything, and they are independent — run them in parallel.**

**L1 (measure the wasm budget) starts now.** It is a build and a byte count, it needs no
design decisions, and its answer changes Track L's shape completely: ~1.9 MB of headroom
against a 48 MB artefact is the difference between "ship it on the CDN" and "self-hosting is
mandatory". Nothing downstream should be designed on a guess about this number.

**I2 (the vault layout) remains the keystone decision for the whole repo.** It is the contract
three surfaces must satisfy; settle it and Tracks G, I, J and half of F follow. Then **E1**
(framework), **B1/H3** (inference runtime), and **J2** (whether the CLI vault is the full
artefact or a thinner one).

**F2 is now answered, not open.** Whether Studio's pseudonym tokens interoperate with the Rust
CLI stops being a design question once both run the same AES-SIV code — Track L makes them
identical by construction. Do not design a TypeScript token format.

Then: **H1** immediately — it is running an existing script for a 2× win, and it is
independent of everything else. **A3 → A1 → A4**: smallest diffs, each independently
observable. **C1 and C2's fixture corpus** are cheap and should not be deferred: C1 is a
paragraph that prevents the most likely wrong turn, and the corpus becomes Track L's
acceptance test (L6), so building it early gives the port a target.

**What the WASM decision removes from the critical path:** A2 (make the redact toggle real)
and F4 (the offset problem) were the two hardest items and are both now Track L's problem
rather than TypeScript work. A2 should *not* be implemented against `pii-detector.ts` — that
file is deleted in L6. If A2's compliance risk (a "redact" toggle that silently does nothing)
must be closed before L6 lands, close it by **disabling the toggle**, not by implementing it
twice. And read L7 before designing F4: `preserveOffsets` on `WasmRedactionConfig` may already
solve it.

**B2** should still wait for D1 so the substitution is measurable.

**I2 → I1 → G1/G2 → I3 → I4** is the product spine. Note that I2 and G2 are the same decision
seen twice (how entities are linked in the exported markdown) — settle it once, in I2.

Track E remains the largest single item. Given the product is folder-in/vault-out with a
Finder view, the practical question is no longer "Svelte or React" but "start from Studio's
pipeline and add hacienda-private's UI, or start from hacienda-private's UI and remove its
server". The second may be shorter: its 40 components, `/browse`, and document viewers already
exist, and what must be *deleted* (matters, folders, auth, mirror push) is well isolated in
`lib/api.ts`. Evaluate both before starting E3 — but either way the worker pipeline,
`kg-export.ts`, `verticals/` and `registry.ts` come from Studio unchanged.

Do not run the full Rust test suite as a matter of course on this host: ~3 GB RAM,
`cargo clean` costs ~10 minutes. Target `cargo test -p hacienda-core pii::` for C2.

---

## Status convention

Unchecked boxes in the superseded plan turned out to mean "not verified", not "not done" —
several were complete. **In this document an unchecked box means the *check* line has not been
observed to pass.** Do not tick a box on the strength of the code existing.
