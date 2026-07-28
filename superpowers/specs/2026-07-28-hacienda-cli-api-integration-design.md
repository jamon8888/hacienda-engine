# Design Spec: hacienda CLI & HTTP API Integration

**Date:** 2026-07-28
**Status:** Draft — decisions recorded, implementation gated on §8
**Scope:** `hacienda-engine` only. No Studio work, no registry, no LoRA training.
**Supersedes:** the deleted `hacienda/src/cli.rs` and `hacienda/src/api.rs` drafts
(removed in `7feaa97`; they had never compiled on any branch)

---

## 1. Executive Summary

hacienda needs a command-line entry point and an HTTP API. The obvious framing —
"hacienda is xberg plus some compliance commands" — is the one framing that must
be rejected, because it produces a product that leaks the exact data it exists to
protect.

xberg's HTTP API exposes `POST /extract`, which returns document text. Mounting
that router inside hacienda gives every client a documented, OpenAPI-advertised
endpoint that bypasses the PII pipeline entirely. The CLI equivalent, a
`hacienda extract` that mirrors `xberg extract`, writes unredacted content to
disk. In a product sold on GDPR/DORA compliance, the headline endpoint returning
raw personal data is not a rough edge; it is a defect that invalidates the
premise.

**This spec therefore defines hacienda as a redacting proxy over xberg, not a
superset of it.** Every path capable of emitting document content passes through
`HaciendaFacade`. Raw extraction remains reachable, but only as an explicit,
off-by-default, audited opt-in.

---

## 2. Confirmed Decisions

| # | Decision | Value |
|---|----------|-------|
| 1 | Product framing | **Redacting proxy**, not superset. Redaction is the default on every surface. |
| 2 | Raw xberg passthrough | **Off by default.** Optional cargo feature `xberg-passthrough`, mounted at `/xberg/v1`. |
| 3 | Route composition | **Allowlist** — hacienda registers its own routes. No denylist middleware over xberg's table. |
| 4 | CLI reuse of `xberg-cli` | **Impossible** (§3). hacienda owns a small command set; raw xberg work delegates to the `xberg` binary via subprocess. |
| 5 | Crate layout | Four crates: `hacienda-core`, `hacienda`, `hacienda-api`, `hacienda-cli`. |
| 6 | Config precedence | Mirrors xberg exactly: defaults < discovered file < `--config` < `--config-json` < flags. |
| 7 | Deployment target | **Single-node first.** Persistence seams designed now; horizontal scale not built. |
| 8 | Span text in API responses | **Withheld by default.** `include_text` requires capability + is audited. |

Decisions 2 and 7 were open questions at the end of the brainstorm; they are
resolved here as the conservative default and are revisable without
architectural churn, because both are isolated behind a feature flag and a trait
seam respectively.

---

## 3. Verified Ground Truth

Checked 2026-07-28 against working trees at `/home/jamin/Documents/xberg` and
`/home/jamin/Documents/hacienda-engine`. Not assumed.

### 3.1 xberg CLI — not reusable as a library

| Fact | Evidence |
|---|---|
| Binary-only crate | `xberg/crates/xberg-cli/Cargo.toml:18-20` declares `[[bin]] name = "xberg"`; there is **no** `src/lib.rs` and no `[lib]` section |
| `Cli` / `Commands` are private | `xberg/crates/xberg-cli/src/main.rs` — declared without `pub` |
| No reusable dispatcher | All dispatch is an inline `match cli.command` inside `fn main()` |
| clap version | `4.6` (workspace dependency) |

**Consequence:** `use xberg_cli::{Cli, Commands}` cannot work at any version. The
deleted `cli.rs` was built on this premise, which is why it never compiled.
Making it work requires adding a library target upstream — out of scope, since
we do not patch xberg.

### 3.2 xberg HTTP API — exists, nestable, state not injectable

| Fact | Evidence |
|---|---|
| API module | `xberg/crates/xberg/src/lib.rs:64-65` — `#[cfg(feature = "api")] pub mod api;` |
| Feature | `xberg/crates/xberg/Cargo.toml:425` — `api = [ ... ]` |
| Router constructor | `xberg/crates/xberg/src/api/router.rs:53` — `pub fn create_router(config: ExtractionConfig) -> Router` |
| With limits | `router.rs:93` — `pub fn create_router_with_limits(config, limits) -> Router` |
| Framework | axum 0.8, tower-http, utoipa 5.5 (OpenAPI at `/openapi.json`) |

