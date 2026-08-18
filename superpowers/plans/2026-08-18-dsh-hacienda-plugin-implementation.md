# DSH-Hacienda Plugin — Implementation Plan (Artifacts View + Local Models + Exa + M2 Safe ZIP)

> **For agentic workers:** implement this plan task-by-task with checkbox tracking. Each phase
> ends on an independently verifiable gate. Phases G/S are gated on upstream PR #81.

**Goal:** Stand up the "hacienda as a bundled DeepSeek Harness plugin" (Approach A) end to end:
(a) a **Files/Artifacts view** in the DSH web UI (Finder-style folder browser + `@extend-ai`
filetype viewers + CodeMirror redacted-markdown), (b) **local models** via Ollama + the in-browser
GLiNER2 worker pool, (c) **Exa deep search** already wired, and (d) the **M2 safe-ZIP export**
(audited, PII-redacted folder → self-verifying ZIP for Claude Desktop).

**Deliverable of this document:** the step-by-step implementation plan (per user: plan/spec only;
no code from this pass). Scope = full end-to-end, gated on P7/PR #81 for the ZIP.

**Spec:** `docs/superpowers/specs/2026-08-18-M2-dsh-plugin-export-and-assure.md` (§1–§17) is the
design this plan executes. Decisions referenced below use its `D-*` keys.

**Locked decisions (from the design + user):**
- Scope: **full end-to-end**. Deliverable: **plan only** (not a build pass). ZIP: **gated on
  P7/PR #81** (do not build the exporter against the current-main structured-field leak).
- Approach A only (bundled profile, not a code fork) — §12.2-D12.2.
- One client = one workspace (§9.1). Browser NER reuses GLiNER2/xberg-wasm (§9.2, D-M3-3).
- Artifacts view mounts as a `conversation.view` tab + 4 Host RPCs, reusing Studio `extend/` +
  `@extend-ai` viewers + CodeMirror (§13-14, D-14.x). WebGPU = deferred (§10.1 L5).
- O1: Ollama via `dsh-llm-pi-ai` hand-declared route (no custom plugin); O2: no new sandbox,
  use `ctx.e2b` only if provisioned (§10.3).

---

## Dependency graph (why this order)

```
S1  Land PR #81 (P7 + hacienda-mcp + xberg parity)   [upstream, the only external gate]
 │
 ├─► S2  Host: M2 facade tools (scan_folder, export_safe_zip)   ── requires P7
 │
 ├─► S3  Host: Exa deep search (done; verify)                    [no dep on S1]
 │
 ├─► S4  Host: Ollama local-models route (config)                [no dep on S1]
 │
 └─► C1  Client: Artifacts view (shell + 4 RPCs + viewers)       [no dep on S1, except
     │                                                             redaction pane needs P7 for
     │                                                             the authoritative pass]
     ├─► C2  Client: in-browser GLiNER2 worker pool (§10.1)       [no dep on S1]
     │
     └─► C3  Bundle: @hacienda/dsh-hacienda + profile             [wraps S2–S4, C1–C2]
```

Everything except S1 and the C2 redaction *authoritative* pass can proceed in parallel; the plan
sequences buildable slices so late-spawned gates don't block early work.

---

## Phase G. Upstream gate — land PR #81 (blocking M2 ZIP)

**Objective:** make `P7` (structured-field redaction fix) + `hacienda-mcp` + xberg parity land on
`main`, so the ZIP exporter's safety claim is true.

- [ ] G1. Confirm PR #81 state (open, `mergeable_state=dirty`; 13 CI checks green). Identify the
      ~35 conflict regions (`git merge-tree`): `.sqlx/*.json`, `Cargo.lock`, and real source.
- [ ] G2. If the full PR can't land promptly, **split P7 out first** (independently reviewable,
      CI-green) and land it; then land `hacienda-mcp`.
- [ ] G3. Resolve conflicts against current `main` (rebase/merge), re-run CI (`cargo fmt`,
      `clippy`, `cargo test -p hacienda-* -p hacienda-mcp`, `test-bindings`, cargo-deny).
- [ ] G4. Merge; correct `alef.toml` `mcp_sources` → `hacienda-mcp/src/server.rs` (housekeeping).
- [ ] G5. Gate-check: `HaciendaFacade::redact_structured_fields` present on `main`;
      `hacienda mcp serve` builds and lists 8 tools.
- **Verify:** a control-corpus value planted in `tables`/nested-archive child comes back redacted
  (the P7 negative invariant), run through the CLI, over MCP, and (once C2 exists) in the pane.
