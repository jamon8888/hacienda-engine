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
| 7 | Deployment target | **Horizontally scalable** (revised 2026-07-28, §12). Segmented audit chains; stateless facade. |
| 8 | Span text in API responses | **Withheld by default.** `include_text` requires capability + is audited. |
| 9 | xberg dependency | **Pinned to release tag `v1.0.2`** via git, not a path dep (§11). |

Decision 2 remains the conservative default and is isolated behind a feature
flag. **Decision 7 was originally "single-node first" and is now reversed** —
§12 gives the architecture. The reversal is affordable precisely because the
persistence seams had not yet been built the wrong way.

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

7. **`Pseudonymize` does not pseudonymise** — `redaction/engine.rs:93` emits a
   per-category constant, and it is the **default** mode. Correctness, compliance
   and scaling defect in one. Full analysis in §12.3.

8. **The upstream adapter cache is unbounded** — `CANDLE_BACKEND_CACHE` never
   evicts, so per-tenant LoRA adapters accumulate merged weights for process
   lifetime. Cannot be fixed upstream; must be bounded by routing (§12.4).

---

## 9. Phasing

Every gap in §8 maps to a phase. A gap with no phase is a gap nobody owns.

| Phase | Deliverable | Closes | Gated on |
|---|---|---|---|
| 0 | Real pseudonymisation via deterministic AEAD (§12.3); rename or remove the misleading default | Gap 7 | — |
| 1 | Segment model + `AuditStore` / `ReviewStore` / `JobStore` traits; in-memory and file backends. Collapse the `record_audit` double lock while in the file. | Gaps 1, 2, 5 | — |
| 2 | `hacienda-cli` with `extract`, `scan`, `config show`, `--concurrency` | Gap 3 | Phases 0, 1 |
| 3 | Capability model of §7 + authn/authz middleware | Gap 6 | — |
| 4 | `hacienda-api` content endpoints + `hacienda serve`, allowlist only | — | Phases 1, 3 |
| 5 | Audit / review / compliance endpoints; `hacienda audit verify` across segments | — | Phases 3, 4 |
| 6 | Postgres store backend; `--shard`; adapter-aware routing | Gaps 4, 8 | Phase 4 **and** a measurement showing it matters |
| 7 | `xberg-passthrough` feature | — | Demonstrated demand |

Three ordering choices are deliberate:

**Phase 2 before Phase 4.** The CLI validates the configuration model against
real use with no server concerns in the way.

**Phase 3 before Phase 4.** §7 says the privilege model should be designed once
rather than retrofitted per endpoint; shipping endpoints first guarantees the
retrofit. The content endpoints already need `documents:process`, so there is no
version of Phase 4 that legitimately predates it.

**Phase 6 last, and gated on measurement.** Gap 4 is a throughput concern, not a
correctness one — the audit chain serialises on one mutex, but it is *correct*.
Optimising before Phase 4 provides a load surface to measure against is
guesswork. The gate is a benchmark, not a hunch.

**Phase 0 is new and comes first.** Gap 7 is not a scaling issue that can wait
for Phase 6: pseudonymisation is the *default* redaction mode, so every document
processed before it is fixed is redacted by a control that does not match its
own name — and §12.3's derivation is what makes redaction consistent across
nodes in the first place. Building the store layer on top of a placeholder
pseudonymiser would mean migrating already-redacted corpora later.

Note the ordering constraint this creates: **the segment model (Phase 1) must
land before anything writes audit records at scale**, because retrofitting
segment identity onto an existing flat chain means rewriting history, which is
precisely what a tamper-evident log is designed to make impossible.

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

## 11. xberg Dependency & Version Policy

### 11.1 What changed

Verified 2026-07-28 against tags in the sibling checkout, not assumed.

| Claim | Finding |
|---|---|
| "latest is rc.42" | **Outdated.** Tags run `rc.38 … rc.42`, then **`v1.0.1`, `v1.0.2`**. `origin/main` is at `v1.0.2` (0 commits ahead). xberg is now a stable 1.x. |
| "new LoRA implementation" | **Not in any release.** `git diff v1.0.0-rc.37 v1.0.2 -- crates/xberg-gliner/src/candle/` is **empty**. PEFT load, `merge_into_base()`, and the base-model-name guard are byte-identical. |
| What actually changed | A **cargo feature split** in `crates/xberg/src/text/ner/candle.rs` (32 lines, all `#[cfg]` gates). |

The split:

```toml
ner-candle-backend = ["ner", "dep:xberg-gliner", "xberg-gliner/candle"]  # no tokio
ner-candle         = ["ner-candle-backend", "tokio-runtime"]            # unchanged meaning
ner-candle-wasm    = ["ner-candle-backend"]                             # new
```

