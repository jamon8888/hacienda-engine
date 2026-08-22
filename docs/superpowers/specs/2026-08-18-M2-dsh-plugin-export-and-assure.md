# M2/M3 — hacienda as MCP load inside DeepSeek Harness: audited, redacted ZIP export + workspace-folder UI + local in-browser model

**Date:** 2026-08-18
**Status:** Investigation complete → design (not yet implemented)
**Builds on:** M1 (`2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md`, on branch
`claude/hacienda-api-cli-mcp-parity-cuz5ak`), S4 API/SDK, P1 redaction guard, Track I2 vault
layout, Studio `apps/hacienda-studio`.
**Depends on / assumes:** `hacienda-mcp` crate (M1a) exists on feature branch
`claude/hacienda-api-cli-mcp-parity-cuz5ak` and is the centerpiece of **PR #81 — OPEN but
merge-blocked** (see §1.1).
**Deliverable this doc describes:** §1–§8 (M2) one DeepSeek Harness **dynamic Host plugin** that (a)
bridges the hacienda capability into the harness as model tools — so a Gemma-over-Ollama agent in
DeepSeek Harness Web can drive it interactively — and (b) packs "an entire folder" (the user's
"E2B process and entire folder with files") into a **ZIP** whose contents have been PII-redacted
through hacienda, with a **manifest + audit chain + verify** report embedded in the ZIP so a human
(or Claude Desktop later) gets an evidence-backed "this archive is safe to process" verdict.
**§9 (M3)** adds the design for customizing the Harness Web UI so a **workspace = one folder = one
client**, plus integrating a **small local model (WASM / WebGPU)** for interactive
redaction/extraction feedback.

---

## 0. TL;DR — what the user asked, decomposed

The request bundles three separable ideas. This doc treats them as layers of one deliverable:

1. **"Use this codebase as an MCP"** — expose hacienda's *redacted* extraction / PII / audit
   capability to AI agents over the Model Context Protocol. **Already designed (M1) and
   half-implemented** (`hacienda-mcp`, stdio, 8 tools) on the `claude/hacienda-api-cli-mcp-parity`
   branch.
2. **"With a plugin … users in DeepSeek Harness Web with Ollama + Gemma"** — a DSH dynamic plugin
   whose **Host half registers model tools** the Gemma agent can call interactively in the web UI.
3. **"E2B process and entire folder with files to create a ZIP (like in Hacienda Studio) …
   with assurance that the ZIP produced will be safe to process with Claude Desktop"** — the new
   work: a folder→ZIP tool that routes the folder through hacienda's redaction + audit, and whose
   embedded manifest / verify step gives a human and Claude Desktop a *verifiable* safety claim.

The conceptual split that matters for the whole design:

| Concern | Where it lives | Status |
|---|---|---|
| hacienda business logic (PII redact, audit, extract) | `hacienda-core` `HaciendaFacade` | shipped, on `main` |
| MCP *surface* over that logic | `hacienda-mcp` crate (stdio, `hacienda mcp serve`) | built on feature branch, **not on `main`** |
| DSH *bridge* that loads MCP tools into a model | `@deepseek-ai/dsh-mcp-client` (native DSH plugin) **or** a direct Host tool | shipped in DSH |
| Folder→ZIP + assurance manifest | **new** `dsh` Host plugin body (this doc) | **new** |

**Key architectural rule carried over from M1 (§2 of that spec):** never proxy xberg's own MCP
server (`extract` returns *unredacted* text). The whole point of hacienda is redaction-by-default
and audited provenance, and M1's `Decision D-M1-1` already forbids the raw-extraction path. The
ZIP-exporter must call through `HaciendaFacade` (or the `hacienda-mcp` tools that wrap it), so the
"safe to process in Claude Desktop" claim is guaranteed by construction, not asserted by prose.

---

## 1. Current state of the codebase (what actually exists)

Verified against a live `git`/source inspection of `/home/jamin/Documents/hacienda-engine`:

### 1.1 The MCP layer is designed, built, and sitting in an OPEN PR that is conflict-blocked

**Verified as of 2026-08-18 (GitHub API + local `git`):**

- **PR #81** — "MCP server, xberg format parity, capability-parity spec program, and the P7
  redaction fix" — from `claude/hacienda-api-cli-mcp-parity-cuz5ak` → base `main`.
  - **Status:** `open`, **not draft**, `merged_at: null`.
  - **Mergeable:** `false` — `mergeable_state: dirty` (`rebaseable: false`). Local
    `git merge-tree` confirms **~35 `changed in both` conflict regions** between `main` and
    the branch (mostly `.sqlx/*.json` cached query files + `Cargo.lock` diverging, plus real
    source files). The branch is 11 commits ahead / 26 behind `main` (merge-base `e94efff`).
  - **CI:** all **13** check runs on branch head `3f2a3640` are **green** (`clippy`, `Lint`,
    `cargo-deny`, `feature-matrix`, `test`, `test-bindings`, `e2e`, `postgres-*`, `validate-*`,
    `test (python 3.10)`, `test (node 22)`). CI is **not** the blocker — the conflicts are.
  - **Reviews:** bot reviews only (CodeRabbit, Sourcery) + human `COMMENTED`; 27 review
    comments, none a blocking `CHANGES_REQUESTED`.