- **Exit:** `main` has P7 + `hacienda-mcp`; the M2 exporter can target the P7-corrected facade.

---

## Phase S3. Exa deep search (verified; make it repeatable)

**Objective:** the Exa + anonymous-fetch wiring already applied to the web profile works on every
boot and is reproducible for the bundled profile.

- [ ] S3.1. Re-verify `/home/jamin/.dsh/profiles/web/cordis.patch.yml`: `web.searchProvider=exa`,
      `web-search-exa` insert (`apiKey: !!js process.env.EXA_API_KEY`), `web-fetch-http` insert,
      `tool-web` enabled (`disabled:false`, `fetch:true`). Both packages in `package.json`.
- [ ] S3.2. Re-verify boot on a fresh port: `bash /home/jamin/.dsh/launch-web-exa.sh 3081` →
      `dsh web: http://127.0.0.1:3081`, process env carries `EXA_API_KEY`, `GET /` → 200.
- [ ] S3.3. Smoke-test `web_search` (Exa) and `web_fetch` in the 3081 surface; confirm sources +
      highlights, and cost accounting (~$0.007/search neural).
- [ ] S3.4. (Bundle) record the same two rows as reusable patch fragment in the bundle (Phase C3),
      so the bundled profile ships Exa without manual re-patching.
- **Verify:** `--dump-config` shows both provider rows, no patch errors; live search returns sources.
- **Exit:** Exa + web_fetch reproducible; no key committed (env-only).

---

## Phase S4. Ollama local models (config-only)

**Objective:** Gemma on Ollama becomes a selectable agent model via the **shipped**
`@deepseek-ai/dsh-llm-pi-ai` adapter (no custom plugin) — §12.1, §10.3-O1.

- [ ] S4.1. Confirm `@deepseek-ai/dsh-llm-pi-ai` row is composed (it ships with the web profile).
- [ ] S4.2. Add the `llm-pi-ai` provider profile for `ollama` (hand-declared route) — either in
      the user `settings.yaml` (`llm-pi-ai.providers.ollama`) or as a bundle base:
      `api: openai-completions`, `baseURL: http://localhost:11434/v1`, keyless,
      `models: [gemma2:2b, gemma3:4b]`.
- [ ] S4.3. (Optional) add the "Local Models / Ollama" settings page (own `settings.section`) whose
      Host tool does `ollama pull` and writes the model into `llm-pi-ai` — §12.1. Defer improve if
      pull-in-Settings is out of MVP.
- [ ] S4.4. Verify: `ctx.llm.listConfigurableProviders()` lists `ollama`; the model picker shows
      Gemma; a chat turn routes to Ollama.
- **Exit:** Gemma selectable via stock UI; no code written (config only).

---

## Phase S2. M2 Host tools: scan & audited safe-ZIP export

**Objective:** Host tools that scan a client folder and export a redacted, audited ZIP, running the
**P7-corrected** facade (§4-§5). *Requires Phase G.*

- [ ] S2.1. `harness.registerTool` `scan_folder` — list folder via `ctx.fs`, detect PII via the
      facade (or MCP `pii_scan`), return the inventory (`redacted_spans`, counts). Declare full
      `output { schema, render }` (§11.2) + `isConcurrencySafe` (§11.3).
- [ ] S2.2. `harness.registerTool` `export_safe_zip` — args `{ sourceDir|sandboxRef, mode: mask,
      includeCompliance?, outZip? }`; enumerate (§14 RPC1), route each doc through the facade's
      P7-corrected redaction, write audit chain + `_manifest.json` + `verify.txt` + `vault/` into
      the ZIP (Track I2 layout, §4), then record the ZIP path as a **mutation `locations`** and
      `presentCall`/`presentResult` → **`card:'diff'`** so it surfaces as a produced-file chip
      (§11.1). Each call asks `ctx.approval.request` (one-shot) (§11.4).
- [ ] S2.3. Assurance tests (§5-S6): leak test (P7-class, over the ZIP), manifest/audit round-trip,
      `hacienda audit verify` on the embedded chain, verdict states `SAFE|REDACTED|MANUAL_REVIEW`.
- [ ] S2.4. E2B variant (optional, S5): if `ctx.get('e2b')` defined, stage/scrub in-sandbox;
      degrade to local folder when absent (§11.5).
- **Verify:** `export_safe_zip` output passes `hacienda audit verify`; no unredacted PII in
  `documents/*` or manifest; the ZIP appears as a produced-file chip.
