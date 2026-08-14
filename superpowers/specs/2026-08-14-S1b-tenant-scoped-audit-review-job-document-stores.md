# S1b — Tenant-scoped audit, review, job, document-version, and preset stores

**Date:** 2026-08-14
**Status:** Proposed, implementation-ready — every claim below verified against current
source, not inferred from `2026-08-01-S1-tenancy-and-projects.md`'s design intent
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