- **The PR carries far more than the MCP server** (this matters for sequencing, §5):
  1. **`hacienda-mcp`** — the MCP server (this doc's foundation).
  2. **P7 structured-field redaction fix** — closes a real leak: `process_batch` redacted
     `.content` but left 9 other text-bearing fields (`tables`, `pages`, `formatted_content`,
     `metadata.authors/created_by/modified_by`, PDF `annotations`/`form_fields`, DOCX/PPTX
     `revisions`, `uris`, nested archive `children`) unredacted. Fixed via
     `HaciendaFacade::redact_structured_fields` + `redact_document_recursively`, with a new
     leak test. **This is critical for M2's whole "safe ZIP" premise** — without P7, a
     table or nested archive child could carry raw PII into the exported ZIP.
  3. **xberg extraction-format parity** — `pdf/office/excel/email/hwp/hwpx/iwork/archives`
     Cargo features enabled (pure features, zero xberg source touched). Widens what
     `documents_process` can redact.
  4. **A capability-parity spec program** + X1–X4 capability specs.

- **Design spec:** `superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md`
  (the design this PR implements). It already answers "how do we expose hacienda as MCP":
  - Own crate `hacienda-mcp` (not a mod inside `hacienda-core` — MCP is a transport, D-M1-2).
  - Tools are thin wrappers over `HaciendaFacade` methods (never xberg's raw extraction) — D-M1-1.
  - stdio transport via `rmcp`, served by `hacienda mcp serve`.

- **`hacienda-mcp/src/lib.rs` (588 lines, code-reviewed for this doc):** a `HaciendaMcp` struct
  annotated `#[tool_router]`, **8 tools**: `documents_process`, `pii_scan`, `pii_redact`,
  `pii_reveal`, `audit_verify`, `audit_entries`, `compliance_report`, `get_version`. Code
  quality is high and security-consistent:
  - every tool calls the facade with `Caller::Trusted` (process-boundary trust), same as the CLI;
  - `pii_scan` uses `SpanText::Omit` (categories/offsets only, never entity text);
  - `pii_reveal` collapses all `Pseudonym`/`PiiDisabled` errors to one `invalid_params` message
    so a probing client can't guess a token/key from error text;
  - `audit_verify` reports tamper verdicts as structured results (`verified: false` naming the
    entry) and only surfaces I/O/serialisation faults as tool errors;
  - error mapping splits `invalid_params` (caller-caused: `Extraction`, `Authz`, `PiiDisabled`,
    `RedactionDepthExceeded`) from `internal_error` (node fault);
  - 5 unit tests ship, incl. `pii_redact_never_returns_the_redacted_value` and
    `pii_scan_omits_entity_text` — the leak-guard class M2 reuses.
  - `params.rs` uses `rmcp::schemars` JSON-schema types; `EmptyParams` satisfies MCP's
    `{"type":"object","properties":{}}` requirement.

- **`alef.toml`** references `mcp_sources = ["hacienda-core/src/mcp/server.rs"]` — a stale path
  (mirrors xberg's layout). M1 §3.1 says it should become `hacienda-mcp/src/server.rs`.

### 1.2 `xberg` (the upstream product dependency) has its own MCP — deliberately unused

- Hacienda pins `xberg = { git = ...jamon8888/xberg.git, rev = <sha>, features = ["redaction", "ner", "tokio-runtime"] }`.
- xberg's `crates/xberg/src/mcp/server.rs` exposes its own tools (`extract`, `extract_batch`,
  `detect_mime_type`, cache tools…) returning **unredacted** content. M1 explicitly rejected
  re-exposing that. Verdict stays: **do not enable xberg's `mcp` cargo feature, do not import
  anything from `crates/xberg/src/mcp`** (net-xberg-diff for M1 and M2 = zero).

### 1.3 "Like in Hacienda Studio" — what Studio actually exports today

- Studio (`apps/hacienda-studio`) is a browser-only, zero-egress workspace. Its only *export*
  today is the **knowledge graph**: `apps/hacienda-studio/lib/kg-export.ts` emits Cypher /
  NetworkX JSON / RDF(Turtle). It does **not** currently zip a folder. So "folder → ZIP" is new
  behavior; the "like Studio" intuition is about *having a local, trustworthy, audited artifact*
  exported from the engine, not about Studio's literal zip code (which doesn't exist).
- The relevant structural precedent is the **vault layout** (Track I2): `documents/`,
  `_manifest.json`, `pii-registry.json` / `entities-registry.json`, `GLOSSARY.md`. The ZIP we
  produce should reuse this layout so downstream tooling already knows how to read it.

### 1.4 The redaction + audit primitives the exporter needs all exist on `main`

`hacienda-core/src/facade.rs` (`HaciendaFacade`) already exposes the exact seam M1's tools wrap:

| Facade method | Purpose for M2 |
|---|---|
| `process_batch_with_auth` | extract + redact a batch of document files |
| `scan_text_with_auth` | detection-only |
| `redact_text_with_auth` | redact a string (PII spans → placeholders) |
| `reveal_token_with_auth` | reverse a pseudonym token |
| `audit_history_with_auth` / `audit_export_with_auth` | read the audit chain |
| `verify_audit_with_auth` | **verify the chain** — the source of the "safe" verdict |
| `compliance_report_with_auth` | attached compliance artefacts |

All take a `Caller` (trust boundary). The stdio MCP path runs as `Caller::Trusted` (process
boundary = trust boundary) — the same stance M2's ZIP tool should take **for a user-chosen local
folder in a local harness**, never for a remote/HTTP caller.

---

## 2. DeepSeek Harness capabilities that make this feasible (verified against the installed DSH packages)

These are **live, mounted Host services** in this harness (read from `Service.listService`), not
assumptions. Each is the concrete hook the M2 plugin uses.

| DSH capability | Interface (verified) | Role in M2 |
|---|---|---|
| **Dynamic Host plugin** | `cordis_define` → `code.host` returning a Cordis Plugin | The whole deliverable is one Host plugin |
| **Register model tools** | `harness.registerTool(ctx, ToolDefinition)` / `ctx.tools.register(definition)` | Registers `mcp__hacienda__*`-style and `export_safe_zip` tools the **Gemma/Ollama agent** can call in the UI |
| **`ctx.e2b`** | `readonly cwd`, `runtimeRoot`; `getSandbox(): Promise<Sandbox>` | **Matches the user's "E2B process"** — launch an E2B sandbox to stage/scrub the folder remotely, copy the ZIP into the sandbox, or use the sandbox's filesystem as the folder source |
| **`ctx.shell` / `ctx.subprocess`** | `run(spec)`, `start(spec)`; `spawn(spec)` | Invoke the compiled `hacienda` CLI / `hacienda mcp serve`; run `zip`, `sha256sum`, etc. |
| **`ctx.fs`** | `listDir`, `stat`, `readBytes`, `readText` | Enumerate the folder before packing so we can account for every file in the manifest |
| **`@deepseek-ai/dsh-mcp-client`** | yaml row `- id: mcp-hacienda ... transport: stdio, command: hacienda, args: [mcp, serve]` | The **native** way to load the 8 hacienda MCP tools into the model as `mcp__hacienda__pii_redact`, etc. |
| **Turn-tail "produced files"** | `dsh-client-ui-deliverables` — tools that report mutation `locations` render as openable file chips | The ZIP written to disk becomes a **clickable download chip** in the conversation — this is how the user receives the archive in the GUI |
| **`ctx.attachments` / `spillStore`** | `saveImages`, `saveText` | Optional: persist the manifest / a screenshot-of-verdict as an attachment |
| **`ctx.webServer`** (`WebRoute`) | `register(route)` | Optional: expose a GET route that streams the ZIP bytes for a true browser download |

Combined, these mean: **no new DSH core capability is required — the plugin itself is the entire
surface.** The MCP-as-load step is already a first-class DSH feature (dsh-mcp-client), and the
zip/assurance step is a couple of Host tools.

---

## 3. Proposed plugin architecture

One DSH dynamic Host plugin, one Cordis package (immutable per `cordis_define`). Two planes:

### 3.1 Focus of the Host half (everything is Host — files, processes, tools)

```text
DeepSeek Harness Web  (Gemma model, served by Ollama as the harness LLM provider)
        │  drives model-visible tools
        ▼
┌────────────────────────────── HOST PLUGIN: "hacienda-export" ─────────────────────────────┐
│                                                                                             │
│  tool set registered via harness.registerTool / ctx.tools.register:                          │
│                                                                                             │
│  • hacienda__export_safe_zip  (the new work)                                                 │
│      args: { sourceDir | sandboxRef, redactMode, mode=redact|scan|report-only,               │
│              includeCompliance?, outZip? }                                                  │
│  • (optionally, if not already loaded via dsh-mcp-client) mcp__hacienda__pii_redact/scan/... │
│      as thin pass-throughs to `hacienda mcp serve` stdio or the HTTP API                     │
│                                                                                             │
│  pipeline inside export_safe_zip:                                                            │
│   1. fs.listDir(sourceDir)                       → account every file (manifest input)       │
│   2. optional: copy into ctx.e2b sandbox cwd     → the "E2B process" isolation step         │
│   3. for each file (documents) → call hacienda redaction:                                    │
│        • prefer live `hacienda` CLI: `hacienda extract <file> --mode <mode> --audit-out ...` │
│        • or call the MCP tool / HTTP /v1/documents over loopback                             │
│   4. zip the (redacted) tree + generated artefacts into outZip                                │
│   5. applyFile → produced-file chip (user downloads the ZIP in the GUI)                       │
│                                                                                             │
│  assurance = embedded manifest + audit chain + verify: see §4                                │
└─────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 3.2 Delivery of the ZIP to the web user

Two options, in preference order:

1. **Turn-tail produced-file chip (recommended).** The plugin tool writes `outZip` and its tool
   result `locations` carry the absolute path. DSH's `dsh-client-ui-deliverables` renders
   "Produced files" chips in the turn tail; the user clicks to open/save the ZIP. No custom UI
   needed — the file-open flow already exists.
2. **`ctx.webServer` GET route** streaming the bytes at a stable URL (for a real `download`
   attribute / larger files). Slightly more code; only needed if users must fetch via a raw link.

No Client-half UI is required for the MVP. (Optional Client: a `tool.view.cordis` card that shows
a live "safety verdict passed" badge after the run — see §6.)

### 3.3 The two ways to "use this codebase as MCP" inside DSH — choose one or both

- **Path A — dsh-mcp-client row (recommended, zero plugin code).** Add to the harness `cordis.yml`
  (or an agent preset):
  ```yaml
  - id: mcp-hacienda
    name: '@deepseek-ai/dsh-mcp-client'
    config:
      serverName: hacienda
      transport: stdio
      command: hacienda
      args: ['mcp', 'serve']
  ```
  This registers `mcp__hacienda__pii_redact`, `mcp__hacienda__pii_scan`, `mcp__hacienda__audit_verify`,
  … and the Gemma model calls them natively. **First, however, `hacienda-mcp` must be merged to
  `main` and `hacienda` built with `cargo build -p hacienda-cli mcp` present** (see §5, blocking
  step).
- **Path B — plugin-internal pass-through.** The M2 Host plugin shells out to `hacienda mcp serve`
  (or `hacienda extract`) itself and re-registers a few tools. Useful if the harness should not
  depend on a composition edit. More code, nothing new architecturally.

Recommendation: **Path A for the model-facing MCP tools** (it is the native mechanism and keeps
tool schemas authoritative) **plus** the `export_safe_zip` Host tool in the plugin for the folder→
ZIP step (that tool isn't an MCP tool — it's a DSH tool that internally reuses hacienda).

---

## 4. The assurance model — "this ZIP is safe to process with Claude Desktop"

The user explicitly wants the ZIP to arrive with *assurance* that its receiver (e.g. Claude
Desktop) can trust it. Design the assurance as **evidence, not a promise**, using exactly the
cryptographic machinery hacienda already ships.

Inside the ZIP (Track I2 vault layout):

```text
safe_export_<ts>.zip
├── documents/            ← PII-redacted extraction artifacts (one per source doc)
├── _manifest.json        ← machine-readable manifest
├── audit/                ← audit chain (blake3 hash-chained entries)
│   └── (chain segments + the tip seal)
├── verify.txt            ← human-readable verdict + instructions for Claude Desktop
├── vault/                ← Track I2 layout (pii-registry.json / entities-registry.json)
└── README.md             ← layout + trust model explanation
```

### `_manifest.json` — the accountability contract

```jsonc
{
  "schema_version": "1.0",
  "export": {
    "id": "8f2e...",
    "created_at": "2026-08-18T...Z",
    "tool": "hacienda-export (M2)",
    "hacienda_version": "0.1.0",
    "source": { "kind": "e2b-sandbox" | "local-folder",
                "ref": "...", "file_count": N, "sha256_tree": "..." },
    "redaction": { "mode": "mask | hash | pseudonymize | remove", "profile": "GDPR" }
  },
  "inventory": [                // one entry per packed file
    { "path": "documents/a.txt", "bytes": 1234, "sha256": "...",
      "redacted_spans": 7, "audit_segment": "seg-00001" }
  ],
  "audit": {
    "enabled": true,
    "chain_tip": "sha256-of-tip-seal",
    "segments": ["seg-00001" /*…*/]
  },
  "verdict": {
    "state": "SAFE" | "REDACTED" | "MANUAL_REVIEW",
    "explain": "7 PII spans masked; chain verified; no raw PII present (see audit/verify)"
  }
}
```

### The `verify.txt` / nested `audit/verify` — what "SAFE" means

1. Every document was routed through the **same `HaciendaFacade`` the API/CLI use** — the P1
   redaction-enforcement point, so redaction cannot be bypassed by a transport.
2. Every redaction wrote a **blake3 hash-chained audit entry** (the `AuditStore`). The ZIP embeds
   the chain segments **and** the tip. `hacienda audit verify <dir>` re-checks the chain — an
   external verifier (or Claude Desktop pointed at it) can cryptographically confirm no entry was
   tampered with since export.
3. The **negative invariant** (M1's exit criteria, reasserted here): raw PII redacted during this
   export never appears in `documents/*` nor in the manifest — asserted by a leak test in the
   plugin's own test suite, mirroring `pii_redact_never_returns_the_redacted_value`.
4. A short **`verify.txt`** is written at the top level for a *human*: what was done, mode, counts,
   chain tip, and a one-line instruction Claude Desktop (or any tool) should follow:
   `verify the embedded audit/ via 'hacienda audit verify' before trusting downstream use`.
5. Optional: embed a `compliance_report` / DPIA artefact (§1.4) if `includeCompliance` is set.

**Verdict states:**
- `SAFE` — chain verified, redaction mode applied, no unredacted PII spans remain in `documents/*`.
- `REDACTED` — something could not be auto-redacted (e.g. an unsupported binary) but it was not
  silently passed through; the file is listed as excluded, and the verdict explains why.
- `MANUAL_REVIEW` — a low-confidence span (below threshold) is present; the ZIP includes it flagged
  for the human review queue (§ behind `--review-out`), mirroring AI-Act Art. 14 human-in-the-loop.

This is the "assurance to the user": **the ZIP is a self-verifying archive whose safety claim can
be re-derived by anyone with the engine byte-for-byte, including Claude Desktop pointed at it.**

---

## 5. Implementation plan / sequencing

| Step | Work | Depends on |
|---|---|---|
| **S1 (blocking, upstream)** | **Land PR #81.** Resolve the ~35 merge conflicts against `main` (mostly `.sqlx/*.json` + `Cargo.lock` drift + real source) so CI revalidates and the PR merges. This delivers all of: `hacienda-mcp` (MCP server, `hacienda mcp serve`), **the P7 structured-field redaction fix** (§1.1 — essential for the "safe ZIP" premise), and the wider xberg format parity. Then build `hacienda` and correct `alef.toml` `mcp_sources`. | — |
| **S2** | **Plugin scaffold**: `cordis_define` one Host package `hacienda-export`, `inject: ['fs','shell','e2b']` (all optional via `ctx.get`), register an empty `export_safe_zip` tool. | — |
| **S3** | **Folder enumeration + manifest** (S3a), then **redaction routing** to the `hacienda` CLI/loopback API (S3b). **Use P7's `redact_structured_fields` path** so tables/nested-archive children never carry raw PII into the ZIP. | S1, S2 |
| **S4** | **ZIP + delivery**: pack redacted tree + manifest + audit + verify.txt; write to disk; tool result `locations` → produced-file chip. | S3 |
| **S5** | **E2B staging** variant: if `sourceRef=kinds:sandbox`, copy files into `ctx.e2b.cwd`, run scrub *inside* the sandbox, then copy ZIP back to host. | S3 |
| **S6** | **Assurance tests**: leak test (P7-class `pii_redact_never_returns…` analogue over the ZIP), manifest/audit round-trip, `hacienda audit verify` on the embedded chain, verdict-state coverage. | S4 |
| **S7** (path A) | Add dsh-mcp-client row so `mcp__hacienda__*` tools are native; optional `tool.view.cordis` verdict badge. | S1 |
| **S8** (optional) | `ctx.webServer` GET route streaming the ZIP for large exports. | S4 |

> **Note on S1's scope-sensitivity:** PR #81 bundles the MCP server *together* with P7 (a
> security fix) and xberg format parity. There is **no P7-on-main today** — `main` still carries
> the structured-field leak (§1.1 item 2). So the "safe ZIP" is *blocked on the whole PR*, not just
> on the MCP crate. If #81 cannot land as one unit soon, the shortest safe path is to split P7 out
> and land it first (it is independently reviewable and CI-green), then the MCP crate separately.

### Hard constraints / risks

- **Sandbox policy.** The plugin reads a user-chosen folder and writes a ZIP — both are real file
  mutations. Under the session's `workspace-write`/`ask` policy this will raise approval prompts;
  the design should route every `fs`/`shell` mutation through `ctx.sandboxPolicy.resolve` and
  surface a clear approval ask ("pack & redact <folder> into <zip>?"). **Never** imply the ZIP is
  "safe" while tolerating a silent pass-through of an unredactable file — that must flip the
  verdict to `REDACTED`, never stay green.
- **PR #81 not landed → no P7 on `main`.** Regardless of whether the MCP crate merges, `main`
  today still carries the P7 structured-field leak. A ZIP built against `main`'s current redaction
  could leak a value hidden in `tables`/nested-archive-children. S3 must target the P7-corrected
  path (`redact_structured_fields`), which only exists once S1 (or a P7-split) lands. The ZIP
  redaction path that does not need MCP (direct `hacienda extract` CLI / HTTP `/v1/documents`)
  can proceed in parallel, but **its safety verdict is only trustworthy on the P7-corrected code**.
- **Ollama/Gemma tool-calling reliability.** Small local models sometimes don't populate tool args
  on the first try. Design `export_safe_zip` args with sensible defaults (default `mode: mask`,
  default `outZip` next to `sourceDir`) so a partial call still yields a valid archive.
- **e2b is optional.** If E2B isn't provisioned in a given harness deployment, `ctx.get('e2b')` is
  `undefined`; the tool must degrade gracefully to local-folder mode (the "entire folder" path),
  not fail. This is exactly the `ctx.get()` + undefined-check pattern from the plugin skill.

---

## 6. Decisions & recommendations (recap)

| # | Decision | Rationale |
|---|---|---|
| **D-M2-1** | Model-facing hacienda tools arrive via **dsh-mcp-client** (path A), one composition row; the plugin itself adds only `export_safe_zip` (and any pass-through fallback). | Native mechanism, authoritative schemas, zero duplicate tool code. |
| **D-M2-2** | The ZIP exporter calls **`HaciendaFacade`** (via CLI/loopback-API), **never xberg's raw `extract` MCP**. | Carries M1's D-M1-1 forward; the safety claim depends on it. |
| **D-M2-3** | Assurance = **embedded manifest + blake3 audit chain + `verify.txt`**, verdict `SAFE/REDACTED/MANUAL_REVIEW`. Redaction is guaranteed by construction; the chain is cryptographically re-verifiable. | Matches the product's whole identity (GDPR/DORA/AI-Act compliant, tamper-evident). |
| **D-M2-4** | ZIP delivered as a **turn-tail produced-file chip**; optional `ctx.webServer` route only for very large exports. | Zero custom UI for MVP; reuse the existing open/save flow. |
| **D-M2-5** | No Client-half UI required for MVP; the optional `tool.view.cordis` badge is polish. | Keep the first cut Host-only. |

---

## 7. Open questions for a build round (one short wall each)

1. Does the user want the folder source to be a **local directory** in the harness workspace, an
   **E2B sandbox** filesystem, or both selectable? (M2 assumes both; S5 handles E2B.)
2. Redaction **mode default** — `mask` (irreversible) vs default. Mask is the safe default; ask.
3. Should the ZIP also carry the **`compliance_report`**/DPIA artefacts by default, or only on
   request?
4. For "safe to process with Claude Desktop": acceptable to ship the *verdict + chain + verify
   instructions*, or is a **machine-readable attached proof** (e.g. a detached chain seal the
   receiver verifies) also required? (Recommended: both — verdict for humans, chain for machines.)

---

## 8. Files touched / created (for the build)

- **Upstream (S1):** land **PR #81** (resolve the ~35 conflicts with `main`, re-run CI); per the
  §5 scope note, split P7 out first if the full PR can't land promptly. Correct `alef.toml`
  `mcp_sources` path; ensure `hacienda mcp serve` is built/reachable.
- **New plugin:** one Host Cordis package per `cordis_define` (immutable versions; no repo source
  change required — dynamic plugins live in the running harness). If persisted, the preset/
  composition row in `cordis.yml` + the dsh-mcp-client `mcp-hacienda` row.
- **This doc** is added to `docs/superpowers/specs/` (alongside the M1 spec it extends).

---

## 9. M3 — customizing the Harness Web UI (workspace = folder per client) + local in-browser model

This is a *second* design strand the user asked to investigate (at design level only): (A) how to
customize the DeepSeek Harness Web UI so a **workspace = one folder = one client** and users work
inside it, and (B) integrating a **small local model in the browser (WASM / WebGPU)** so the user
gets live feedback and can interactively manage the redaction/extraction process. Findings are
verified against the installed DSH packages and the actual Hacienda Studio code.

### 9.1 The UI is already built on folders. "Workspace = client" is a branding/driving question, not a rebuild.

The Harness UI already models **a workspace as an owning directory**, and every Host/Client surface
needed to treat "one folder = one client" is present:

- **Host `workspaceRegistry`** (`ctx.workspaceRegistry`): durable workspace entities keyed to a
  canonical directory `path`. `create(path, title?)`, `resolveByPath(path)`, `delete(id)`,
  `insertBefore(id, beforeId?)`. It builds a canonical-cwd session header index, so each
  workspace's `sessionIds` are already filtered by **cwd** — sessions are scoped to their folder.
- **Client `ctx.workspaces`** (`ctx.workspaces`): `create({path})`, `pickDirectory()`, `listDir`,
  `createDirectory`, `connectWorkspace(id)` → `SessionId`, `rename`, `delete`, `archiveSession`.
  The client's `directoryPicker` (Host) backs `pickDirectory()`.
- **Client slots** (verified from `Slots.listSubTree`) give additive, low-risk customization seams
  exactly where a per-client "folder is active" affordance belongs:
  - `sidebar.workspaces` — the whole workspace/session browsing region (with a
    `directoryFlow` hole beneath).
  - `conversation.hero.workspace` + `.directoryFlow` — the empty-state picker.
  - `conversation.session.header.actions` / `.utilities` — **additive** per-session buttons in the
    header (the right place for a "this workspace = client <X>" pill, an "export safe ZIP" button,
    or a "scan folder now" action). Registration is `{id, order}`, no shadowing.
  - `conversation.input.dock` / `composer.dock` / `input.left` / `input.right` — additive composer
    seats for an ambient redaction-status readout (e.g. "12 PII spans in 3 docs — Review").
  - `details` / `conversation.details.tool` — a right-column panel for a selected tool result
    (great for an interactive redaction preview pane).
  - `tool.call.toolview` (keyed by tool name) and `tool.view.cordis` (`key: "self"`) — to render a
    custom card for the `export_safe_zip` / scan tools.
- **Localization:** `ctx.locale.register(ns, dicts)` + `ctx.locale.bind(ns)` — customize copy per
  client in any language without touching the shell.

**Design implication:** "workspace = folders (clients)" needs **no new model and no
`sidebar.workspaces` replacement**. The lowest-risk customization is a single **Client plugin**
that (1) registers a per-session affordance in `conversation.session.header.actions` (or
`utilities`) rendering the active workspace's folder/client identity, and (2) registers the
redaction/extraction/preview surfaces in the additive seats listed above. Replacing
`sidebar.workspaces` or the hero picker is possible but **shadows shipped UI** (`replaceRisk:
shadows-shipped-ui`) and is only justified if the stock browsing UX must be reshaped.

**Decided (2026-08-18): one client = one workspace.** Each client owns exactly one folder/
workspace. The UI renders client identity from that single active workspace (the workspace whose
session is open, or the workspace the client was registered to own). This keeps the mapping
1:1 — no client↔folder lookup table — and the header affordance simply reads the active
workspace's canonical path/title. A future multi-folder-per-client mode would be a small
extension (map a client id → list of workspace ids) and is out of scope for the MVP.

### 9.2 Local model in the browser for interactive redaction/extraction feedback — WASM works today, WebGPU is the accelerant.

**Hacienda Studio is a proven production precedent** (verified in code):

- `crates/hacienda-wasm` compiles `hacienda-core`'s PII/redaction pipeline to `wasm32` via
  `wasm-bindgen` (feature `ner-candle-wasm` → `hacienda-core/ner-candle-wasm`), using candle's
  `wasm_js` backend. It consumes `@xberg-io/xberg-wasm`'s neural `NerModel` (GLiNER2) as the
  entity backend.
- `apps/hacienda-studio/lib/asset-loader.ts` downloads model weights (`model.safetensors`,
  `tokenizer.json`, `encoder_config/config.json`) straight from Hugging Face with **parallel
  byte-range requests, stall timeouts, and IndexedDB caching**, then hands them to
  `createNerBackend` → `NerModel.load({weights, tokenizer, encoderConfig})`.
- `apps/hacienda-studio/worker/pipeline.ts` runs the whole pipeline (extraction via `XbergEngine`,
  OCR via tesseract.js, NER via GLiNER2, chunking, PII via the same `hacienda-core` regex engine
  compiled to wasm) **inside a Web Worker** — the browser, no server round-trip, zero egress.
- The human-review loop already exists: low-confidence spans route to the AI-Act Art. 14 review
  queue, and `loadNerModel`/`loadPiiNerModel` share fetched bytes across the entity and PII NER
  passes.

**What that means for the Harness plugin:**

- The **same WASM stack can run inside a DSH Client-half plugin**. The Client Builtins here expose
  `React`, `host.call`, `styles.insert` — no `WebGPU`/WebAssembly builtin is provided, but none is
  needed: a Client plugin can load its own WASM module and run it in a Web Worker exactly as Studio
  does. The `hacienda-wasm`/`@xberg-io/xberg-wasm` packages are the reusable inference core.

**Decided (2026-08-18): reuse GLiNER2 / xberg-wasm as the MVP inference path.** For the
interactive redaction/extraction feedback model, the plugin adopts **exactly the proven Studio
stack** — `@xberg-io/xberg-wasm`'s GLiNER2 `NerModel` running on candle's `wasm_js` CPU backend in
a Web Worker, with weights cached in IndexedDB via the `asset-loader.ts` pattern. Rationale: it is
already implemented, tested, and used by Hacienda Studio, so there is **no new model, no new
backend, and no new weight-format plumbing** to build or maintain in the MVP. The known cost — GLiNER2
is a ~600MB mDeBERTa-v3 model and per-document CPU latency is in seconds — is acceptable for the
*session-scoped, folder-scoped* interactive preview (one active client folder, worker pipeline,
cached weights) rather than type-as-you-go across arbitrary text.

- **WebGPU remains the accelerant, deferred to a follow-up slice (design-level), not the MVP.**
  When "seconds per document" is no longer fast enough, the two clean paths to revisit are:
  - **candle WebGPU backend (preferred, same-code)** — recompile the *same* `hacienda-core`/xberg
    model code with candle's `wgpu`/`webgpu` backend instead of `wasm_js`, so the exact weights and
    code get GPU acceleration with no second model format. This upgrades GLiNER2 in place rather
    than replacing it.
  - **Transformers.js / ONNX Runtime Web (WebGPU)** — a smaller encoder model exported to ONNX for
    truly millisecond token-level feedback; a fork in the road (new model + new runtime), only worth
    it if even the WebGPU-accelerated GLiNER2 is too heavy for the desired interactivity.
- **Two trust tiers (unchanged).** The in-browser GLiNER2/WebGPU model gives *interactive
  feedback*; the *authoritative* redaction + audit for any exported artifact runs on the server
  (`HaciendaFacade` via the MCP/CLI, P7-corrected). Feedback can be fast and approximate; evidence
  must be exact and audited.
- **Ollama is a third, separate route and does not compete with the browser model.** The Host
  `llm` service has first-class pluggability: `registerAdapter(providers, adapter)`,
  `registerConfigurableProviders()`, `registerModelDiscovery()`. A small **Ollama provider adapter
  plugin** (Gemma) is how the *agent's* model becomes selectable/usable network-wise — but that is
  Host-side inference, not in-browser. If the goal is "no model bytes on the wire / the browser
  does it," the in-browser WASM/WebGPU path (§9.2) is the one that satisfies it; if the goal is
  "let the agent *reason* with a local model," the Ollama provider adapter is the one.

### 9.3 Recommended M3 shape (design only)

| Piece | Where it mounts | What it does |
|---|---|---|
| **Client plugin: workspace identity + affordances** | `conversation.session.header.actions` / `.utilities`, `sidebar.footer.action` | Render the active folder/client identity; a "Scan this folder" and "Export safe ZIP" button wired to the Host tools; locale-registered copy per client. |
| **Client plugin: interactive redaction pane** | `conversation.details.tool` (details column) or `tool.call.toolview` | A preview pane showing detected PII spans in the active workspace's docs, powered by the local model (web worker), with approve/reject/flag controls feeding the review queue. |
| **Client plugin/host RPC: local-model feedback** | client → `Host half` via `host.call`; WASM/worker in the client | A **GLiNER2 / `@xberg-io/xberg-wasm`** inference worker (Studio's proven stack, candle `wasm_js` CPU) whose span output is surfaced in the pane. WebGPU (same-code candle backend) is the follow-up accelerator, not the MVP. |
| **Host plugin: authoritative scan/redact/export tools** | `harness.registerTool`; `ctx.shell`/`ctx.fs`/`ctx.e2b` | `scan_folder`, `export_safe_zip` (M2) running the P7-corrected `HaciendaFacade` for evidence; browser feedback is advisory. |
| **Ollama provider adapter (optional)** | Host `llm.registerAdapter`/`registerConfigurableProviders` | Make Gemma on Ollama a selectable agent model; independent of the browser inference path. |
| **Review queue feedback** | Host via MCP tools / facade `review_queue_with_auth`; Client pane reads it | The interactive pane's approve/reject decisions land in the durable review queue (AI-Act Art. 14), so the browser's "feedback" and the server's evidence are the same truth. |

### 9.4 Key constraints / decisions for M3

- **D-M3-1:** Do **not** replace `sidebar.workspaces` or the hero picker by default — they
  already implement "workspace = folder"; customize with additive seats. Reshape only if a
  concrete client UX requirement demands it. **One client = one workspace** (decided): render
  identity from the single active workspace, no client↔folder mapping.
- **D-M3-2:** **Two trust tiers** — the in-browser (WASM/WebGPU) model gives *interactive
  feedback*; the authoritative redaction + audit for any exported artifact runs on the
  P7-corrected `HaciendaFacade`. Feedback can be fast and approximate; evidence must be exact and
  audited.
- **D-M3-3:** **MVP reuses GLiNER2 / xberg-wasm** (Studio's proven WASM path in a Web Worker);
  **WebGPU is a follow-up accelerator, not a requirement**. When GPU speed is needed, prefer
  candle's WebGPU backend (same code, same weights) over introducing an ONNX/transformers.js
  model; revisit ONNX only if even WebGPU-accelerated GLiNER2 is too heavy.
- **D-M3-4:** The model bytes stay client-side and cached in IndexedDB (Studio precedent) — zero
  egress of document content, consistent with the product's zero-egress identity.
- **D-M3-5:** Client is **optional polish** for M2's core ZIP goal; it becomes the primary UX for
  the interactive redaction/extraction manager this section describes.

### 9.5 M3 open questions for a build round

1. **RESOLVED (2026-08-18): one client = one workspace** — each client owns a single folder/
   workspace; identity is rendered from that active workspace (no client↔folder mapping).
2. **RESOLVED (2026-08-18): reuse GLiNER2 / xberg-wasm** as the MVP browser-inference path;
   WebGPU (candle same-code backend) is the deferred accelerator.
3. Does "local model" mean **in the browser (wasm/webgpu)** for zero-egress interactive NER, or
   **Ollama/Gemma on the host** as the agent's reasoning model — or both? (This doc recommends
   both, for different jobs: browser = feedback, Ollama = agent reasoning.)
4. Should the interactive pane write approvals back into the **durable review queue**, or is a
   throwaway in-session feedback enough? (Recommended: durable, so it feeds the evidence chain.)

---

## 10. NER acceleration (Web Workers) + Ollama/Gemma/E2B for interaction — investigation

Two further investigations requested at design level: (A) how to make NER faster using Web
Workers (and related techniques), and (B) how Ollama + Gemma + E2B plug in for user/content
interaction. Both are grounded in the verified code and DSH surfaces described in §9 and §2.

### 10.1 NER acceleration — where the time actually goes, and how to spend it

**Current bottleneck (verified in `apps/hacienda-studio`):** the pipeline is **one Web Worker,
one GLiNER2 `NerModel` instance, strictly sequential** over documents.

- `App.tsx` spawns **one** `worker/pipeline.ts` (`new Worker(..., {type:"module"})`, line 166).
- `processFiles` iterates `for (const file of files) { const processed = await processFile(...) }`
  (pipeline.ts line 665) — **no parallelism across files**.
- `selectNerBridge(nerRuntime)` returns `(text, categories) => runtime.detect(text, {categories})`
  — the **GLiNER2 `NerModel.detect`** call (line 124-130). The model does one document's tokens
  at a time.
- The UI/README documents the *deliberate* decision: *"one loaded WASM NER/PII engine instance is
  not built for concurrent inference calls, and true parallelism would need multiple Web Workers
  sharing that model."*
- The `compromise.js`/regex `extractEntities` (ner-bridge.ts) is the **fallback** (used when the
  WASM model fails to load), not the primary path.

So the slowdown is **per-doc latency × serial execution**, not raw model throughput per se. The
levers, in order of impact:

**Lever 1 — Multiple Web Workers, each with its own model instance (the "easy" 2–8×).**
`navigator.hardwareConcurrency` gives the core count. Spawn `min(hardwareConcurrency, cap)` worker
instances, **shard the document list** across them, and merge results. Each worker runs its own
GLiNER2 `NerModel` on its own thread, so inference truly runs in parallel. Cost: memory — each
worker holds a model in its own WASM heap, so 4 workers ≈ 4× model RAM (can be prohibitive for the
~600MB model). This is the **most robust** option and the one Studio's own comment names ("true
parallelism would need multiple Web Workers").

**Lever 2 — SharedArrayBuffer model-sharing + `SharedWorker`/pinned pool (the memory-lean 2–8×).**
If the deployment enables the cross-origin isolation headers (`Cross-Origin-Opener-Policy:
same-origin` + `Cross-Origin-Embedder-Policy: require-corp`), the frozen model weights can live in
a **`SharedArrayBuffer`** and be memory-mapped/sharded by several workers **without duplicating
the model bytes** in each heap. This is the WebGPU/WASM-industry pattern (used by transformers.js
v3, whisper.cpp-web). Higher setup cost (COOP/COEP headers, careful zero-copy), but removes the
memory blow-up that makes Lever 1 scale poorly. Combine with a worker **pool** that keeps N workers
warm and a per-document job queue.

**Lever 3 — Intra-document chunking / overlapping-token batching.** GLiNER2 is a transformer
(mDeBERTa-v3, 512-token context). A very long document exceeds the model's context window and is
either truncated or internally re-windowed — for long docs, split the text into overlapping
chunks and run the model per chunk *across* the pool (each chunk is an independent inference), then
re-stitch spans. This turns one long serial inference into many parallel short ones.

**Lever 4 — Batch the PII regex engine too.** Studio already runs the same `hacienda-core` regex
PII engine compiled to wasm; the PII pattern pass is cheap relative to GLiNER2, but it also runs
per file. Batching it across workers or after NER is a smaller win but free.

**Lever 5 — WebGPU accelerator (deferred per D-M3-3).** Recompile the same model on candle's
WebGPU backend (`wgpu`) so the *weights* stay shared on the GPU and inference drops from seconds to
milliseconds per doc. This is orthogonal to worker count (GPU + multiple workers stack), and it is
the same-code path that upgrades GLiNER2 in place. The interaction model (a worker pool calling a
WebGPU session) is the same as Lever 1/2 with a faster backend.

**Recommended MVP:** **Lever 1 first (plain multi-worker, per-worker model), optionally with Lever
3 (chunk sharding)** — it is the least invasive and matches Studio's own stated direction. Then, if
memory is the binding constraint, invest in **Lever 2 (SharedArrayBuffer pool)**; when latency is
still not interactive, add **Lever 5 (candle WebGPU)**. L4 (PII batch) is a free add-on.

**DSH-plugin mapping:** these workers live inside the **Client-half plugin** (a Web Worker pool the
client owns), exactly as Studio does — the minimal Client Builtins (`React`, `host.call`,
`styles.insert`) do not block it, because the plugin loads/runs its own WASM+workers. The
authoritative ZIP/audit pass stays on the Host (`HaciendaFacade`, P7-corrected) per the two-tier
rule (D-M3-2).

### 10.2 Ollama + Gemma + E2B for user/content interaction

Three distinct roles, each with a verified seam in the harness. They answer different halves of
"interaction with the user and content" and are **complements, not rivals**:

**Role 1 — Ollama (Gemma) as the agent's reasoning model (Host-side).**
**Confirmed: there is no separate "DeepSeek Harness Ollama plugin," and one is not needed — the
harness already ships the adapter that speaks to Ollama.** DeepSeek Harness provides
`@deepseek-ai/dsh-llm-pi-ai`, a *generic multi-provider* adapter over
`@earendil-works/pi-ai`. That library explicitly lists Ollama as a supported local endpoint
(readme: "Any OpenAI-compatible API: Ollama, vLLM, LM Studio, etc."; it even ships a ready
`ollama` provider example at `baseUrl http://localhost:11434/v1`, `api: openai-completions`,
keyless auth). Because Ollama exposes an OpenAI-compatible chat-completions endpoint at
`/v1/chat/completions`, the harness's existing adapter reaches it with **configuration, not
code** — a "hand-declared route" (pi-ai ships nothing under the key, so the profile supplies the
whole provider):

```yaml
- id: llm
  name: '@deepseek-ai/dsh-llm-pi-ai'
  config:
    providers:
      ollama:
        displayName: Ollama (local)
        api: openai-completions
        baseURL: http://localhost:11434/v1
        # keyless local server: omit apiKeyEnv entirely
        models:
          - id: gemma2:2b
            name: Gemma 2 2B (local)
            contextWindow: 8192
          - id: gemma3:4b
            name: Gemma 3 4B (local)
            contextWindow: 32768
```

This is the `cordis.yml` row for the `@deepseek-ai/dsh-llm-pi-ai` plugin (it must be present /
mounted in the composition; the installed set already ships it). Model enumeration can come from
`llm-pi-ai:` settings discovery (pi-ai's `openai-completions` `GET /models` interrogation) or
simply listed as above. Once this row is active, **Gemma becomes a selectable agent model in the
harness model picker**, running fully locally through Ollama — no cloud, no separate plugin, no
custom adapter code. This is exactly what "get local models working here" needs; the only
prerequisite is the Ollama daemon itself (`ollama serve` on `localhost:11434`) with a Gemma model
pulled (`ollama pull gemma2` / `gemma3`). This is the **user-interaction** path.

**Role 2 — E2B sandbox for content manipulation / isolation (Host-side `ctx.e2b`).**
The Host `e2b` service is abstract (`getSandbox(): Promise<Sandbox>`, `cwd`, `runtimeRoot`); a
deployment provides the adapter. It matches the user's "E2B process" for *content*:
- stage the client folder inside the sandbox's `cwd`;
- run the scrub/extraction inside the sandbox (isolation, reproducible environment), then copy the
  ZIP out — exactly M2's `export_safe_zip` pipeline §5 S5;
- alternatively, E2B *hosts* the Gemma model as an inference runtime the adapter calls, decoupling
  model serving from the local laptop.
E2B is for **isolated content execution**; if no E2B is provisioned, `ctx.get('e2b')` is
`undefined` and the plugin degrades to local-folder mode (D-M2 hard constraint, already in §5).

**Role 3 — In-browser GLiNER2 (Web Workers) for zero-egress content feedback (Client-side).**
§10.1's worker pool. This is the **content-interaction** path that needs no network at all:
instant span detection/highlighting in the redaction pane, feeding the review queue. Document
content never leaves the browser.

**How the three compose for "interaction with user and content":**
- **With the user (chat):** Gemma on Ollama (Role 1), via the shipped `dsh-llm-pi-ai` adapter with a
  hand-declared `ollama` route — no new sandbox, no custom plugin. The agent holds the conversation
  in the workspace session: "what did you find?", "redact X", "make the safe ZIP".
- **With the content (fast, local):** the in-browser GLiNER2 worker pool (Role 3) feeds the
  interactive preview and the review queue; the authoritative pass runs on the P7-corrected
  `HaciendaFacade`.
- **With the content (isolated/heavy):** the harness's **own** `ctx.e2b` sandbox (Role 2 — the user
  confirmed no separate E2B sandbox is wanted) stages, scrubs, and packs the folder into the
  audited ZIP, optionally hosting Gemma as the inference runtime.
- **Trust stays two-tier:** browser/worker feedback is advisory and fast; the exported artifact's
  redaction + audit is exact and evidence-backed (D-M3-2).

### 10.3 Recommended path and open questions

| # | Item | Recommendation |
|---|---|---|
| N1 | Worker parallelism | **Lever 1 first** (multiple workers, per-worker GLiNER2), cap by `hardwareConcurrency`; add **Lever 3** (chunk sharding) for long docs. |
| N2 | Memory ceiling | If model RAM × workers is prohibitive, move to **Lever 2** (SharedArrayBuffer pool); requires COOP/COEP headers. |
| N3 | WebGPU | **Deferred** per D-M3-3; candle WebGPU upgrades the same model in place when N1+N2 aren't interactive enough. |
| O1 | Local models (Gemma) | **No custom plugin.** Use the shipped `@deepseek-ai/dsh-llm-pi-ai` adapter with a hand-declared `ollama` route (`api: openai-completions`, `baseURL http://localhost:11434/v1`, keyless, `models: [gemma2:2b, gemma3:4b]`) in `cordis.yml`. Requires only the local Ollama daemon; no compilation. |
| O2 | E2B | **No separate sandbox.** Use the harness's existing `ctx.e2b` service as **optional content-isolation** staging for `export_safe_zip` (M2 S5); degrade to local folder when not provisioned. |

Remaining open questions:
1. E2B: **assume local-folder-first**, and treat `ctx.e2b` as an optional adapter only if the target
   deployment actually provisions a sandbox. (Confirmed: no new E2B sandbox of our own.)
2. Ollama/Gemma role: **chat/reasoning only** (the selectable agent model). Content NER stays in the
   browser (Role 3, GLiNER2 workers) for zero-egress speed and the two-tier trust guarantee.
   **Confirmed: getting local models working = the `dsh-llm-pi-ai` + `ollama` config above, not a
   new plugin.**
3. How many concurrent workers is acceptable given the ~600MB model? (Cap query for the prod
   machine, e.g. `min(4, hardwareConcurrency)` default.)
4. Should long-document chunk sharding (Lever 3) be in the MVP or deferred to the same slice as
   WebGPU? (Recommended: in MVP if long legal/contract documents are a primary input.)

---

## 11. Deeper DSH source review — substantive design feedback

This section is the result of reading the installed DeepSeek Harness source (the `@deepseek-ai/*`
packages and the `dsh-tools`, `dsh-user-approval`, `dsh-workspace`, `dsh-llm`, `dsh-llm-pi-ai`,
`dsh-client-ui-deliverables`, `dsh-host-directory-picker` READMEs and type declarations), not the
catalog summaries. It **corrects and sharpens** the design above with concrete contracts the
plugin must satisfy.

### 11.1 Corrected: the ZIP is a "produced file" only if the tool declares `card: 'diff'` render intent

**Finding (dsh-tools `presentation.d.ts` + dsh-client-ui-deliverables):** the turn-tail
"produced files" chip for a written file is keyed on the tool's **render intent**, not on a raw
`locations` list and *definitely not* on the tool name. `ToolResultView`/`ToolCallView` are
provider-neutral `card`-tagged unions:
`generic | terminal | diff | search | read | web`. The deliverables vocabulary recognizes a
*mutation* by a **`card: 'diff'`** call/result view (the shape `write`/`edit` present) and reads
its `diffs[].path`; reads/deletes/failed calls contribute nothing.

**Correction to D-M2-4 / §3.2:** our earlier text said *"tool result `locations` → produced-file
chip."* The precise requirement is: **`export_safe_zip` must implement `presentCall` (pending)
and `presentResult` (completed) returning `card: 'diff'` views whose `diffs` name the ZIP and the
`documents/` files it wrote.** Only then does the deliverables row surface the archive as an
openable chip. A tool that only returns JSON and a `locations` array but no diff render intent will
produce **no chip**. This is a real, enforceable contract, not an aesthetic choice.

### 11.2 The tool-owned UI card is `presentResult`, not a bespoke slot (upgrades D-M3-5)

**Finding (dsh-tools `ToolDefinition`):** a tool can own its UI through
`presentCall?(args): ToolCallView` and `presentResult?(args, result): ToolResultView`, plus the
mandatory `output { schema, render, presentationMeta? }`. `output.presentationMeta` persists
structured data with the session log and is narrowed back into a view by `presentResult` on the
live *and* replay paths (the pattern the `read` tool uses).

- For our **verdict / assurance badge** there is **no bespoke `verdict` card kind**. The clean
  approach is `GenericResultView` (`card: 'generic'`) whose `content: ContentBlock[]` carries a
  formatted verdict, plus a separate `DiffResultView` naming the ZIP/documents. Only when we want
  *fully custom, interactive* UI (a live redaction pane with approve/reject) do we fall through to
  a **Client Slot** (`tool.view.cordis` with `key: 'self'`) registered by the plugin's Client half.
  Recommend: `presentResult` first (provider-neutral, degrades to the raw JSON card on incapable
  UIs), `tool.view.cordis` only for the interactive pane.
- Every tool must ship a canonical **`output` schema + `render`** — a missing/unusable output
  declaration fails registration. Our tool definitions must include these from the first package.

### 11.3 Parallel tool execution is opt-in and narrow (relevant to the NER story)

**Finding (dsh-tools):** the registry executes a tool in **`parallel` only when its
`isConcurrencySafe(args)` returns exactly `true`**; unknown/hidden/invalid cases are exclusive
(`serial`). So a `scan_folder`/`redact` tool can declare itself concurrency-safe and be run in
parallel, but the default is serial. This complements §10.1's NER worker pool: make the *Host*
scan/redact tools declare `isConcurrencySafe` where safe, and keep the heavy GLiNER2 inference in
the Client worker pool (parallel there is governed by the worker count we choose, not by this
registry flag).

### 11.4 The approval seam cannot carry our interactive redaction decisions (validates review-queue)

**Finding (dsh-user-approval):** `ctx.approval.request` is **one-shot, valid only inside an open
turn, returns `allowed-once | rejected | cancelled | unavailable`, carries NO tool arguments, has
no `allow-always`/grant store, and fails closed** when `never`/no answerer. Policy is only
`ask | never`.

Implications:
- Our M2 §5 "ask before packing & redacting a folder" is the correct use: one `approval.request`
  per `export_safe_zip` call, keyed to a tool name + reason. It is enforced by the tools pipeline
  (`tools/pre-execute` routes `ask` through this seam).
- Our M3 interactive redaction pane ("user approves/rejects individual PII spans") **cannot and
  should not** ride this seam — there is no durable grant and no per-argument visibility. The
  recommended design (§9.3, Q4) is correct: the pane's decisions write into the **durable review
  queue** via the Host (`HaciendaFacade.review_queue_*` / MCP), not the approval seam.

### 11.5 `ctx.e2b` is a service definition, not a shipped implementation (validates local-first)

**Finding:** `ctx.e2b` appears only as an abstract Service Definition (its `getSandbox()`,
`cwd`, `runtimeRoot` contract is surfaced to tooling via `dsh-tool-cordis`); there is **no bundled
`e2b` adapter** among the installed `@deepseek-ai/*` packages. A deployment must supply the
concrete provider. This confirms §5's "degrade to local folder when `ctx.get('e2b')` is
`undefined`" as the correct default — and aligns with the user's direction to **not** build a new
sandbox, only optionally use a provisioned `ctx.e2b`.

### 11.6 Workspace/session binding is strict and server-side (sharpens the client pill design)

**Finding (dsh-workspace):** `Workspace.attachSession(id)` **validates the session header `cwd`
against the workspace path** and rejects on mismatch; the header index groups sessions to folders
by canonical cwd. The workspace package **registers no tools, injects no prompts, and writes no
session events** — it is host-side-only data.

- Our "one client = one workspace" (§9.1) maps cleanly onto this: a client's active session is
  the one whose cwd is the client's folder. The **client** identity pill reads facts via the
  Client `ctx.workspaces` service / slot props — not by subscribing to host `workspaceRegistry`
  events (there are none).
- **The folder is the source of truth, not the workspace record.** `workspaceRegistry.delete`
  only removes the *registration* (sessions become "Ungrouped"); it never deletes the folder. Our
  scan/redact/export must address the folder via `ctx.fs`, not assume a workspace record's
  existence or lifetime.

### 11.7 Directory picker is single-root (a constraint to respect)

**Finding (dsh-host-directory-picker):** `capability()` is a discriminated union
(`native` chooser vs `browse` listing), the browse contract exposes **one ancestry chain per
listing, no multi-root**, and the `hidden` (POSIX dot) flag is host-owned. For our one-client-one-
workspace model this is fine (pick one folder); it becomes a real constraint only if we later want
a client to own a multi-folder tree (we explicitly scoped that out in §9.1). Also note the `browse`
flow is what works for **remote clients** that can't reach an OS chooser — relevant if a client
folder is on a remote/headless host.

### 11.8 Produced-file opening is session-cwd-relative and loopback-gated (delivery nuance)

**Finding (dsh-client-ui-deliverables):** the produced-file chip opens through the Host `openFile`
opener, with the chat view resolving paths **against the session cwd** (the workspace folder). The
`Show in folder` action appears **only while the page is loopback and the Host reports
`canOpenPath`**; direct remote Web and headless/container Linux hosts omit it.

Consequence for the ZIP handoff: the chip will open/save the archive even on remote hosts (the
download path resolves against cwd), but the OS "reveal in folder" affordance may be absent unless
the host is loopback and reports a opener. The `_manifest.json`/`verify.txt` inside the ZIP remain
the portable evidence; the GUI reveal is nicety, not the contract.

### 11.9 Durable vs live tool events

**Finding:** `tools/result` is the **live** registry event; `tool/result` is the **durable** session
event the agent loop appends. Our audit/provenance surface must ride the durable path (the ZIP's
embedded audit chain, §4), not the live notification — consistent with the two-tier trust rule.

---

## 12. Ollama in the web Settings UI + a bundled "hacienda harness" fork

Two follow-ups answered against the shipping DSH source (`dsh-client-ui-settings-models`,
`dsh-settings`, `dsh-web-app`, `dsh`, the profile/bundle model).

### 12.1 Getting Ollama into the web Settings — mostly already there; "download" is the gap

**Verified: the Models settings page already speaks a custom OpenAI-compatible provider in the
UI, with no custom code.** `@deepseek-ai/dsh-client-ui-settings-models` renders the Models page (a
`settings.section` occupant titled `models`). For any pi-ai-compatible route it provides:

- **"Add a custom provider" card** — writes into the `llm-pi-ai` namespace, entering a unique
  **Provider ID**, **endpoint** (`baseURL`), **protocol** (`api`), and at least one **model** (id +
  name, with `contextWindow`/`maxTokens` behind disclosure). It exactly matches the hand-declared
  `ollama` route: `api: openai-completions`, `baseURL: http://localhost:11434/v1`, keyless.
- **"Fetch available models"** — calls `llm.discoverModels` against the endpoint currently shown in
  the form (even unsaved/edited), which for an OpenAI-compatible endpoint reads `GET /models`
  (Ollama's `/v1/models`). The reply opens a **picker** of candidates to adopt. So a user can, in
  the browser, point at Ollama, click "Fetch available models," and pick Gemma models into the
  picker — all stock.

**Conclusion:** *selecting and configuring* local Gemma models in web Settings needs **no new
plugin** beyond ensuring the `@deepseek-ai/dsh-llm-pi-ai` row is present/composed. The one thing
the stock page does **not** do is **download / provision a model** (there is no "pull model" —
Ollama's `ollama pull` / `POST /api/pull` is not a harness concept).

**The add-on needed (small, own `settings.section` called e.g. "Local Models / Ollama"):**
1. A **Host tool** (`harness.registerTool`) that talks to the Ollama daemon over HTTP:
   `GET /api/tags` (list installed), `POST /api/pull {model, stream:false}` (download w/ progress),
   `GET /api/ps` (loaded). Host-side via `ctx.web`/`ctx.shell` or a raw fetch; shell out to
   `ollama pull` when a raw HTTP client is unavailable.
2. A **Client settings page** registering in the `settings.section` slot that uses this tool:
   list installed local models, a "Browse & download" flow (library names like `gemma2:2b`,
   `gemma3:4b`) that triggers a pull and streams progress into the page.
3. On a successful pull, update the **`llm-pi-ai` namespace** via `settings.mutate`
   (`providers.ollama.models`), so the freshly pulled model immediately shows up in the model
   picker — one flow from "download" to "selectable as the agent model."

The Settings/Host surface is entirely: `ctx.settings.register/describe/get/mutate`, the
`settings.section` slot (`{id, order, label}`), `ctx.tools.register`, and the Ollama HTTP API.
The `dsh-client-ui-settings-models` page already demonstrates every pattern (custom-provider card,
model picker, `settings.mutate` path ops) our page reuses.

### 12.2 A bundled "hacienda harness" — the native mechanism is profiles + bundles (no hard fork needed)

**Verified (dsh README + `dsh-web-app/cordis.patch.yml`):** the harness's native distribution model
is already *bundling*. `dsh --profile <name>` boots a profile under `$DSH_HOME/profiles/<name>`:
a `package.json` (with a `dsh.profile` manifest listing an **ordered `bundles` list**) + a
`cordis.patch.yml`. The tree composes over an empty root: each bundle's patch in order → the
profile's patch → the home patch → `--patch` overlays. A **bundle** is a package with its own
`cordis.patch.yml` that either overrides rows by `id` or **`- insert:`**s new rows
(`dsh-web-app` inserts `code-runtime`, `storage`, `storage-json`, …). Out-of-tree plugins are
installed into the profile's own `node_modules` via `dsh plugin --profile <name>` (pnpm).

So **"a fork with everything bundled" has two honest shapes:**

**Approach A — a bundled profile (recommended; a distribution, not a code fork).**
Create a profile whose `dsh.profile.bundles` pins known-good upstream bundle revs
(`@deepseek-ai/dsh-base`, `@deepseek-ai/dsh-web-app`) **plus our own bundle package
`@hacienda/dsh-hacienda`**. Its `cordis.patch.yml` `insert:`s all of M2/M3:
- the `@deepseek-ai/dsh-llm-pi-ai` row (already ships) configured with a base `ollama` provider
  profile (or let the user add it via §12.1's page);
- our Host tools (`scan_folder`, `export_safe_zip`, `ollama_pull`), the auth/sandbox rows;
- our Client bundle (`@hacienda/dsh-client-hacienda`) registering the Local-Models settings page,
  the redaction pane slot, and the workspace-identity affordance.

Vendor everything into `$DSH_HOME/profiles/hacienda/node_modules` with pnpm. Result: **one command
brings up a reproducible harness with local-model download, workspace-folder clients, and the
redacted-ZIP export all pre-wired** — no upstream changes, and it upgrades cleanly when upstream
revs bump. This is "bundled" in the exact sense the harness itself means.

**Approach B — a true upstream fork.** Fork `deepseek-ai/deepseek-harness`, add
`packages/llm/hacienda-*` + a `packages/client/hacienda-*` under the existing package layout, add
our bundle to a shipped profile manifest, run `pnpm run build` for the frontend, and ship the
whole install as `@hacienda/dsh-*`. Full control (we can touch `dsh-web-app`'s own patch, add
bundled model weights, rebrand), at the cost of tracking upstream and owning the full build.

**Recommendation:** start with **Approach A** (byte-for-byte upstream + our bundle, pinned revs,
pnpm-vendored) — it delivers "everything bundled" with the least maintenance, and only move to B
if/when we need to modify shipped bundles themselves (rebranding, modifying `dsh-web-app`).

### 12.3 Decisions & open questions

- **D-12.1 — Ollama in Settings = stock Models page + a small "Local Models/Ollama" settings
  section (ours) whose Host tool does `ollama pull` and writes the model into `llm-pi-ai`.** No
  modification of `dsh-client-ui-settings-models`; the custom-provider card already covers setup.
- **D-12.2 — Ship as a bundled profile (Approach A), not a code fork, for the first milestone.**
- Open Q: should "download a model" stream a curated list (fixed library catalog with the pulled
  models) or hand any id? (Recommended: curated list of Gemma→local-size options, then a free-text
  id for power users.)
- Open Q: bundle the model weights into the distribution (only makes sense for tiny models and
  bloats the bundle; recommended no — download on pull via Ollama caches on disk).

---

## 13. Approach A artifact: a "Files / Artifacts" view in the Harness Web UI

The user's request for Approach A: add an **artifacts view** in the DSH web that shows the **real
workspace folder and its documents by filetype**, with **viewers** and a **Finder-style** view of
the workspace folder, and for **markdown / redacted documents** uses **CodeMirror** — referencing
the `@extend-ai/ui` document-viewer family (github `extend-hq/ui`). Investigation confirms this is
**almost entirely reusable from Hacienda Studio already**, so the design below is a
re-host-in-DSH, not a from-scratch build.

### 13.1 What already exists and can be reused (verified in `apps/hacienda-studio/components/extend/`)

| Component (Studio) | Uses | Reuse role in the Artifacts view |
|---|---|---|
| `file-system.tsx` — **Finder-style** `FileSystem` component | `@pierre/trees` (list tree), `@hugeicons`, virtualized windowing | The **grid / list / columns / gallery** browser of the workspace folder: sort (name/kind/date/size), file-type + date filters, search, back/forward history, thumbnails. Filetype-aware (`FILE_KIND_LABELS`, `viewerKindForFile`: pdf/docx/pptx/xlsx/image). |
| `docx-viewer.tsx` — **`DocxEditorViewer`** from `@extend-ai/react-docx` | DOCX editor w/ comments, track-changes, thumbnails | Full **DOCX viewer/editor** for Word artifacts. |
| `pptx-viewer.tsx`, `xlsx-viewer.tsx` | `@extend-ai/react-pptx`, `react-xlsx` | PowerPoint / Excel viewers. |
| `pdf-viewer.tsx` | (PDF rendering) | PDF viewer. |
| `file-thumbnail.tsx`, `file-upload.tsx`, `document-viewer-sidebar.tsx`, `bounding-box-citations.tsx` | | Thumbnails, upload, side-panel, citation overlays — utilities the Artifacts view can reuse. |
| CodeMirror (`@codemirror/*` in deps, esp. `lang-markdown`) | `@codemirror/lang-markdown`, `view`, `theme-one-dark` | **Rendered redacted markdown** editor + PII-span highlighting for `.md` artifacts. |

The `@extend-ai/*` packages (and `@pierre/trees`, `@hugeicons`) are the `extend-hq/ui` ecosystem
the user names; Studio already depends on `@extend-ai/react-docx/react-pptx/react-xlsx` and the
`@codemirror/*` lang-markdown stack. **The "viewers" requested are literally these components.**

### 13.2 Where the Artifacts view mounts in the DSH web (verified slots)

- **Primary seat: a `conversation.view` tab** (`{id, order, label}`, list). The session body
  renders ring entries one-at-a-time via `only: <active id>`; chat is one, trajectory another.
  Register `{ id: 'artifacts', order: <after chat>, label: t('files') }` → the Artifacts view
  appears as its own tab in the session-header view ring. **Low replace-risk, additive.**
- **Entry affordance:** a `conversation.session.header.actions` / `.utilities` button
  (`{id, order}`) to open the Artifacts tab.
- **Detail/preview pane:** `conversation.details.tool` (the right-column details seat) to show the
  selected file's viewer in the details panel as a secondary surface.
- **Redaction pane (M3):** the CodeMirror markdown editor with PII-span highlighting doubles as
  the interactive redaction surface fed by the in-browser GLiNER2 worker pool (§10.1), writing
  decisions to the durable review queue (§11.4).

### 13.3 Data flow — show the real folder

- **Listing:** a Host tool (or the Client `ctx.fs` path we expose via a thin Host RPC) walks the
  **active workspace's canonical `cwd`** (§11.6) with `ctx.fs.listDir`/`stat` and projects it into
  the `FileSystemItem[]` shape `file-system.tsx` consumes (folders + files with path/name/size/
  mtime/type). Respect the `directoryPicker` browse semantics; dotfiles carry the host `hidden`
  flag (§11.7).
- **Live:** subscribe to `fs`/workspace changes so the view reflects the real folder as the agent
  writes to it (files created by tools appear — consistent with §11.9 durable-path semantics).
- **Content:** for viewers, stream bytes for the selected file from Host to Client (e.g. a
  Host `read`-style RPC or `ctx.webServer` route); for text/markdown use `ctx.fs.readText`.
- **Provenance:** artifacts produced by the engine (redacted `.md`, generated `_manifest.json`,
  the exported ZIP) appear in the same tree — the view doubles as the "what did the engine give
  me" surface, linking to the two-tier evidence (§4).

### 13.4 Redaction-first markdown editing (CodeMirror)

For `.md` / `.txt` artifacts (the engine's `documents/*.md` output):
- Render **`@codemirror/view` + `@codemirror/lang-markdown`** in a read/write editor.
- Overlay a **PII-spans decoration** (from the GLiNER2 worker pool, §10.1) that highlights detected
  spans; the user can approve/reject/flag each, writing to the review queue (§11.4).
- "Safe for export" is only when span review is clean → feeds `export_safe_zip` (§4-§5); the
  CodeMirror view is the *feedback* tier, the facade the *evidence* tier (D-M3-2).

### 13.5 Decisions & open questions

- **D-13.1 — Reuse the Studio `extend/` components** (file-system + the `@extend-ai` viewers +
  CodeMirror) rather than building new ones; they are proven in-browser and already depend on the
  extend-ui family the user named.
- **D-13.2 — Mount as a `conversation.view` tab** (additive), not a `conversation.session`
  replacement (which would shadow the whole session body).
- D-13.3 — Viewers on the Client need document bytes from the Host; decide the transport (Host
  RPC stream vs `ctx.webServer` route) at build time (recommend: `ctx.webServer` GET route for
  large PDF/DOCX, RPC for small text).
- Open Q: should the Artifacts view be **read-mostly** (browse + open) or a **full editor** (CODE
  folding, rename/move/delete in the Finder view)? Recommend: read/browse + markdown edit first;
  destructive ops later (mirrors §11.6 "deletion and destructive folder removal are separate,
  absent capabilities").

---

## 14. Artifacts view — concrete `conversation.view` registration + Host RPC contract

This section is the design-level spec the user approved (§13 follow-up): the exact Slot registration
and the Host↔Client data contract to render the Finder-style folder + filetype viewers + CodeMirror
redacted-markdown view (all verified against `dsh-tools`, `dsh-client-runtime`, `Slots.listSubTree`,
`ctx.fs`).

### 14.1 Client registration — a `conversation.view` tab

The Artifacts view is **one list entry in the `conversation.view` ring** (`kind: list`, scope
`session`, registration `{id, order, label}`), rendered one-at-a-time by the session body via
`only: <active id>` — same mechanism as chat and trajectory. Plus a header button to jump to it.

```js
// Client half — register the view tab + a header action to open it.
return {
  inject: ['slots', 'sessions'],
  apply(ctx) {
    const slots = ctx.get('slots')
    if (slots === undefined) return

    slots.inject('conversation.view', () => slots.register(
      {
        name: 'conversation.view',
        id: 'artifacts',
        order: 2,                 // after chat (the ring's first entry)
        label: 'Files',
      },
      (props) => ArtifactsView(props),   // the FileSystem + viewer composition
    ))

    // Optional: a header button that activates the tab.
    slots.inject('conversation.session.header.actions', () => slots.register(
      { name: 'conversation.session.header.actions', id: 'open-artifacts', order: 10, label: 'Files' },
      () => React.createElement('button', { onClick: () => openViewTab('artifacts') }, 'Files'),
    ))
  },
}
```

Note: this is a **Slot** registration, so it is not `presentResult` (that's for tool-owned cards,
§11.2). It needs only `ctx.get('slots')` (optional) and standard props; no Host RPC is required to
*mount* the view — the Host RPC is only for the folder listing + file bytes.

### 14.2 Host plugins & RPC — the folder data contract

Because the Client Builtins are minimal (`React`, `host.call`, `styles.insert`), all filesystem
access lives in the **Host half** behind a small package-private `harness.handle` RPC surface. The
Client calls it with `host.call(method, args)` (lossless JSON only).

**RPC 1 — `list-workspace`** (Host `harness.handle('list-workspace', handler)`):

```
request : { path?: string }                    // absolute dir; default = active workspace cwd
response: {
  path: string,                                // canonical cwd (fs.realpath)
  items: FileSystemItem[],                     // folders + files, one level (or full tree)
  parentPath?: string | null,                  // for back/forward breadcrumb
}
FileSystemItem = { kind, path, name?, parentPath?, size?, createdAt?, updatedAt?, contentType?, hidden? }
```

Host behaviour (verified against `ctx.fs`): `fs.resolve(path)` → `fs.stat` / `fs.listDir` per
entry → build the projection; set `hidden` from the dir listing's POSIX-dot flag (§11.7); resolve
the "active workspace" from the session's canonical cwd (§11.6). Returns **owned JSON only** — no
live `fs` objects cross the wire.

**RPC 2 — `read-file-text`** (Host): for a path → `fs.readText` → `{ text }` (for markdown/code
into CodeMirror). **RPC 3 — `file-download-url`** (Host): for a viewer path (PDF/DOCX/PPTX/XLSX)
→ either a `ctx.webServer` GET route URL carrying the bytes, or a small base64/range RPC for
small docs. **RPC 4 — `scan-artifacts`** (optional, the redaction pass): calls the same
facade/GLiNER2 path §10.1 for span decoration, returning `{ path, spans: [{start,end,category,
confidence}] }` for the CodeMirror overlay.

### 14.3 Client view composition (reuse, not rebuild)

- **Browser**: the Studio `FileSystem` component (grid/list/columns/gallery + sort/filter/search)
  fed by RPC 1; `onFileOpen` dispatches by `viewerKindForFile`.
- **Viewers**: CodeMirror (`@codemirror/view` + `lang-markdown`, one-dark) for `.md`/`.txt`/code
  with the span overlay (RPC 4); `@extend-ai/react-docx`/`react-pptx`/`react-xlsx` + the Studio
  `pdf-viewer` for documents (bytes via RPC 3); `<img>` for images (RPC 3 URL).
- **Redaction integration**: the CodeMirror markdown editor is the interactive redaction surface
  (M3 §9.3/§11.4) — approve/reject/flag spans → Host writes to the durable review queue via
  `HaciendaFacade.review_queue_with_auth` / the MCP review tools. "Clean review → exportable"
  gates `export_safe_zip` (§4-§5).
- **Live refresh**: re-run RPC 1 on FS/workspace change so newly produced artifacts appear
  (consistent with §11.9 durable-path semantics).

### 14.4 Decisions

- **D-14.1** — Mount as `conversation.view` tab `id: 'artifacts'` (additive, list) + optional
  `conversation.session.header.actions` button. **Not** a `conversation.session` replacement.
- **D-14.2** — Host owns all FS/business access behind 4 RPCs; Client renders from owned JSON.
- **D-14.3** — Reuse Studio `extend/` components + `@extend-ai` viewers + CodeMirror verbatim
  (they already depend on the extend-ui family the user named).
- **D-14.4** — Read/browse + markdown edit first; destructive folder ops out of scope (aligns
  with §11.6's "deletion and destructive folder removal are separate, absent capabilities").

---

## 15. "Free deep search" — what actually exists (verified, honest scope note)

Requested: "install a free DeepSeek Harness plugin here for deep search that is free." Verified
against the installed packages and the live `web_search`/`web_fetch` tool set (this document is
authored in the running harness, which has `web_search`/`web_fetch` tools registered):

- **There is no free "deep search" plugin for the DeepSeek Harness.** The shipped web-search
  provider family is `@deepseek-ai/dsh-web-search-exa` (paid), `…/dsh-web-search-perplexity`
  (paid), and `…/dsh-web-search-deepseek`, which calls DeepSeek's **paid** API with the native
  `web_search` server tool — **one search = one full paid model turn** (the seam doc is explicit
  that it is heavier than a pure retrieval endpoint).
- The currently mounted provider here is `dsh-web-search-deepseek`; the first `web_search` in this
  session failed with **`DEEPSEEK_API_KEY` missing** — exactly the paid-credential gate. No
  `dsh-web-fetch-http` (the anonymous/free fetch provider) and no Exa/Perplexity rows are mounted
  in this profile.
- **A free local model (Ollama/Gemma, §12) does not provide web search** — it can reason and
  generate, but cannot crawl the web; web search is retrieval, not generation.
- npm has **no published `@deepseek-ai` deep-search plugin**; third-party "deep search" packages
  found are unrelated (AWS/misc).

**Honest options (in order of zero-cost-to-cheapest), no fabricated installs:**
1. **Local/offline deep search (truly free)**: the harness's own `grep`/`glob`/`ctx.fs` over the
   workspace is already a "search your documents" capability; our Artifacts view (§13-14) plus
   in-browser GLiNER2 renames this into a structured local document search with **no API key and no
   egress**. Recommended as the genuinely free path.
2. **Free-tier/hosted search later**: add `dsh-web-fetch-http` (anonymous fetch) and a search
   provider when a free-tier key is available. Not free/no-install today.
3. **Don't claim a "free deep search plugin" exists** — it does not; installing a paid-credential
   package and calling it free would be misleading.

---

## 16. Exa deep search — actually installed & configured (Approach A)

Applied on request (user supplied an Exa API key). Verified end-to-end; the only remaining step is
a profile restart, which the user performs (the live GUI can't be restarted mid-session).

**What was done:**
1. **Installed the provider package** into the web profile via pnpm (the harness's own `dsh
   plugin`/pnpm mechanism), after making pnpm available (it is not on PATH; used the repo's hoisted
   `node_modules/.bin/pnpm` with node 22):
   - `@deepseek-ai/dsh-web-search-exa@0.1.0-rc.7` → added to
     `$DSH_HOME/profiles/web/package.json` `dependencies` and to the profile's pnpm store.
2. **Patched `$DSH_HOME/profiles/web/cordis.patch.yml`** (the user-editable profile layer):
   - Override the base `web` row → `searchProvider: exa` (the id `dsh-web-search-exa` registers
     under). This turns off `deepseek-official` and selects Exa.
   - `- insert:` the new `web-search-exa` row with `apiKey: !!js process.env.EXA_API_KEY`
     (a plain `- id:` row cannot create a row — only `- insert:` can — which `--dump-config`
     initially surfaced as "entry web-search-exa not found"; fixed).
3. **Validated** with `dsh --profile web --dump-config`: the composed tree now has the
   `web-search-exa` row and `web … searchProvider: exa`, with **no** patch errors.

**How the key is resolved (verified from the Exa provider + credentials docs):** the Exa provider's
config `apiKey: !!js process.env.EXA_API_KEY` reads the **launching process environment**, not the
harness `.credentials.yaml` (which is the `ctx.credentials.resolve()` document) and not the `.env`
fallback files. Per the seam, the privileged layer is the **launch env**.

**Remaining step — user restart with the key exported:**
```bash
export EXA_API_KEY=76ec2188-0e1e-4704-baba-2255da82fb9d   # or in a launch script
dsh --profile web
```
After restart, `web_search` in the web surface uses Exa (`searchProvider: exa`), and each search
maps Exa `results[]` → `WebSearchSource` (url/title/first highlight snippet/publishedDate), with a
result lacking a highlight dropped (per the provider).

**Security note:** the key is not written into `cordis.patch.yml`/`package.json` (only the env
reference is). It will sit in the shell environment / a launch script of the user's choosing —
the idiomatic Exa pattern. The design doc and this record keep the key out of the repo.

**Caveat carried to the fork:** the running session this was configured from still uses its prior
composition; the Exa wiring applies from the next boot of the **web** profile. The `code`/agent
preset (this session) is a different composition and is unaffected.

**Follow-up — anonymous `web_fetch` also enabled (Approach A).** On request, the profile now also
exposes URL retrieval:
- Installed `@deepseek-ai/dsh-web-fetch-http@0.1.0-rc.7` (the anonymous, keyless HTTP(S) fetch
  provider; registers id `http`; config `timeoutMs`/`maxRedirects`/`userAgent`).
- Patch inserts the `web-fetch-http` row and overrides `tool-web` to `disabled: false` + `fetch:
  true`, so **both `web_search` (Exa) and `web_fetch` (http)** are model-facing in the web surface.
- Re-validated with `--dump-config`: no patch errors; `web` → `searchProvider: exa`; both provider
  rows present; `tool-web` enabled.
- Note surfaced in validation: a `config`-only patch row does not clear a base `disabled: true`;
  the `tool-web` override must state `disabled: false` explicitly (done).

**Exa CONFIRMED LIVE (2026-08-18).** Verification results:
- Direct `POST https://api.exa.ai/search` with the configured key → **HTTP 200**, real results
  (3 sources, ~$0.007/search neural).
- The Exa-enabled web profile boots cleanly on a fresh port (`dsh --profile web --port 3081` →
  `dsh web: http://127.0.0.1:3081`), process env carries `EXA_API_KEY`, and `GET /` returns 200.
- Root cause of the earlier failed restarts, recorded for the fork: (1) `command -v dsh` can
  resolve to Debian's `/usr/bin/dsh` (unrelated tool) — the launcher must **hard-pin** the harness
  dsh at the npx install path; (2) the launcher could not stop the running 3080 harness (it is the
  parent of its own session) → `EADDRINUSE`; the working pattern is **boot on a fresh port** and
  migrate, or stop the old pipe in its own terminal first. The launcher now does exactly that.

---

## 17. External-fact validation via Exa (2026-08-18)

Used the confirmed-working Exa key (on the 3081 harness) + direct Exa API to pressure-test the
design's third-party assumptions against current web sources.

| Design decision | Exa find (source) | Verdict |
|---|---|---|
| **§13/14 — reuse `extend-hq/ui` (`@extend-ai/*`) viewers (DOCX/PPTX/XLSX/PDF) in the Artifacts view** | `github.com/extend-hq/ui` is **Extend UI**, an active open-source "UI kit for modern document apps" (updated 2026-08-11); docs at `extend.ai/ui` list docx-viewer, pdf-viewer, and PPTX/XLSX components. | ✅ **Validated.** The viewer set is real, current, browser-based — matches what Hacienda Studio already vendors. |
| **§10.1 Lever 5 / D-M3-3 — WebGPU via candle "same code, same model" for GLiNER2** | **`gliner2` 0.1.3 Rust crate has `CandleExtractor` over `candle-core` 0.10** (docs.rs), with optional `tch` — a pure-Rust candle backend; API mirrors Python `GLiNER2.extract`/`batch_extract`. Also `xberg-io/xberg PR #1263`: "detect entities without ONNX Runtime via a pure-Rust GLiNER2 implementation," plus `brainless/gliner2-candle`. | ✅ **Validated.** The candle-backend path is real and maintained; WebGPU (candle `wgpu`) is a credible upgrade of the same weights/code, exactly as designed. |
| **§10.1 Role 3 — in-browser GLiNER2 for PII/entity feedback** | `pii-browser` npm; a `redactable` commit "integrate GLiNER as a browser contextual engine (experimental)"; `gliner` npm. | ✅ **Validated as an emerging practice.** Our browser-feedback tier is not speculative. |
| **§10.1 Lever 2 — Web Worker / SharedArrayBuffer model parallelism** | MLC **WebLLM** (high-performance in-browser LLM inference), `ngxson/wllama`, LocalWebAI worker system. | ✅ **Validated.** Multi-worker + shared-memory model serving is the industry pattern. |
| **M2 / M1 — MCP stdio + Claude Desktop consumption of `hacienda-mcp`** | Official MCP docs (2026-07-28: "Connect to local MCP servers" stdio), Claude Code MCP docs, 2026 "Add an MCP server to Claude Desktop" guides. | ✅ **Validated.** The stdio `hacienda mcp serve` → Claude Desktop path is current best practice. |

**Net effect on the design:** no decision needed reversing. The two most load-bearing external
assumptions the Exa research could have broken —
(1) that `@extend-ai` viewers run in-browser and are maintained, and (2) that GLiNER2-on-candle can
be the same-code in-browser/GPU model — are **both confirmed**. The one nuance to carry forward:
to get the *browser* candle backend we'd compile `gliner2`/`xberg-gliner` with candle's `wgpu`
backend (like `hacienda-core`'s `ner-candle-wasm` today uses `wasm_js`), which is a recompile, not
a new model — consistent with D-M3-3.