- **Exit:** safe-ZIP export proven on the P7-corrected facade.

---

## Phase C1. Client: Artifacts view (shell + RPC + viewers)

**Objective:** the Finder-style folder browser with filetype viewers and CodeMirror markdown, as a
`conversation.view` tab (§13-14). No dependency on S1 except the redaction pane.

- [ ] C1.1. Client registers `conversation.view` entry `{ id:'artifacts', order:2, label:'Files' }`
      + optional `conversation.session.header.actions` button (§14.1).
- [ ] C1.2. Host RPCs (`harness.handle`; Client `host.call`, lossless JSON §14.2):
      `list-workspace` (→ `FileSystemItem[]`, owned JSON), `read-file-text`, `file-download-url`
      (`ctx.webServer` GET for large PDF/DOCX; RPC for text), `scan-artifacts` (span overlay).
- [ ] C1.3. Reuse Studio `components/extend/file-system.tsx` (grid/list/columns/gallery, sort,
      filter, thumbnails) fed by RPC1 (§13.1, D-14.3).
- [ ] C1.4. `onFileOpen` dispatch by `viewerKindForFile`: DOCX/PPTX/XLSX → `@extend-ai` viewers,
      PDF → `pdf-viewer`, image → `<img>`, markdown/code → **CodeMirror** (`@codemirror/view` +
      `lang-markdown`, one-dark) with PII-span decoration from RPC4 (§13.4).
- [ ] C1.5. Live refresh: re-run RPC1 on FS/workspace change so engine-produced artifacts appear
      (§11.9).
- **Verify:** a folder renders by filetype; a DOCX/PDF opens in its viewer; a `.md` opens in
  CodeMirror with spans; a file written by a tool appears on refresh.
- **Exit:** artifacts view browse + view works (read/browse; no destructive ops per D-14.4).

---

## Phase C2. Client: in-browser GLiNER2 worker pool (NER acceleration)

**Objective:** fast, parallel, zero-egress NER for the redaction pane (§10.1). Reuses
`@xberg-io/xberg-wasm` + `hacienda-wasm`; WebGPU deferred (§10.1 L5 / D-M3-3).

- [ ] C2.1. Vendor/load `@xberg-io/xberg-wasm` `NerModel` + `hacienda-wasm` (`ner-candle-wasm`
      feature) in a Client-half plugin Web Worker — the Studio `asset-loader.ts`/`pipeline.ts`
      pattern (§10.1, §10.2).
- [ ] C2.2. **Lever 1**: spawn `min(hardwareConcurrency, 4)` workers, shard the document list,
      merge results (§10.1 N1).
- [ ] C2.3. **Lever 3** (MVP if long docs are primary input): overlapping-token chunk sharding,
      re-stitch spans (§10.1 L3).
- [ ] C2.4. Wire spans into the CodeMirror redaction pane (C1.4); approve/reject/flag writes to the
      durable review queue via the Host (C2→S2 facade / MCP review tools) — two-tier trust
      (§11.4, D-M3-2). Durable review queue is the recommended feedback channel.
- [ ] C2.5. (Deferred) Lever 2 SAB pool (needs COOP/COEP) then Lever 5 candle-WebGPU recompile —
      only if latency demands (§10.1 N2/N3).
- **Verify:** N–way parallelism measured (wall time vs serial); spans surface in the pane; review
  decisions persist to the queue.
- **Exit:** interactive redaction feedback works in-browser; authoritative pass still on P7 facade.

---

## Phase C3. Bundle: `@hacienda/dsh-hacienda` profile (Approach A)

**Objective:** ship everything as one bundled profile — a distribution, not a fork (§12.2 / D-12.2).

- [ ] C3.1. Create bundle package `@hacienda/dsh-hacienda` with a `cordis.patch.yml` that
      `- insert:`s: the Host tools (S2), the Ollama `llm-pi-ai` base (S4), the Exa rows (S3.4), and
      the Client plugin (C1/C2). Reuse `- insert:` (plain `- id:` can only override, not create —
      §16 root cause).
- [ ] C3.2. Create `@hacienda/dsh-client-hacienda` carrying the Client-half (Artifacts view,
      redaction pane, Local-Models settings page).
- [ ] C3.3. Register `bundles: [@deepseek-ai/dsh-base, @deepseek-ai/dsh-web-app,
      @hacienda/dsh-hacienda]` in the profile manifest; pnpm-install both packages into
      `$DSH_HOME/profiles/hacienda/node_modules`.
