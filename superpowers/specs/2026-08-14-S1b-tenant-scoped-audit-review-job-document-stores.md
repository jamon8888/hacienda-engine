# S1b — Tenant-scoped audit, review, job, document-version, and preset stores

**Date:** 2026-08-14
**Status:** Partially shipped — `AuditStore` (Task 2) is implemented, tested (unit +
live-Postgres), and merged; `ReviewStore`/`JobStore`/`DocumentVersionStore`/`PresetStore`
(Tasks 3-6) remain designed but not implemented. See §7 for what actually shipped and
where it diverges from the design below.
**Extends:** `2026-08-01-S1-tenancy-and-projects.md` — this is the part of S1 that was
never actually delivered (see §1)
**Sibling:** `2026-08-14-P3a-tenant-scoped-pseudonym-keys.md` — P3a covers the
pseudonymisation key layer specifically; this spec covers everything else S1 promised and
didn't wire up. Read together, the two close S1 completely.
**Depends on:** nothing unshipped for the in-memory/file backends. The Postgres backend
(§4) needs one additive migration, no data loss, no downtime beyond a normal deploy.
**Blocks:** selling hacienda into any deployment where more than one tenant shares a
process — see §1 for why this is a correctness gap, not a missing nice-to-have.

---

## 1. Problem, verified against current code

`TenantCtx { tenant, actor, project }` (`hacienda-core/src/tenancy.rs`) exists.
`Caller::tenant_ctx()` (`hacienda-core/src/auth/mod.rs:255`) correctly resolves one for
both `Caller::Trusted` and an authenticated `Caller::Principal` — the auth layer already
knows which tenant a caller belongs to, from the JWT/API-key `AuthContext.tenant` field.

**None of that identity reaches a store.** Checked every store trait directly:

| Store | Method signatures | Tenant parameter? |
| --- | --- | --- |
| `AuditStore` (`audit/store.rs`) | `append`, `entries`, `history`, `tip`, `seals` | **No** |
| `ReviewStore` (`review/store.rs`) | `submit`, `assign`, `decide`, `list`, `get`, `stats` | **No** |
| `JobStore` (`jobs/store.rs`) | `create`, `get`, ... | `owner: Option<String>` — an **actor**, not a tenant (see §1.3) |
| `DocumentVersionStore` (`store/postgres/versions.rs`) | `create_version`, `list_versions`, `get_version` | **No** |
| `PresetStore` (`store/postgres/presets.rs`) | `create`, `get`, `get_by_name`, `list`, `delete` | **No** |
| `ObjectStore` (`store/object/mod.rs`) | `presign_put`, `head` | **No** |

And the Postgres schema (`hacienda-core/migrations/0001_init.sql`) has no `tenant_id`
column on `audit_segments`, `audit_entries`, `review_items`, `jobs`, `document_versions`,
`presets`, or `api_keys`. This is not a Rust-only gap — it is a database-schema gap too.

**Consequence, concretely.** In any deployment that authenticates more than one tenant
against one `HaciendaFacade`/API process (the only shape a shared SaaS deployment has),
today:

- Tenant B's `GET /v1/audit/entries` returns tenant A's redaction history too — including
  `principal` (who), `category`/`confidence`/`source` (what kind of PII), and enough shape
  to infer what tenant A processes, even though the text itself stays redacted.
- Tenant B's `GET /v1/review/queue` shows tenant A's pending PII decisions —
  `text_snippet` included, which is *unredacted* by design (a human reviewer needs to see
  it to decide).
- `GET /v1/documents/{id}/versions` and `/diff` are reachable by UUID alone — any
  authenticated caller who has or guesses another tenant's `document_id` gets that
  tenant's version history and redacted content.
- `GET /v1/presets` is global: `presets.name` has a bare `UNIQUE` constraint, so two
  tenants cannot even each have a preset named `"default"`.

This is worse than P3a's pseudonym-token leak in one respect: P3a requires two tenants to
process the *same value* to collide. This gap discloses cross-tenant activity and content
on the very first request from a second tenant, no coincidence required.

### 1.1 Why this predates being caught

S1's own spec (`2026-08-01-S1-tenancy-and-projects.md` §4, referenced from the platform
program) scoped `TenantCtx` and said stores "take it as a scope parameter" — but the
child spec was never written to the same implementation-ready detail P1/P2/P3 got, and
the type landing (`tenancy.rs`, `Caller::tenant_ctx()`) was mistaken for the feature
landing. `git blame`-adjacent: every store trait predates `tenancy.rs`, and no PR since
has touched a store signature to add the parameter.

### 1.2 The audit chain needs a design decision, not just a parameter

Every other store in the table above just needs a `tenant_id` column and a `WHERE`
clause. The audit chain is different: it is a **hash chain**, and `AuditStore::history`'s
own doc comment already establishes the scoping precedent this spec follows —
*"Scope: this node, not this deployment... a [`FileAuditStore`] sees only its own
`node_id` directory... What comes back is therefore this node's history."* Chains are
already partitioned by `NodeId`, once, by design.

**Decision: partition by `(NodeId, TenantId)`, not by a `tenant_id` filter over one
shared chain.** Two options exist; only the first is compatible with what the chain is
*for*:

- **One chain per `(node, tenant)` (recommended).** Each tenant's chain proves only their
  own activity, in isolation. Tenant A's `verify()` cannot fail because of tenant B's
  data; tenant A's auditor never has to trust that a filter was applied correctly to a
  chain that also contains other customers' entries. This is the same reasoning that
  already justifies per-node chains, applied one dimension further.