Every gate moved from `feature = "ner-candle"` to `feature = "ner-candle-backend"`,
and the `block_in_place` call in `CandleBackend::detect` is now additionally gated
on `feature = "tokio-runtime"` — so a no-tokio build calls inference directly
instead of through the blocking pool.

**The significance is where LoRA can now run, not how it works.** `ner-candle-wasm`
makes adapter-merged GLiNER2 buildable without a tokio runtime, which is what a
browser target needs. For `apps/hacienda-studio`, whose entire premise is that
document content never leaves the browser, that is the enabling change.

Non-breaking for hacienda: `ner-candle`, `ner`, `redaction`, and `tokio-runtime`
all still exist with the same meaning. `NerBackend`, `CandleBackend::get_or_init`,
`ExtractInput`, `ExtractionConfig`, `ExtractionResult` and `api::create_router`
are unchanged.

**hacienda must continue to enable `ner-candle`, not `ner-candle-backend`.** The
latter omits `tokio-runtime`, which silently changes inference from
`block_in_place` to a direct call — CPU-bound work on the async runtime, which
the `async-and-concurrency` rule forbids.

### 11.2 Why a git tag instead of a path dependency

The sibling checkout at `../xberg` is a working tree, not a release: it sits on
rc.37 and carries 37 uncommitted changes, one of which adds an `xberg::pii`
module re-exporting the very `pii_*` crates this repo vendored away from.
Building against it made hacienda's output a function of someone's unstaged work,
which is not a reproducible build and not something CI can reproduce at all.

```toml
xberg = { git = "https://github.com/xberg-io/xberg.git", tag = "v1.0.2", features = [...] }
```

`Cargo.lock` now pins commit `9dcc864d8591d30f8039266e0bb3e82ec3918556`. For
local co-development, override without editing the manifest via `[patch]` in
`.cargo/config.toml`.

Upgrade policy: pin to a release tag; bump deliberately; re-verify the four
touched surfaces (`NerBackend`, `get_or_init`, extraction types,
`create_router`) on every bump.

---

## 12. Horizontal Scaling

Reverses the original Decision 7. §8's gaps are no longer just durability
problems — they are the scaling architecture.

### 12.1 The hard constraint: a hash chain is sequential

`AuditChain` computes `entry.hash = H(prev_hash || entry)`. That is what makes it
tamper-evident, and it is also why two nodes cannot append concurrently: both
need the same `prev_hash`. Serialising every node through one writer would
reintroduce the bottleneck and add a single point of failure.

**Resolution: segmented chains.** Each writer owns its own chain segment.

```text
Segment {
  segment_id, node_id, config_hash,
  prev_segment_tip: Option<Hash>,   // links this node's previous segment
  entries: [...],                   // internally hash-chained (unchanged)
  sealed_tip: Hash,
  opened_at, sealed_at,
}
```

Verification becomes three independent checks:

1. Each segment verifies internally — `AuditChain::verify()`, unchanged.
2. Each node's segments link: `prev_segment_tip == previous.sealed_tip`.
3. Periodic **anchors** record a merkle root over all live segment tips,
   giving a global tamper-evident checkpoint without serialising the write path.

This is defensible because DORA and GDPR require that a record exists, is
unaltered, and can be produced — **not** that all records share one global total
order. Buying a total order at the cost of a serialised writer trades a real
property for a legally unnecessary one.

The payoff: **a CLI run and an API replica become the same kind of writer.** A
laptop invocation and a Kubernetes pod both emit segments, and one
`hacienda audit verify` covers both.

### 12.2 State inventory

| State | Today | Scaled |
|---|---|---|
| Audit chain | `Mutex<AuditChain>`, RAM-only | Segment per writer behind `AuditStore` |
| Review queue | in-memory | `ReviewStore`; transitions are compare-and-swap |
| Glossary | `Mutex<BTreeMap>` | Grow-only map — per-node accumulate, merge on read |
| Async job state | does not exist | `JobStore` |
| Pseudonym mapping | does not exist (§12.3) | Derived, never stored |
| Model + adapter cache | per-process, unbounded (upstream) | Adapter-aware routing (§12.4) |

`HaciendaFacade` holds `Arc<dyn AuditStore>` and friends instead of mutexes.
Replicas become interchangeable: no sticky sessions, no leader election on the
hot path. Readiness gates on model loaded **and** stores reachable.

`ReviewQueue::assign` already refuses to act on an item that is no longer
pending — that is a compare-and-swap, and it maps directly onto a conditional
`UPDATE ... WHERE status = 'pending'`. The semantics survive the move.

