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

- [ ] **A1. Call `detectPII` from the worker, gated on `config.enablePiiDetection`.**
      `worker/pipeline.ts`, after markdown is produced (~line 200, before the NER block).
      *Check:* a document containing `jean.dupont@cabinet-exemple.fr` yields a PII entity in
      the result; with the toggle off, it yields none.

- [ ] **A2. Call `redactPII` when `config.redactPiiInOutput` is set.**
      Must apply to the markdown that reaches the zip, and must run *after* NER so entity
      offsets are computed against unredacted text. Decide and record what happens to the
      entity registry and KG export under redaction — exporting a redacted document beside a
      knowledge graph naming every entity would defeat the feature. *Check:* extend
      `tests/e2e/egress.spec.ts`'s contract fixture — the downloaded markdown must not contain
      the IBAN, and the KG export must not reintroduce it.

- [ ] **A3. Widen the upload gate for audio/video.**
      Add `audio/` and `video/` to `SUPPORTED_MIME_PREFIXES` and matching extensions to
      `accept=` (`App.svelte:178`). Note `SUPPORTED_MIME_PREFIXES` and `validateFile` are
      **duplicated verbatim** in `lib/asset-loader.ts:157` and `lib/types.ts:104` — collapse
      to one before editing, or the fix lands in the copy nobody imports. `App.svelte` imports
      the `asset-loader` one. *Check:* an `.mp3` reaches `worker/pipeline.ts:149`.

- [ ] **A4. Stop reporting success on model-load failure.**
      `App.svelte:60-62` catches the download error and sets `assets.nerModel = true` anyway;
      the outer catch at `:71` does the same. Surface degraded state instead — this is what
      would hide a Track B regression. *Check:* with the model URL blocked, the UI says the
      neural backend is unavailable.

---

## Track B — Use the model that is already downloaded (Studio)

This is the fix for the product's worst behaviour, not polish: compromise.js is English-only
and the documents are French. The downloaded model is `GLiNER2-Guardrails-PII-Multi` —
multilingual and PII-specific.

- [ ] **B1. Decide the inference backend. Blocking; do this first.**
      Two paths exist and neither is used. `xberg-wasm`'s `NerModel` — `createNerBackend()` is
      already written against it and its `detect()` return shape
      (`{category,text,start,end,confidence}`) already matches what the worker's bridge
      consumes. Versus `@lmoe/gliner-onnx`, which is installed, unused, and would need ONNX
      weights rather than the safetensors currently cached.
      **Recommendation: xberg-wasm; delete the `@lmoe/gliner-onnx` dependency.** It is the one
      the existing code targets, and carrying two unused neural backends is how this state
      arose. Record the decision in this file — the reason to revisit is B3, not preference.

- [ ] **B2. Call `createNerBackend()` and route the bridge through it.**
      `worker/pipeline.ts:204-212` constructs `XbergEngine` with `{ner:{ner: extractEntities}}`.
      Load the model bytes in the worker (IndexedDB is worker-accessible, so `loadNerModel()`
      can be called there) and substitute `runtime.detect`. Keep `extractEntities` as the
      fallback when the model is unavailable — that path is what A4 must surface.
      *Check:* a French fixture naming `Maître Jean Dupont` and `Acme SAS` yields a `person`
      and an `organization`. Assert this fails against compromise.js first.

- [ ] **B3. Resolve model delivery — 1.23 GB is not shippable.**
      Unquantized safetensors, fetched from huggingface.co on first run. Options: quantized
      weights, a smaller GLiNER2 variant, or self-hosting via the `VITE_MODEL_BASE_URL`
      override that `asset-loader.ts:19` already provides (EU-resident hosting is a selling
      point for these clients, not just a size fix). If quantization forces ONNX, B1 reverses.
      *Check:* record measured first-run download size and time-to-first-document.

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

- [ ] **C3. Studio gets the real audit trail via Track L, not a second implementation.**
      Previously an open question. It is now answered by the WASM decision: the blake3 chain
      and AES-SIV reversible pseudonyms come to the browser as the same Rust code, with an
      IndexedDB `AuditStore` in place of `FileAuditStore`. **Do not build a TypeScript audit
      chain.** The only open sub-question is *persistence semantics*: a file-backed chain
      survives the process, an IndexedDB one dies with a cleared browser profile. If Studio
      output must be legally defensible, the chain has to be exported inside the vault
      (Track I2 / G) rather than merely retained locally.

---

## Track D — Tests that would have caught this

Current suite: 3 Playwright specs, 5 vitest files. The e2e pipeline test passes and proves
extraction works. Nothing asserts PII, redaction, transcription, or that a toggle does
anything — which is precisely why 31 unchecked boxes coexisted with substantial working code.

- [ ] **D1. Assert toggles have effect.** For each of `enablePiiDetection`,
      `redactPiiInOutput`, `enableTranscription`, `enableVerticalNer`: one test where the
      output differs with the flag on and off. A dead toggle then fails a test instead of
      shipping.