- **One shared chain, `tenant_id`-tagged, filtered on read (rejected).** Cheaper to
  build, but it means `GET /v1/audit/entries` for tenant A is "the whole chain, minus
  what got filtered out" — the chain's own hash linkage still covers tenant B's entries,
  so a compliance auditor verifying tenant A's chain is, structurally, trusting a
  redaction step performed by the query layer, not by the chain itself. That is exactly
  the shape of trust P1 exists to eliminate for document content; this spec should not
  reintroduce it for the audit trail that proves P1 worked.

**Consequence for `FileAuditStore`.** Segment files move from
`<audit_dir>/<node_id>/segment-*.jsonl` to `<audit_dir>/<node_id>/<tenant_id>/segment-*.jsonl`
— an additional directory level, not a new file format. `NodeId` and `TenantId` are both
already opaque path-safe identifiers (`TenantId`'s own doc comment: "intentionally opaque,
no ordering, no arithmetic" — the same property makes it safe to use as a path segment
once trivially sanitised, matching how `NodeId` is used today).

**Consequence for the Postgres backend.** `audit_segments` gains `tenant_id TEXT NOT
NULL`, and every uniqueness/lookup that currently keys on `node_id` alone keys on
`(node_id, tenant_id)` instead — segment lookup, "the currently open segment," seal
ordering. `audit_entries` does not need its own `tenant_id` column: it already scopes
through `segment_id`, and every segment now belongs to exactly one tenant.

### 1.3 `JobStore.owner` is not a substitute for tenant scoping

`JobStore::create`'s doc comment (`hacienda-core/src/jobs/store.rs:38-42`) already
describes isolation — *"the transport layer uses this field to enforce per-tenant
isolation... a principal who requests a job they do not own receives 404."* Read closely,
this is **actor-level** isolation (a user can't see another user's jobs), not
**tenant-level** isolation (an org's admin has no way to see their own org's jobs
collectively, and `owner: None` — every `Caller::Trusted` job — has no isolation at all,
by the same doc comment's own admission). This spec adds `tenant_id` alongside `owner`,
not instead of it: `owner` keeps answering "whose job is this," `tenant_id` answers "which
customer's data did touching this job disclose."

## 2. Non-objectives

| Deferred | Reason |
| --- | --- |
| Per-tenant rate limiting / quotas | S1's own scope note already puts this out of scope ("Quotas et limites par tenant" is listed as future S1 work, not this closing pass) |
| SSO/SAML, a tenant-provisioning admin console | Explicitly out of scope for the whole platform-parity program (`2026-08-01-hacienda-platform-parity-program.md` §9) |
| Tenant-scoped `ObjectStore` (S3 presigned uploads) key-naming | `ObjectStore::presign_put`/`head` operate on caller-supplied `key` strings; the *quarantine* mechanism (E3, already shipped per `uploads.rs`) is the actual boundary here — this spec only requires that the key-naming convention embed `tenant_id` so two tenants cannot collide on or guess each other's object keys. A full `ObjectStore` trait redesign is not in scope. |
| Retroactively partitioning already-written single-tenant audit/review data by real tenant identity | Impossible in general — that data was never tagged. §4's migration path assigns it all to `TenantId::default_tenant()`, matching P3a's same precedent. |
| A `Project` (sub-tenant) dimension anywhere in these stores | `S1`'s own decision D-S1-2: `ProjectId` organizes, does not cloister. Nothing here changes that. |

## 3. Design

### 3.1 Trait signature changes

Every store method that reads or writes tenant-owned rows gains a `&TenantId` (or, where
a `Caller`/context is already threaded, reads it from `caller.tenant_ctx().tenant` at the
call site rather than widening every signature — see §3.3):

```rust
// audit/store.rs
async fn append(&self, tenant: &TenantId, inputs: Vec<AuditEntryInput>) -> Result<Vec<AuditEntry>, AuditError>;
async fn entries(&self, tenant: &TenantId) -> Result<Vec<AuditEntry>, AuditError>;
async fn history(&self, tenant: &TenantId, after: Option<&AuditCursor>, limit: usize) -> Result<AuditPage, AuditError>;
async fn tip(&self, tenant: &TenantId) -> Result<String, AuditError>;
async fn seals(&self, tenant: &TenantId) -> Result<Vec<SegmentSeal>, AuditError>;

// review/store.rs — `submit` already receives a fully-built ReviewQueueItem; add a
// tenant field to that struct instead of a parameter, since every other method (assign/
// decide/list/get/stats) needs to resolve "which tenant's item" and a struct field
// travels with the item through the whole workflow more naturally than a repeated
// parameter would.
pub struct ReviewQueueItem { /* existing fields */ tenant: TenantId }
async fn list(&self, tenant: &TenantId, filter: Option<ReviewStatus>) -> Result<Vec<ReviewQueueItem>, ReviewError>;
async fn get(&self, tenant: &TenantId, id: &str) -> Result<Option<ReviewQueueItem>, ReviewError>;
async fn stats(&self, tenant: &TenantId) -> Result<QueueStats, ReviewError>;
// assign/decide take `id` alone today; they must first look up the item's tenant and
// confirm it matches the caller's before mutating — same "unknown, not forbidden"
// discipline as D-S1-6/D-P3-6 (a cross-tenant id is reported not-found, never forbidden).

// jobs/store.rs
async fn create(&self, tenant: &TenantId, owner: Option<String>) -> Result<Job, JobError>;
async fn get(&self, tenant: &TenantId, id: &str) -> Result<Option<Job>, JobError>;
// ... every other JobStore method gains the same leading &TenantId

// store/postgres/versions.rs
async fn create_version(&self, tenant: &TenantId, document_id: Uuid, ...) -> Result<i64, VersionError>;
async fn list_versions(&self, tenant: &TenantId, document_id: Uuid) -> Result<Vec<DocumentVersion>, VersionError>;
async fn get_version(&self, tenant: &TenantId, document_id: Uuid, sequence: i64) -> Result<Option<DocumentVersion>, VersionError>;

// store/postgres/presets.rs
async fn create(&self, tenant: &TenantId, name: &str, config: Value) -> Result<Preset, PresetError>;
async fn get(&self, tenant: &TenantId, id: Uuid) -> Result<Option<Preset>, PresetError>;
async fn get_by_name(&self, tenant: &TenantId, name: &str) -> Result<Option<Preset>, PresetError>;
async fn list(&self, tenant: &TenantId) -> Result<Vec<Preset>, PresetError>;
async fn delete(&self, tenant: &TenantId, id: Uuid) -> Result<(), PresetError>;
```

**Decision D-S1b-1 — a cross-tenant id is "not found," never "forbidden."** Every `get`/
`assign`/`decide`/`get_version`/`delete` above must resolve the row's actual tenant and
compare it to the caller's before acting, and on mismatch return the same "not found"
result an absent id produces — not a distinguishable "forbidden" response. This is D-S1-6
and D-P3-6 applied uniformly; a 403 on a well-formed but cross-tenant id confirms the id
is valid *somewhere*, which is itself a disclosure.

### 3.2 Postgres migration — two steps, not one

> **Correction (§7):** this section's two-migration design was superseded before
> implementation — `0002`/`0003` were already taken by unrelated migrations by the time
> this shipped, and this repo's own actual convention (`0002_document_version_content.sql`)
> is single-file expand+contract, not a two-file split, because migrations and code deploy
> together here with no rolling-upgrade window. What shipped is one file,
> `hacienda-core/migrations/0004_tenant_scoping.sql`. See §7 for the full reasoning.

**This must ship as two migrations, not one.** A single migration that adds the column
*and* drops its default in the same transaction creates a window — between that migration
landing and the corresponding Rust code deploying — where the schema requires `tenant_id`
on every insert but the running binary doesn't supply it yet, breaking every write. This
codebase's own migration convention (`hacienda-core/migrations/`, sequential numbered
files, one deploy per migration) already assumes migration-then-code as two separate
steps; the migration must tolerate that gap, not assume atomic co-deployment.