**Registered routes** (`router.rs:169-187`):
`/extract`, `/extract-async`, `/jobs/{job_id}`, `/detect`, `/formats`, `/health`,
`/info`, `/version`, `/cache/stats`, `/cache/clear`, `/cache/manifest`,
`/cache/warm`, `PUT /process`, `/v1/convert/file`, `/openapi.json`.

**Constraint:** `create_router` accepts only `ExtractionConfig` and constructs its
own internal state. There is no seam to inject hacienda state. The router can be
**nested**, never **wrapped**. The deleted `api.rs` assumed
`xberg::api::create_app(state.xberg_state())`; no such function exists.

### 3.3 hacienda-core facade — what exists today

| Fact | Evidence |
|---|---|
| Entry points | `hacienda-core/src/facade.rs:121` `process(ExtractInput)`, `:130` `process_batch(Vec<ExtractInput>)` |
| Redaction is in-place | `process_batch` sets `document.content = result.redacted_text` before returning |
| Glossary ordering is correct | `observe_glossary` runs against original text *before* the overwrite |
| Audit chain | `hacienda-core/src/audit/chain.rs` — `push`, `append`, `verify`, `tip`, `entries` |
| Audit sink trait exists | `hacienda-core/src/audit/sink.rs:11` `AuditSink`, `:23` `FileSink` |
| **Facade does not use it** | `grep -n 'sink' hacienda-core/src/facade.rs` → no matches. Audit is RAM-only. |
| Batch is sequential | `process_batch` awaits `pipeline.process` one document at a time |
| Locks not held across await | Verified — `observe_glossary` / `record_audit` are sync and release internally |

---

## 4. Crate Layout

`hacienda` is today a dependency-light library facade. Adding clap and axum to it
forces both on every library consumer, and enabling `xberg/api` drags in
tower-http and utoipa — undoing the deliberate feature trimming recorded in the
root `Cargo.toml`.

```text
hacienda-core     logic (unchanged)
hacienda          thin lib facade (unchanged — no clap, no axum)
hacienda-api      axum Router over HaciendaFacade          [new]
hacienda-cli      bin "hacienda"; optional dep on hacienda-api  [new]
```

Feature-gating within a single crate is the alternative and is rejected: Cargo
feature unification means one workspace member enabling `cli` pulls clap into
every consumer of the library. This mirrors xberg's own `xberg` / `xberg-cli`
split.

---

## 5. HTTP API

### 5.1 Endpoint triage

Every xberg route classified by whether it can emit document content:

| xberg route | Class | Disposition |
|---|---|---|
| `/extract`, `/extract-async`, `/jobs/{id}`, `PUT /process`, `/v1/convert/file` | Content-bearing | **Re-implement** through the facade |
| `/detect`, `/formats`, `/health`, `/info`, `/version` | Safe | Re-expose (trivial) |
| `/cache/stats`, `/cache/manifest`, `/cache/clear`, `/cache/warm` | Ambiguous — a manifest can leak filenames and content hashes | Privileged; not public |

### 5.2 Composition strategy

hacienda registers its own routes explicitly. It does **not** nest xberg's router
and block paths with middleware.

Rationale: a denylist over someone else's route table fails open the moment
upstream adds a route. xberg is at `1.0.0-rc.37` and still moving. An allowlist
fails closed. This is the `input-validation` rule (allowlists over denylists)
applied to routing.

A useful consequence: re-implementing rather than passing through means
`xberg/api` is **never enabled** in the default build, preserving the trimmed
dependency graph. axum is a direct hacienda-api dependency regardless.

### 5.3 Surface

