# M1 — MCP server, and closing the CLI/SDK parity gap

**Date:** 2026-08-13
**Status:** M1a shipped (this revision) — stdio MCP server, extraction-format parity. See
§0.
**Track:** new **Piste M** (MCP), plus a parity slice on the existing CLI surface
**Program:** `2026-08-01-hacienda-platform-parity-program.md` §9 ("Serveur MCP — chantier
distinct… à spécifier à part") and §4 S4 (`2026-08-01-S4-api-contract-and-clients.md`)
**Depends on:** P1, P2, P3, P4, P5, S4 (all already shipped — see §1)
**Blocks:** nothing in S/E/V — this track is additive

---

## 0. What actually shipped in this revision

The first draft of this spec was written from documentation and the route table alone.
This revision was written after cloning `jamon8888/xberg` at the pinned commit and reading
its actual source (`crates/xberg/src/api/`, `crates/xberg/src/mcp/`, `crates/xberg-cli/`) —
a few corrections and one new finding came out of that, folded in below. Two things are
now real code, not proposal:

- **Extraction-format parity (new, not in the first draft).** The root `Cargo.toml`'s
  `xberg` dependency now enables `pdf`, `office`, `excel`, `email`, `hwp`, `hwpx`, `iwork`,
  `archives` — every xberg extraction format that is pure Rust, needs no native toolchain
  and no model download. Confirmed against `crates/xberg/Cargo.toml`'s own feature
  definitions (`pdf_oxide`, `lopdf`, `calamine`, `mail-parser`, `outlook-pst`, `biblatex`,
  `biblib`, `org`, `dbase`, `roxmltree`, `quick-xml`, `unhwp` are all crates.io Rust
  libraries). `xberg::extract_batch` is hacienda's single extraction call site
  (`hacienda-core/src/facade.rs`), so this is a pure feature-flag change — zero xberg
  source touched — and `cargo check -p hacienda-core -p hacienda-api -p hacienda-cli`
  passes clean with it. Still deliberately not enabled: `ocr`/`heic`/`wordperfect` (native
  system libraries) and `embeddings`/`reranking`/`layout-detection`/`ner-onnx`/`candle-ocr`
  (ONNX Runtime + a HuggingFace model fetch) — those need infrastructure provisioning
  beyond a Cargo feature; tracked as follow-up, not silently dropped.
- **`hacienda-mcp` (M1a, new crate).** Implements exactly the architecture §2–§3 below
  describe: eight tools, each a thin wrapper over an existing `HaciendaFacade` method,
  stdio transport via `rmcp` (the same MCP SDK xberg itself uses, confirmed at
  `crates/xberg/Cargo.toml`: `rmcp = "2.2.0"`), wired up as `hacienda mcp serve`. Verified
  end-to-end by hand over stdio: `initialize` → `tools/list` (all eight, correct schemas)
  → `tools/call pii_redact` on a value containing a control corpus email and IBAN returns
  `"contact me at [EMAIL], IBAN [IBAN:****]"` and two audit entries, with the corpus value
  absent from both the text content block and `structuredContent`. Four automated tests
  ship with the crate, including the same never-leaks-the-value assertion. §3.6's rmcp
  spike is therefore resolved: rmcp fit cleanly, no fallback needed.
- **One correction to §3.2/§3.3 below, from reading the real xberg source instead of the
  `.ai-rulez` skill doc the first draft leaned on:** that skill doc's MCP tool list
  (`extract`, `extract_batch`, `get_capabilities`) doesn't match xberg's actual registered
  tools, which are `extract`, `extract_batch`, `detect_mime_type`, `cache_stats`,
  `cache_clear`, `cache_manifest`, `cache_warm`, `get_version` (confirmed against
  `crates/xberg/src/mcp/server.rs`'s own test assertions). Doesn't change this spec's
  architecture — hacienda's tool set is derived from `ROUTE_TABLE`, not from xberg's tool
  list, precisely because the two are deliberately different surfaces — but the skill doc
  itself is now known-stale on this point.
- **One real finding, out of scope for this spec, flagged rather than fixed:**
  `hacienda-cli`'s `run_serve` builds its facade with `HaciendaFacade::new(config)`, which
  uses an **in-memory** audit/review store — not `FileAuditStore` or `PostgresAuditStore`,
  even though both exist in `hacienda-core`. The only production call site that wires a
  durable store is `hacienda-api/src/handlers/auth.rs` (for API keys); every
  `PostgresAuditStore` construction in `hacienda-api/src/routes.rs` is inside its
  `#[cfg(test)]` module. This means `hacienda serve` today loses its whole audit/review
  history on restart — a materially different (and worse) claim than "single-node,"
  which is how S2 currently frames the gap. It is the reason §4's CLI parity slice
  (`audit entries`/`review`/`compliance` as standalone one-shot commands) is **not** part
  of this revision: a one-shot CLI command reading an in-memory store that dies with the
  process it belongs to has nothing durable to show. `hacienda-mcp`'s `audit_entries` /
  `audit_verify` / `compliance_report` tools cover the live-process case today (proven in
  the smoke test above); a CLI surface for durable state needs S2's durable-store wiring
  in `run_serve` first, not just a new subcommand. Filing this precisely so it doesn't get
  rediscovered as "S2 is slow because of scale" when the actual gap is "the wiring was
  never done."

---

## 1. Where the program actually stands today

The parity program (`2026-08-01-hacienda-platform-parity-program.md`) was written as if S/P/E
were mostly future work. Reading the code instead of the program doc, most of it has already
shipped:

| Piste | Program said | Code today |
| --- | --- | --- |
| S1 (tenancy) | to write | `Capability`/`CapabilitySet`/`Caller`, `hacienda-core/src/auth/` — shipped |
| S4 (API contract & clients) | to write | full OpenAPI 3.1 from `ROUTE_TABLE`, generated Python (`sdks/python`) and TypeScript (`sdks/typescript`) clients, 44 operations / 14 tags, CI-generated against a live `hacienda serve` — shipped |
| P1–P5 (proof layer) | to write | `/v1/audit/{entries,verify,seals,export,tip}`, `/v1/review`, `/v1/review/{id}/decide`, `/v1/compliance/{report,dpia}`, `/v1/pii/{scan,redact,reveal,config}`, `/v1/glossary` — all live, all capability-gated, reflected in `ROUTE_TABLE` | 
| E0–E5 (Enterprise parity) | to write | `/v1/documents(/async)`, `/v1/jobs`, `/v1/presets`, `/v1/documents/{id}/versions`, `/diff`, `/v1/uploads/{presign,confirm}`, `/v1/rag/collections/**` (ingest, retrieve, migrate-embeddings, answer), `/v1/usage` — all live |

`hacienda-api/src/routes.rs`'s `ROUTE_TABLE` is ~40 paths, ~44 operations, every one guarded by a
`Capability` and reflected in `/openapi.json` by construction (invariant I4). This spec does not
redo that work. It closes the three things that are **not** covered by it, and that this task was
asked to investigate: MCP, CLI parity, and SDK parity.

| Surface | State | Gap |
| --- | --- | --- |
| **API** | Essentially complete against the program's own target | None new here |
| **SDK** (Python/TS) | Generated from `/openapi.json`, tracks `ROUTE_TABLE` by construction | None new here — see §5 |
| **CLI** | `extract`, `scan`, `config show`, `serve`, `pii reveal`, `audit verify` (local export only) | `audit {entries,seals,export,tip}`, `review`, `compliance` exist in the API and have no CLI front door — see §4 |
| **MCP** | Does not exist anywhere in this repo | Whole surface — see §2–§3 |

The program's own §9 lists the MCP server as explicitly out of scope, with one sentence of
direction: *"la moitié extraction est une feature `xberg/mcp` à activer, les outils
PII/conformité se greffent dessus."* §3 explains why that direction is wrong for this repo and
what to do instead.

---

## 2. Why hacienda must not "activate `xberg`'s `mcp` feature"

`xberg` ships its own MCP server (`crates/xberg/src/mcp/server.rs` per
`.ai-rulez/skills/api-server-mcp/SKILL.md`): three tools (`extract`, `extract_batch`,
`get_capabilities`), three resources, two prompts, stdio and HTTP transports. It is tempting to
just turn on a cargo feature and re-expose it.

That would violate the program's non-negotiable principle (program §1): **"chaque chemin capable
d'émettre du contenu documentaire traverse `HaciendaFacade`."** Xberg's `extract` tool returns raw
extracted text — no PII detection, no redaction, no audit entry. An agent calling it through a
proxied xberg MCP server would receive unredacted document content by a path that P1's guard
(`2026-08-01-P1-redaction-enforcement-point.md`) was built specifically to make unreachable. MCP
is exactly the kind of "future transport" P1 §1 already names as the reason the guard lives in
`hacienda-core` and not at any one transport: *"chaque futur appelant — FFI, CLI, un second
transport HTTP — n'a pas à la réimplémenter, l'un d'eux oubliera."* MCP is that second transport.

**Decision D-M1-1 — hacienda's MCP server is its own, built against `HaciendaFacade`, not a proxy
or re-export of xberg's.** Concretely:

- Do not enable xberg's `mcp` cargo feature. `Cargo.toml`'s pinned feature set stays
  `["redaction", "ner", "tokio-runtime"]` — unchanged by this spec, for the same reason the
  workspace `[workspace.dependencies]` comment already gives for every other feature: no call
  site, no feature.
- Do not vendor, wrap, or import anything from `crates/xberg/src/mcp/` or `crates/xberg/src/api/`.
  hacienda's MCP tools call `HaciendaFacade` methods directly — `process`/`process_with_auth`,
  `scan_text_with_auth`, `redact_text_with_auth`, `reveal_token_with_auth`,
  `audit_history_with_auth`/`audit_seals_with_auth`/`audit_export_with_auth`,
  `verify_audit_with_auth`, `glossary_snapshot_with_auth`, `compliance_report_with_auth`, plus the
  review-queue and RAG facades already called by `hacienda-api`'s handlers. These already exist;
  this spec adds a second transport in front of them, not new business logic.
- Net xberg-crate diff for this entire spec: **zero.** No new feature flags, no patched commit, no
  new dependency edges into `crates/xberg`. Everything lives in hacienda's own crates.

This also answers the task's framing directly: the way to get "everything xberg exposes" without
touching xberg's code is to not re-expose xberg's surface at all, but hacienda's own equivalent —
extraction is already behind `/v1/documents` and `POST /v1/pii/scan`, redacted and audited by
construction. An MCP client gets the redacted answer through the same seam an HTTP client already
does. Nothing xberg exposes (`extract`, `extract_batch`, `get_capabilities`) is missing from
hacienda's *capability* set — it's just reached through the guarded facade instead of the raw
crate.

---

## 3. Shape of the MCP server

### 3.1 Crate placement

`alef.toml` already anticipates this, at `[workspace.docs] mcp_sources =
["hacienda-core/src/mcp/server.rs"]` — but that path is aspirational and wrong for the same reason
the changelog's "Unreleased" entry already flagged `alef.toml`'s `cli_sources` as pointing at files
that don't exist: it was written before any of this was built, by analogy with xberg's own layout,
not from this repo's crate-layout rule.

**Decision D-M1-2 — a new thin crate, `hacienda-mcp`, not a module in `hacienda-core`.** The
crate-layout rule (`.ai-rulez/domains/hacienda-pii/DOMAIN.md`) is explicit: `hacienda-core` holds
the pipeline, `hacienda-api` and `hacienda-cli` are thin transport consumers. MCP is a transport,
exactly like Axum is for `hacienda-api` — it does not belong in core any more than `routes.rs`
does. `hacienda-mcp` depends on `hacienda-core` (for `HaciendaFacade`, `Capability`, `Caller`) and
optionally on `hacienda-api`'s `ApiState`/DTOs (to avoid a second copy of every request/response
shape — see §3.3). It is consumed by:

- `hacienda-cli`, adding `hacienda mcp serve` (stdio transport) — the primary use case (Claude
  Desktop, local agents), matching the precedent `pii reveal` and `serve` already set for
  in-process trusted callers.
- `hacienda-api`, optionally mounting `hacienda-mcp`'s router at `/mcp/*` on the same Axum
  `Router` `build_router` already produces (HTTP/SSE transport) — additive, behind a cargo feature
  (`hacienda-api/mcp`) so a deployment that doesn't want MCP doesn't pay for it.

`alef.toml`'s `mcp_sources` gets corrected to `hacienda-mcp/src/server.rs` once the crate lands —
housekeeping, same class of fix as the bindings-table correction already in the changelog.

### 3.2 Tool inventory: generated from `ROUTE_TABLE`, not hand-maintained

The program's invariant I4 — *"la table de routes reste l'unique source de vérité chemin /
capacité / handler"* — extends naturally to MCP. `hacienda-mcp` does not maintain its own
capability list. It walks `hacienda_api::routes::ROUTE_TABLE` and derives one MCP tool per
content-bearing route (everything except the four public, side-effect-free routes — see §3.4),
using the same `Capability` each route already declares.

This gets the anti-drift property in the API's `openapi_path_set_equals_route_table` test for
free, extended with a sibling: `mcp_tool_set_equals_route_table`, asserting the MCP tool set is
exactly `ROUTE_TABLE`'s guarded paths (minus resources). A route added to the table without a
capability decision already fails `every_guarded_route_reflected_in_auth_state`; this closes the
same gap for MCP instead of leaving it as a second place to forget.

Tool naming mirrors the OpenAPI operation IDs the SDKs already generate against (§5), so a tool
name and an SDK method name refer to the same operation: `documents_process`,
`documents_process_async`, `pii_scan`, `pii_redact`, `pii_reveal`, `audit_entries`,
`audit_verify`, `audit_export`, `review_decide`, `compliance_report`, `rag_collections_retrieve`,
and so on for all ~40 guarded routes. Request/response JSON Schemas come from the same `dto.rs`
types `hacienda-api` already serializes with `utoipa`/serde — no second schema to hand-write or
let drift (§3.3).

### 3.3 Reuse `hacienda-api`'s DTOs — do not fork request/response shapes

`hacienda-api/src/dto.rs` (1,154 lines) already has every request/response type this needs, with
serde derives. `hacienda-mcp` takes `hacienda-api` as a dependency for these types only (not for
its handlers), and its tool handlers do exactly what the REST handlers already do: deserialize the
tool call's `arguments` into the same DTO the REST handler deserializes its JSON body into, call
the same facade method, serialize the same response DTO into the tool result. A tool handler is a
few lines of glue, not a reimplementation.

### 3.4 Resources vs. tools

The four public, informational routes become MCP **resources**, not tools — matching xberg's own
split (`xberg://formats`, `xberg://features`, `xberg://api-reference`) so an agent already
familiar with xberg's MCP server recognizes the pattern:

- `hacienda://health` → `GET /health`
- `hacienda://version` → `GET /version`
- `hacienda://info` → `GET /info`
- `hacienda://openapi` → `GET /openapi.json` (the same document the SDKs generate against — an
  agent can introspect the full guarded surface from inside the MCP session)

### 3.5 Auth: identical capability model, no new one

- **Stdio transport** (`hacienda mcp serve`) runs as `Caller::Trusted`, the same precedent already
  documented for CLI `serve` and `pii reveal`: the process boundary is the trust boundary. No
  token is needed locally, same as today's CLI.
- **HTTP transport** (`/mcp/*` mounted on `hacienda-api`) requires the same bearer token and
  `Capability` the equivalent REST route requires — reusing `hacienda-api`'s existing
  `auth_middleware`/`AuthState`, not a parallel auth path. A tool call without the right capability
  gets the same 403 the REST route would, not a silently different MCP-shaped error.

### 3.6 Protocol library

**Decision D-M1-3 (open, to confirm in the implementation plan) — use `rmcp`, the official Rust
MCP SDK, rather than hand-rolling JSON-RPC 2.0 the way xberg's own server does.** Reasons: it is
maintained against the protocol spec directly (tool/resource/prompt registration, stdio and
streamable-HTTP transports out of the box), and picking it means zero new protocol-framing code to
maintain in this repo, on top of already touching zero xberg code. A short spike against `rmcp`'s
current API shape should precede the implementation plan, since MCP SDK churn is real; if it does
not fit cleanly, the fallback is the same hand-rolled JSON-RPC-over-stdio approach xberg's own
server already demonstrates works, reimplemented independently (still zero xberg code touched —
the *pattern* is public, not the code).

### 3.7 Prompts (low priority, optional for the first cut)

One prompt, `extract_and_prove` — guidance for an agent that wants "redact this and be able to
show it was redacted": call `documents_process`, then `audit_verify` or fetch `audit_export` for
the returned chain segment. This is the hacienda-specific analogue of xberg's `extract_for_rag`
prompt; not required for M1's exit criteria, and can ship after the tool surface is solid.

### 3.8 Exit criteria

- A control corpus run through the `documents_process` / `pii_scan` MCP tools never returns
  unredacted text in the tool result (same negative test class the API's guarded-routes test
  already runs, reused against the MCP transport).
- `mcp_tool_set_equals_route_table` passes and fails loudly (CI red) if a route is added to
  `ROUTE_TABLE` without a corresponding tool decision.
- `hacienda mcp serve` works against a real Claude Desktop config (`command: "hacienda", args:
  ["mcp", "serve"]`) end to end against at least `pii_scan` and `documents_process`.
- An HTTP MCP call without the route's required capability gets the same status the REST route
  would.

---

## 4. CLI parity: front doors for API surface that already exists

**Status: blocked on the durable-store finding in §0, not implemented in this revision.**
`hacienda mcp serve` (§0, §3) now covers the live-process version of everything below —
`audit_entries`, `audit_verify`, `compliance_report` are real, tested MCP tools today. What
follows is the original design for a *standalone, one-shot CLI* surface, kept because the
design itself is still right; it is gated on `run_serve` (or an equivalent long-lived
process) actually persisting to `FileAuditStore`/`PostgresAuditStore` first, since a
one-shot command reading an in-memory store that dies with the process producing it has
nothing to read.

`hacienda-cli/src/cli.rs`'s own doc comment already names the gap precisely: *"`review`,
`compliance`, the rest of `audit`, `completions`, and the `xberg` passthrough remain deliberately
absent rather than present and stubbed."* That stance was correct when written — the backing
methods didn't exist yet, or weren't proven. They are now: `hacienda-api`'s handlers for all three
have shipped, are capability-gated, and are exercised by the SDK test suites. The reason to keep
them absent from the CLI no longer holds for `audit` and `review` and `compliance` — only for
`completions` (pure convenience, no parity question) and the `xberg` passthrough (deliberately
rejected by the CLI's own framing: "the polarity is inverted from `xberg` on purpose").

**Scope — new subcommands, each a thin wrapper over the facade method the equivalent API handler
already calls, no new business logic:**

| New CLI command | Calls | Mirrors |
| --- | --- | --- |
| `hacienda audit entries` / `seals` / `export` / `tip` | `audit_history_with_auth`, `audit_seals_with_auth`, `audit_export_with_auth`, `audit_tip` | `GET /v1/audit/{entries,seals,export,tip}` |
| `hacienda review queue` | review queue read path (`audit:read`, per the `GET /v1/review` capability fix already in the Unreleased changelog) | `GET /v1/review` |
| `hacienda review decide <id>` | review decision path (`review:decide`) | `POST /v1/review/{id}/decide` |
| `hacienda compliance report` / `dpia` | `compliance_report_with_auth` | `GET /v1/compliance/{report,dpia}` |
| `hacienda mcp serve` | new, §3 | — |

`hacienda audit verify <dir>` (local `--audit-out` export) stays as-is — it verifies a different
artifact (a flat export) than `hacienda audit export` will (the live durable store), and the CLI
doc comment already explains why they're not the same thing. Both coexist under `audit`, not
merged.

**Non-goals, unchanged:** `completions`, the `xberg` passthrough. Nothing in this spec revisits
those; the existing reasoning stands.

---

## 5. SDK parity: already true by construction, nothing to build

`sdks/python` and `sdks/typescript` are generated from the live `/openapi.json` in CI
(`sdks/scripts/fetch-openapi.sh`), against the same commit under test. Every route in
`ROUTE_TABLE` that has a real OpenAPI operation is already reachable from both SDKs — that's what
"44 operations across 14 tags" in the changelog means. There is no separate SDK parity backlog to
write: the moment a new API route lands with a schema (already required by S4's anti-drift test),
both SDKs pick it up on their next generation run, with no hand-written client code to update.

Two follow-ups, both small and neither blocking:

- **MCP is not SDK surface.** Do not add MCP tool-calling to `sdks/python`/`sdks/typescript` — MCP
  is for agent clients (Claude Desktop, agent frameworks), the generated SDKs are for programmatic
  HTTP callers. Keep the two paths separate rather than growing a third client shape.
- **Native bindings (alef, 14 languages)** stay exactly where S4 already put them:
  "découplées, pas abandonnées" — a distinct offering for Studio/embedding, off this critical
  path. This spec does not reopen that decision.

---

## 6. Sequencing

M1 (MCP) depends only on P1–P5 and S4, all already shipped — it has no dependency on S2/S3
(durable persistence, async jobs) or the V-track (vertical LoRA routing), and blocks none of them.
It can start immediately, in parallel with whatever the program's waves 2–4 are already doing.

| Slice | Content | Why this order |
| --- | --- | --- |
| **M1a** | **Shipped** (§0). `hacienda-mcp` crate, stdio transport, `hacienda mcp serve`. Eight hand-picked tools (`documents_process`, `pii_scan`, `pii_redact`, `pii_reveal`, `audit_verify`, `audit_entries`, `compliance_report`, `get_version`), each proven against a real `HaciendaFacade` call. **Not yet done, and honestly still open against §3.2/§3.8's original design:** the `ROUTE_TABLE`-driven generation and the `mcp_tool_set_equals_route_table` anti-drift test — this revision hand-wrote eight tools rather than deriving all ~40 guarded routes (RAG collections, presets, versions/diff, uploads, usage, auth management are not yet tools). Closing that gap is the next slice of M1a, not a new phase. | Smallest slice that's independently useful (Claude Desktop today) and fully exercises the guard (proven by the redaction/leak tests in `hacienda-mcp/src/lib.rs`) before adding a second transport |
| **M1b** | HTTP/streamable transport mounted on `hacienda-api` at `/mcp/*` | Additive once M1a's tool-handler logic is proven; reuses the existing `auth_middleware` rather than a new one |
| **M1c** | Resources (§3.4) and the `extract_and_prove` prompt | Polish, not exit-blocking |
| **CLI parity** (§4) | `audit {entries,seals,export,tip}`, `review`, `compliance` subcommands | **Blocked**, not merely independent — see §0's durable-store finding and §4's status line |

No change to the program's existing S/P/E/V wave table (§8 of the program doc) is required — this
track runs beside it, not inside it.

---

## 7. Recap: what stays untouched

- `xberg`'s pinned feature set (`redaction`, `ner`, `tokio-runtime`) — unchanged.
- The pinned commit/fork on `jamon8888/xberg` — unchanged, and this spec introduces no new reason
  to touch it.
- `crates/xberg/src/mcp/`, `crates/xberg/src/api/` — never imported, never vendored, never patched.
- The program's S/P/E/V tracks and wave table — unaffected; M1 and the CLI slice are additive.

The entire deliverable of this spec lives in two new/extended hacienda crates (`hacienda-mcp`,
new; `hacienda-cli`, extended) plus one new capability-reflection test in `hacienda-api`. Net xberg
diff: zero lines.