**Migration `0002_tenant_scoping_expand.sql`** (ships first, safe with old code running):

```sql
ALTER TABLE audit_segments ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
-- (node_id) uniqueness/open-segment lookups become (node_id, tenant_id)

ALTER TABLE review_items ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
CREATE INDEX idx_review_items_tenant ON review_items (tenant_id, status);

ALTER TABLE jobs ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
CREATE INDEX idx_jobs_tenant ON jobs (tenant_id, owner);

ALTER TABLE document_versions ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
CREATE INDEX idx_document_versions_tenant ON document_versions (tenant_id, document_id);

ALTER TABLE presets ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
-- constraint swap deferred to the contract migration (0003): dropping the bare
-- UNIQUE(name) now would already reject a same-named preset from a second tenant
-- created by code that doesn't supply tenant_id yet.

ALTER TABLE api_keys ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
CREATE INDEX idx_api_keys_tenant ON api_keys (tenant_id, owner);
```

Existing rows backfill to `TenantId::default_tenant()`'s string form via the column
default; old (pre-this-spec) code keeps inserting successfully because the default still
applies to every column it doesn't mention.

**Migration `0003_tenant_scoping_contract.sql`** (ships only after the Rust changes in
§3.1/§3.3 are deployed and confirmed writing `tenant_id` explicitly on every insert):

```sql
ALTER TABLE audit_segments ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE review_items ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE jobs ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE document_versions ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE presets ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE presets DROP CONSTRAINT presets_name_key;
ALTER TABLE presets ADD CONSTRAINT presets_tenant_name_key UNIQUE (tenant_id, name);
ALTER TABLE api_keys ALTER COLUMN tenant_id DROP DEFAULT;
```

Dropping the default is what turns an omitted tenant into a hard failure — a caller that
bypasses the Rust layer (a raw SQL script, a future backend written carelessly) gets a
constraint violation instead of silently landing in `default_tenant()`. Sequencing it
into its own, later migration is what makes that safe: by the time it ships, every code
path already state its tenant explicitly, so the stricter constraint has nothing left to
break.

**Why `api_keys` is included though §1's table didn't flag it as broken today.** An API
key currently has no tenant of its own — `AuthContext.tenant` must come from somewhere,
and today it's synthesized per-request rather than stored with the key. Once keys carry
`tenant_id`, `AuthContext.tenant` is read from the key at issuance time instead, closing
a second, quieter gap: today nothing stops an operator from accidentally issuing a key
whose `AuthContext.tenant` resolution disagrees with what the key was actually meant for.

### 3.3 Facade call sites: threading the tenant down

Every facade method already receives `Caller<'_>` (confirmed throughout `facade.rs` — no
exceptions found). The pattern at every store call site becomes:

