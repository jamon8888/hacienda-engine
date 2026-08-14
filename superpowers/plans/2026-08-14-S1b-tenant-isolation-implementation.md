# S1b — Tenant-scoped stores — Implementation plan

**Status (2026-08-14):** Task 1 (migration) and Task 2 (`AuditStore`) shipped — see the
spec's §7 "What actually shipped" for the real design/detail, which diverges from this
plan in a few places the plan didn't anticipate (single migration file, not two;
`verify`/`rotate`/`close` also gained `&TenantId`; a `recovery_lock` needed adding to
`FileAuditStore` to close a real recovery race the plan didn't foresee;
`audit_tip_with_auth` needed adding as a new facade method). Tasks 3-8 below are
unstarted — still an accurate task breakdown for whoever picks them up, just not yet
executed. Facade threading for `AuditStore` (part of what Task 7 describes) already
shipped alongside Task 2, since the trait signature change forced it — see the spec's §7
for exactly which facade methods changed.

**Spec:** `superpowers/specs/2026-08-14-S1b-tenant-scoped-audit-review-job-document-stores.md`
**Sibling plan:** none written yet for P3a (`2026-08-14-P3a-tenant-scoped-pseudonym-keys.md`)
— that spec is implementation-ready but has no separate plan file; its §3 is detailed
enough to execute directly. Recommend writing P3a's own implementation task list before
or alongside Task 6 below, since both land in the same `HaciendaFacade` call sites.

**Baseline:** `cargo test --workspace --exclude hacienda-wasm`, 2026-08-14: 5 pre-existing
environmental failures (root-in-container `chmod`-based write-failure-injection tests; no
live Postgres for `connect_and_migrate`) — not regressions, do not attempt to fix them
here. Re-baseline before starting if this plan is picked up more than a few days after
this date.

**Sequencing constraint that shapes every task below:** the Postgres migration cannot be
one step (see spec §3.2) — Task 1 (expand) must ship and be deployed before Tasks 2-6
touch a single Rust call site, and Task 7 (contract) must not ship until every task before
it is deployed and confirmed writing `tenant_id` explicitly. Tasks 2-6 (one per store) are
independent of each other and can proceed in any order, or in parallel, once Task 1 is
live — each touches a disjoint set of files.

---

## Ground truth, verified against current source

| Fact | Location |
| --- | --- |
| Zero store traits take a tenant parameter today | `hacienda-core/src/{audit,review,jobs}/store.rs`, `hacienda-core/src/store/postgres/{versions,presets}.rs` — read directly, confirmed no `TenantId`/`TenantCtx` anywhere |
| `Caller::tenant_ctx()` already resolves a `TenantCtx` for both `Trusted` and `Principal` | `hacienda-core/src/auth/mod.rs:255-262` |
| Every facade method already receives `Caller<'_>` | `hacienda-core/src/facade.rs`, verified across all public methods during this investigation |
| `AuditStore::history`'s own doc comment already establishes per-`NodeId` chain partitioning as precedent | `hacienda-core/src/audit/store.rs:59-77` |
| Postgres schema has no `tenant_id` on any of `audit_segments`, `audit_entries`, `review_items`, `jobs`, `document_versions`, `presets`, `api_keys` | `hacienda-core/migrations/0001_init.sql`, read in full |
| `presets.name` has a bare `UNIQUE` constraint, not `UNIQUE(tenant_id, name)` | `hacienda-core/migrations/0001_init.sql:111` |
| `JobStore::create`'s own doc comment describes actor-level (`owner`) isolation, explicitly not tenant-level | `hacienda-core/src/jobs/store.rs:38-45` |
| `TenantId` is "intentionally opaque... no ordering, no arithmetic" — already safe to use as a path/column key | `hacienda-core/src/tenancy.rs:20-23` |
| No `hacienda-api`/`hacienda-cli`/`hacienda-mcp` file needs a signature change — confirmed by the same reasoning P3a's own §3.3 already relies on | this investigation, cross-checked against P3a |

---

## Task 1 — Postgres expand migration (ships alone, first)