- [ ] C3.4. Validate with `--dump-config` (no "entry not found" / EADDRINUSE class errors) and a
      fresh-port boot.
- [ ] C3.5. Publish/dev path: `git tag` + registry or local tarball; document `dsh --profile
      hacienda` + `launch-web-exa.sh` (fresh port, key via launch env).
- **Verify:** one profile brings up Exa + Ollama + Artifacts view + M2 export together.
- **Exit:** `@hacienda/dsh-hacienda` is a reproducible, bundled harness profile.

---

## Cross-cutting requirements (apply in every phase)

- **Key handling:** never commit the raw Exa key; use `process.env.EXA_API_KEY` (§16). Ollama is
  keyless. No secrets in `cordis.patch.yml`/`package.json`.
- **Tool contracts (§11):** every tool ships `output { schema, render }`; mutation tools return
  `card:'diff'` views + `locations` so produced files surface; `isConcurrencySafe` where safe.
- **Approval (§11.4):** per-call `ctx.approval.request` for destructive/heavy tools; interactive
  span decisions use the **durable review queue**, not the approval seam.
- **Live vs durable (§11.9):** audit/provenance rides the durable `tool/result` path; the artifacts
  view refreshes off FS events.
- **Sandbox/policy:** route `fs`/`shell` mutations through `ctx.sandboxPolicy.resolve`; the profile
  patch lives under `$DSH_HOME/profiles/*` (user-editable, not shipped bundles).

## Risks / mitigations

| Risk | Mitigation |
|---|---|
| PR #81 conflicts drag on | Split P7 out and land first (G2); it is the only safety-critical piece. |
| 600MB GLiNER2 × N workers RAM | Cap workers (C2.2 default 4); move to SAB pool (C2.5/L2) when prod RAM known. |
| Exa cost (~$0.007/search) | It's metered, not free; budget + expose only where intended (§15). |
| Two harnesses / port conflicts | Launcher boots on a fresh port; migrate to 3080 only after stopping the old (§16). |
| Model bytes / egress | Zero-egress in-browser path; authoritative pass is server-side. |
| CodeMirror vs Extend markdown | Studio already has CodeMirror; Extend UI's markdown (if present) is optional — KISS with CodeMirror (D-14.3). |

---

## Detailed task breakdown (execution-ready)

Each task is concrete: the exact file/contract to touch, the acceptance test, and the pass/fail
signal. Work the phases in dependency order; anything not gated on S1 can be picked up immediately.

### Phase G — upstream gate (tasks G1–G6)

- [ ] **G1** Check PR #81 with the GitHub API: `gh pr view 81 --json state,mergeable,mergeableState,mergeable_state` and `gh api repos/jamon8888/hacienda-engine/pulls/81`. Expect `state=OPEN`, `mergeable=false`, `mergeable_state=dirty`. Confirm the 13 CI runs are green (they were at head `3f2a3640`).
- [ ] **G2** If the full PR can't land quickly, open a focused PR containing **only** the P7 slice: `HaciendaFacade::redact_structured_fields` + `redact_document_recursively` and their tests. This is independently reviewable and CI-green. Land it first.
- [ ] **G3** Rebase/merge the MCP slice (`hacienda-mcp/` crate, `hacienda mcp serve`, xberg format parity) onto the new `main`. Resolve conflicts: re-run `git merge-tree $(git merge-base main HEAD) main HEAD` and fix `.sqlx/*.json` + `Cargo.lock` drift + any real source hunks.
- [ ] **G4** Re-run the full gate: `cargo fmt --check`, `cargo clippy --workspace`, `cargo test -p hacienda -p hacienda-core -p hacienda-api -p hacienda-cli -p hacienda-mcp`, `cargo test -p hacienda-api` (bindings/E2E), `cargo deny check`. Fix regressions.
- [ ] **G5** Fix housekeeping: edit `alef.toml` `[workspace.docs] mcp_sources` → `["hacienda-mcp/src/server.rs"]`. Run `cargo alef check` so bindings stay consistent.
- [ ] **G6** Merge. **Gate test:** `cargo run -p hacienda-cli -- mcp serve` (stdio) → `tools/list` returns the 8 documented tools; new negative test `pii_redact_never_returns_the_redacted_value` passes. Confirm `redact_structured_fields_covers_every_known_field` exists on `main`.
- **Acceptance:** `main` has P7 + `hacienda-mcp`; all gates green.

### Phase S3 — Exa deep search (tasks S3.1–S3.5)