```rust
let tenant = caller.tenant_ctx().tenant;
self.audit_store.append(&tenant, inputs).await?;
```

No `hacienda-api`/`hacienda-cli`/`hacienda-mcp` handler signature changes — exactly the
same property P3a's design relies on (§3.3 there), verified true again here because the
mechanism is identical: `Caller` already carries everything needed, at every call site
that needs it.

### 3.4 File backends

`FileAuditStore`/`FileReviewStore` gain a `tenant_id` path segment (§1.2 for the audit
case; `FileReviewStore` similarly moves from one JSONL file to one JSONL file per
tenant). Both stores already open their backing file(s) lazily and support multiple
concurrent logical stores in one process directory tree (that's what `node_id` already
does for audit) — extending the same lazy-open-per-key pattern to `(tenant_id)` or
`(node_id, tenant_id)` is a directory-nesting change, not a new concurrency model.

## 4. Migration path for existing deployments

Identical precedent to P3a §4:

- Every row written before this ships had no tenant of its own — the migration (§3.2)
  backfills all of them to `TenantId::default_tenant()`.
- A caller that never set up multi-tenancy — every `Caller::Trusted` call, every
  `Caller::Principal` whose `AuthContext.tenant` was never populated — resolves to
  `TenantId::default_tenant()` today (via `Caller::tenant_ctx()`, already shipped) and
  keeps seeing exactly what it saw before this spec ships.
- A deployment provisioning a second tenant gets real isolation from the moment that
  tenant's first row is written — no backfill needed for a tenant that never existed
  before.

## 5. Tests

| Test | Assertion |
| --- | --- |
| `two_tenants_audit_chains_are_independent` | Tenant A's `verify()` is unaffected by tenant B's store corruption; each tenant's `tip()` reflects only their own entries. |
| `audit_history_never_returns_another_tenants_entries` | Across in-memory, file, and Postgres backends — the same test body run three times, once per backend, mirroring how `should_construct_arc_dyn_audit_store`-style tests already run across backends in this codebase. |
| `review_get_for_another_tenants_item_id_is_not_found` | D-S1b-1: a well-formed id belonging to a different tenant returns `None`/404, not a distinguishable error. |
| `job_store_owner_isolation_still_holds_within_a_tenant` | Regression guard: adding `tenant_id` must not weaken the existing actor-level `owner` check (§1.3) — two users in the *same* tenant still can't see each other's jobs. |
| `document_version_diff_across_tenants_is_not_found` | Same D-S1b-1 discipline applied to `GET /v1/documents/{id}/diff`. |
| `preset_names_collide_across_tenants_without_error` | Two tenants can each create a preset named `"default"` — the exact case the old bare `UNIQUE(name)` constraint made impossible. |
| `default_tenant_data_unchanged_by_migration` | Every existing test in `hacienda-api`'s `routes.rs`/`hacienda-core`'s store test modules continues to pass unmodified except for the mechanical addition of `&TenantId::default_tenant()` at call sites that construct a store directly in test setup — the same "no other call site changes" property migration §4 promises. |

## 6. Exit criteria

- Every store trait in §1's table takes a `&TenantId` (or, for `ReviewQueueItem`, carries
  one) and every backend (in-memory, file, Postgres) enforces it — not just the Postgres
  one, since the in-memory/file backends are what every non-Postgres deployment runs.
- The migration in §3.2 applies cleanly to a database seeded from `0001_init.sql` and
  every existing row resolves to `TenantId::default_tenant()` afterward.
- All five tests in §5 pass, across every backend `AuditStore`/`ReviewStore` actually
  ship (in-memory, file, Postgres) — not just one.
- No signature change above `hacienda-core` (§3.3) — same falsifiable claim P3a makes
  about the pseudonym layer, now extended to every other store.

## 7. What actually shipped

**AuditStore (Task 2) is done. ReviewStore/JobStore/DocumentVersionStore/PresetStore
(Tasks 3-6) are not implemented** — this section is honest about that split rather than
claiming the whole spec landed. Same posture as P3a's own "spec, not implement" choice
and P6's §7: partial delivery stated plainly, not folded into "done."

### Migration: one file, not two (§3.2) — but not one contract either

Shipped as `hacienda-core/migrations/0004_tenant_scoping.sql` — a single migration file
adding `tenant_id` to every table in §3.2's list including `api_keys`. Two things forced
this away from §3.2's two-migration-*file* design:

1. By the time this was written, `hacienda-core/migrations/0002_document_version_content.sql`
   and `0003_job_progress.sql` already existed (added by unrelated work after this spec's
   source investigation), so the filenames `0002_tenant_scoping_expand.sql`/
   `0003_tenant_scoping_contract.sql` this spec names were already taken.
2. Reading `0002_document_version_content.sql` showed this repo's actual, already-
   established convention is single-file expand+contract, not a two-migration split —
   because migrations and the Rust code that requires the new column ship together in one
   deploy here, with no rolling-upgrade window between them to protect against. §3.2's
   two-file reasoning assumed a gap this codebase doesn't have.