- [x] Write `hacienda-core/migrations/0002_tenant_scoping_expand.sql` exactly as spec §3.2
      shows: `ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default'` on `audit_segments`,
      `review_items`, `jobs`, `document_versions`, `presets`, `api_keys`, plus the four new
      indexes. Do **not** drop any default or touch the `presets.name` constraint in this
      migration — that is Task 7.
- [x] Run it against a live Postgres instance (`postgres-integration-tests`' fixture) and
      confirm every existing row backfills to `'default'` and every existing insert (from
      unmodified, pre-this-plan code) still succeeds unchanged.
- [x] `SQLX_OFFLINE=true cargo check -p hacienda-core --features postgres` to catch any
      `sqlx::query!`/`query_as!` macro whose compile-time-checked SQL now disagrees with
      the widened schema (expect none — this migration only adds columns, doesn't touch
      any column an existing query already selects `*` around... verify column lists are
      explicit, not `SELECT *`, before assuming this).
- [x] Deploy. This step alone changes no observable behaviour — every caller still sees
      exactly what they saw before, because nothing reads the new column yet.

## Task 2 — `AuditStore`: the hash-chain partitioning decision (§1.2/§3.1)

- [x] Add `&TenantId` to `AuditStore::append`/`entries`/`history`/`tip`/`seals`
      (`hacienda-core/src/audit/store.rs`).
- [x] `InMemoryAuditStore`: key its internal chain state by `TenantId`, not a single chain
      — a `HashMap<TenantId, AuditChain>` or equivalent, lazily inserted on first use per
      tenant (mirrors how the existing single-chain state is already lazily-initialized-once
      logic, just keyed now).
- [x] `FileAuditStore` (`hacienda-core/src/audit/store_file.rs`): move segment files from
      `<audit_dir>/<node_id>/segment-*.jsonl` to
      `<audit_dir>/<node_id>/<tenant_id>/segment-*.jsonl`. Sanitise `tenant_id` the same
      way `node_id` is already sanitised for path use (find that sanitisation, reuse it —
      do not write a second one).
- [x] Postgres backend (`hacienda-core/src/store/postgres/audit.rs`): every query that
      currently filters/keys on `node_id` alone gains `AND tenant_id = $n`; "the currently
      open segment for this node" becomes "for this (node, tenant)" — this is the change
      most likely to have a subtle bug, since it is the one place uniqueness genuinely
      changes shape (one open segment per node *per tenant* now, not one per node).
- [x] Test: `two_tenants_audit_chains_are_independent` and
      `audit_history_never_returns_another_tenants_entries` (spec §5), run against all
      three backends — copy the existing pattern this codebase already uses for
      backend-parametrised tests (`should_construct_arc_dyn_audit_store`-style, or
      wherever `InMemoryAuditStore`/`FileAuditStore`/Postgres already share a test body).
- [x] Update every direct `AuditStore` construction in test setup across the workspace to
      pass `&TenantId::default_tenant()` — mechanical, not a design change (same "no other
      call site changes" claim the spec's exit criteria make).

## Task 3 — `ReviewStore`: tenant on the item, not just the call (§3.1) — done

- [x] Add `tenant: TenantId` to `ReviewQueueItem` (`hacienda-core/src/review/types.rs`).
- [x] `ReviewStore::list`/`get`/`stats` take `&TenantId` and filter by it.
- [x] `assign`/`decide` match on `id && tenant` together in one lookup and return
      `ReviewError::NotFound` on a cross-tenant id — indistinguishable from a nonexistent
      one (D-S1b-1). Tests: `review_get_for_another_tenants_item_id_is_not_found`,
      `review_assign_and_decide_for_another_tenants_item_id_is_not_found`,
      `two_tenants_review_queues_are_independent`.
- [x] `InMemoryReviewStore`/`FileReviewStore`/Postgres backend all updated. **Deviation**:
      `FileReviewStore` did *not* move to one JSONL file per tenant — it kept its existing
      flat, eagerly-replayed `HashMap<String, ReviewQueueItem>` and filters by each item's
      own `tenant` field at query time. Items already self-identify their tenant (unlike
      audit segments, which don't carry one), so per-tenant directory nesting would have
      added complexity without adding correctness. See spec §7's ReviewStore section for
      the full reasoning.
- [x] `ReviewQueue::submit` (`hacienda-core/src/review/queue.rs`) threads `&TenantId`
      through to `ReviewQueueItem`'s `tenant` field; `HaciendaFacade::submit_for_review`
      gained a `Caller` parameter to resolve it, since its one call site
      (`process_batch_with_auth`) already had one in scope.

## Task 4 — `JobStore`: add `tenant_id` alongside `owner`, don't replace it (§1.3/§3.1) — done

- [x] Add `&TenantId` as a new leading parameter on every `JobStore` method
      (`hacienda-core/src/jobs/store.rs`), keeping `owner: Option<String>` exactly as is.
      `create` also gains a `pub tenant: TenantId` field on `Job` itself (mirrors
      `ReviewQueueItem`) so `get`/`transition`/`finish`/`fail`/`update_progress` can match
      on `id && tenant` together in one lookup, same D-S1b-1 pattern as Task 3.
- [x] Updated `JobStore::create`'s doc comment to state both dimensions explicitly: `owner`
      answers "whose job" (actor-level, unchanged), `tenant` answers "which customer's
      data" (new) — neither supersedes the other.
- [x] Regression test: `job_store_owner_isolation_still_holds_within_a_tenant` — two
      different `owner`s in the *same* tenant are both visible through the store's
      tenant-scoped `list` (owner-level filtering stays the transport layer's job, per
      `hacienda-api/src/handlers/jobs.rs`, unchanged and still covered by its own
      pre-existing cross-owner tests). Also added
      `job_get_for_another_tenants_id_is_not_found` and
      `job_mutations_for_another_tenants_id_are_not_found` for D-S1b-1.
- [x] `InMemoryJobStore`/Postgres backend, same treatment as Tasks 2-3. **Note**:
      `PostgresJobStore::transition`'s pre-existing (not new) limitation — a single
      `UPDATE ... RETURNING` can't distinguish "no such id"/"wrong tenant" from "wrong
      status" without a second query, so both report `StatusMismatch` with a best-effort
      `actual` — was not fixed, only the `AND tenant_id = $n` predicate was added to close
      the isolation gap. Not reachable with an attacker-controlled cross-tenant id through
      any handler (`transition`/`finish`/`fail`/`update_progress` are only ever called by
      the same request/background task that just created the job), so D-S1b-1's
      not-found-not-forbidden guarantee — which matters for externally-reachable
      `get`/`list` — still holds where it counts.
- [x] Also closed a real gap found while threading tenant through:
      `GET /v1/documents/{id}/diff/{diff_job_id}` (`versions::get_diff_job`) had no
      `parts: Parts`/caller extraction at all before this change — any authenticated
      caller could poll any diff job id and read its line-diff content regardless of
      tenant. Added tenant scoping there as part of this task rather than deferring it,
      since the `JobStore::get` call site needed a `tenant` argument either way.

## Task 5 — `DocumentVersionStore`: tenant-scope `create_version`/`list_versions`/`get_version` — done

- [x] Add `&TenantId` to all three methods (`hacienda-core/src/store/postgres/versions.rs`).
      Confirmed Postgres-only, no in-memory backend, as anticipated.
- [x] `get_version`/`list_versions` on a `document_id` belonging to a different tenant:
      same D-S1b-1 discipline — not-found, not forbidden.
- [x] **Real bug found and fixed, not anticipated by this plan**: `document_id` is
      caller-supplied (the client picks the UUID), so two tenants choosing the same id is
      plausible, not hypothetical. The old bare
      `UNIQUE(document_id, version_sequence)` meant a second tenant's own first version
      (`version_sequence = 1`) would collide with a first tenant's version 1 for the "same"
      `document_id` — a real cross-tenant write failure, not just a read-side leak. Fixed
      by swapping the constraint to `UNIQUE(tenant_id, document_id, version_sequence)` in
      the same migration revision (same fix shape Task 6 already anticipated for
      `presets_name_key`, applied here a task early since the bug was found while
      implementing this one). Verified against live Postgres: two tenants versioning the
      same `document_id` both get `version_sequence = 1` independently.
- [x] `[deferred]` Test: `document_version_diff_across_tenants_is_not_found` (spec §5) —
      **could not be exercised through the actual HTTP handler as originally planned**.
      Discovered while writing it: this repo's test harness has no way to construct two
      distinct-tenant principals over HTTP today — every `Token`/`InMemoryTokenStore`-based
      `AuthContext` resolves through `AuthContext::new`, which hardcodes
      `TenantId::default_tenant()` (`hacienda-core/src/auth/authn.rs`). Multi-tenant
      `AuthContext` resolution is wired for `ApiKeyStore` in principle but no test fixture
      in this codebase exercises a second tenant via a live HTTP request yet. Covered
      instead at the store level, verified against live Postgres:
      `version_get_and_list_for_another_tenants_document_id_is_not_found` and
      `two_tenants_can_independently_version_the_same_document_id`
      (`hacienda-core/src/store/postgres/versions.rs`). The handler call sites
      (`list_document_versions`, `get_document`, `diff_document`, `get_diff_job`) all
      correctly thread `tenant` through — same code path a real HTTP cross-tenant prober
      would hit — just not exercised by an automated HTTP-level test in this change.
      Flagged here rather than silently claimed as done.
- [x] Also discovered and fixed in passing: `list_document_versions`
      (`GET /v1/documents/{id}/versions`) and `get_document` (`GET /v1/documents/{id}`)
      extracted **no caller at all** before this change — any authenticated caller could
      read any tenant's document versions and content. Added the missing
      `parts: Parts`/caller extraction to both as part of this task, same as the
      `get_diff_job` gap closed in Task 4.

## Task 6 — `PresetStore`: tenant-scope and fix the global-uniqueness bug

- [ ] Add `&TenantId` to all five methods (`hacienda-core/src/store/postgres/presets.rs`).
- [ ] Test: `preset_names_collide_across_tenants_without_error` (spec §5) — this is a
      behaviour *fix*, not just an isolation add: today two tenants literally cannot both
      have a preset named `"default"`, which is a functional bug independent of any
      security concern. Confirm this test fails against pre-Task-7 schema (the unique
      constraint hasn't been swapped yet — see Task 7) and only starts passing once Task 7
      ships; don't be surprised if it's red until then.

## Task 7 — Facade call-site threading + Postgres contract migration (last)

- [ ] Thread `caller.tenant_ctx().tenant` into every store call `HaciendaFacade` makes,
      across all six stores touched by Tasks 2-6. This is mechanical once each store's
      signature is updated — the pattern is `let tenant = caller.tenant_ctx().tenant;`
      once per facade method, then pass `&tenant` to whichever store call already existed.
- [ ] Full workspace build + test (`cargo check --workspace --exclude hacienda-wasm`,
      `cargo test --workspace --exclude hacienda-wasm`) — this is the point where every
      previously-mechanical "add `&TenantId::default_tenant()` in test setup" edit from
      Tasks 2-6 either compiles clean or surfaces a call site this plan missed.
- [ ] Deploy Tasks 1-7's Rust changes (with Task 1's migration already live from before).
      Confirm in production/staging logs or a smoke test that inserts now carry explicit
      `tenant_id` values, not the column default.
- [ ] Only then: write and ship `hacienda-core/migrations/0003_tenant_scoping_contract.sql`
      exactly as spec §3.2 shows (drop every default, swap the `presets` unique
      constraint). This is the migration that makes `preset_names_collide_across_tenants_without_error`
      (Task 6) finally pass.
- [ ] Re-run the full test suite one more time against the contracted schema to confirm
      nothing depended on the now-removed defaults.

## Task 8 — Cross-reference and close out

- [ ] Update `2026-08-01-hacienda-platform-parity-program.md`'s S1 section (French) with
      an amendment note, same pattern as the existing P7 alert: S1's `TenantCtx` landing
      was necessary but not sufficient; S1b + P3a is what actually closes tenant isolation.
      Cross-reference both child specs.
- [ ] Update `2026-08-14-P3a-tenant-scoped-pseudonym-keys.md`'s own status if it has
      shipped by this point, or note it as a parallel, independent piece of the same S1
      closure if it hasn't.
- [ ] `CHANGELOG.md` `### Security` entry, same style as the P6/P7 entries already there:
      name the specific disclosure this closes (audit history, review queue content,
      document versions, preset names — cross-tenant, pre-fix), not just "added tenant
      scoping."