- [ ] **S3.1** Inspect `/home/jamin/.dsh/profiles/web/cordis.patch.yml` (already correct): `web.searchProvider=exa`, `web-search-exa` insert, `web-fetch-http` insert, `tool-web` with `disabled:false`+`fetch:true`. Confirm both deps in `profiles/web/package.json`.
- [ ] **S3.2** Boot on a fresh port to verify (do not disturb the running GUI): `bash /home/jamin/.dsh/launch-web-exa.sh 3081`; check `dsh web: http://127.0.0.1:3081` appears and `GET /` → 200; check `EXA_API_KEY` present in the child process env via `/proc/<pid>/environ`.
- [ ] **S3.3** Live smoke test on 3081: ask a `web_search` for Exa results (sources + snippet highlights), and a `web_fetch` of a URL. Confirm the Exa `results[]`→`WebSearchSource` mapping (url/title/highlight/publishedDate) and that a highlight-less result is dropped (provider behaviour).
- [ ] **S3.4** Extract the exact rows into a portable patch fragment (`bundle/exa.patch.yml`) so the C3 bundle can ship them without re-typing. Commit **no** key (env-only).
- [ ] **S3.5** Record observed per-search cost (neural ~$0.007) in the plan/README so spend is expected.
- **Acceptance:** Exa + web_fetch work on every boot; reproducible fragment; no key on disk.

### Phase S4 — Ollama local models (tasks S4.1–S4.4)

- [ ] **S4.1** Confirm `@deepseek-ai/dsh-llm-pi-ai` is composed in the profile (it ships with `dsh-base`/`dsh-web-app`). If not present, add the row.
- [ ] **S4.2** Add the hand-declared `ollama` route to the `llm-pi-ai` namespace (user `settings.yaml` or bundle base):
  ```yaml
  llm-pi-ai:
    providers:
      ollama:
        api: openai-completions
        baseURL: http://localhost:11434/v1
        models:
          - id: gemma2:2b
            name: Gemma 2 2B (local)
            contextWindow: 8192
          - id: gemma3:4b
            name: Gemma 3 4B (local)
            contextWindow: 32768
  ```
- [ ] **S4.3** Verify discovery: `ctx.llm.discoverModels('llm-pi-ai', { baseURL:'http://localhost:11434/v1' })` → Ollama's `/v1/models` list; model picker shows Gemma.
- [ ] **S4.4** Optional (defer if not MVP): add the "Local Models / Ollama" `settings.section` page whose Host tool does `ollama pull` (stream progress) and writes pulled ids into `llm-pi-ai.providers.ollama.models` so the picker updates immediately (§12.1).
- **Acceptance:** Gemma selectable via stock Models page; a chat turn routes to `localhost:11434`.

### Phase S2 — M2 Host tools (tasks S2.1–S2.6; *requires G*)

- [ ] **S2.1** `scan_folder` tool: `ctx.fs.listDir`/`stat` → per-file `{path,size,mtime,type}`; facade PII scan; return `{ files, totalRedactedSpans, detected:[{file,category,count}] }`. `output { schema, render }`; `isConcurrencySafe` true.
- [ ] **S2.2** `export_safe_zip` tool. Args `{ sourceDir?, sandboxRef?, mode:'mask', includeCompliance?, outZip? }`. Body: enumerate (reuse RPC1 shape) → for each doc call the **P7-corrected** facade `process_batch_with_auth`/`redact_structured_fields` → collect audit chain → build Track I2 layout (`documents/`, `_manifest.json`, `verify.txt`, `vault/`, `README.md`) → `JSZip`-pack (Host-side; Studio uses `jszip`) → write ZIP.
- [ ] **S2.3** Tool result contract (§11.1): `presentCall` → `DiffCallView` ({card:'diff', diffs:[{path, oldText:null, newText:'<zip created>'}]}), `presentResult` → `DiffResultView` naming ZIP + `documents/*`; record the ZIP absolute path in the result `locations` so the deliverables row surfaces it. This is what makes it a produced file.
- [ ] **S2.4** Approval (§11.4): call `ctx.approval.request` once per `export_safe_zip` with a clear reason; on `rejected`/`never` fail closed before touching the folder.
- [ ] **S2.5** Assurance tests: (a) leak test — plant a control-corpus email/IBAN in a `.md` body AND in a table row / nested archive child (P7 fields); assert not present in `documents/*` text or `_manifest.json`; (b) `hacienda audit verify <audit-dir>` on the embedded chain; (c) verdict coverage `SAFE|REDACTED|MANUAL_REVIEW`; (d) `verify.txt` present with instructions.
- [ ] **S2.6** E2B variant (optional): if `ctx.get('e2b')` defined, copy folder into `ctx.e2b.cwd`, run scrub in-sandbox, copy ZIP back; else local-folder. Guard the `undefined` case.
- **Acceptance:** ZIP passes `hacienda audit verify`; no PII in output; produced-file chip appears.