**One file, but not one *contract*, per table.** §3.2's underlying reasoning — don't drop
a column's default until the code writing to it has actually stopped needing that default
— still applies, just at finer grain than "one migration file." The first version of
`0004` dropped `tenant_id`'s default on every table it touched, `audit_segments` included.
That broke immediately: `postgres-store-tests` in PR #81's CI failed with `null value in
column "tenant_id" ... violates not-null constraint` on every `review_items`/`jobs`/
`document_versions`/`presets`/`api_keys` insert, because only `AuditStore`'s Rust code was
updated in this change — those five tables' stores still insert exactly as they did before
S1b, with no `tenant_id` in the statement. Caught by CI before merge, fixed by keeping
`DEFAULT 'default'` on those five tables' `tenant_id` columns; only `audit_segments`' gets
its default dropped, since `AuditStore` is the only store this PR actually updated to
supply it. Whoever implements Tasks 3-6 drops the corresponding table's default as part of
that table's own change — same discipline §3.2 always intended, now applied per-table
instead of per-file. The `presets` unique-constraint swap (`UNIQUE(name)` →
`UNIQUE(tenant_id, name)`) was safe to keep despite this: every row, old and new, shares
the column default until `PresetStore` changes, so the new constraint is behaviourally
identical to the old one until then.

### AuditStore: trait design matches §3.1 exactly, with one addition

§3.1's proposed signatures (`&TenantId` as a parameter on `append`/`entries`/`history`/
`tip`/`seals`) shipped unchanged. `verify`/`rotate`/`close` — not listed in §3.1's example
block but obviously needing the same treatment for a store serving more than one tenant
(each needs to know *which* tenant's open segment to verify/rotate/close) — also gained
`&TenantId`, for all eight trait methods.

An earlier implementation pass considered a facade-level "one store instance per tenant,
cached" factory instead of trait-parameter threading, reasoning that the in-memory/file
backends' existing design ("one instance = one isolated unit of state") would need zero
changes under that scheme. This was abandoned **before any code was written** on reading
`tenancy.rs`'s own module doc, which already documents this exact tradeoff as decision
D-S1-1: a parameter fails to compile at every call site that forgets it; a
one-store-per-tenant factory lets that same omission go unnoticed at construction time,
where it's invisible. The trait-parameter design in §3.1 was correct as written; the
divergence was caught by re-reading `tenancy.rs`, not by any change to the spec.

### Per-backend notes

- **`InMemoryAuditStore`** (`hacienda-core/src/audit/store.rs`): one `State` per tenant,
  in a `HashMap<TenantId, State>` behind the store's single `Mutex`, lazily created on
  that tenant's first call. `node_id`/`config_hash` stay store-wide (one writer node
  serving many tenants). Two new tests: `two_tenants_audit_chains_are_independent`,
  `audit_history_never_returns_another_tenants_entries`.
- **`FileAuditStore`** (`hacienda-core/src/audit/store_file.rs`): layout gains one level,
  `root/{node_id}/{tenant_id}/...`. Recovery no longer runs at `open()` — which tenants
  exist isn't known until a caller names one — it runs lazily per tenant on first touch
  (`ensure_tenant_loaded`), via `spawn_blocking` off the async executor. A dedicated
  `recovery_lock: tokio::sync::Mutex<()>` serialises that first-touch recovery: an earlier
  version without it let two concurrent first-touches of the same tenant race against each
  other's on-disk files mid-recovery (one's freshly-created empty `.jsonl` could be
  observed by the other's `find_unsealed_jsonl` scan and mistaken for a crash orphan) —
  caught by `should_write_concurrent_appends_to_the_file_in_chain_order` failing
  non-deterministically, fixed with double-checked locking around the recovery step. Three
  existing tests changed from asserting `FileAuditStore::open` itself fails on a broken/
  tampered chain to asserting the *first call that touches that tenant* fails instead —
  `open` performing no recovery is the correct, intended behavior change, not a bug the
  tests were pinning.
- **`IndexedDbAuditStore`** (`hacienda-core/src/audit/store_idb.rs`, wasm32-only, gated
  `#[cfg(target_arch = "wasm32")]`): updated to the same per-tenant `HashMap` pattern,
  with per-tenant IndexedDB record keys. Could not be compiled or tested in this session —
  no wasm32 target available in the sandbox this work ran in. Reviewed carefully by hand
  against the same pattern proven correct in the other two backends, but genuinely
  unverified; treat as higher-risk than the other backends until a real wasm32 build
  exercises it.
- **`PostgresAuditStore`** (`hacienda-core/src/store/postgres/audit.rs`): every query
  gained a `tenant_id = $n` predicate; `get_or_create_open_segment`'s advisory lock key
  now includes the tenant (`hashtext('hacienda_audit_open_segment:' || tenant)`) so one
  tenant's first-append serialization doesn't block another's; `get_latest_seal_hash` —
  not called out in §3.1/§3.2 but discovered during implementation — needed the same
  filter, since without it a tenant's new seal would link its `prev_seal_hash` to
  whichever tenant sealed most recently, producing a seal chain that spans tenants and
  fails `verify_seal_chain` for both. `rotate`'s and `close`'s `fetch_one`/`FOR UPDATE`
  queries needed the filter for a sharper reason than "correctness": without it, they
  panic ("query returned more than one row") the moment a second tenant has its own open
  segment, since those queries assumed exactly one open segment existed store-wide.
  Verified against a live Postgres in this session (migrations applied by hand, since the
  repo's disposable-Postgres test fixture needs Docker image pulls this sandbox's network
  policy blocks) — all eight methods' tenant isolation confirmed functionally, including
  the rotate/close/advisory-lock paths under two tenants each holding their own open
  segment simultaneously.
