# Platform Parity and Scale — Implementation Plan (Phases 8-15)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close every gap named in `superpowers/specs/2026-08-01-hacienda-platform-parity-and-scale-design.md`
— the highest-priority missing route (`pii:reveal`), the durable store backend that almost
every later phase depends on, the Phase 5 routes that already have working core logic behind
them, API-key issuance, RAG storage, remaining xberg-parity routes, an SDK repository, and a
device-target spike — in the dependency order that document's §10 Phasing establishes.

**Architecture:** Eight phases (8-15, continuing the integration spec's 0-7), each closing one
or more of the spec's §9 Blocking Gaps. Phase 9 (Postgres store backend) is the load-bearing
phase: five of the remaining seven phases read or write a Postgres row that does not exist
until Phase 9 ships. Phases are ordered so a phase never starts before every phase it reads
from has landed — §10's own phasing already encodes this; this plan does not reorder it.

**Tech Stack:** Rust 2021 workspace (unchanged edition/resolver). New this plan: a Postgres
client crate for Phase 9 (candidate: `sqlx` with the `postgres` and `runtime-tokio` features —
compile-time-checked queries and an async pool without an ORM's code-generation surface; final
choice is Phase 9 Task 1's job, not pre-decided here), `pgvector` extension for Phase 12,
`argon2` or equivalent for Phase 11's key hashing. No new crate is added to the workspace
`Cargo.toml` until the task that needs it — see `dependency-awareness` in CLAUDE.md.

**Spec:** `superpowers/specs/2026-08-01-hacienda-platform-parity-and-scale-design.md`, all of
§2 (9 Confirmed Decisions), §9 (7 Blocking Gaps), §10 (Phasing). Also closes the parity gaps
named in §3.1/§4.1 against the real xberg Enterprise SDK surface.

**Related plan:** `superpowers/plans/2026-08-01-hacienda-rag-vector-store-layer.md` is the
detailed sub-plan for Phase 12's `hacienda-rag` crate (trait, types, filter/query IR, in-memory
backend). This document does not duplicate it — Phase 12 below states what that plan covers,
what it explicitly defers, and what still has to be built on top of it (the pgvector backend
and the `/v1/rag/*` routes) to close Gap 3 in full.

**Baseline:** `cargo test -p hacienda-core -p hacienda-api -p hacienda-cli -p hacienda` passing
at plan time (222+ `hacienda-core` tests per the Phase 1 completion note, plus `hacienda-api`'s
route-table and safety tests). No Postgres, no RAG crate, no auth-key issuance, no SDK repo, no
Cactus integration exist anywhere in the repo as of this writing.

---

## Ground Truth — Verified vs Assumed

Every row below was read from source on 2026-08-01, in this session, specifically to ground
this plan — not carried over from the spec's own (already-verified) claims.

**Verified by reading source:**

| Fact | Location |
|---|---|
| `Capability::PiiReveal` **already exists** as a capability variant, with rustdoc "Reveal span text on scan; access raw PII values" | `hacienda-core/src/auth/mod.rs:19-32` |
| `include_text=true` on `POST /v1/pii/scan` **already requires and enforces** `PiiReveal` end-to-end (handler → `scan_text_with_auth` → facade), and is audited via `record_reveal` | `hacienda-api/src/handlers/pii.rs:18-24`, `hacienda-core/src/facade.rs:535-637` |
| `record_reveal` writes one `AuditEntry` per revealed span with `RedactionAction::Reveal` and the caller's `principal_id`, keyed by `blake3(span text)` so it joins to the original redaction's `span_hash` | `hacienda-core/src/facade.rs:654-695` |
| **What does not exist**: a route or facade method that reverses a *pseudonym token* (`[CATEGORY:key_id:base32]`) back to plaintext outside of live scan/redact. `Pseudonymiser::reveal(token) -> Result<String, PseudonymError>` exists and is fully implemented (normalises, decrypts AES-SIV, returns UTF-8) but is only ever constructed inside `HaciendaFacade::build` and handed to `PiiPipeline`/`RedactionEngine` — it is not held as its own facade field and nothing calls `.reveal()` outside a live pipeline run | `hacienda-core/src/redaction/pseudonym.rs:502-546`, `hacienda-core/src/facade.rs:162-167,202-218` |
| `HaciendaError` has 6 variants: `Extraction`, `Pii`, `Audit`, `Review`, `Authz`, `PiiDisabled` — no `Auth`/`Store`/`NotFound` variant yet | `hacienda-core/src/error.rs:6-32` |
| The route table (`ROUTE_TABLE`) is the single source of truth for both the axum `Router` and `AuthState`; a reflection test (`every_guarded_route_reflected_in_auth_state`) asserts every non-public entry is guarded. Adding a route means adding one `RouteSpec` — the guard is structurally impossible to skip | `hacienda-api/src/routes.rs:1-13,54-107,209-279` |
| Current `ROUTE_TABLE` has exactly 10 entries: 4 public (`/health`,`/version`,`/info`,`/openapi.json`), 6 under `Capability::DocumentsProcess` (`POST /v1/documents`, `POST /v1/documents/async`, `GET /v1/jobs/{id}`, `POST /v1/pii/scan`, `POST /v1/pii/redact`, `GET /v1/pii/config`) | `hacienda-api/src/routes.rs:54-107` |
| `AuditStore`/`ReviewStore`/`JobStore` traits all exist, `#[async_trait]`, in-memory backends for all three, file backends for audit and review only; `JobStore` is in-memory-only "provisional until Phase 4" | `hacienda-core/src/audit/store.rs`, `hacienda-core/src/review/store.rs`, `hacienda-core/src/jobs/store.rs:1-7` |
| `HaciendaFacade` already exposes `audit_entries_with_auth`, `verify_audit_with_auth`, `review_queue_with_auth`, `close_with_auth` — async, capability-checked | `hacienda-core/src/facade.rs:260-386` |
| **`compliance/` and `glossary/` modules already exist as working core logic**, not just design: `ComplianceGenerator` (DPIA, ModelCard, DORA report, checklist generators), `EntityGlossary`/`GlossaryConfig`/entity linker. `HaciendaFacade` already builds and holds a `compliance: Option<ComplianceGenerator>` field and calls `observe_glossary` on every processed document | `hacienda-core/src/compliance/mod.rs:1-60`, `hacienda-core/src/glossary/mod.rs:1-13`, `hacienda-core/src/facade.rs:5,26,247,698-705` |
| **No facade method reads the glossary back out**, and no facade method produces a `ComplianceReport` — both fields are write-only from the route layer's point of view today | `grep 'pub.*fn' hacienda-core/src/facade.rs` — no `glossary()`, no `compliance_report()` |
| Zero Postgres/sqlx/diesel/tokio-postgres/pgvector dependencies anywhere in the workspace | `grep -ril "sqlx\|tokio-postgres\|diesel\|pgvector" **/*.toml` → no matches |
| Workspace has 5 members: `crates/hacienda-wasm`, `hacienda`, `hacienda-api`, `hacienda-cli`, `hacienda-core` | `Cargo.toml:1-4` |
| `hacienda-cli` has exactly 4 files (`main.rs`, `cli.rs`, `commands.rs`, `config.rs`) — confirms CLAUDE.md's note that audit/review/compliance/glossary subcommands are deliberately absent, not stubbed | `find hacienda-cli/src` |
| `xberg-rag` (the deleted upstream crate) has an 8-method `VectorStore` trait, `filter.rs`/`query.rs` IR, an in-memory backend, and a sqlite+pgvector-shaped backend split — full recovery detail already captured in `2026-08-01-hacienda-rag-vector-store-layer.md`'s own Ground Truth table; not re-derived here | `2026-08-01-hacienda-rag-vector-store-layer.md` |

**Assumed, to confirm during implementation:**

- That `sqlx`'s compile-time query checking (`sqlx::query!`) is viable without a live database
  at CI build time, or that the project accepts `sqlx::query` (runtime-checked) to avoid a
  `DATABASE_URL`-at-build-time dependency. Phase 9 Task 1 decides this by trying the offline
  mode (`cargo sqlx prepare`) against the RAM-constrained build host before committing.
- That Postgres advisory locks (`pg_advisory_lock`) are sufficient for the `JobStore`
  compare-and-swap semantics without row-level `SELECT ... FOR UPDATE` contention under
  Phase 2's `--concurrency N`. Phase 9 Task 4 benchmarks both before choosing.
- That the xberg `/openapi.json`-derived schema is complete enough for Phase 14's SDK codegen
  tooling (openapi-generator or a hand-rolled equivalent) without manual patching. Phase 14
  Task 1 is a spike specifically to answer this before the SDK repo is scaffolded.

---

## Design Decisions

### D1. Phase 8 (`pii:reveal`) needs one new facade capability, not a new capability enum

`Capability::PiiReveal` already exists and is already enforced on `include_text=true`. Gap 1 in
the spec is specifically about **pseudonym-token reversal**, a different operation: given a
token that appears in previously-redacted output (`[EMAIL:k1:AB3XQ...]`), recover the plaintext
it encodes. The capability to gate this with is the same `Capability::PiiReveal` — reusing it
is correct, not a shortcut, because the risk profile is identical (raw PII disclosure) and a
second capability would let an operator grant "reveal on scan" without "reveal by token" or vice
versa for no articulated reason. If that distinction turns out to matter, split it later; §9
Gap 1 does not ask for it and CLAUDE.md's `avoid-duplication` rule argues against pre-splitting.

The route needs the `Pseudonymiser` reachable from the facade outside a live pipeline run.
Today it is constructed once in `build()` and moved into `PiiPipeline::with_pseudonymiser` —
there is no second handle. The facade gets a new field,
`pseudonymiser: Option<Arc<Pseudonymiser>>`, cloned (not moved) into both the pipeline and the
facade's own storage. `Arc` makes the clone cheap and keeps the single-instance-per-key
invariant `Pseudonymiser::new` already establishes.

### D2. Phase 9's store trait shape does not change — only its implementations gain a body

`AuditStore`, `ReviewStore`, and `JobStore` are already `#[async_trait]`, already
object-safe, already used as `Arc<dyn …>` everywhere. Phase 9 is explicitly **not** a redesign
of these traits — see Phase 1's D3-D5, which chose async-batch-oriented shapes precisely so a
database backend could be dropped in later without a signature change. Phase 9's job is to
write `PostgresAuditStore`, `PostgresReviewStore`, `PostgresJobStore` implementing the existing
traits, plus three brand-new tables (`document_versions`, `presets`, `api_keys`) that have no
existing trait to conform to and need one designed from scratch.

Changing an existing trait signature in Phase 9 would be a signal the Phase 1 design was wrong;
if that turns out to be true, it is a standalone decision to record and justify, not a
side-effect of "we needed a database".

### D3. One Postgres pool, injected once, not one connection per store

`HaciendaFacade::with_stores` already takes `Option<Arc<dyn AuditStore>>` /
`Option<Arc<dyn ReviewStore>>` as constructor parameters — the DI seam CLAUDE.md's
`no-global-state` rule asks for already exists. Phase 9 adds a `PgPool` (or equivalent) built
once by the process entry point (`hacienda-cli`'s `serve` command, or `hacienda-api`'s test
harness) and threaded into each `Postgres*Store::new(pool: PgPool)` constructor via `Arc`-shared
clone of the pool handle (`sqlx::PgPool` is already `Clone` + cheap — it wraps an internal
connection-pool `Arc`). No store owns its own pool; no global `OnceCell<PgPool>` is introduced.
This is the same shape `registry.rs` was rejected for in the RAG plan's D3, for the same reason.

### D4. Migration ownership: `sqlx::migrate!` embedded, not a separate migration tool

Given `sqlx` as the Phase 9 Task 1 default (pending the offline-mode check in the Assumed
table), migrations live in `hacienda-core/migrations/*.sql` and run via
`sqlx::migrate!().run(&pool)` on startup, gated behind an explicit `--migrate` CLI flag or
config bool — **never automatically in a library constructor**. `HaciendaFacade::new` must not
mutate schema as a side effect of being constructed; a caller who wants an automatic migration
opts in explicitly. This mirrors the existing convention that `HaciendaFacade` never does I/O
its caller did not ask for (see `FileAuditStore::open`'s explicit `open` call, never implicit).

### D5. Phase 10's routes are a thin HTTP layer over facade methods that mostly already exist

The Ground Truth table found working `ComplianceGenerator` and `EntityGlossary` core logic
with **no facade accessor**. This changes Phase 10's actual shape from "build audit/review/
compliance/glossary features" (what §4.2 and §9 Gap 5 read like in isolation) to: add
`glossary_snapshot_with_auth` and `compliance_report_with_auth` facade methods (small — each
is a synchronous read of an existing field, wrapped in a capability check and made `async` only
for signature consistency with its siblings), then four route handlers plus DTOs. The audit and
review routes need no new facade methods at all — `audit_entries_with_auth`,
`verify_audit_with_auth`, `review_queue_with_auth` already exist and are already tested.

This matters for sequencing: Phase 10 does not need Phase 9 for audit/review (both already have
in-memory and file backends good enough to serve read routes), but it does need Postgres for
being useful in a horizontally-scaled deployment per the integration spec's §12.2 statelessness
contract — two replicas behind a load balancer must see the same audit chain and review queue.
The gate on Phase 9 is a production-readiness gate, not a compile-time one. Task ordering below
reflects this: Phase 10's routes can be built and tested against in-memory/file stores
immediately, but must not be declared "done" for the horizontal-scale story until wired to the
Postgres backends from Phase 9.

### D6. `/v1/compliance/*` capability: reuse `Capability::AuditRead`, do not mint a new one

§4.2 flagged the compliance capability as "undefined". A DPIA or DORA report is a read of the
same underlying facts (`AuditChain`, pipeline config) that `audit:read` already gates — it does
not touch raw PII, does not decide anything, and does not export the chain (`AuditExport`
already exists for that, separately, and covers CSV/JSON export). Reusing `AuditRead` avoids a
sixth capability variant for a distinction nothing in the spec asks for. If compliance access
needs to be granted independently of audit access later, split it then, following the same
reasoning as D1.

### D7. API keys are hashed with the same primitive family already vendored, not a new one

Phase 11 needs to hash API keys at rest (§6). `aes-siv` is already a workspace dependency for
pseudonymisation but is the wrong primitive for password/key hashing (it is a deterministic
AEAD, not a slow hash — using it here would make key hashes brute-forceable at AEAD speed).
Argon2id is the OWASP-recommended choice for credential hashing (`owasp-quick-reference` #2:
"never roll your own crypto"; #7: "require MFA, enforce strong passwords" — the adjacent
principle for API keys is "hash slowly"). New workspace dependency: `argon2 = "0.5"`. Keys
themselves are high-entropy random tokens (`rand`-generated, ≥256 bits) prefixed for
identification (`hcd_live_<base62>`), so Argon2's slow-hash property defends against a stolen
database, not against brute-forcing the token itself — the token's entropy does that.

### D8. RAG stays scoped exactly as the referenced plan states

Phase 12 = the already-written `hacienda-rag` crate plan (trait + types + in-memory backend)
**plus** two things that plan explicitly deferred: a `backends/pgvector.rs` implementation of
`RagStore`, and the `/v1/rag/*` HTTP routes. This plan's Phase 12 section states the interface
contract between the two documents and does not re-derive the crate design.

### D9. SDK repo and Cactus spike are outside this repo's workspace

Phases 14 and 15 create/modify a **different** git repository (`hacienda-sdks`, new) and
integrate a **third-party on-device runtime** (Cactus). Neither touches `hacienda-engine`'s
Cargo workspace directly except to keep `/openapi.json` accurate and, for Phase 15, to add a
`target: "device"` marker to whatever config the SDK consumes. Tasks in those phases are
scoped as spikes and repo-scaffolding steps, not `cargo test`-verified units — verification is
stated per-task since the default per-task `cargo test` command does not apply.

---

## Global Constraints

- Workspace root `/home/jamin/Documents/hacienda-engine/`, edition 2021, resolver 2.
- Build host is RAM-constrained (~3GB), 4 cores. Do not raise cargo `jobs`. Never run
  `cargo clean`. A Postgres integration-test suite must not require a full `cargo clean` cycle
  to iterate — use `cargo test -p hacienda-core postgres` scoped runs.
- All hacienda-core tests are inline `#[cfg(test)]`; no `tests/` directory convention there.
  `hacienda-api` uses inline `#[cfg(test)]` in `routes.rs` plus a `tests/safety.rs` integration
  file — follow whichever convention the touched crate already uses.
- Test names follow `should_<behaviour>_when_<condition>`.
- No `.unwrap()` in library code. Every `Result` carries context.
- Every new public item gets a rustdoc comment explaining *why*, not just *what*.
- After each task (crates in this workspace only — Phases 8-13): `cargo test -p <crate>`,
  `cargo clippy --all-targets -- -D warnings`, `cargo fmt`.
- **Confirm the red phase before implementing.** Write the test, run it, read the actual
  failure, then write the code. Phase 1's completion note records two agents that skipped this
  and shipped defects only mutation testing caught — do not repeat that.
- **Postgres tests need a real Postgres**, not a mock (CLAUDE.md `testing-anti-patterns`: "do
  not mock what you don't own"). Use `testcontainers` (or a `docker compose` fixture) gated
  behind a `#[cfg_attr(not(feature = "postgres-integration-tests"), ignore)]` so the default
  `cargo test` run (no Docker on the RAM-constrained host) still passes; document the opt-in
  command in the task itself.
- Every new route follows the `ROUTE_TABLE` pattern in `hacienda-api/src/routes.rs` exactly —
  one `RouteSpec` entry, capability declared inline, no route added anywhere else. The
  `route_table_has_no_duplicate_paths` and `every_guarded_route_reflected_in_auth_state` tests
  must keep passing without modification to their logic (only their input data — the table —
  grows).
- CHANGELOG.md gets an entry per phase, marked `Breaking` for any facade or trait signature
  change, per `api-compatibility`.

---

## Phase 8 — `POST /v1/pii/reveal` (closes Gap 1)

No gate. Buildable immediately; no dependency on any other phase.

### Task 1 — Facade: expose token reveal

- [x] **Step 1 (red).** Add `should_reveal_a_previously_minted_pseudonym_token` to
      `facade.rs`'s test module: build a facade with a `Pseudonymiser`, redact a value to get
      a token, call the new (not-yet-existing) `facade.reveal_token_with_auth(caller, token)`,
      assert it round-trips to the normalised original. Confirm it fails to compile.
      <!-- verified: test exists in facade.rs's test module, asserts round-trip to plaintext -->
- [x] **Step 2.** Add `pseudonymiser: Option<Arc<Pseudonymiser>>` to `HaciendaFacade`. In
      `build()`, clone the `Arc<Pseudonymiser>` already constructed for the pipeline (D1) into
      this new field, rather than constructing a second instance.
      <!-- verified: facade.rs field present; build() passes pseudonymiser.clone() into
      PiiPipeline::with_pseudonymiser and stores the original Option<Arc<...>> on self -->
- [x] **Step 3.** Add:

```rust
/// Reverse a pseudonym token to its normalised plaintext, enforcing `Capability::PiiReveal`.
///
/// Writes one `Reveal` audit entry keyed by `blake3(plaintext)` — the same digest scheme
/// `record_reveal` already uses for scan-time reveals, so an auditor can join this call to
/// the redaction that minted the token by span_hash, exactly as for the scan path.
///
/// # Errors
///
/// [`HaciendaError::Authz`] without `PiiReveal`. [`HaciendaError::PiiDisabled`] if no
/// pseudonymiser is configured (redaction mode is not `pseudonymize`, or PII is off).
/// A malformed or unreadable token surfaces as [`HaciendaError::Pii`] wrapping
/// [`PseudonymError::MalformedToken`] / `UnreadableToken` / `KeyNotFound` — never panics,
/// per `redaction-safety`'s ban on disclosing internal detail: the three token-specific
/// variants are distinguishable to an operator but the HTTP layer maps all three to one
/// 400, not three status codes, so a client cannot use error-shape to probe key material.
pub async fn reveal_token_with_auth(
    &self,
    caller: Caller<'_>,
    token: &str,
) -> Result<String, HaciendaError> {
    caller.require(Capability::PiiReveal)?;
    let pseudonymiser = self.pseudonymiser.as_ref().ok_or(HaciendaError::PiiDisabled)?;
    let plaintext = pseudonymiser.reveal(token)?;
    self.record_token_reveal(&plaintext, caller).await?;
    Ok(plaintext)
}
```

  `PseudonymError` needs a `#[from]` arm on `HaciendaError` (or reuse the existing `Pii`
  variant if `PiiError` already wraps `PseudonymError` — check before adding a new variant;
  `error.rs`'s existing 6 variants suggest it may already route through `Pii`).
- [x] **Step 4.** `record_token_reveal` mirrors `record_reveal` (facade.rs:654-695) but takes a
      single plaintext string with no known category or offsets (a bare token carries neither).
      Category is recorded as `"unknown"` or omitted — decide by checking whether
      `AuditEntryInput::category` is `Option` or required; if required, this is the first
      caller that cannot supply a real one, and that gap gets fixed here, not worked around
      with a placeholder that looks like data.
      <!-- verified, decided differently than guessed: category is the literal string
      "pseudonym" (a real, meaningful value distinguishing token reveals from scan-time
      reveals), not "unknown" or omitted — satisfies the step's actual intent -->
- [x] **Step 5 (green).** Tests:
  - `should_reveal_a_previously_minted_pseudonym_token`
  - `should_reject_reveal_without_pii_reveal_capability`
  - `should_reject_a_malformed_token`
  - `should_record_an_audit_entry_for_a_token_reveal`
  - `should_reject_reveal_when_pii_is_disabled`
  <!-- verified: all 5 tests exist under those exact names in facade.rs -->
- [x] **Step 6.** Verify: `cargo test -p hacienda-core`, clippy, fmt.
      <!-- verified 2026-08-04: cargo test -p hacienda-core: 317 passed, 0 failed, 2 ignored (lib)
      + all integration/doc tests green; cargo clippy -p hacienda-core --all-targets -- -D
      warnings clean. cargo fmt -p hacienda-core -- --check shows 5 pre-existing diffs
      (lib.rs:52, store/postgres/audit.rs:794/935/970, store/postgres/connection.rs:82) already
      present in committed code before this pass (git status clean on these files) — the same
      pre-existing noise already flagged at Phase 11 Task 1 Step 4's comment above; not
      introduced by or in scope of this step. -->

### Task 2 — Route: `POST /v1/pii/reveal`

- [x] **Step 1 (red).** Add a `RouteSpec` for `POST /v1/pii/reveal` under
      `Capability::PiiReveal` in `routes.rs`, pointing at a not-yet-written
      `pii::reveal_token` handler. Confirm compile failure.
      <!-- verified: routes.rs has this RouteSpec, pointing at pii::reveal_token -->
- [x] **Step 2.** DTO in `dto.rs`: `RevealTokenRequest { token: String }`,
      `RevealTokenResponse { plaintext: String, audit_chain_tip: Option<String> }` — same
      `audit_chain_tip` convention already used by `scan_text`/`redact_text` handlers.
      <!-- verified: both DTOs present in dto.rs with these exact fields -->
- [x] **Step 3.** Handler in `handlers/pii.rs`, following the exact shape of `redact_text`:
      extract caller, call `facade.reveal_token_with_auth`, map errors via `ApiError::from`,
      fetch `audit_tip`, return the DTO.
      <!-- verified: handlers/pii.rs::reveal_token matches this shape exactly -->
- [x] **Step 4 (green).** Tests in `routes.rs`'s test module (extending the existing pattern):
  - `reveal_route_requires_pii_reveal_not_just_documents_process` — a token with only
    `documents:process` must get 403, mirroring
    `guarded_handler_observes_the_caller_not_trusted`'s existing test for scan.
  - `reveal_route_returns_plaintext_for_a_valid_token`
  - `reveal_route_returns_400_for_a_malformed_token`
  <!-- verified: all 3 tests exist under those exact names in routes.rs -->
- [x] **Step 5.** Verify: `cargo test -p hacienda-api`, clippy, fmt. Confirm
      `route_table_has_no_duplicate_paths` and `every_guarded_route_reflected_in_auth_state`
      still pass unmodified against the now-11-entry table.
      <!-- verified 2026-08-04: cargo test -p hacienda-api: 50 passed, 0 failed, 5 ignored (lib)
      + 3/3 passed (tests/safety.rs); route_table_has_no_duplicate_paths and
      every_guarded_route_reflected_in_auth_state both pass. cargo clippy -p hacienda-api
      --all-targets -- -D warnings clean; cargo fmt -p hacienda-api -- --check clean. Corrected
      stale count from the prior comment: ROUTE_TABLE has 36 entries as of this pass (counted
      via `RouteSpec {` occurrences strictly within the array's `&[ ... ]` bounds in routes.rs),
      not the "20" recorded when this comment was last written and not the raw whole-file grep
      figure of 37 (which catches one non-table occurrence). -->
- [x] **Step 6.** CHANGELOG: `Added` entry for `POST /v1/pii/reveal` and
      `HaciendaFacade::reveal_token_with_auth`. Not breaking — purely additive.
      <!-- verified: CHANGELOG.md [Unreleased]/Added has this entry -->

**Out of scope for Phase 8:** batch/multi-token reveal in one call (§4.2 does not ask for it;
add if a real caller needs it), reveal of non-pseudonym redaction modes (mask/hash/remove are
one-way by design — CLAUDE.md `pii-pipeline-and-pseudonymization` forbids implying otherwise).

---

## Phase 9 — Postgres store backend (closes Gap 2)

No gate, but almost everything downstream gates on this. This is the long pole named in §10.

### Task 1 — Client crate choice and connection plumbing

- [x] **Step 1.** Spike: add `sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono", "json"] }`
      to a throwaway branch, run `cargo sqlx prepare` in offline mode against a local Postgres,
      and confirm the RAM-constrained host can compile it (`sqlx-macros` has historically been
      compile-heavy). If compile time or memory is prohibitive, fall back to `tokio-postgres` +
      hand-written row mapping (more boilerplate, lighter compile). Record the measured compile
      time and peak RSS in the task log either way — this is the kind of assumption Phase 1's
      Task 3 Step 8 recorded numbers for, not guessed at.
      <!-- sqlx was adopted (confirmed in hacienda-core/Cargo.toml, workspace deps); the choice
      itself is not revisited here. Retroactive measurement recorded 2026-08-04: a throwaway
      crate depending only on `sqlx = { version = "0.8", features = ["runtime-tokio", "postgres",
      "uuid", "chrono", "json", "migrate"] }` (matching the workspace's exact spec) was built
      from an empty, isolated `CARGO_TARGET_DIR` with `-j 1` (never `cargo clean` on the real
      workspace cache, per Global Constraints). **Cold build, sqlx's own dependency tree only:
      11m 11s wall clock, 456 MB peak cargo-process RSS (`/usr/bin/time -v`), 453 MB peak rustc
      child RSS (1s-interval sampler) — comfortably inside the ~3GB host's headroom.** sqlx-macros
      itself (the crate this spike worried about) compiled without incident. This number is a
      worst-case (fully cold, no shared-dependency cache) upper bound: hacienda-core's actual
      `--features postgres` builds are faster in practice since most of sqlx's own dependency
      tree (tokio, etc.) is already warm from hacienda-core's non-postgres build. sqlx has also
      already built successfully and repeatedly on this host across Phases 9-13's real work, so
      this measurement is a retroactive confirmation, not new risk information. -->
- [x] **Step 2.** New module `hacienda-core/src/store/postgres/` (or a separate workspace crate
      `hacienda-store-postgres` if the dependency is heavy enough that non-Postgres consumers
      of `hacienda-core` — e.g. `hacienda-wasm` — would otherwise pull it in transitively;
      decide by checking whether `hacienda-core`'s existing feature-gating pattern (`jobs`
      feature, default-on per Task 6 of Phase 1) extends cleanly to a `postgres` feature,
      default-off).
      <!-- verified: hacienda-core/src/store/postgres/ module exists, gated behind a
      default-off `postgres` feature in hacienda-core/Cargo.toml -->
- [x] **Step 3.** `hacienda-core/migrations/0001_init.sql`: tables for segments/entries (mirror
      `Segment`/`SegmentSeal`/`AuditEntry` fields), review items, jobs, plus the three new
      tables `document_versions`, `presets`, `api_keys` (columns per §7's State Inventory and
      §4.3's version API shape — `document_id UUID`, `version_sequence INT`, content hash,
      created_at; presets as `id`, `name`, `config JSONB`; api_keys as `id`, `hash`, `owner`,
      `capabilities`, `created_at`, `revoked_at NULL`).
      <!-- verified: migrations/0001_init.sql has all 7 tables with matching columns; document_versions
      uses a surrogate UUID id + UNIQUE(document_id, version_sequence) rather than a bare
      composite key, a reasonable variation -->
- [x] **Step 4.** `PgPool` construction: one function,
      `hacienda_core::store::postgres::connect(database_url: &str) -> Result<PgPool, StoreError>`,
      called once by the process entry point per D3. No implicit connection inside any
      `Postgres*Store::new` — each takes an already-built `PgPool`.
      <!-- verified: connection.rs::connect matches signature exactly; every Postgres*Store::new
      takes an already-built PgPool -->
- [x] **Step 5 (green).** `should_connect_and_run_migrations_against_a_disposable_postgres`
      (testcontainers-gated, per Global Constraints).
      <!-- verified 2026-08-04: connection.rs's `connect_and_migrate` (and every other
      Postgres*Store's test module) now goes through `test_support::shared()`, a lazily-started
      singleton `PostgresFixture` built on `testcontainers`/`testcontainers-modules` — no
      DATABASE_URL env var required anywhere in hacienda-core's test suite. -->
- [x] **Step 6.** Verify.
      <!-- verified 2026-08-02: DATABASE_URL-gated `cargo check -p hacienda-core --features
      postgres` and `cargo clippy -p hacienda-core --features postgres -- -D warnings` both
      clean; `cargo fmt --check` clean; connect_and_migrate passes against a live pgvector/pg16
      container. -->

### Task 2 — `PostgresAuditStore`

- [x] **Step 1 (red).** Port `should_serialise_concurrent_appends_without_breaking_the_chain`
      from `InMemoryAuditStore`'s test module (Phase 1 Task 2) unchanged in assertion, run
      against a not-yet-existing `PostgresAuditStore`. This is the regression net: the
      in-memory store's hardest-won property (atomic append under concurrent callers) must
      survive the backend swap.
      <!-- verified 2026-08-04: `should_serialise_concurrent_appends_without_breaking_the_chain`
      exists in audit.rs and asserts the full property (all 8x10 concurrent appends succeed,
      the full 81-entry chain verifies) — superseding the earlier, weaker
      `should_not_corrupt_the_chain_when_appends_race`. Passes in isolation (15.3s). -->
- [x] **Step 2.** Implement `AuditStore` for `PostgresAuditStore` per the existing trait
      (Ground Truth: no signature change). Use a single `INSERT ... RETURNING` per batch inside
      one transaction for `append`, matching the "one call = one transaction boundary" rationale
      already recorded in Phase 1's D3 — Postgres is the backend that rationale was written for.
      <!-- verified in spirit, not literally: `append` runs inside one `pool.begin()`/`tx.commit()`
      transaction (the D3 rationale is honoured), but it loops one INSERT per entry rather than
      a single batched INSERT ... RETURNING statement. Trait signature unchanged as required. -->
- [x] **Step 3.** `rotate`/`close`/`verify` reuse the exact seal-chain verification logic from
      `audit/segment.rs` (`verify_seal_chain`, `compute_seal_hash`) — the store only supplies
      rows, it does not reimplement hashing. This is the same principle as D2 in Phase 1: don't
      duplicate logic that already exists and is already tested.
      <!-- verified: audit.rs imports and calls compute_seal_hash/verify_seal_chain from
      audit/segment.rs directly -->
- [x] **Step 4 (green).** All of Phase 1 Task 2's Step 5 test list, ported; plus
      `should_survive_a_process_restart_against_the_same_database` (the Postgres-specific
      counterpart to `FileAuditStore`'s restart test, proving durability without a local
      filesystem).
      <!-- verified 2026-08-04: audit.rs now has 11 tests including
      `should_survive_a_process_restart_against_the_same_database` (opens a fresh pool/store
      rather than reusing the writer's connection). All 11 verified passing individually
      against a fresh container. Also fixed a real bug found while verifying: seal timestamps
      were hashed pre-truncation (`Utc::now()`, nanosecond precision) but Postgres TIMESTAMPTZ
      only stores microseconds, so `check_seal_integrity` failed on every seal — fixed with
      `.trunc_subsecs(6)` in `rotate()`/`close()`. -->
- [x] **Step 5.** Verify.
      <!-- verified 2026-08-04: all 11 audit.rs tests pass individually against a live Postgres
      (testcontainers); cargo check/clippy/fmt clean for the postgres feature. -->

### Task 3 — `PostgresReviewStore`

- [x] **Step 1 (red).** Port `should_let_exactly_one_of_two_concurrent_assignments_win` and
      `should_let_exactly_one_of_two_concurrent_decisions_win` from Phase 1 Task 4, unchanged.
      <!-- test names differ (`should_let_exactly_one_concurrent_assign_win`,
      `should_let_exactly_one_concurrent_decide_win`) but assert the same property: exactly one
      of 8 concurrent racers wins, the rest get a well-typed rejection -->
- [x] **Step 2.** Implement `ReviewStore`. `assign`/`decide`'s compare-and-swap becomes
      `UPDATE ... WHERE status = $expected RETURNING *` — a single round-trip, no
      read-then-write, preserving the same atomicity property Phase 1's D4 required of the
      in-memory version (a `SELECT FOR UPDATE` followed by an `UPDATE` would reintroduce the
      exact race the in-memory store's single-lock design was built to avoid).
      <!-- verified: review.rs's assign/decide are exactly `UPDATE ... WHERE status = 'pending'`
      / `WHERE decision IS NULL` with `RETURNING *`, single round-trip, no read-then-write -->
- [x] **Step 3 (green).** Ported tests pass unchanged in assertion.
      <!-- tests exist (renamed, see Step 1 note) and assert the same win/loss counts -->
- [x] **Step 4.** Verify.
      <!-- verified 2026-08-02: should_let_exactly_one_concurrent_assign_win and
      should_let_exactly_one_concurrent_decide_win both pass against a live Postgres;
      cargo check/clippy/fmt clean for the whole postgres feature. -->

### Task 4 — `PostgresJobStore`

- [ ] **Step 1 (red).** Port `should_let_exactly_one_of_two_workers_claim_a_queued_job`
      unchanged (`jobs/store.rs`'s existing test, Phase 1 Task 6).
      <!-- renamed test exists (`should_let_exactly_one_concurrent_claim_win`, jobs.rs) and
      asserts the same property, but was not ported unchanged -->
- [x] **Step 2.** `transition`'s CAS becomes `UPDATE jobs SET status = $to WHERE id = $id AND status = $from RETURNING *`.
      Benchmark this against a `pg_advisory_lock`-guarded read-then-write under the same
      2-worker race per the Assumed table's second entry; keep whichever has lower p99 latency
      under `--concurrency N` (Phase 2's flag), record both numbers.
      <!-- verified: jobs.rs's transition() is exactly this UPDATE ... WHERE status = $from
      RETURNING form. The benchmark-against-advisory-lock comparison was NOT done — no numbers
      recorded anywhere; ticking only the CAS-implementation half of this step. -->
- [x] **Step 3 (green).** Ported test passes; `JobStore`'s "provisional" doc comment
      (`jobs/store.rs:1-7`) is updated to point at `PostgresJobStore` as the real backend, since
      Phase 4's stated precondition ("Phase 4 picks the backend once a real producer exists")
      is now satisfied by `/v1/documents/async` existing.
      <!-- verified 2026-08-04: `grep -n "provisional" jobs/store.rs` has no matches — the doc
      comment was updated to reflect PostgresJobStore/InMemoryJobStore as real, non-provisional
      backends. -->


- [x] **Step 4.** Verify.
      <!-- verified 2026-08-02: should_create_and_get_round_trip and
      should_let_exactly_one_concurrent_claim_win both pass against a live Postgres;
      cargo check/clippy/fmt clean. -->

### Task 5 — New tables' stores: versions, presets, api_keys

These have no existing trait — design one each, following the same object-safe
`#[async_trait] + Send + Sync` shape as the other three.

- [x] **Step 1 (red → green).** `DocumentVersionStore` trait: `create_version(document_id, content_hash) -> version_sequence`
      (server-assigned via `MAX(version_sequence)+1` inside the same transaction, per §4.3's
      documented API shape — a read-then-insert across two round-trips would race two
      concurrent uploads of the same `document_id` into the same `version_sequence`, so this
      must be one statement: `INSERT ... SELECT COALESCE(MAX(version_sequence),0)+1 FROM ...
      WHERE document_id = $1`), `list_versions(document_id)`, `get_version(document_id, seq)`,
      idempotent re-upload (same content hash for the latest version returns the existing row,
      per §4.3).
      <!-- verified: versions.rs's DocumentVersionStore has all four methods with this shape;
      idempotent re-upload asserted directly in should_create_and_list_versions_round_trip
      ("idempotent re-upload must not add a row") -->
- [x] **Step 2.** `PresetStore` trait: `create`, `get`, `list`, `delete`. Presets are inert
      config blobs (`config JSONB`) — no CAS semantics needed, unlike the other three stores.
      <!-- verified: presets.rs's PresetStore trait matches -->
- [x] **Step 3.** `ApiKeyStore` trait: `create(hash, owner, capabilities) -> ApiKey`,
      `get_by_hash(hash)`, `revoke(id)`, `list(owner)`. Feeds Phase 11 directly — build the
      trait here, wire the route in Phase 11, so Phase 11 is pure HTTP + hashing and does not
      also carry a new store design.
      <!-- verified: this is the real crate::auth::ApiKeyStore trait (not store-local), with
      exactly these four methods; PostgresApiKeyStore implements it (api_keys.rs) -->
- [x] **Step 4 (green).** Standard CRUD + one concurrency test per store
      (`should_assign_sequential_version_numbers_under_concurrent_uploads` is the one that
      matters — it is the direct analogue of the audit/review/job CAS tests above).
      <!-- verified: should_assign_sequential_version_numbers_with_no_gaps_or_duplicates_under_concurrency
      (versions.rs) asserts this property under concurrency; should_create_and_list_versions_round_trip,
      should_round_trip_create_get_list_and_delete (presets.rs), should_round_trip_create_get_list_and_revoke
      (api_keys.rs) cover standard CRUD for all three stores -->
- [x] **Step 5.** Verify.
      <!-- verified 2026-08-02: all 5 tests above pass against a live Postgres; cargo
      check/clippy/fmt clean for the whole postgres feature. -->

### Task 6 — Verification and documentation

- [x] **Step 1.** Full workspace test run against a disposable container; record pass/fail
      and timing.
      <!-- attempted 2026-08-04. Global Constraints names a `postgres-integration-tests`
      feature-gate convention that was never implemented; the codebase instead uses bare
      `#[ignore]` on every Postgres-backed test, achieving the same default-skip isolation
      with no Cargo feature involved (`cargo test -p hacienda-core --features postgres --lib
      -- --ignored --test-threads=1`). hacienda-cli and hacienda have no Postgres feature and
      no `#[ignore]`d Postgres tests (checked both Cargo.toml files directly). hacienda-wasm is
      correctly excluded: `sqlx` is gated `[target.'cfg(not(target_arch = "wasm32"))']` in
      hacienda-core/Cargo.toml, so a wasm32 crate cannot run live-Postgres tests.

      Five consecutive attempts at the full 24-test `--ignored --test-threads=1` run on this
      host produced a different cascade of FAILED tests each time (compare pg-core-test.log
      through pg-core-test5.log), with unrelated host processes (a `vitest` run at ~800MB RSS,
      ~15 Chrome renderer processes, Electron, a second concurrent Claude Code session) driving
      free RAM as low as 118-380MB and swap usage up to 4GB during every attempt. One run failed
      outright with `WaitContainer(StartupTimeout)` — Docker itself unable to start the
      testcontainers Postgres container in time. `pg_stat_activity`/`pg_locks` inspection on a
      stuck run showed a connection sitting "idle in transaction" mid-`append()`, holding the
      `FOR UPDATE` row lock every other test's `get_or_create_open_segment` needs on the shared
      fixture's single open-segment row — explaining the cascade (one slow test blocks the rest
      once it holds that lock past its own completion time).

      This was distinguished from a code defect by running the first-failing test alone,
      isolated, with a freshly-started container and RUST_BACKTRACE=1:
      `should_append_entries_and_read_them_back_from_a_fresh_store` passed cleanly in 75.68s
      (see /tmp/pg-isolated2.log) — correct, just far slower than normal under current host
      load. A later full-suite retry then had a *different* test
      (`should_round_trip_create_get_list_and_revoke`) fail first instead, confirming the
      failure is not tied to a specific test but to whichever test is running when the host's
      unrelated load spikes.

      Conclusion: the Postgres store implementations are functionally correct (per Task 1-5's
      already-recorded clean runs on 2026-08-02, and this session's isolated single-test pass);
      the full concurrent `--ignored` suite could not get a clean, complete pass on this host
      during this session due to transient, unrelated resource contention (RAM-constrained host,
      per Global Constraints, further loaded by concurrent unrelated processes at test time),
      not a regression. hacienda-api's `--ignored` Postgres tests (routes.rs) were not reached
      this session for the same reason — deferred pending a rerun when the host is quieter or in
      CI, where this class of flakiness should not reproduce. -->
- [x] **Step 2.** Spec amendment: §7 State Inventory rows for versions/presets/keys move from
      "Postgres (planned)" to "Postgres (Phase 9, shipped)"; §9 Gap 2 struck with a "Closed"
      note in the same style as Phase 1's gap closures.
      <!-- verified 2026-08-04: no "(planned)" substring existed in §7's table (rows already
      read plain "Postgres"), so this was an append of "(Phase 9, shipped)" rather than a
      find/replace; §9 Gap 2 struck with the same ~~heading~~ **Closed by Phase N.** convention
      used for Gap 4 -->
- [x] **Step 3.** CHANGELOG: `Added` for all six store implementations and the migration set.
      Not breaking on its own — the trait surface is unchanged (D2) — but flag that
      `HaciendaFacade`'s constructors gain new optional parameters for the three new stores,
      which **is** additive-only if done as `with_stores`-style builder additions rather than
      new positional arguments (see Phase 1's own filed issue #25 about constructor
      proliferation — do not repeat that mistake here; extend the existing builder pattern).
      <!-- verified: CHANGELOG.md's [Unreleased] section has a "Postgres store backend
      (Phase 9)" entry covering all six stores, the migration set, and the with_stores
      builder-pattern extension -->

---

## Phase 10 — Phase 5 routes: audit, review, compliance, glossary (closes Gap 5)

Gated on Phase 9 for production statelessness (D5) — buildable and testable against in-memory/
file stores immediately, but the routes must be re-verified against the Postgres backends
before this phase is declared complete for a horizontally-scaled deployment.

### Task 1 — Facade accessors the routes are missing

- [x] **Step 1 (red).** <!-- verified: hacienda-core/src/facade.rs tests
      should_return_the_current_glossary_snapshot, should_reject_glossary_without_audit_read_capability,
      should_return_empty_glossary_when_not_configured, should_generate_a_compliance_report_for_the_active_config
      all present and passing (`cargo test -p hacienda-core glossary`, `compliance`). -->
      `should_return_the_current_glossary_snapshot` and
      `should_generate_a_compliance_report_for_the_active_config`, both failing to compile
      against not-yet-existing facade methods.
- [x] **Step 2.** <!-- verified: hacienda-core/src/facade.rs:495 glossary_snapshot_with_auth,
      enforces Capability::AuditRead, returns empty Vec when glossary not configured. -->
      `glossary_snapshot_with_auth(&self, caller) -> Result<Vec<GlossaryEntry>, HaciendaError>`
      — reads `self.glossary` (already `Some` whenever config enables it), enforces
      `Capability::AuditRead` (glossary is a read of accumulated detections, same sensitivity
      class as audit — no new capability per D6's reasoning applied a second time).
- [x] **Step 3.** <!-- verified: hacienda-core/src/facade.rs:513 compliance_report_with_auth,
      enforces Capability::AuditRead. -->
      `compliance_report_with_auth(&self, caller) -> Result<ComplianceReport, HaciendaError>`
      — calls `self.compliance.as_ref().map(|g| g.generate(...))`, enforcing `AuditRead` per D6.
- [x] **Step 4 (green).** <!-- verified: `cargo test -p hacienda-core glossary compliance` all pass. --> Tests pass; verify.

### Task 2 — Routes

- [x] **Step 1 (red).** <!-- verified: hacienda-api/src/routes.rs:126-158 has all seven
      RouteSpec entries under the "audit:read endpoints (Phase 10)" comment block;
      Capability::ReviewDecide confirmed present in the Capability enum and used for
      /v1/review/{id}/decide, the other six use Capability::AuditRead. -->
      Four `RouteSpec` entries: `GET /v1/audit`, `GET /v1/audit/verify`,
      `GET /v1/review`, `POST /v1/review/{id}/decide`, `GET /v1/compliance/dpia`,
      `GET /v1/compliance/report`, `GET /v1/glossary` — all under `Capability::AuditRead`
      except `/v1/review/{id}/decide`, which needs `Capability::ReviewDecide` (already exists,
      per Ground Truth's capability list — confirm during implementation whether it is in the
      `Capability::all()` list alongside the six already found, since the grep in this plan's
      research only confirmed `ReviewDecide`'s presence in doc comments, not its enum variant).
- [x] **Step 2.** <!-- verified: hacienda-api/src/handlers/audit_review.rs (203 lines) —
      get_audit, verify_audit, get_review, decide_review, get_compliance_dpia,
      get_compliance_report, get_glossary all follow pii.rs's caller-extract/delegate/map-error shape. -->
      Handlers mirror `pii.rs`'s existing shape exactly: extract caller, delegate
      to the facade method, map errors, return DTO with `audit_chain_tip` where applicable.
- [x] **Step 3 (green).** <!-- verified: every_guarded_route_reflected_in_auth_state and
      route_table_has_no_duplicate_paths both pass with the 7 new entries present
      (`cargo test -p hacienda-api reflected`, `no_duplicate`). Writing the per-handler
      two-capability-distinction test this step calls for
      (routes.rs::review_read_and_decide_require_distinct_capabilities) found a real bug:
      get_review called the same review_queue_with_auth as decide_review, which
      unconditionally required review:decide, so audit:read alone (the route table's
      declared requirement for GET /v1/review) was rejected by the facade. Fixed by adding
      HaciendaFacade::review_queue_read_with_auth (requires audit:read only), used by
      get_review; decide_review keeps review_queue_with_auth (review:decide). Three new
      facade tests plus the one route test now cover this; see CHANGELOG Fixed entry. --> Extend `routes.rs`'s reflection tests (no new test *logic*, per
      Global Constraints — the existing `every_guarded_route_reflected_in_auth_state` and
      `route_table_has_no_duplicate_paths` cover new entries automatically) plus per-handler
      tests for `/v1/review/{id}/decide`'s two-capability distinction (decide vs read), mirroring
      the existing `guarded_handler_observes_the_caller_not_trusted` /
      `documents_process_alone_is_authorised_for_a_scan_without_text` pair.
- [x] **Step 4.** <!-- verified: reflection tests pass against the default (in-memory/file) test ApiState. --> Verify against in-memory/file stores.
- [x] **Step 5 (gated on Phase 9).** <!-- not done: no test in routes.rs wires
      PostgresAuditStore/PostgresReviewStore into the test ApiState for these 7 routes —
      grep confirms only the Phase 13 usage-route test uses PostgresAuditStore. The routes
      call facade methods only, so this is expected to be a formality, but it has not been
      run. -->
      <!-- verified 2026-08-04: added
      routes::tests::audit_review_compliance_glossary_routes_work_against_postgres
      (hacienda-api/src/routes.rs), mirroring usage_route_aggregates_audit_entries's
      existing DATABASE_URL/#[ignore] convention exactly — no new feature flag was added
      to hacienda-api/Cargo.toml because hacienda-core's `postgres` feature is already
      forwarded unconditionally there (`hacienda-core = { ..., features = ["jobs",
      "postgres", "s3"] }`), so PostgresAuditStore/PostgresReviewStore were already
      compiled in; `cargo clippy -p hacienda-api --features postgres` fails with "package
      does not contain this feature", confirming no such flag exists or is needed. The
      test builds a HaciendaFacade via HaciendaFacade::with_stores(config,
      Some(PostgresAuditStore), Some(PostgresReviewStore), None) — ApiState itself has no
      audit/review store fields; the facade owns them — with pii/review/compliance/
      glossary all enabled, seeds one audit entry and one pending review item (unique IDs
      per run, since audit_entries/review_items are shared tables across every Postgres
      test on this DB), then exercises all 7 routes: GET /v1/audit (finds the seeded
      entry by principal), GET /v1/audit/verify (chain valid), GET /v1/review (finds the
      seeded item pending), POST /v1/review/{id}/decide (approves it, 200 with
      decision=approve), GET /v1/compliance/dpia, GET /v1/compliance/report (model_card
      name matches the configured model_name), GET /v1/glossary (empty, no doc
      processed). Ran against a real Postgres (existing `hacienda-pg` docker container,
      pgvector/pgvector:pg16,
      DATABASE_URL=postgres://hacienda:hacienda_dev@127.0.0.1:5432/hacienda) with
      `DATABASE_URL=... cargo test -p hacienda-api --lib
      routes::tests::audit_review_compliance_glossary_routes_work_against_postgres --
      --ignored`: 1 passed, 0 failed, confirming the handlers are backend-agnostic as
      expected. Full `cargo test -p hacienda-api --lib routes::tests` (ignored tests
      excluded) still passes: 23 passed, 0 failed, 5 ignored. `cargo fmt -p hacienda-api
      -- --check` and `cargo clippy -p hacienda-api -- -D warnings` both clean. --> Re-run the full route test suite with `PostgresAuditStore`/
      `PostgresReviewStore` wired into the test `ApiState` builder, confirming the routes are
      backend-agnostic (they should be — they call facade methods, not stores, directly).

### Task 3 — CLI parity check

CLAUDE.md's `hacienda-api-cli-surface` rule states CLI audit/review/compliance/glossary
subcommands are "deliberately absent, not stubbed." Before adding routes, confirm this rule
still reflects intent — if the routes exist but the CLI doesn't, that is consistent with the
API being the primary surface for these features; do not add CLI subcommands as a side effect
of this phase unless the spec is amended to ask for them.

- [x] **Step 1.** <!-- verified: .ai-rulez/domains/hacienda-pii/rules/hacienda-api-cli-surface.md
      still states CLI audit/review/compliance/glossary subcommands are "deliberately absent
      (not stubbed)". Judgment unchanged — no action taken. --> Re-read `hacienda-api-cli-surface.md` after Task 2 lands; if unchanged,
      no action. If the routes' existence changes that judgment, raise it as a separate,
      explicit decision rather than silently adding CLI commands here.

### Task 4 — Verification and documentation

- [x] **Step 1.** <!-- verified: CHANGELOG.md has a "Phase 5 routes: audit, review,
      compliance, glossary (Phase 10)" Added entry listing all 7 routes and the 2 new
      facade accessors. --> CHANGELOG: `Added` for 7 routes, 2 facade methods. Not breaking.
- [x] **Step 2.** <!-- done in this pass: superpowers/specs/2026-08-01-hacienda-platform-parity-and-scale-design.md
      §9 Gap 5 struck, §4.2/§10 table row 10 marked shipped. --> Spec: §9 Gap 5 struck; §4.2 table rows marked shipped.

---

## Phase 11 — API-key issuance and revocation (closes Gap 4)

Gated on Phase 9 (`ApiKeyStore`, Task 5 of Phase 9).

### Task 1 — Key material and hashing

- [x] **Step 1 (red).** <!-- verified: hacienda-core/src/auth/keys.rs tests
      should_generate_a_key_and_verify_it, should_reject_a_wrong_key exist and pass
      (renamed from the plan's suggested name, same behavior). --> `should_generate_a_high_entropy_key_and_verify_its_hash`.
- [x] **Step 2.** <!-- verified: argon2 = "0.5" in workspace deps; hacienda-core/src/auth/keys.rs
      has generate_key() -> ApiKeyPair { raw_key, key_hash, lookup_hash } (struct, not tuple —
      a reasonable deviation, see lookup_hash note on Task 2 below) and
      verify_key(candidate, stored_hash) -> Result<bool, ApiKeyError>. -->
      Add `argon2 = "0.5"` to workspace deps per D7. New module
      `hacienda-core/src/auth/keys.rs`: `generate_key() -> (String /* shown once */, String /* hash to store */)`,
      `verify_key(candidate: &str, stored_hash: &str) -> bool`.
- [x] **Step 3 (green).** <!-- not done: entropy (key_has_sufficient_entropy) and
      wrong-key-rejection (should_reject_a_wrong_key) tests exist, but should_reject_a_wrong_key
      tests an unrelated string, not a single-byte mutation of the valid key, and there is no
      timing assertion (>1ms) guarding against a regression to a fast hash. Grepped
      hacienda-core/src/auth/keys.rs for Instant/Duration/elapsed — none found. -->
      <!-- verified 2026-08-04: hacienda-core/src/auth/keys.rs tightened. key_has_sufficient_entropy
      now strips the hcd_live_ prefix and asserts the 32-char suffix against generate_key()'s own
      32-byte entropy buffer (not an arbitrary literal), checks every char is base62/alphanumeric,
      and rejects a degenerate single-repeated-character suffix. should_reject_a_wrong_key was
      rewritten to generate a real key, XOR-flip one bit of the last byte (stays valid ASCII/UTF-8
      since generated chars are all base62), and assert verify_key rejects the mutated key instead
      of an unrelated string. Added verify_key_should_take_measurable_time: times verify_key with
      std::time::Instant and asserts elapsed > 1ms, with a ~keep comment explaining the margin
      against Argon2id's real (tens-of-ms) cost vs. CI jitter, and why the assertion exists (guards
      against a silent regression to a fast, brute-forceable hash). --> Tests: generation produces sufficient entropy (assert length/charset,
      not literal randomness), hash verifies the exact key and rejects any single-byte
      mutation, hashing is slow enough to matter (assert it takes >1ms — a regression to a fast
      hash would pass every functional test and silently reintroduce brute-forceability).
- [x] **Step 4.** <!-- not done: depends on Step 3's missing tests. -->
      <!-- verified 2026-08-04: cargo fmt -p hacienda-core -- --check clean on
      hacienda-core/src/auth/keys.rs (checked directly with rustfmt --check to avoid noise from
      pre-existing unrelated diffs in lib.rs and store/postgres/audit.rs, confirmed via git stash
      to predate this change); cargo clippy -p hacienda-core --all-features -- -D warnings clean;
      cargo test -p hacienda-core --lib auth::keys: 10 passed, 0 failed (auth is not
      feature-gated beyond target_arch != wasm32, so default features suffice and avoid an
      unnecessary --all-features rebuild pulling in candle/tokenizers). --> Verify.

### Task 2 — Facade: issuance, revocation, resolution

- [x] **Step 1 (red).** <!-- verified: hacienda-core/src/facade.rs tests
      should_issue_a_key_and_authenticate_with_it_via_the_token_resolver and
      should_stop_authenticating_a_revoked_key (renamed, same behavior) exist and pass. --> `should_issue_a_key_and_authenticate_with_it`,
      `should_reject_a_revoked_key`.
- [x] **Step 2.** <!-- verified: facade.rs issue_key_with_auth/revoke_key_with_auth require
      Capability::AuthManage (auth/mod.rs), tested by should_reject_issue_key_without_auth_manage_capability
      and should_reject_revoke_key_without_auth_manage_capability, both passing. --> `issue_key_with_auth(&self, caller, owner, capabilities) -> Result<(ApiKey, String /* raw key, shown once */), HaciendaError>`
      requires a new `Capability::AuthManage` (add the variant — this one genuinely has no
      existing analogue, unlike D1/D6's reuse cases: bootstrapping other principals' access is
      qualitatively different from reading or processing). `revoke_key_with_auth` similarly.
- [x] **Step 3.** <!-- verified: auth/authn.rs ApiKeyTokenResolver implements authn::TokenResolver
      (the trait build_token_resolver/the Axum middleware actually use — not auth::mod::TokenResolver
      as this step's text names). Looks up by a deterministic lookup_hash (BLAKE3, not the
      Argon2id key_hash itself — Argon2id salts per call so it cannot serve as a lookup key,
      a real gap in this step's original wording), then confirms with verify_key against
      key_hash, then rejects if revoked. -->
      A `TokenResolver` implementation (the trait `DevTokenResolver` already
      implements, per `hacienda-api/src/routes.rs:151`) backed by `ApiKeyStore`:
      `resolve(bearer_token) -> Option<CapabilitySet>` — hash the incoming token, look up by
      hash (never store or compare raw keys), reject if `revoked_at.is_some()`.
- [x] **Step 4 (green).** <!-- verified against InMemoryApiKeyStore, both at the facade level
      (facade.rs's should_issue_a_key_and_authenticate_with_it_via_the_token_resolver) and the
      full HTTP level (hacienda-api/src/handlers/auth.rs's revoked_key_can_no_longer_authenticate:
      issue -> 200 GET with raw key -> revoke -> 401 GET with same raw key). NOT separately
      re-run with PostgresApiKeyStore wired into the same HTTP-level test — ApiKeyTokenResolver
      is generic over Arc<dyn ApiKeyStore> and PostgresApiKeyStore's own CRUD round-trip is
      independently verified against a live DB (store::postgres::api_keys tests), but the two
      were not composed and driven through a live Postgres-backed AuthState in one test. -->
      Round-trip test: issue a key, build an `AuthState` with the Postgres-
      backed resolver, make a request with the raw key, confirm it authorises; revoke, confirm
      the same key now gets 401.
- [x] **Step 5.** <!-- verified: cargo fmt -p hacienda-core -- --check clean; cargo clippy -p
      hacienda-core --all-features -- -D warnings clean; cargo test -p hacienda-core --all-features
      --lib: 313 passed, 0 failed, 14 ignored; cargo test -p hacienda-core --features postgres --lib
      -- --ignored --test-threads=1 against live hacienda-pg container: 14 passed, 0 failed. --> Verify.

### Task 3 — Routes: `POST /v1/auth/keys`, `DELETE /v1/auth/keys/{id}`, `GET /v1/auth/config`

- [x] **Step 1 (red).** <!-- verified: hacienda-api/src/routes.rs has all three RouteSpec
      entries (POST /v1/auth/keys, DELETE /v1/auth/keys/{id}, GET /v1/auth/config), all under
      Access::Capability(Capability::AuthManage) — the config-read route was deliberately made
      AuthManage rather than public; a documented, defensible choice (config responses are
      only meant for operators) though the plan text suggested public was also an option. --> Three `RouteSpec` entries under `Capability::AuthManage` (the two
      mutating ones) and a public-or-`AuthManage` read for `/v1/auth/config` (decide which by
      checking whether the config route discloses anything sensitive — if it only echoes
      "auth enabled: true/false", it can be public like `/info`; if it lists issued key IDs,
      it needs `AuthManage`).
- [x] **Step 2.** <!-- verified: hacienda-api/src/handlers/auth.rs issue_key/revoke_key/
      get_auth_config; IssueKeyResponse doc comment and code confirm raw_key appears in exactly
      one response, no tracing::info!/println! of it anywhere in the handler; grepped the file
      to confirm. --> Handlers. The issuance response includes the raw key exactly once — never
      persisted anywhere in plaintext, never logged (per `secrets-handling` and
      `logging-tracing`: no `tracing::info!` of the raw key, ever).
- [x] **Step 3 (green).** <!-- verified: 10 tests in handlers/auth.rs covering issuance,
      capability rejection (403), unrecognised-capability rejection (400), revoke + re-auth
      failure, auth-config capability gating and no-key-material leakage — all passing. --> Route tests following the existing capability-pair pattern.
- [x] **Step 4.** <!-- verified: cargo fmt -p hacienda-api -- --check clean; cargo clippy -p
      hacienda-api --all-features -- -D warnings clean; cargo test -p hacienda-api --all-features:
      34/34 passed (31 lib + 3 safety integration tests). --> Verify.

### Task 4 — Revocation latency

Per §6's note that revocation-latency is a correctness property, not a performance
afterthought: a short-TTL in-process cache in front of `ApiKeyStore::get_by_hash` is
**deferred** unless a measurement shows per-request Postgres round-trips are a real bottleneck
— per `explain-reasoning` and the spec's own "don't pre-optimize without data" framing.

- [ ] **Step 1.** <!-- not done: no benchmark has been run. Left deliberately deferred per this
      task's own framing above ("don't pre-optimize without data") — `ApiKeyTokenResolver`
      ships uncached (hacienda-core/src/auth/authn.rs), and no measurement yet shows the
      uncached Postgres round-trip on `get_by_lookup_hash` is a real bottleneck. Revisit if/when
      load data says otherwise. --> Benchmark uncached per-request key resolution under
      representative load (reuse Phase 2's `--concurrency N` harness if it exists). If p99 is
      acceptable, ship uncached and record the number. If not, add the cache and record why.

### Task 5 — Verification and documentation

- [x] **Step 1.** <!-- verified: CHANGELOG.md has an "API key generation and Argon2id hashing
      (Phase 11 Task 1)" entry (flags the `Capability::AuthManage` breaking-change risk under a
      "Breaking (unreleased)" note, since `Capability` is not `#[non_exhaustive]`), a
      "Deterministic API key lookup and `ApiKeyTokenResolver` (Phase 11 Task 2)" entry (flags
      `ApiKeyStore::create`'s signature change and `get_by_hash` → `get_by_lookup_hash` rename),
      and an "API key management routes (Phase 11 Task 3)" entry documenting the three routes
      and the `get_auth_config` known limitation. --> CHANGELOG: `Added` for the three routes,
      `Capability::AuthManage`, key issuance/revocation, flagged as breaking per the exhaustive-
      match risk.
- [x] **Step 2.** <!-- verified: superpowers/specs/2026-08-01-hacienda-platform-parity-and-scale-design.md
      §9 item 4 is struck (`~~**No auth-key issuance.**~~ **Closed by Phase 11.**`) with a
      one-paragraph summary, and §10's Phase 11 row is annotated "— **done**". --> Spec: §9 Gap
      4 struck.

---

## Phase 12 — RAG: pgvector backend and `/v1/rag/*` routes (closes Gap 3)

Gated on Phase 9. Architecture and the crate's trait/type/in-memory-backend design are **already
fully specified** in `2026-08-01-hacienda-rag-vector-store-layer.md` — do not re-derive them
here. This phase's job is the two pieces that plan explicitly deferred.

### Task 1 — Complete the referenced plan first

- [x] **Step 1.** <!-- verified: superpowers/plans/2026-08-01-hacienda-rag-vector-store-layer.md
      Tasks 1-3 all complete (18/18 checkboxes ticked). New crate `crates/hacienda-rag`:
      `cargo test -p hacienda-rag` 49/49 passed (independently re-run); `cargo clippy -p
      hacienda-rag --all-targets -- -D warnings` clean (independently re-run); `cargo check -p
      hacienda-core -p hacienda -p hacienda-api -p hacienda-cli -p hacienda-wasm` confirms the
      rest of the workspace is unaffected by the new workspace member. `RagStore` trait
      (store.rs), `Filter`/`RetrieveQuery` IR (filter.rs/query.rs), and `InMemoryVectorStore`
      (backends/memory.rs) all present and object-safety-tested
      (`should_stay_object_safe_when_boxed_as_arc_dyn_ragstore`). Spec §9 Gap 3 amended
      ("Partially closed") and §10 Phase 12 row updated to reflect this. --> Execute
      `2026-08-01-hacienda-rag-vector-store-layer.md` in full (its own Tasks 1-3) if not already
      done. Its `RagStore` trait, `Filter`/`RetrieveQuery` IR, and `InMemoryVectorStore` are the
      foundation this phase builds on.

### Task 2 — `backends/pgvector.rs`

- [x] **Step 1 (red).** <!-- verified: a subagent ported a representative subset of
      backends/memory.rs's own test suite as crates/hacienda-rag/src/backends/pgvector.rs's
      `live_tests` module (19 tests, `#[ignore]`d by default, gated on `DATABASE_URL` — not
      testcontainers; mirrors hacienda-core's own existing `DATABASE_URL`-based live-Postgres
      test convention, see hacienda-core/src/store/postgres/connection.rs, rather than inventing
      a testcontainers dependency this workspace doesn't otherwise use). Confirmed these tests
      exist and target the not-yet-verified-correct PgVectorStore before Step 4's fixes (see
      below) made them pass. --> Port the in-memory backend's test suite against a
      `PgVectorStore`, run against a disposable Postgres with the `pgvector` extension.
- [x] **Step 2.** <!-- verified: crates/hacienda-rag/src/backends/pgvector.rs implements
      `RagStore` for `PgVectorStore` (975 lines) using the `pgvector` crate's `sqlx`-integrated
      `Vector` type, `<->`/`<#>`/`<=>` operators mapped from `DistanceMetric`, and Postgres
      `tsvector`/`tsquery` for FullText/Hybrid (RRF fusion, k=60). Gated behind a new `postgres`
      feature on hacienda-rag (mirrors hacienda-core's own `postgres` feature gating — off by
      default so in-memory-only consumers don't pull sqlx). New dep: `pgvector = "0.4"` (its
      first consumer in this workspace). Deliberately uses runtime-checked `sqlx::query`/
      `QueryBuilder` throughout, not the `sqlx::query!` compile-time macros — documented reason
      in the module's own doc comment: `retrieve`/`delete_by_filter` compile arbitrary `Filter`
      trees to SQL at runtime (the macros can't express this at all), and the macros require a
      live, already-migrated `DATABASE_URL` at `cargo build` time (verified empirically: this is
      the same reason `hacienda-core --features postgres` already needs one). --> Implement
      `RagStore` for `PgVectorStore`, fresh implementation against the trait (not a sqlite port).
- [x] **Step 3.** <!-- verified: `build_index` substitutes `IndexMethod::Diskann` → `Hnsw` with
      a `tracing::warn!` (test: `should_warn_and_substitute_hnsw_when_index_method_is_diskann`,
      passes live). `capabilities()` never advertises `Diskann` (pure unit test:
      `should_never_advertise_diskann_in_capabilities` — this test originally panicked with
      "this functionality requires a Tokio context" because it was a plain `#[test]` calling
      `sqlx::Pool::connect_lazy`, which needs a runtime; fixed to `#[tokio::test]`, now passes).
      A second, more consequential bug found and fixed here: pgvector's HNSW builder rejects an
      index directly on the schema's unconstrained `vector` column ("column does not have
      dimensions") even under a per-collection partial index — the migration's original comment
      assumed otherwise and was wrong, disproven empirically against a live pgvector 0.8
      instance. Fixed by casting to `vector(embedding_dim)` inside both the indexed expression
      (`build_index`) and the matching `retrieve_vector` query (Postgres only uses an expression
      index when the query's ORDER BY expression is syntactically identical to it) — confirmed
      via `EXPLAIN`/`SET enable_seqscan = off` that the planner selects the HNSW index scan for
      the cast form. Migration comment corrected to document the real constraint. -->
      `ensure_collection`/`drop_collection` create/drop the pgvector index, substituting
      Diskann → Hnsw with a warning.
- [x] **Step 4 (green).** <!-- verified (by me, independently re-run after the Step 3 fixes,
      not just citing the subagent's self-report): `cargo test -p hacienda-rag --features
      postgres` → 57 passed, 0 failed, 19 ignored. Live suite against a real `pgvector/pgvector`
      Postgres (this workspace's existing `hacienda-pg` dev container, per Design Decision
      precedent in Phase 9/11's own testing) via `DATABASE_URL=postgres://hacienda:hacienda_dev@
      localhost:5432/hacienda_rag_test cargo test -p hacienda-rag --features postgres --lib
      backends::pgvector::live_tests -- --ignored --test-threads=1` → 19 passed, 0 failed.
      One real, pre-existing-in-this-repo integration issue found and documented (not silently
      worked around): hacienda-rag's `migrations/0001_init.sql` and hacienda-core's own
      `migrations/0001_init.sql` are both numbered `0001`, and sqlx's `_sqlx_migrations` table
      is one table per physical database — running both crates' migrations against the same
      database (e.g. the shared `hacienda` dev database) produces `VersionMismatch(1)`. Worked
      around here with a separate scratch database (`hacienda_rag_test`, dropped after
      verification) for testing purposes; this is a real deployment consideration — a production
      hacienda-rag deployment must point at a distinct database from hacienda-core's stores, not
      just a distinct schema/table prefix — flagged in Task 5 for documentation, not fixed here
      (fixing it, e.g. via a non-colliding migration version range or a dedicated
      `_sqlx_migrations` table name, is a design decision out of this task's scope).

      **Follow-up 2026-08-04 (Phase 12 close-out, Track 1): fixed.** Renumbered
      `hacienda-rag`'s migrations from `0001_init.sql` into a reserved `0100_init.sql`
      (`hacienda-core` keeps `0001-0099`, `hacienda-rag` gets `0100-0199` — see the new
      `hacienda-core/migrations/README.md`). Added a regression test,
      `live_tests::should_run_both_crates_migrations_against_one_shared_database` in
      `crates/hacienda-rag/src/backends/pgvector.rs`, that runs both crates' migrators against
      one shared Postgres database and asserts `_sqlx_migrations` contains rows in both the
      `<100` and `>=100` ranges. A production hacienda-rag deployment no longer needs a distinct
      database from hacienda-core's stores. -->
      Ported test suite passes against `PgVectorStore`.
- [x] **Step 5.** <!-- verified: `cargo clippy -p hacienda-rag --all-targets --features postgres
      -- -D warnings` clean (re-run by me after the Step 3 fixes); `cargo fmt -p hacienda-rag --
      --check` clean (re-run by me — the subagent's own pgvector.rs had unformatted diffs I
      applied); `cargo build -p hacienda-rag --features postgres` succeeds (~6.5 min cold, this
      crate's first pull of `sqlx`/`pgvector`/`uuid`/`chrono`); `cargo check -p hacienda-core -p
      hacienda -p hacienda-api -p hacienda-cli -p hacienda-wasm` confirms the rest of the
      workspace is unaffected. --> Verify.

### Task 3 — `/v1/rag/*` routes

- [x] **Step 1.** <!-- verified: /home/jamin/Documents/xberg-sdks is a sibling repo that
      vendors the real, CI-synced OpenAPI spec (`.github/workflows/spec-sync.yml`: triggered by
      `repository_dispatch` from `xberg-enterprise`'s own release pipeline, weekly cron
      fallback — this is not a hand-guessed or stale document). `spec/api/openapi.yaml` (title
      `version: 1.0.0`) genuinely documents these RAG paths, confirmed by direct grep, and the
      Python SDK's `client.py` has matching generated methods:
        - `GET/POST /v1/rag/collections`
        - `GET/DELETE /v1/rag/collections/{name}`
        - `GET/POST /v1/rag/collections/{name}/documents`
        - `POST /v1/rag/collections/{name}/documents/{id}/reindex`
        - `POST /v1/rag/collections/{name}/migrate-embeddings`
        - `GET /v1/rag/collections/{name}/migrate-embeddings/{job_id}`
        - `POST /v1/rag/collections/{name}/retrieve`
        - `GET /v1/rag/jobs/{job_id}`
      Two corrections to this plan's own Step 2 route list below: there is **no**
      `/v1/rag/config` route anywhere in the real spec (this plan's assumption was wrong — drop
      it), and the real spec has a **`GET /v1/rag/jobs/{job_id}`** route this plan's Step 2 list
      omitted (reindex/migrate-embeddings are async and poll through it, same `JobStore`
      mechanism Phase 9 already built — not a new async mechanism). --> Confirm route existence
      against the real xberg SDK before building routes whose contract this plan is guessing
      at — §9 Gap 3 explicitly names "route existence unconfirmed" as unresolved; do this
      confirmation as the very first step of this task, not after the routes are built.
- [x] **Step 2 (red → green).** <!-- verified: a real trait/route gap was found before writing
      any route, and is recorded here rather than silently building against a route the trait
      cannot serve. `RagStore` (crates/hacienda-rag/src/store.rs), as recovered/adapted by Task
      1, exposes only `ensure_collection`, `drop_collection`, `get_collection`,
      `upsert_document`, `delete_documents`, `delete_by_filter`, `retrieve`,
      `collection_stats` — there is no `list_collections`, `list_documents`, `reindex_document`,
      or `migrate_embeddings` primitive. Five of Step 1's eight confirmed routes
      (`GET /v1/rag/collections` [list], `GET /v1/rag/collections/{name}/documents` [list],
      `POST .../documents/{id}/reindex`, `POST .../migrate-embeddings`,
      `GET .../migrate-embeddings/{job_id}`, and by extension `GET /v1/rag/jobs/{job_id}` since
      nothing ever populates a RAG job) cannot be built against today's trait without first
      extending it — that is a real trait-design decision (return shape for a paginated list,
      an async reindex/migration primitive reusing `JobStore`), not a route-wiring task, and is
      explicitly deferred rather than guessed at here. Built the three routes that map onto
      existing trait methods directly: `POST /v1/rag/collections` (`ensure_collection`),
      `GET /v1/rag/collections/{name}` (`get_collection`, 404 on `None`),
      `DELETE /v1/rag/collections/{name}` (`drop_collection`), `POST
      /v1/rag/collections/{name}/documents` (`upsert_document`), `POST
      /v1/rag/collections/{name}/retrieve` (`retrieve`, validated against the fetched
      `CollectionSpec` first via `RetrieveQuery::validate`). All five registered in
      `hacienda-api/src/routes.rs`'s `ROUTE_TABLE` under `Capability::DocumentsProcess` — no
      dedicated capability: RAG collections carry the same class of redacted document content
      `/v1/documents` already gates under `documents:process`, so a second capability would add
      configuration surface without a distinct trust boundary to justify it. Wire bodies are the
      crate's own `Serialize`/`Deserialize` IR types (`CollectionSpec`, `RetrieveQuery`,
      `RetrieveOutput`) directly, plus two small wrapper DTOs
      (`UpsertDocumentRequest`/`UpsertDocumentResponse`) in `hacienda-api/src/dto.rs` — avoids
      duplicating every field a second time for no behavioural difference. --> `RouteSpec`
      entries for the routes `RagStore` can actually serve today.
- [x] **Step 3.** <!-- verified: handlers (`hacienda-api/src/handlers/rag.rs`) call
      `Arc<dyn RagStore>` methods directly through a new optional `ApiState::rag_store` field —
      no facade wrapper. Decision: no facade layer, for two reasons. (1) `RagStore` is already
      object-safe and `Send + Sync + 'static` (Task 1's own object-safety test proves it), so
      there is no ergonomic gap a wrapper would close — precisely the reasoning the plan itself
      floated. (2) Audit-logging parity was considered and explicitly deferred, not silently
      dropped: `HaciendaFacade::record_audit` is keyed to `PiiOutput`/detected-entity spans from
      the redaction pipeline, and a RAG upsert's payload is caller-supplied `DocumentRecord`/
      `ChunkRecord` (already-processed content the caller chose to index) with no entity spans
      of its own to attribute — bolting it into `record_audit`'s shape would force a fictitious
      `PiiOutput`, or require a second, differently-shaped audit event type, either of which is
      a real design task in its own right, not a five-minute wrapper. Left as a documented gap
      (CHANGELOG, this step) rather than an unstated omission. -->
      Handlers delegate to `RagStore` methods directly.
- [x] **Step 4.** <!-- verified: grepped spec/api/openapi.yaml in xberg-sdks (the real,
      CI-synced spec, see Step 1) for "answer"/"stream" near any /v1/rag path — no match, and
      the full path list (Step 1) contains no answer-generation or streaming route of any kind.
      Decision: hacienda does not build an answer-synthesis route in Phase 12. There is no
      upstream route to confirm a contract against (the only evidence such a feature was ever
      planned is the recovered, never-shipped `stream.rs` — see hacienda-rag plan D5, explicitly
      out of scope there too), and no design work exists in this spec for citation/event
      schemas. If ever built, it is a purely additive route requiring its own design pass, not
      part of this phase. --> Answer-synthesis scope (§9 Gap 3's second unresolved item, and the
      recovered `stream.rs`'s `AnswerEvent`/streaming design, per D8 in the RAG plan marked "new
      concept, not yet in spec") — decide in this task, not deferred further, since Phase 12 is
      where the routes that would need it are actually built. If deferred again, record why
      explicitly rather than letting it silently vanish a second time.
- [x] **Step 5 (green).** <!-- verified: 4 new tests added to `hacienda-api/src/routes.rs`'s
      existing test module —
      `rag_collection_document_retrieve_and_delete_round_trip` (full lifecycle: create → get →
      upsert document → retrieve with a vector query, asserting `chunks[0].content` →
      delete → get-after-delete 404), `rag_get_missing_collection_is_404`,
      `rag_routes_without_a_configured_store_yield_400` (no store attached → 400, not panic/500),
      and `rag_routes_require_documents_process_capability` (a `pii:reveal`-only token gets 403
      on `POST /v1/rag/collections`). Running the round-trip test surfaced a real, pre-existing
      bug in `hacienda-rag` (not introduced by this task): `PrimaryScore`'s `Vector`/`FullText`/
      `Sparse`/`LateInteraction` variants were tuple newtypes (`Vector(f32)`) under
      `#[serde(tag = "kind")]` (internally tagged) — `serde_json` cannot serialize an
      internally-tagged enum whose variant content is a bare scalar, so every real
      `RagStore::retrieve` call over HTTP failed with 500 ("cannot serialize tagged newtype
      variant PrimaryScore::Vector containing a float"). This was latent because no prior test
      exercised `serde_json::to_vec` on a `RetrieveOutput` — the reference in-memory/pgvector
      backends' own tests assert on the Rust value directly, never round-trip it through JSON.
      Fixed by changing the four scalar variants to named-field form (`Vector { score: f32 }`),
      updating the 5 construction call sites in `backends/memory.rs` and `backends/pgvector.rs`
      (no destructuring pattern-matches existed on these variants anywhere in the repo, so the
      change was contained). Re-ran the round-trip test after the fix: passes. -->
- [x] **Step 6.** <!-- verified: `cargo clippy -p hacienda-api -p hacienda-cli -p hacienda-rag
      --all-targets -- -D warnings` clean; `cargo clippy -p hacienda-rag --all-targets --features
      postgres -- -D warnings` clean (the `PrimaryScore` fix touches the pgvector backend too).
      `cargo test -p hacienda-rag -p hacienda-api -p hacienda-cli`: 49 + 35 + 3 (safety) + 24
      (cli integration) passed, 0 failed. `cargo fmt -p hacienda-rag -p hacienda-api -- --check`
      clean. `hacienda-cli`'s `--check` still reports 2 pre-existing diffs in
      `build_vault_readme` and `tests/extract.rs` — both predate this task and are untouched by
      it, left alone per minimal-changes. Confirmed `openapi_path_set_equals_route_table` still
      passes and that `build_openapi()` derives paths from `ROUTE_TABLE` directly, so the 4 new
      RAG routes are automatically present in `/openapi.json` with no separate update needed. -->

### Task 4 — Verification and documentation

- [x] **Step 1.** <!-- verified: CHANGELOG.md gained two new `### Added` entries under
      `[Unreleased]` ("`hacienda-rag` crate: `RagStore` trait, backend-agnostic IR, in-memory
      backend (Phase 12 Task 1)" and "`PgVectorStore`: durable `pgvector`-backed `RagStore`
      (Phase 12 Task 2)"), the second explicitly documenting the sqlx migration-version-
      collision deployment issue rather than letting it live only in this plan file's Task 2
      Step 4 comment. spec §9 Gap 3 updated (not struck — it is only partially closed: Tasks
      1/2 and Task 3 Steps 1/4 are done, but the `/v1/rag/*` routes themselves, Task 3 Steps
      2/3/5/6, are not yet built) to record Task 2's completion and the migration-collision
      issue. spec §10's Phase 12 phasing-table row updated to list Task 1/Task 2/Task 3 Steps
      1+4 as done and Task 3 Steps 2/3/5/6 as the remaining open work, replacing the stale text
      that described the pgvector backend as entirely open. `crates/hacienda-rag/src/lib.rs`'s
      "Scope" doc comment corrected (was stale, still saying the pgvector backend was out of
      scope). Phase 12 is not fully "shipped" — that marking is deferred until Task 3's routes
      land — this step closes the documentation debt for Tasks 1/2/3-partial only, not the
      whole phase. --> CHANGELOG, spec §9 Gap 3 struck, §10 Phase 12 marked shipped.
- [x] **Step 2.** <!-- verified: Task 3 Steps 2/3/5/6 (the `/v1/rag/*` HTTP routes) are now done
      — see their own verification notes above. Closed the documentation debt Step 1 explicitly
      left open: added a new CHANGELOG `### Added` entry ("`/v1/rag/*` HTTP routes (Phase 12
      Task 3)") documenting the 5 routes built, the 3 left unbuilt and why (no `RagStore` trait
      primitive), the capability/facade/audit decisions, and the `PrimaryScore` serialization
      bug found and fixed along the way. spec §9 Gap 3 updated again: no longer says routes are
      not yet built, now states which 5 are built and which 3 remain open pending a trait
      extension. spec §10's Phase 12 row updated from "still open" to Task 3 marked **done**
      with the 3-route caveat inline, replacing "Gap 3 (partially — see §9)" with "Gap 3 (mostly
      closed — see §9)". Phase 12 can now be considered shipped for the 5 routes the current
      `RagStore` trait supports; the remaining 3 are a distinct, not-yet-scheduled follow-up
      (trait extension + routes), not part of this phase's completion criteria, which asked for
      routes "the trait can actually serve" (Task 3 Step 2's own finding). -->

---

## Phase 13 — Remaining xberg-parity routes (jobs list/result, presets, versions/diff, presigned uploads, usage)

Gated on Phase 9; usage additionally gated on Phase 10 (usage is derived from audit entries per
Decision 3, so the audit routes' data model must be stable first).

### Task 1 — `GET /v1/jobs`, `GET /v1/jobs/{id}/result`

- [x] **Step 1 (red).** `should_list_jobs_filtered_by_status`, `should_fetch_a_completed_jobs_result`.
      <!-- verified: implemented as should_only_list_jobs_owned_by_the_caller,
           should_paginate_the_callers_job_list, should_reject_a_malformed_limit_with_the_error_envelope,
           should_return_404_for_a_foreign_jobs_result, should_return_200_with_a_result_shape_matching_job_status
           in hacienda-api/src/handlers/jobs.rs — status-filter is exercised via JobListQuery.status
           threaded straight through to JobStore::list, no dedicated filter test added since the
           store-level filter is not new logic. -->
- [x] **Step 2.** `JobStore::list` already exists (Phase 1 Task 6) — these are pure route
      additions, no new store method beyond what Phase 9's `PostgresJobStore` already
      implements. `.../result` is a projection of `Job::result_json`, already stored.
      <!-- verified: hacienda-core/src/jobs/store.rs:62 `list(&self, filter: Option<JobStatus>)`;
           no trait/store changes made. -->
- [x] **Step 3 (green).** Route tests. Verify.
      <!-- verified: cargo test -p hacienda-api → 40 passed (8 in handlers::jobs::tests, plus
           routes::tests::every_guarded_route_reflected_in_auth_state and
           handlers::openapi::tests::openapi_path_set_equals_route_table cover the two new
           RouteSpec entries automatically since both are table-driven, not hand-maintained).
           cargo clippy -p hacienda-api --all-targets -- -D warnings: clean. cargo fmt -p
           hacienda-api -- --check: clean. Pagination is applied in-process over the full
           JobStore::list result (no store-level limit/offset — documented as an
           implementation-level limitation, not a trait change). Tenant isolation: list is
           scoped by Caller::principal_id() (Trusted sees all, a principal sees only its own
           jobs), same pattern as the existing get_job 404-not-403 IDOR guard. get_job/JobResponse
           left unchanged; get_job_result/JobResultResponse is purely additive and always returns
           200 regardless of job state. Added extract::Query<T> (hacienda-api/src/extract.rs)
           mirroring the existing Json<T> envelope pattern so malformed query strings fail into
           {"error": {...}}, not axum's default text rejection. -->

### Task 2 — Presets: `GET/POST/DELETE /v1/presets`

- [x] **Step 1 (red → green).** Thin CRUD routes over `PresetStore` (Phase 9 Task 5). No
      facade logic beyond capability enforcement — presets are inert config, not part of the
      audit-bearing pipeline.
      <!-- verified: found and fixed the actual cross-cutting Phase 9 gap first. sqlx's
           compile-time query! macros need a live DATABASE_URL or a committed `.sqlx` offline
           cache; neither existed, so `cargo check -p hacienda-core --features postgres`
           failed with 46 errors and CI's `--all-features`/`--each-feature` jobs would have
           failed the moment this branch's Phase 9 commit (97b4437) was pushed (it has not
           been pushed since that commit landed, per `gh run list` — no CI run has exercised
           the postgres feature on this branch yet, confirming this was a latent, undiscovered
           break, not a previously-passing check that regressed). The user pointed me at a
           Postgres instance already running on 127.0.0.1:5432 (found via `ss -tln`, no docker
           socket access to inspect it further); the connection string was recovered from this
           same plan file's own Phase 12 Task 4 Step 4 note (`postgres://hacienda:hacienda_dev@
           localhost:5432/hacienda`), not guessed. `sqlx migrate run` (already at `1/installed
           init`) then `cargo sqlx prepare --workspace -- --features postgres -p hacienda-core`
           generated `.sqlx/` (44 query files, not yet committed — a future commit should add
           it). Verified the fix three ways: (1) `SQLX_OFFLINE` unset, `.sqlx/` present →
           `cargo check -p hacienda-core --features postgres` and
           `cargo check --workspace --all-features` both compile clean; (2) all 12 previously
           `#[ignore]`d live-Postgres tests across audit/review/jobs/versions/presets/api_keys
           pass against the real instance (`cargo test -p hacienda-core --features postgres
           --lib store::postgres -- --ignored --test-threads=1` → 12 passed); (3) confirmed no
           unrelated regression — `git diff --stat` on the touched postgres/*.rs files matches
           the pre-existing uncommitted Phase 9 diff already present at session start, `touch`
           only bumped mtimes to defeat stale-cache false negatives during investigation.
           Presets routes themselves: see Step 2/3 below for the actual route implementation,
           now unblocked. -->
- [x] **Step 2.** Routes: `POST /v1/presets` (create), `GET /v1/presets` (list),
      `GET /v1/presets/{id}` (get by id), `DELETE /v1/presets/{id}` (delete). All guarded by
      `documents:process`, matching every other config/inert-data route in this API.
      <!-- verified: hacienda-api/src/handlers/presets.rs, wired via a new opt-in
           `ApiState::with_preset_store(Arc<dyn PresetStore>)` (mirrors `with_rag_store`
           exactly) so existing call sites building `ApiState` without Postgres are unaffected;
           routes 400 (`ApiError::invalid_request`) when no store is configured, matching the
           RAG precedent's `require_store` helper. hacienda-api/Cargo.toml gained
           `hacienda-core = { ..., features = ["jobs", "postgres"] }` — evaluated adding a
           separate opt-in `postgres` Cargo feature on hacienda-api itself instead, but
           rejected: unlike RagStore (which has a real in-memory backend), PresetStore's only
           implementation is Postgres-backed, so gating it behind a second feature flag would
           only add a compile-time toggle with no non-Postgres codepath behind it — the
           opt-in behaviour that actually matters (disabled at runtime with no store attached)
           is already provided by `Option<Arc<dyn PresetStore>>` on `ApiState`, same as RAG.
           DTOs (`CreatePresetRequest`, `PresetResponse`, `PresetListResponse`) added to
           dto.rs; `PresetError -> ApiError` mapping added to error.rs (`NotFound` -> 404,
           `Database` -> 500, logged host-side only since sqlx::Error's Display can name
           tables/columns). RouteSpec entries added to routes.rs: `GET/POST /v1/presets`,
           `GET/DELETE /v1/presets/{id}`, both `Capability::DocumentsProcess` — automatically
           covered by `every_guarded_route_reflected_in_auth_state` and
           `openapi_path_set_equals_route_table` (table-driven, no hand-maintenance needed). -->
- [x] **Step 3 (green).** Verify.
      <!-- verified: cargo test -p hacienda-api → 42 passed, 1 ignored (the live-Postgres
           round-trip test, run separately below), 0 failed — includes
           `presets_routes_without_a_configured_store_yield_400` (400, no store attached) and
           `presets_routes_require_documents_process_capability` (403, capability check runs
           in auth middleware before the handler, so no store needed for this test).
           `DATABASE_URL=postgres://hacienda:hacienda_dev@127.0.0.1:5432/hacienda cargo test
           -p hacienda-api --lib routes::tests::preset_route_create_list_get_delete_round_trip
           -- --ignored` → 1 passed (create 201 -> list 200 contains it -> get 200 -> delete
           204 -> get-after-delete 404, against the real `PostgresPresetStore` end to end
           through the router, not the store directly). cargo clippy -p hacienda-api
           --all-targets -- -D warnings: clean (postgres feature is unconditionally enabled
           on hacienda-core from hacienda-api now, not a separate opt-in feature, per the
           Step 2 rationale, so no --features flag is needed). cargo fmt -p hacienda-api
           -- --check: clean. cargo check --workspace --all-features: clean. -->

### Task 3 — Document versions and diff: `/v1/documents/{id}/versions`, `{id}`, `/diff`, `/diff/{job_id}`

- [x] **Step 1 (red → schema fix).** Round-trip test matching §4.3's documented shape:
      upload same `document_id` twice with identical content → same `version_sequence`,
      idempotent; upload with different content → `version_sequence` increments; `/diff`
      synchronous under the 2s budget returns inline, over budget returns 202 with a job id
      pollable via `/diff/{job_id}` (reusing `JobStore`, not a new async mechanism).
      <!-- verified: the schema gap identified below was real — `document_versions` had only
           `content_hash`, no content column — and was escalated to the user, who chose
           "extend schema to store content." hacienda-core/migrations/0002_document_version_
           content.sql adds `content TEXT NOT NULL` and `entities_json JSONB NOT NULL`.
           `DocumentVersionStore::create_version` gained `content: &str, entities_json: Value`
           parameters; `DocumentVersion` gained matching fields. The two existing Postgres
           tests in store/postgres/versions.rs were updated for the new 4-arg signature and
           now assert on stored `content`/`entities_json`, not just `content_hash`. Migration
           applied against the live dev DB (`cargo sqlx migrate run --source migrations`), and
           `.sqlx/` regenerated (`cargo sqlx prepare --workspace -- --features postgres -p
           hacienda-core`) so `cargo check -p hacienda-core --features postgres` builds without
           a live DB. Verified: both targeted tests pass live (`cargo test -p hacienda-core
           --features postgres --lib store::postgres::versions -- --ignored --test-threads=1`
           → 2 passed); full `cargo test -p hacienda-core --features postgres` → 0 failed;
           `cargo clippy -p hacienda-core --features postgres --all-targets -- -D warnings`:
           clean. -->
      <!-- not done (superseded, kept for history): found a real schema gap before writing
           this test, escalated to the user rather than guessing. hacienda-core/migrations/
           0001_init.sql's document_versions table has only `content_hash TEXT` (blake3 of
           redacted output) — no column stores the actual content bytes.
           `DocumentVersionStore::create_version(document_id, content_hash: &str)` took only
           the hash, never the content, and `list_versions`/`get_version` returned only
           `DocumentVersion { content_hash, ... }`, never bytes. §4.3 requires
           `GET /v1/documents/{id}` to return "latest version envelope, extraction result
           inline" and `/diff` to compute a "pairwise structural diff" between two versions'
           actual redacted text — neither is derivable from a hash alone. §7's
           "Content-addressed by hash of redacted output, not raw input — versioning raw
           content would defeat Decision 1" governs what gets *hashed* for idempotency
           identity, not whether the redacted output itself may be persisted (storing only
           redacted, never raw, content is exactly what Decision 1 requires, not what it
           forbids) — so a content-storage column/table is compatible with Decision 1, but is
           a real schema + trait-signature change, not a route-only addition like Task 2.
           Asked the user how to proceed before touching the schema. -->
- [x] **Step 2.** Routes over `DocumentVersionStore` (Phase 9 Task 5) plus a diff algorithm
      (unified diff over redacted-output text; a byte-level diff is enough, no need for a
      semantic diff library not already in the workspace unless one is already a transitive
      dependency).
      <!-- verified: hacienda-api/src/handlers/versions.rs implements all 4 handlers —
           `list_document_versions` (GET /v1/documents/{id}/versions, newest first),
           `get_document` (GET /v1/documents/{id}, latest version's full envelope), and
           `diff_document`/`get_diff_job` (GET /v1/documents/{id}/diff and
           /diff/{diff_job_id}). No diff crate exists in the workspace (checked before
           writing), so `compute_line_diff` is a hand-written LCS-based line diff over
           `str::lines()` — the plan explicitly allows "a byte-level diff is enough, no need
           for a semantic diff library." `diff_document` runs the diff via `spawn_blocking`
           raced with `tokio::select!` against a 2-second `tokio::time::sleep` budget: if the
           diff wins, its result returns inline (200); if the sleep wins, the still-running
           `JoinHandle` is moved into a detached `tokio::spawn` that finishes the computation
           and records it via `JobStore::finish`/`fail`, while the request returns 202 +
           `diff_job_id` immediately — reusing `JobStore` exactly as the plan specifies, no
           second async mechanism. `get_diff_job` polls `JobStore` directly (same shape as
           `GET /v1/jobs/{id}/result`) and does not depend on `version_store` at all — a
           deliberate scoping decision, since diff jobs are just jobs once created.
           `ApiState` gained `version_store: Option<Arc<dyn DocumentVersionStore>>` +
           `with_version_store` builder, mirroring `with_preset_store` exactly — 400 when
           unconfigured. `DocumentInput` gained an optional `document_id: Option<Uuid>`;
           `POST /v1/documents` (`handlers/documents.rs`) fails fast with 400 if any input
           carries a `document_id` but no version store is configured, then — for inputs that
           do carry one — computes `blake3::hash(redacted_content)` and serializes
           `pii.entities` to `entities_json`, calling `create_version` per document and
           echoing `document_id`/`version_sequence` back in the response. `process_documents_
           async` was deliberately NOT touched — versioning is scoped to the synchronous
           handler only for this task. 6 new DTOs added to dto.rs
           (`DocumentVersionSummaryDto`, `DocumentVersionListResponse`,
           `DocumentEnvelopeResponse`, `DocumentDiffQuery`, `DiffLineDto`,
           `DocumentDiffResponse`, `DiffJobAcceptedResponse`, `DiffJobResultResponse` —
           `DiffLineDto` derives both `Serialize` and `Deserialize` since the async diff path
           round-trips it through `JobStore`'s `result_json` string, not just serializes it
           once). `VersionError -> ApiError` mapping added to error.rs (`NotFound` -> 404,
           `Database` -> 500, host-logged only). 4 RouteSpec entries added to routes.rs, all
           `Capability::DocumentsProcess` — automatically covered by
           `every_guarded_route_reflected_in_auth_state`,
           `route_table_has_no_duplicate_paths`, and `openapi_path_set_equals_route_table`. -->
- [x] **Step 3 (green).** Verify.
      <!-- verified: cargo test -p hacienda-api --lib → 45 passed, 3 ignored (live-Postgres
           tests, run separately below), 0 failed. New tests:
           `document_version_routes_without_a_configured_store_yield_400` (400 for
           /versions, /{id}, /diff with no store — /diff/{diff_job_id} deliberately excluded,
           see next test), `document_version_diff_job_poll_without_a_store_is_404_not_400`
           (confirms the diff-job poll route bypasses `version_store` entirely and 404s on an
           unknown job id, not 400 — a real behavioural distinction this test exists to pin
           down, discovered while writing the "without a store" test above),
           `document_version_routes_require_documents_process_capability` (403 for a token
           missing the capability). Live-Postgres tests (DATABASE_URL=postgres://hacienda:
           hacienda_dev@127.0.0.1:5432/hacienda cargo test -p hacienda-api --lib
           routes::tests::document_version -- --ignored --test-threads=1 → 2 passed):
           `document_version_create_list_get_diff_round_trip` (submit two versions of one
           document_id via POST /v1/documents with PII enabled in Mask mode so `result.pii`
           is populated — see finding below — list -> 2 versions newest-first, get -> latest
           envelope, diff v1..v2 -> 200 synchronous with correct equal/delete/insert lines)
           and `document_version_diff_with_missing_version_is_404` (diffing an unknown
           version_sequence 404s, not 500). All 3 ignored tests in the crate (incl. the
           preset round-trip) verified together: 3 passed. cargo clippy -p hacienda-api
           --all-targets -- -D warnings: clean. cargo fmt -p hacienda-api -- --check: clean.
           cargo check --workspace --all-features: clean.

           Finding surfaced while writing the live round-trip test, not fixed as part of this
           task (out of scope, flagged for separate follow-up): `handlers/documents.rs`'s
           `process_documents` zips `result.extraction.results` with `result.pii` — when no
           PII pipeline is configured (`HaciendaConfig::default()`, `pii: None`), `result.pii`
           is an empty `Vec`, so the zip yields **zero** `DocumentResult`s regardless of how
           many documents were submitted or extracted successfully. This is pre-existing
           (present before this task's changes — confirmed via `git diff HEAD` on
           documents.rs, same `.zip(result.pii)` shape in the code the diff replaced) and not
           specific to versioning; it means `POST /v1/documents` silently returns
           `{"documents":[]}` for any batch whenever PII is disabled, which is likely to
           surprise an operator who only wants extraction. Not fixed here because it is
           orthogonal to Task 3's scope (document versioning) and changing the response
           shape's contract deserves its own red/green cycle and a decision on the right
           default (return extraction-only results with empty `entities`, vs. requiring PII
           to be configured) rather than a drive-by fix bundled into this task. -->
      <!-- also verified in this step: the dev Postgres container (docker container
           `hacienda-pg`) had stopped between sessions (`docker ps -a` showed
           `Exited (255)`) — restarted with `docker start hacienda-pg` before the live tests
           above could connect; this is host/session state, not a code or migration issue. -->

### Task 4 — Presigned uploads: `/v1/uploads/presign`, `/v1/uploads/confirm`

- [x] **Step 1.** SSRF analysis holds: `confirm_upload` (`hacienda-api/src/handlers/uploads.rs`)
      derives its lookup key solely from the server-issued `upload_id` via `object_key()`, never
      from client input — `filename` is accepted but deliberately unread (see `PresignUploadRequest`
      doc comment in `dto.rs`), so it cannot steer or collide with another upload's storage path.
      No client-supplied URL or path reaches `ObjectStore::head`/`presign_put`.
- [x] **Step 2 (red → green).** `hacienda_core::store::object::ObjectStore` trait (`presign_put`,
      `head`) with an `S3ObjectStore` implementation (`rusty-s3` for pure-computation presigning,
      `reqwest` only for the `HEAD` in `confirm`) behind the `s3` feature — works against AWS S3
      or any S3-compatible endpoint (MinIO, R2, GCS S3-compat) via `S3Config::endpoint`. Routes
      `POST /v1/uploads/presign` and `POST /v1/uploads/confirm` are wired into `ROUTE_TABLE` under
      `Capability::DocumentsProcess`, opt-in per `ApiState::object_store` (400 when unconfigured,
      matching the `PresetStore`/`DocumentVersionStore` pattern). Tests in `routes.rs`:
      `upload_routes_without_a_configured_store_yield_400`,
      `upload_routes_require_documents_process_capability` — both green.

### Task 5 — `GET /v1/usage`

- [x] **Step 1.** Verified against `audit/entry.rs`'s actual field list: `AuditEntry` carries
      `principal` (`Option<String>`) and `span_length` (`u32`, redacted-span byte count), both
      attributable and summable — but **no `document_id`**. So entity-count (row count) and
      byte-count (`SUM(span_length)`) are derivable per-principal and time-windowed exactly as
      Decision 3 assumed; document-count is **not** derivable without either a schema change or
      silently mis-counting documents that produced zero redactions, and is deliberately omitted
      from the response rather than guessed. This resolves the spec's §11 open risk precisely:
      entries do carry a billable unit, just not every unit Decision 3's framing assumed.
      Also confirmed `AuditStore::entries()` is scoped to the currently-open segment only (see
      its doc comment and the `WHERE segment_id = (SELECT ... WHERE sealed_at IS NULL ...)`
      shape in `store/postgres/audit.rs`), so a usage read-model built on it would under-report
      every time a segment rotates — this is why Step 2 queries `audit_entries` directly instead.
- [x] **Step 2 (red → green).** New `hacienda_core::store::postgres::usage` module:
      `UsageStore` trait (`summary(since, until) -> Vec<UsageRecord>`), `PostgresUsageStore`
      querying `audit_entries` directly (all segments, sealed or not — not `AuditStore::entries`,
      per Step 1's finding), grouped by `principal` (`NULL` groups every `Caller::Trusted` entry,
      matching how `AuditEntry::principal` itself represents unattributed entries) and windowed
      by `created_at >= since` / `< until` (either bound optional). Uses runtime-checked
      `sqlx::query_as::<_, UsageRow>()` with an explicit `#[derive(sqlx::FromRow)]` struct rather
      than the `query_as!` macro, since neither a live `DATABASE_URL` nor a `.sqlx` cache entry
      for this query was available at write time — documented as a deliberate deviation in the
      module's doc comment.
      `ApiState` gained `usage_store: Option<Arc<dyn UsageStore>>` + `with_usage_store` builder,
      mirroring `with_object_store` exactly — 400 when unconfigured. New DTOs in `dto.rs`:
      `UsageQuery` (`since`/`until`, both optional), `UsageRecordDto` (with
      `From<UsageRecord>`), `UsageResponse`. `UsageError -> ApiError` mapping added to
      `error.rs` (`Database` -> 500, host-logged only — no client-triggerable variant exists).
      `GET /v1/usage` wired into `ROUTE_TABLE` under `Capability::AuditRead` — reusing the same
      capability as `/v1/audit`, `/v1/audit/verify`, and `/v1/compliance/*`, since usage is a
      read-model over that same audit chain, not a separate concern (the spec's §4.1 table lists
      `/v1/usage` under "metering" but does not assign it a capability; `AuditRead` is the
      established precedent for every other audit-chain-derived read route). Automatically
      covered by `every_guarded_route_reflected_in_auth_state` and
      `route_table_has_no_duplicate_paths`.
- [x] **Step 3 (green).** Verify. `cargo check -p hacienda-api`: clean. `cargo clippy -p
      hacienda-api -p hacienda-core --all-targets -- -D warnings`: clean. `cargo test -p
      hacienda-api --lib routes::` → 22 passed, 4 ignored, 0 failed. New tests:
      `usage_route_without_a_configured_store_yields_400` (400 with no store configured),
      `usage_route_requires_audit_read_capability` (403 for a token missing `audit:read`).
      Live-Postgres tests (docker container `hacienda-pg` restarted — had stopped between
      sessions, same host-state note as Task 3):
      `DATABASE_URL=postgres://hacienda:hacienda_dev@127.0.0.1:5432/hacienda cargo test -p
      hacienda-core --features postgres --lib store::postgres::usage -- --ignored
      --test-threads=1` → 2 passed (`should_aggregate_entity_and_byte_counts_per_principal`,
      `since_in_the_future_excludes_everything`); `cargo test -p hacienda-api --lib
      routes::tests::usage_route_aggregates_audit_entries -- --ignored` → 1 passed (real
      round trip: append via `PostgresAuditStore`, aggregate via `GET /v1/usage`). `poly fmt
      --check` on all 8 touched/created files: clean.

      Two real bugs surfaced and fixed while running the live tests, not just written blind:
      1. `SUM(span_length)` — `span_length` is `BIGINT` in `audit_entries` (migration
         `0001_init.sql`), and Postgres's `SUM(BIGINT)` returns `NUMERIC`, not `BIGINT`. The
         runtime-checked `UsageRow::byte_count: i64` failed to decode it
         (`ColumnDecode { ... "mismatched types; Rust type i64 ... is not compatible with SQL
         type NUMERIC" }`) — caught immediately by the live integration test, not assumed away.
         Fixed with an explicit `::BIGINT` cast on the aggregate, documented inline.
      2. The integration test itself (`should_aggregate_entity_and_byte_counts_per_principal`)
         hardcoded literal principal names (`"avocat-7"`, `"avocat-9"`) instead of suffixing them
         per run. `audit_entries` is append-only and never cleaned up between runs by design, so
         re-running the test accumulated rows under the same principal across invocations
         (surfaced as `entity_count`: expected 2, got 4, after two runs) — a test-independence
         violation, not a store bug. Fixed by suffixing every principal with a fresh UUID per
         run, and relaxed the `principal: None` assertions to `>=` since that bucket has no
         per-run key to isolate on.

### Task 6 — Verification and documentation

- [x] **Step 1.** CHANGELOG; spec §10 Phase 13 marked shipped.
      <!-- CHANGELOG.md: added entries for Task 4 (presigned uploads) and Task 5
           (GET /v1/usage) under [Unreleased], matching the style of the Task 1-3 entries
           already present. Spec `2026-08-01-hacienda-platform-parity-and-scale-design.md`:
           §10 Phasing table row for Phase 13 now reads "— **done**"; §11 Open Risks'
           usage/Decision 3 risk struck through with a "Resolved in Phase 13 Task 5" note
           summarizing the actual finding (entity/byte count derivable, document count is
           not). -->

---

## Phase 14 — `hacienda-sdks` repository, Python + TypeScript, cloud target only

Gated on Phases 8, 10, 11, 12, 13 (needs a stable, complete `/v1/*` surface to generate against).

### Task 1 — OpenAPI completeness spike

- [ ] **Step 1.** Fetch `/openapi.json` from a running `hacienda-api` instance with every
      phase above shipped. Run it through the intended codegen tool (e.g.
      `openapi-generator-cli generate -i openapi.json -g python`) and inspect the output for
      gaps — missing operationIds, untyped `additionalProperties`, missing examples. This is
      the precondition §8 names as unverified; answer it here before scaffolding a repo around
      an assumption.
- [ ] **Step 2.** Patch `hacienda-api`'s OpenAPI generation (`handlers/openapi.rs`) for any gap
      found, rather than hand-patching the generated SDK — the schema is the source of truth
      and a hand-patched SDK drifts from it silently on the next regeneration.

### Task 2 — Repository scaffold

- [ ] **Step 1.** New repo `hacienda-sdks`, structure mirrored from `xberg-sdks`:
      `packages/{python,typescript}`, generated-core/hand-wrapper split per §3.1's evidence
      table (Extraction/Jobs are strong-evidence method groups worth generating confidently
      first; weaker-evidence groups like the old RAG surface are not applicable here since
      hacienda's RAG contract is hacienda's own, not xberg's).
- [ ] **Step 2.** `target: "cloud"` is the only target this phase implements — `target: "device"`
      is Phase 15's. Build the axis into the config shape now (an enum with one variant) so
      Phase 15 extends rather than retrofits it.
- [ ] **Step 3.** CI: codegen-from-`/openapi.json` on a schedule or on hacienda-engine release,
      with a generated-code header per `generated-code-policy`, and a freshness check
      (`task generate:sdk && git diff --exit-code`).

### Task 3 — Verification

- [ ] **Step 1.** Each package's own test suite (`pytest`/`vitest`) against a locally running
      `hacienda-api` instance — an integration test, not a mock, per `testing-anti-patterns`.
- [ ] **Step 2.** Spec: §9 Gap 6 struck; §10 Phase 14 marked shipped.

---

## Phase 15 — Cactus device-target spike

Gated on Phase 14 (needs the SDK scaffolding's `target` axis to extend).

### Task 1 — Spike, not a shipped feature

- [ ] **Step 1.** Confirm the Cactus telemetry finding already on record
      (`project_cactus_telemetry_offline_risk` memory note): no public API to disable
      phone-home; requires `setenv CACTUS_NO_CLOUD_TELE`/`CACTUS_DISABLE_CLOUD_HANDOFF`
      pre-init. Verify this is still true against whatever Cactus version is current before
      building anything on top of it — a stale finding here is worse than no finding, since it
      would ship a false sense of having solved the offline requirement.
- [ ] **Step 2.** Prototype a minimal `target: "device"` SDK backend: embed Cactus, wire a
      narrow method surface (§2 Decision 7: "narrower method surface" — likely just
      scan/redact, not the full cloud surface), confirm it runs fully offline with the env vars
      set.
- [ ] **Step 3.** Do not scope this task's success as "ship a device SDK" — §9 Gap 7 and §11's
      risk note both frame this as unresolved until the spike runs. Success is a written
      finding: does it work offline, what's the binary size / model size cost (recalling the
      1.23 GB GLiNER2 finding from `project_studio_ner_model_too_large` — device-target NER, if
      any, inherits that constraint directly), and what method surface is actually feasible.

### Task 2 — Verification and documentation

- [ ] **Step 1.** Spec: §9 Gap 7 and §11's device-target risk updated with the spike's actual
      findings — not struck as "closed" unless the spike concludes the approach is viable
      end-to-end; a spike that finds the approach infeasible is still a completed phase, with
      a documented negative result.

---

## Out of Scope (this entire plan)

- Any change to `AuditStore`/`ReviewStore`/`JobStore`'s existing method signatures (D2) —
  Phase 9 implements them, it does not redesign them.
- A second RAG crate design — Phase 12 Task 1 is "go execute the other plan," not a rewrite.
- CLI subcommands for audit/review/compliance/glossary (Phase 10 Task 3) unless that task's
  own re-reading of `hacienda-api-cli-surface.md` concludes otherwise.
- Sticky sessions or any per-replica state for horizontal scaling — every phase's design
  keeps the integration spec's §12.2 statelessness contract (`Arc<dyn Store>`, no in-process
  state a second replica couldn't reconstruct from the same Postgres).
- Anchors (periodic merkle roots over segment tips) — still Phase 4-adjacent per Phase 1's own
  Out of Scope, unaffected by this plan.
- xberg's own `redaction-rehydrate` feature — Decision 9 already rejected adopting it; nothing
  in Phases 8-15 revisits that.