```text
POST /v1/documents              redacted extraction + entities + audit ids
POST /v1/documents/async        → job id
GET  /v1/jobs/{id}

POST /v1/pii/scan               entities only, no rewrite
POST /v1/pii/redact             redacted text + entities
GET  /v1/pii/config             effective config: thresholds, mode, model provenance

GET  /v1/audit/entries          paginated
GET  /v1/audit/verify           chain integrity
GET  /v1/audit/export           ?format=json|csv|jsonl

GET  /v1/review                 ?status=
POST /v1/review/{id}/assign
POST /v1/review/{id}/decision

GET  /v1/compliance/dpia
GET  /v1/compliance/model-card
GET  /v1/compliance/checklist
POST /v1/compliance/dora        incident → report

GET  /v1/glossary

GET  /health  /info  /version  /openapi.json
```

Under `--features xberg-passthrough` only:

```text
*    /xberg/v1/*                raw xberg router, UNREDACTED
```

The prefix is deliberately not `/v1`. It should read as dangerous at the call
site, and it must be absent from the default OpenAPI document.

### 5.4 Two response-shape rules

**`/v1/pii/scan` is itself a PII exposure surface.** Returning the matched span
*text* hands back precisely what the product suppresses. Default response carries
category, byte offsets, confidence and source — not text. Span text requires
`include_text=true`, a capability grant, and produces an audit entry of its own.

**Every content-bearing response carries the audit chain tip.** `AuditChain::tip()`
already exists, so this is nearly free, and it lets a client prove which
configuration and which chain state produced a given output. This is the DORA
evidence story.

---

## 6. CLI

### 6.1 Strategy

`xberg-cli` cannot be imported (§3.1). The options are re-implement or delegate.

Re-implementing Extract/Batch/Cache/Embed/Chunk/Completions means tracking a
pre-1.0 upstream's entire flag surface indefinitely, for commands that are not
hacienda's value-add. Instead hacienda owns a small compliance-shaped command
set. For genuinely raw xberg work, `hacienda xberg -- <args>` execs the `xberg`
binary as a subprocess: no compile-time coupling, no version-skew breakage, and
the raw operation is visibly raw.

### 6.2 Commands

```text
hacienda extract <input>...      redacted by default
    --mode mask|hash|pseudonymize    --threshold <f32>
    --model-dir <dir>  --lora-dir <dir>
    --format text|json               --audit-out <path>
    --no-redact                      refuses without explicit acknowledgement

hacienda scan <input>...         detect only, no rewrite
hacienda audit      verify | export | show
hacienda review     list | assign | decide
hacienda compliance dpia | model-card | dora | checklist
hacienda glossary
hacienda serve                   the API of §5
hacienda config     show | validate    effective config + provenance per value
hacienda completions
hacienda xberg -- <args>         subprocess passthrough, unredacted
```

The polarity is inverted from xberg on purpose: `xberg extract` yields text,
`hacienda extract` yields *redacted* text, and anything else must be asked for
loudly.

### 6.3 Configuration

Precedence mirrors xberg (`xberg/crates/xberg-cli/src/commands/config.rs:30-53`):

```text
defaults < discovered hacienda.{toml,yaml,json} < --config < --config-json < flags
```

`HaciendaConfig` already composes the stage configs with `#[serde(default)]`, so
a single file maps 1:1 to what exists today:

```toml
[extraction]   # xberg ExtractionConfig
[pii]          # PipelineConfig — regex_first, thresholds, model dir, LoRA dir
[pii.redaction]
[pii.audit]
[review]
[compliance]
[glossary]
```

`hacienda config show` must report **where each value came from**. Silent config
precedence is the most common source of "why is this not redacting" incidents.

---

## 7. Security Model

Distinct privileges, to be designed once rather than retrofitted per endpoint:

| Capability | Guards |
|---|---|
| `documents:process` | Normal extraction + redaction |
| `pii:reveal` | `include_text=true` on scan; raw span access |
| `audit:read` / `audit:export` | Audit trail access |
| `review:decide` | Approving or rejecting detections |
| `raw:extract` | `/xberg/v1/*` when passthrough is compiled in |

Every use of `pii:reveal` and `raw:extract` writes an audit entry. A compliance
product whose own audit trail omits "who looked at the unredacted data" is not
credible.

---

## 8. Blocking Gaps

Preconditions, not design questions. Ordered by severity.