- **`PostgresUsageStore`** (`hacienda-core/src/store/postgres/usage.rs`) — not part of
  this spec's scope, but discovered while auditing every `audit_entries`/`audit_segments`
  reader for tenant leaks: `summary()` aggregates usage/billing numbers across **every**
  tenant, with no `tenant_id` filter at all. This is a real cross-tenant billing-data leak,
  reachable via `GET /v1/usage`. Left unfixed here — closing it needs an API-layer change
  too (the handler and route currently have no tenant-scoped variant) — but flagged
  explicitly rather than silently left for the next person to rediscover.

### Facade threading (§3.3) — audit only

Every `HaciendaFacade` method that touches `audit_store` now resolves
`caller.tenant_ctx().tenant` and threads it through — `audit_entries_with_auth`,
`audit_history_with_auth`, `audit_seals_with_auth`, `audit_export_with_auth`,
`verify_audit_with_auth`, `close_with_auth`, and the internal `record_audit`/
`record_audit_entries`/`record_token_reveal` paths. §3.3's "no `hacienda-api`/
`hacienda-cli`/`hacienda-mcp` signature changes" claim held for everything **except**
`audit_tip`, which — deliberately, per its own doc comment — takes no `Caller` at all (not
capability-gated, so every content-bearing response can attach chain evidence regardless
of the caller's capabilities). That property is orthogonal to tenancy: the tip still needs
to be *this caller's tenant's* tip, so it needed a tenant even though it needed no
capability. Fixed with an additive `audit_tip_with_auth(caller)` alongside the unchanged
`audit_tip()` (which now delegates to `Caller::Trusted`), and every `hacienda-api` handler
that called the old form updated to pass its already-in-scope `caller` — a real, if small,
handler-layer change §3.3's blanket claim didn't anticipate.

`close_with_auth` gained a documented multi-tenant caveat: it seals only the calling
tenant's open segment, because `AuditStore` has no "every tenant this store has ever
served" enumeration to close them all, and building one was out of scope here. Not a
data-loss gap (recovery reseals an orphaned segment automatically on next open), but a
real behavior a multi-tenant deployment doing an orderly shutdown needs to know: closing
once via `Caller::Trusted` does not seal every tenant's chain.

### ReviewStore: tenant lives on the item, not threaded through `submit`

Unlike `AuditStore`'s per-tenant `HashMap<TenantId, State>` partitioning, `ReviewQueueItem`
(`hacienda-core/src/review/types.rs`) gained its own `pub tenant: TenantId` field. The
struct has no `Default` impl, so the compiler already forces every construction site to
supply a tenant — `submit(item: ReviewQueueItem)` did not need a separate `&TenantId`
parameter to get D-S1-1's core property (an omitted tenant fails to compile, not to run).
`assign`/`decide`/`list`/`get`/`stats` — the methods that only have an `id` in hand, not a
full item — each gained a leading `&TenantId` parameter, matching §3.1's proposed shape.

The not-found-not-forbidden discipline (D-S1b-1) is enforced the same way in all three
backends: `assign`/`decide`/`get` match on `id == id && tenant == tenant` together in one
lookup, so a cross-tenant id and a nonexistent id fall into the identical `NotFound`
branch — no second, distinguishing branch exists to get wrong.

- **`InMemoryReviewStore`** (`hacienda-core/src/review/store.rs`): flat
  `Mutex<Vec<ReviewQueueItem>>`, filtered by each item's own `tenant` field at query time —
  no per-tenant partitioning needed since items already self-identify. Three new tests:
  `review_get_for_another_tenants_item_id_is_not_found`,
  `review_assign_and_decide_for_another_tenants_item_id_is_not_found`,
  `two_tenants_review_queues_are_independent`.
- **`FileReviewStore`** (`hacienda-core/src/review/store_file.rs`): same flat-map-plus-
  filter shape as the in-memory backend — one shared `HashMap<String, ReviewQueueItem>`
  built by `open()`'s existing eager, whole-log replay, with a new
  `FileState::get_mut_for_tenant` helper (`self.by_id.get_mut(id).filter(|item| item.tenant
  == *tenant)`) used by `assign`/`decide`. Deliberately did **not** adopt `FileAuditStore`'s
  lazy-per-tenant-directory recovery: review items don't need physically separated
  per-tenant chains, so the simpler eager-replay-plus-filter design is both correct and
  less code.
- **`PostgresReviewStore`** (`hacienda-core/src/store/postgres/review.rs`): every query
  gained a `tenant_id = $n` predicate, including the `UPDATE ... WHERE id = $n AND
  tenant_id = $n AND status = 'pending'`/`... AND decision IS NULL` compare-and-swap
  queries for `assign`/`decide`. `row_to_item` now takes `tenant: TenantId` as a plain
  parameter instead of selecting a `tenant_id` column — every call site already knows the
  tenant from the query's own `WHERE` predicate, so there is nothing to select. Verified
  against a live Postgres in this session (same disposable-Postgres-needs-Docker
  limitation as `AuditStore`'s note above): submit, cross-tenant `get`/`assign`/`decide`
  all confirmed to resolve as documented (`None`/`NotFound`, never a distinguishable
  error), same-tenant `assign`/`decide` confirmed to succeed.

`review_items.tenant_id`'s `DEFAULT 'default'` is dropped in this change's revision of
`0004_tenant_scoping.sql` (the migration has not shipped to any real deployment yet, so
editing it in place — rather than adding a second migration — carries no rolling-upgrade
risk), following the same reasoning already applied to `audit_segments`'s default: every
`ReviewStore` insert, across all three backends, now supplies `tenant_id` explicitly.

**API-layer deviation from `AuditStore`'s pattern**: `HaciendaFacade` exposes `ReviewQueue`
references directly to callers (`review_queue()`, `review_queue_with_auth(caller)`,
`review_queue_read_with_auth(caller)` all return `Option<&ReviewQueue>`) rather than
wrapping every review operation itself, the way it does for audit. §3.3's "no
`hacienda-api` signature changes" claim does not hold for review: `hacienda-api/src/
handlers/audit_review.rs`'s `get_review` and `decide_review` needed direct edits to
resolve `caller.tenant_ctx().tenant` and pass it into `queue.list`/`queue.decide` — a real,
necessary deviation, not an oversight. `hacienda-cli` and `hacienda-mcp` have no
review-store touch points, so neither needed any change.

`HaciendaFacade::submit_for_review` gained a leading `caller: Caller<'_>` parameter (its
one caller, `process_batch_with_auth`, already had one in scope) to resolve the tenant for
the `ReviewQueueItem` it constructs.

### JobStore: `tenant` added alongside `owner`, neither supersedes the other

`Job` (`hacienda-core/src/jobs/types.rs`) gained a `pub tenant: TenantId` field, same
self-identifying-item pattern as `ReviewQueueItem` — `create` sets it once, and
`get`/`transition`/`finish`/`fail`/`update_progress` match on `id && tenant` together in
one lookup so a cross-tenant id and a nonexistent id both resolve the same way (D-S1b-1).
`owner: Option<String>` is unchanged: it still answers "whose job" (actor-level,
handler-enforced), while `tenant` answers "which customer's data" (store-enforced). §1.3's
warning not to let one dimension imply the other held throughout — `list`'s tenant filter
does not touch `owner` at all, and the pre-existing owner-based 404-not-403 checks in
`hacienda-api/src/handlers/jobs.rs` are untouched.

- **`InMemoryJobStore`** (`hacienda-core/src/jobs/store.rs`): flat `Mutex<HashMap<String,
  Job>>`, filtered by each job's own `tenant` field — same shape as
  `InMemoryReviewStore`/`FileReviewStore`. New tests:
  `job_get_for_another_tenants_id_is_not_found`,
  `job_mutations_for_another_tenants_id_are_not_found`,
  `job_store_owner_isolation_still_holds_within_a_tenant` (spec §5's named regression
  guard — asserts both owners are visible via the tenant-scoped `list`, confirming
  `tenant` scoping did not accidentally start doing `owner`'s job too).
- **`PostgresJobStore`** (`hacienda-core/src/store/postgres/jobs.rs`): every query gained
  a `tenant_id = $n` predicate, including both `query_as!` (compile-time-checked) and the
  one runtime `query_as` call (`progress_json`-selecting queries, per this file's existing
  convention of not requiring a live-Postgres `.sqlx/` regeneration for that one column).
  **Pre-existing limitation, not introduced by this change**: `transition`'s single
  `UPDATE ... RETURNING` cannot distinguish "no such id"/"wrong tenant" from "wrong
  status" without a second query — both already reported `StatusMismatch` with a
  best-effort `actual` before this spec, and still do after the `AND tenant_id = $n`
  predicate was added. Left as-is rather than fixed here because it is out of this
  change's scope and, more importantly, not reachable with an attacker-controlled
  cross-tenant id through any handler: `transition`/`finish`/`fail`/`update_progress` are
  only ever called by the same request or background task that just created the job, never
  by an external caller supplying an arbitrary id. D-S1b-1's not-found-not-forbidden
  guarantee matters for the externally-reachable methods — `get` (used by
  `GET /v1/jobs/{id}`, `GET /v1/jobs/{id}/result`,
  `GET /v1/rag/collections/{name}/migrate-embeddings/{job_id}`,
  `GET /v1/documents/{id}/diff/{diff_job_id}`) and `list` (`GET /v1/jobs`) — both of which
  correctly resolve a cross-tenant id as `None`/empty, verified against a live Postgres in
  this session.

**Handler-layer changes** (§3.3's "no signature changes" claim does not fully hold for
jobs, same as it didn't for review): `hacienda-api/src/handlers/documents.rs`,
`rag.rs`, and `versions.rs` all resolve `caller.tenant_ctx().tenant` and thread it through
every `state.jobs.*` call, including into the `tokio::spawn`ed background tasks that
finish/fail a job after the handler has already returned (the tenant is cloned into the
task's captured state alongside the existing job id, same pattern the task already used).
`hacienda-api/src/handlers/jobs.rs`'s `get_job`/`list_jobs`/`get_job_result` and `rag.rs`'s
`get_migrate_status` needed only the same one-line addition each, since they already
extracted a `caller`.

**A real gap closed in passing**: `versions::get_diff_job`
(`GET /v1/documents/{id}/diff/{diff_job_id}`) had no `parts: Parts`/caller extraction at
all before this change — its own doc comment said so explicitly ("deliberately stricter
than `versions::get_diff_job`'s poll endpoint, which applies no ownership check at all",
written when `get_migrate_status` was added). Any authenticated caller who obtained or
guessed a `diff_job_id` could read its line-diff content — potentially containing PII,
since document versioning exists specifically to hold PII-bearing content — regardless of
which tenant created it. Since `JobStore::get`'s call site needed a `tenant` argument
either way for this task, adding the missing `parts: Parts`/caller extraction to close
this gap was not extra scope, just the same change applied to a handler that had been
skipped. `diff_document`, which creates the job, was already `caller`-free too and needed
the same addition.

### `jobs.tenant_id`'s default dropped in the same migration revision

`hacienda-core/migrations/0004_tenant_scoping.sql` now drops `jobs.tenant_id`'s
`DEFAULT 'default'` in this change, following the same reasoning already applied to
`audit_segments` and `review_items`: every `JobStore` insert, across both backends, now
supplies `tenant_id` explicitly. The migration has not shipped to any real deployment yet,
so editing it in place carries no rolling-upgrade risk.

### DocumentVersionStore: tenant scoping plus a real caller-supplied-id collision fix

`DocumentVersionStore` (`hacienda-core/src/store/postgres/versions.rs`) is Postgres-only,
as §3.1 anticipated (no in-memory backend to update). All three methods gained a leading
`&TenantId`, following the same pattern as the other Postgres backends —
`create_version`'s idempotency check and insert are both scoped by `tenant_id`, and
`get_version`/`list_versions` resolve a cross-tenant `document_id` as `None`/empty, the
same D-S1b-1 discipline as everywhere else.

**A real bug found while implementing this, not anticipated by the original plan**:
`document_id` is caller-supplied (the client picks the UUID via `POST /v1/documents`'s
`document_id` field), so two tenants choosing the same id is plausible, not a hypothetical
edge case — unlike the audit/review/job ids, which the store itself assigns. The old bare
`UNIQUE(document_id, version_sequence)` constraint meant a second tenant's own first
version (`version_sequence = 1`) would collide with a first tenant's version 1 for the
"same" `document_id`, failing the INSERT with a constraint violation on an operation that
has nothing to do with the first tenant — a cross-tenant *write* failure, not just a
read-side leak. Fixed by swapping the constraint to
`UNIQUE(tenant_id, document_id, version_sequence)` in this same migration revision — the
identical fix shape already planned for `presets_name_key` in Task 6, applied here one
task early since the bug surfaced during this task's implementation. Verified against a
live Postgres: two tenants versioning the same `document_id` both independently receive
`version_sequence = 1`.

**Handler-layer changes, three of them real gaps closed in passing** (§3.3's "no signature
changes" claim continues not to fully hold, same pattern as review/jobs):
`hacienda-api/src/handlers/documents.rs`'s `process_documents` (the synchronous
`create_version` call site) and `versions.rs`'s `diff_document`/`get_diff_job` (already
touched for Task 4's `JobStore` threading) all resolve `caller.tenant_ctx().tenant` and
pass it to the version-store calls. **`list_document_versions`
(`GET /v1/documents/{id}/versions`) and `get_document` (`GET /v1/documents/{id}`) extracted
no caller at all before this change** — any authenticated caller could read any tenant's
document versions and content, the same class of gap already found and fixed for
`versions::get_diff_job` in Task 4. Both gained the missing `parts: Parts`/caller
extraction as part of this task.

**Test coverage gap, disclosed rather than silently skipped**: §5 names
`document_version_diff_across_tenants_is_not_found`, intended to exercise the actual
`GET /v1/documents/{id}/diff` handler per the original plan. Attempting to write it
surfaced a limitation in this repo's own test harness: every `Token`/
`InMemoryTokenStore`-based `AuthContext` resolves through `AuthContext::new`
(`hacienda-core/src/auth/authn.rs`), which hardcodes `TenantId::default_tenant()` — there
is currently no fixture anywhere in this codebase for constructing a second-tenant
principal and driving a request through it over HTTP. (Multi-tenant `AuthContext`
resolution exists in principle via a `tenant_id`-carrying `ApiKeyStore` — S1b Task 6 adds
`tenant_id` to `api_keys` for exactly this — but nothing exercises that path with a live
request yet.) Rather than force an out-of-scope harness change or silently drop the test,
isolation is instead verified at the store level against a live Postgres:
`version_get_and_list_for_another_tenants_document_id_is_not_found` and
`two_tenants_can_independently_version_the_same_document_id`
(`hacienda-core/src/store/postgres/versions.rs`). The handler call sites all correctly
thread `tenant` through — the same code path a real HTTP-level cross-tenant prober would
exercise — this is a gap in automated test coverage at the HTTP layer specifically, not in
the underlying enforcement.

### Not implemented: Task 6

`PresetStore` remains exactly as described in §3.1 — designed, not touched. Whoever picks
this up next: the `AuditStore`/`ReviewStore`/`JobStore`/`DocumentVersionStore`
implementations above are the reference patterns (trait-parameter threading, per-tenant
`HashMap` or self-identifying-item-plus-filter for in-memory/file backends, `tenant_id =
$n` predicates for Postgres, and — new as of this task — a caller-supplied-identifier
UNIQUE constraint needs `tenant_id` folded in, not just an added column, when the
identifier isn't store-assigned), and `hacienda-core/src/tenancy.rs`'s module doc
(decision D-S1-1) is the reasoning to re-read before considering any alternative. Presets
are Postgres-only (no in-memory backend), so this task is lighter than Tasks 2-5 — and its
own uniqueness fix (`presets_name_key` → `presets_tenant_name_key`) is now exactly the
same shape as the one just applied to `document_versions` above, so that part of Task 6 is
already proven correct by this task's verification.