### 12.3 Pseudonymisation must be stateless — and today it is not implemented

Once two nodes redact the same person, they must produce the same token, or the
output is worthless: you could no longer tell whether two mentions co-refer.

**But `RedactionMode::Pseudonymize` does not pseudonymise.**
`redaction/engine.rs:93` is:

```rust
RedactionMode::Pseudonymize => format!("[{:?}:****]", entity.category).to_uppercase(),
```

Every email becomes the identical `[EMAIL:****]`. No mapping store exists
anywhere in the crate. This is masking, and **it is the default mode**
(`redaction/types.rs:53`).

That matters beyond naming. Under GDPR Art. 4(5) pseudonymisation is a specific
measure with specific consequences — pseudonymised data remains personal data
and is recognised under Art. 32. Irreversible masking is a different measure with
a different legal character. A DPIA generated by §5.3's `/v1/compliance/dpia`
would describe a control the code does not implement. It also contradicts
`2026-07-28-hacienda-studio-client-app-design.md`, which promises **reversible**
pseudonymisation with a keyfile — unachievable when every value of a category
collapses to one token.

The fix is also what makes it scale. Derive, never store:

```text
token = base32( AES-SIV( tenant_key, category || normalize(text) ) )[..n]
```

Deterministic authenticated encryption (SIV / synthetic-IV AEAD) is stable across
nodes with no shared table, and reversible with the key — satisfying the Studio
promise and cross-node consistency with the same construction. Deterministic
encryption leaks equality of plaintexts, which here is precisely the property
wanted. An HMAC variant is simpler if reversibility is dropped, but the Studio
spec says it must not be.

The tenant key is configuration and a secret, not runtime state, so it does not
reintroduce coordination.

### 12.4 Adapter-aware routing — where LoRA meets scaling

xberg caches merged backends per process:

```rust
static CANDLE_BACKEND_CACHE: LazyLock<RwLock<AHashMap<(PathBuf, Option<PathBuf>), Arc<CandleBackend>>>>
```

**This map has no eviction.** Every `(model, adapter)` pair ever requested keeps
merged GLiNER2 weights resident for the life of the process. That is correct for
a CLI and for a server with one adapter. It is a memory leak the moment there are
per-tenant LoRA adapters and a round-robin load balancer, because every replica
eventually loads every adapter.

We do not patch xberg, so hacienda bounds it from outside:

- **Route by adapter.** Rendezvous-hash `adapter_id` to a small subset of
  replicas (k > 1 for availability). Each node then holds a bounded working set
  instead of the union of all tenants.
- **Admission control**: cap distinct adapters per process; reject or redirect
  past the cap rather than swelling.
- **Process recycling** as a backstop — blunt, but it reclaims merged weights.
- **Push inference to the client** where possible: `ner-candle-wasm` (§11.1)
  removes server-side adapter pressure entirely for the Studio path.

Adapter affinity is routing, not session state; replicas stay interchangeable for
everything else.

### 12.5 CLI

"Scaling a CLI" means two distinct things:

- **Within one invocation** — a worker pool over documents, replacing the
  sequential loop in `process_batch` (gap 3). `--concurrency N`, default CPU
  count.
- **Across invocations** — `--shard i/N` for embarrassingly parallel runs on
  many machines. Each writes its own segment; `hacienda audit verify` reconciles
  them afterwards. `--node-id` labels segments, defaulting to
  hostname + pid + run uuid.

### 12.6 Deployment

| Concern | Position |
|---|---|
| Store backend | Postgres for segments, review, jobs. Sealed segments may go to object storage. |
| Model weights | Baked into the image or a shared read-only volume; readiness waits on load. |
| Sticky sessions | Not required — adapter affinity is routing. |
| Scaling signal | Queue depth for async jobs; per-adapter concurrency for sync. |

---

## 13. Open Risks

- **The Candle NER path is stub-tested only.** `PiiPipeline` is covered by test
  doubles; loading real GLiNER2 weights with a LoRA adapter end-to-end is
  unverified. An API that advertises model-backed detection while silently
  running regex-only would be a worse failure than not shipping it. `GET
  /v1/pii/config` must report whether a model is actually loaded
  (`PiiPipeline::has_model()`).
- **xberg is pinned at `v1.0.2`.** Re-implemented endpoints will drift from
  upstream behaviour. Accepted: the alternative is passing through unredacted
  content.
- **`apps/hacienda-studio` is the natural first API consumer** — its
  `worker/pipeline.ts` currently processes client-side. Its actual contract has
  not been read; the §5.3 surface is not yet validated against it.