1. **Audit is RAM-only.** `AuditSink` and `FileSink` exist
   (`audit/sink.rs:11,23`) but `facade.rs` never references them. A restart
   discards the entire tamper-evident chain. Acceptable for a one-shot CLI;
   a compliance defect for a long-running server, since DORA requires a durable
   audit trail. **Blocks `hacienda serve`.**

2. **No persistence seam for review state.** `ReviewQueue` is in-memory. Review
   decisions are the human-in-the-loop evidence; losing them on restart defeats
   the purpose. Needs a `ReviewStore` trait alongside `AuditSink`.

3. **Batch processing is sequential.** `process_batch` awaits each document in
   turn. Fine for CLI, and it is the throughput ceiling for a server.

4. **The audit chain is a global write lock.** Every document serializes through
   one mutex. Correct today — locks are not held across `.await`, so there is no
   `!Send` future or deadlock — but it is where concurrency will queue.

5. **`record_audit` locks twice** in consecutive statements (once for
   `config_hash()`, once for the guard). Safe, because the first temporary drops
   at the semicolon, but fragile enough to trip a later editor.

6. **No authn/authz exists at all.** §7 is unimplemented.

---

## 9. Phasing

Every gap in §8 maps to a phase. A gap with no phase is a gap nobody owns.

| Phase | Deliverable | Closes | Gated on |
|---|---|---|---|
| 1 | `AuditSink` + `ReviewStore` wired into the facade; file-backed default. Collapse the `record_audit` double lock while in the file. | Gaps 1, 2, 5 | — |
| 2 | `hacienda-cli` with `extract`, `scan`, `config show` | — | Phase 1 |
| 3 | Capability model of §7 + authn/authz middleware | Gap 6 | — |
| 4 | `hacienda-api` content endpoints + `hacienda serve`, allowlist only | — | Phases 1, 3 |
| 5 | Audit / review / compliance endpoints | — | Phases 3, 4 |
| 6 | Concurrent batch; reduce audit lock contention | Gaps 3, 4 | Phase 4 **and** a measurement showing it matters |
| 7 | `xberg-passthrough` feature | — | Demonstrated demand |

Three ordering choices are deliberate:

**Phase 2 before Phase 4.** The CLI validates the configuration model against
real use with no server concerns in the way.

**Phase 3 before Phase 4.** §7 says the privilege model should be designed once
rather than retrofitted per endpoint; shipping endpoints first guarantees the
retrofit. The content endpoints already need `documents:process`, so there is no
version of Phase 4 that legitimately predates it.

**Phase 6 last, and gated on measurement.** Gaps 3 and 4 are throughput
concerns, not correctness ones — `process_batch` is sequential and the audit
chain serializes on one mutex, but both are *correct*. Optimising before Phase 4
provides a load surface to measure against is guesswork. The gate is a
benchmark, not a hunch.

---

## 10. Non-Goals

| Deferred | Reason |
|---|---|
| Horizontal scaling / shared state backend | Decision 7. Seams exist; no demand yet. |
| Re-implementing xberg's Embed / Chunk / Cache commands | Not hacienda's value-add; use `xberg` directly |
| MCP server (`hacienda mcp`) | xberg has an `mcp` feature; a redacted-tool analog is plausible but unscoped |
| Adding a library target to `xberg-cli` upstream | We do not patch xberg |
| LoRA registry, adapter CRUD endpoints | Covered by the LoRA plans under `docs/superpowers/plans/`, whose phase numbering is independent of §9 |

---

## 11. Open Risks

- **The Candle NER path is stub-tested only.** `PiiPipeline` is covered by test
  doubles; loading real GLiNER2 weights with a LoRA adapter end-to-end is
  unverified. An API that advertises model-backed detection while silently
  running regex-only would be a worse failure than not shipping it. `GET
  /v1/pii/config` must report whether a model is actually loaded
  (`PiiPipeline::has_model()`).
- **xberg is `1.0.0-rc.37`.** Re-implemented endpoints will drift from upstream
  behaviour. Accepted: the alternative is passing through unredacted content.
- **`apps/hacienda-studio` is the natural first API consumer** — its
  `worker/pipeline.ts` currently processes client-side. Its actual contract has
  not been read; the §5.3 surface is not yet validated against it.