### Phase C1 — Artifacts view (tasks C1.1–C1.6)

- [ ] **C1.1** Client: `slots.inject('conversation.view', ...)` registering `{ id:'artifacts', order:2, label:'Files' }`; optional `conversation.session.header.actions` button `{id:'open-artifacts'}`. Use `ctx.get('slots')` (optional), `React.createElement` only.
- [ ] **C1.2** Host `harness.handle` RPCs: `list-workspace`→`{path,items:FileSystemItem[]}`, `read-file-text`→`{text}`, `file-download-url`→`{url}` (via `ctx.webServer` route), `scan-artifacts`→`{path,spans[]}`. Return **owned JSON only** (§14.2).
- [ ] **C1.3** Port/import Studio `components/extend/file-system.tsx` `FileSystem` component (grid/list/columns/gallery, sort/filter/search) feeding RPC1.
- [ ] **C1.4** `onFileOpen` dispatch: `viewerKindForFile` → `@extend-ai/react-docx` (`DocxEditorViewer`), `react-pptx`, `react-xlsx`, Studio `pdf-viewer`, `<img>` for images, CodeMirror for md/txt/code.
- [ ] **C1.5** CodeMirror markdown editor (`@codemirror/view`+`lang-markdown`, one-dark) with PII-span decorations from RPC4; approve/reject/flag → durable review queue (via facade/MCP, not approval seam) (§11.4).
- [ ] **C1.6** Live refresh: re-run RPC1 on FS/workspace change; `/` relative paths resolve against session cwd (§11.8).
- **Acceptance:** folder renders by filetype; a .md opens in CodeMirror with spans; a produced file appears on refresh.

### Phase C2 — GLiNER2 worker pool (tasks C2.1–C2.5)

- [ ] **C2.1** Load `@xberg-io/xberg-wasm` `NerModel` + `hacienda-wasm` in a Client Web Worker (Studio `asset-loader.ts` + `pipeline.ts` pattern; weights cached in IndexedDB).
- [ ] **C2.2** Lever 1: spawn `min(navigator.hardwareConcurrency, 4)` workers, shard docs, merge. Measure wall-time vs serial (§10.1 N1).
- [ ] **C2.3** Lever 3 (MVP if long legal docs): overlapping-token (512) chunk sharding + re-stitch spans.
- [ ] **C2.4** Surface spans in the CodeMirror pane (C1.5); decisions write to the durable review queue.
- [ ] **C2.5** Deferred: Lever 2 (SAB pool, COOP/COEP) then Lever 5 (candle `wgpu` recompile) — only if latency demands (§10.1 N2/N3, D-M3-3).
- **Acceptance:** N-way wall-clock improvement; spans interactive; review persists.

### Phase C3 — Bundle (tasks C3.1–C3.6)

- [ ] **C3.1** `bundle/exa.patch.yml` (S3.4) + `bundle/ollama.patch.yml` (S4.2) as portable fragments.
- [ ] **C3.2** Create `@hacienda/dsh-hacienda` package: `cordis.patch.yml` with an `- insert:` block listing Host tools (S2), `llm-pi-ai` base (S4/Ollama), Exa rows (S4/S3), and an include of the Client plugin. Use `- insert:` (plain `- id:` cannot *create* rows — §16).
- [ ] **C3.3** Create `@hacienda/dsh-client-hacienda` with the Client half (C1 + C2 + Local-Models page).
- [ ] **C3.4** Profile manifest `bundles: [@deepseek-ai/dsh-base, @deepseek-ai/dsh-web-app, @hacienda/dsh-hacienda]`; `pnpm add -w` both packages into `$DSH_HOME/profiles/hacienda/node_modules`.
- [ ] **C3.5** Validate: `dsh --profile hacienda --dump-config` → no errors, all rows present; boot on a fresh port; confirm Exa+Ollama+Artifacts+M2 all live.
- [ ] **C3.6** Release: tag + local tarball/private registry; document `dsh --profile hacienda` and `launch-web-exa.sh` (fresh port; key via launch env).
- **Acceptance:** one profile reproduces the full stack; `--dump-config` clean.