- [ ] **D2. French-language NER fixture** (supports B2).
- [ ] **D3. Audio fixture through the full pipeline** (supports A3; the old plan's Phase 5
      referenced an `audio/mpeg` fixture that was never added).

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

- [ ] **E1. Decide the framework. Blocking — everything in Track E and F depends on it.**
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

- [ ] **E2. Port the design tokens first, independently of E1.** Tailwind config + CSS
      variables are framework-agnostic and can land before any component work.

- [ ] **E3. Screens.** Upload (`file-dropzone`), processing (`progress`), document view
      (`resizable` dual-pane), entity/PII panel (`card` + `badge` + `popover`), export.
      Studio needs no matters/folders/auth — that is hacienda-private's multi-user model and
      must not be imported along with the components.

---

## Track F — PII review, pseudonymization, and the editor (Studio)

This is the flow the user asked for, and it is where the two apps differ most.

- [ ] **F1. Adopt the PII reveal UX; do not adopt its data model.**
      `PiiPanel.tsx` is the pattern worth copying: detected spans render as tokens with a
      `Badge` for the kind, clicking opens a `Popover` that asks for a passphrase, and the
      plaintext is shown in a `Dialog` and forgotten on close — never persisted. The vault is
      WebCrypto-sealed client-side. `PiiReviewPanel.tsx` adds accept / correct / reject per
      detection. All of this is directly applicable to Studio and is the single most valuable
      thing to port.

- [ ] **F2. Build reversible pseudonymization — it does not exist anywhere in JS yet.**
      hacienda-private does **redaction only**: opaque stable tokens `{{C0_PERSON_1}}` with
      no exportable mapping. Reversible pseudonymization exists only in Rust
      (`redaction/pseudonym.rs:520` `reveal()`, AES-SIV, `[CATEGORY:key_id:BASE32]`), and per
      the WASM analysis above that code cannot reach the browser. So Studio must implement it
      in TypeScript against WebCrypto.
      **Match the Rust token format exactly** so a document pseudonymized in Studio can be
      revealed by the CLI and vice versa. If that is not wanted, say so explicitly here —
      silently choosing a different format is how Track C's divergence happens again.
      *Check:* round-trip a document through Studio pseudonymize → CLI reveal.

- [ ] **F3. CodeMirror 6 markdown editor with inline PII decorations.**
      Nothing exists to build on: CodeMirror is not a dependency in any repo, and
      hacienda-private's right pane is a read-only `<pre>` of redacted text rendered with
      `react-markdown`. Use CodeMirror decorations to render PII spans as inline pills that
      open the F1 popover.
      *Check:* editing text before a PII span does not misplace that span's highlight.

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

- [ ] **G1. Preserve entity linking through pseudonymization.** Already implemented at
      `worker/pipeline.ts:93` as `[Acme SAS](entity:organization/acme-sas)`, with a
      `## Entities` glossary (`:118`) and entity metadata in frontmatter (`:99`). Pseudonymizing
      an entity must not orphan its link or its glossary row.

- [ ] **G2. Make the links resolvable in Claude Desktop.** `entity:` is a custom URI scheme
      nothing outside Studio understands. Switch to in-document anchors into the `## Entities`
      glossary, or wikilinks, so the exported markdown is self-contained. *Check:* open the
      exported markdown in Claude Desktop and follow a link to its glossary entry.

- [ ] **G3. Ship a README in the zip** describing what the bundle is, so a Claude Desktop
      session has the context to use the registry and KG files rather than only the prose.

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

- [ ] **H1. Reuse the f16 artifact.** Run the existing script, publish the output, point
      `VITE_MODEL_BASE_URL` (`asset-loader.ts:19`) at it. 1.23 GB → 614 MB for near-zero
      engineering.
- [ ] **H2. Decide whether 614 MB is acceptable.** It is a 2× win, not a solution. If not,
      the manifest at `services/mcp-server/models/manifest.json` shows the fallback they
      chose: `onnx-community/gliner_small-v2.1` with int8 variants — a much smaller model at
      some accuracy cost. Evaluate against French legal fixtures before switching, since
      multilingual quality is the whole point (Track B).
- [ ] **H3. Note their runtime differs.** hacienda-private loads GLiNER2 in a dedicated Web
      Worker via `packages/wasm-pipeline/src/gliner2-worker.ts` with a `@xenova/transformers`
      tokenizer, whereas Studio's `createNerBackend()` targets xberg-wasm's `NerModel`. Both
      consume the same safetensors. B1 should account for this third option.

---

## Track I — Folder in, vault out

The core product. Design the output format before writing pipeline code.

### I1. Folder ingest

- [ ] Add `webkitdirectory` to the file input and carry `file.webkitRelativePath` through
      `FileInput` so the source tree survives into the output. Today only `multiple` is set
      and relative paths are discarded.
- [ ] Ingest is sequential (`for (const file of files)`, `worker/pipeline.ts:331`). A folder
      of a hundred documents with a 614 MB model loaded will be slow and gives no way to
      cancel. Decide whether to parallelise or just report honest per-file progress; do not
      leave a progress bar that stalls for minutes with no explanation.
- [ ] Skip/report unsupported files rather than failing the batch — a real folder contains
      `.DS_Store`, images, and things Studio cannot extract.

### I2. The vault layout — the thing to get right

Proposed zip structure. This is a design proposal, not a finding; disagree with it before it
gets built:

```text
README.md                    ← what this bundle is, how an agent should use it
documents/<source tree>.md   ← one markdown per input, original folder structure preserved
entities/<slug>.md           ← one file per entity: type, vertical, roles, aliases, backlinks
GLOSSARY.md                  ← index of every entity, linking into entities/
_manifest.json
entities-registry.json
kg-export/{neo4j.cypher,networkx.json,rdf.ttl}
```

- [ ] **Links must be relative markdown paths**, e.g. `[Acme SAS](../entities/acme-sas.md)`.
      This replaces the current `entity:organization/acme-sas` custom scheme
      (`worker/pipeline.ts:93`), which nothing outside Studio can resolve. Relative paths are
      followable by a Claude Desktop agent with filesystem access, and by Obsidian, and by
      anything else that reads a markdown folder.
- [ ] **One file per entity, with backlinks.** This is what makes the bundle RAG-ready rather
      than merely readable: the agent can open `entities/acme-sas.md` and find every document
      mentioning it. The data already exists — `BatchEntityRegistry` plus
      `inferRelationships` — it is currently only serialised to `entities-registry.json` and
      to the KG exports, which an agent will not naturally read.
- [ ] **`GLOSSARY.md` is the entry point.** The existing per-document `## Entities` section
      (`worker/pipeline.ts:118`) becomes a local summary; the global glossary is the index.
- [ ] **`README.md` tells the agent what it has.** Without it a Claude Desktop session sees a
      folder of prose and ignores the registry and graph files entirely.
- [ ] *Open question:* should redaction/pseudonymisation state be recorded in the bundle —
      i.e. does the vault declare which entities were pseudonymised and under which key id?
      Useful for provenance, but it is also a map of where the sensitive material was.

### I3. Finder-like browser

- [ ] `hacienda-private/apps/web/components/ui/file-system.tsx` is a **4,586-line virtualized
      grid/list** file browser, with `file-thumbnail.tsx` (154) and
      `document-viewer-sidebar.tsx` (112) alongside. Its interface is a flat
      `FileSystemItem[]` of `{ kind: "folder" | "file", path, key }` (`app/browse/page.tsx`),
      which Studio can build directly from in-memory batch results — the component itself is
      not coupled to their API, only their `/browse` page is.
- [ ] Show per-file state in the browser: extracted / PII count / edited / error. The Finder
      view is where the user decides which documents need attention, so it has to carry that.

### I4. Per-document PII editing

"Add and remove PII" is two operations, and neither is accept/reject on a fixed list:

- [ ] **Remove** — a false positive is unmarked and the original text stands.
- [ ] **Add** — the user selects text the model missed and marks it as PII with a category.
      This is manual annotation, not review of a detection. It is the operation that makes the
      tool trustworthy for a lawyer, and neither app has it today.
- [ ] Both require **F4's overlay model**: source text immutable, spans as separate data.
      Adding a span to spliced text is not implementable; this is why F4 gates the editor.
- [ ] Edits must survive re-export and be visible in the Finder view.

---

## Track J — Make the CLI emit the same vault

The reframe's main new work. Today `hacienda extract` takes `inputs: Vec<String>` and a
`--format` of JSON or text (`hacienda-cli/src/cli.rs:110`); it cannot produce a markdown
folder. If the vault is the product, the CLI not producing one means the product only exists
in a browser.

- [ ] **J1. `hacienda extract --vault <dir>`** emitting the Track I2 layout — same
      `documents/`, `entities/`, `GLOSSARY.md`, `kg-export/`. A firm processing 10,000
      documents overnight should get the same artefact a lawyer gets from dropping a folder.
- [ ] **J2. Port entity linking, glossary and KG export to Rust — or don't, deliberately.**
      These exist only in Studio's TypeScript (`worker/pipeline.ts`, `lib/kg-export.ts`,
      `lib/registry.ts`, `lib/verticals/`). Two honest options: reimplement in Rust (a real
      cost, and a third divergence surface), or declare the CLI's vault a thinner artefact —
      markdown plus registry, no cross-document relationship inference. **Decide explicitly.**
      Silently shipping two different things both called "the vault" is the failure mode.
- [ ] **J3. The CLI has capabilities Studio cannot have** — `--audit-out` writing a real
      blake3 chain, reversible pseudonyms, `--no-redact` gated behind
      `--i-accept-unredacted-pii`. If a vault carries an audit chain when produced by the CLI
      and not by Studio, the bundle format must say so rather than leaving a consumer to
      guess.
- [ ] **J4. `--concurrency` already exists** (uncommitted, `ConcurrencyArgs` at
      `cli.rs:157`, wired to `PipelineConfig::concurrency`). Studio's batch loop is sequential
      (`worker/pipeline.ts:331`). Same knob, two behaviours — align the defaults or document
      why they differ.

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
