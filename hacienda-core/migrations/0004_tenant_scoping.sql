-- Tenant scoping (S1b): `TenantCtx`/`Caller::tenant_ctx()` (S1) resolve a tenant for
-- every caller, but no store actually scoped by it -- verified directly against every
-- table below before writing this migration. In a shared deployment, two tenants sharing
-- one process saw each other's audit history, review-queue content (including
-- unredacted `text_snippet`, by design), document versions/diffs, and preset names
-- (a bare `UNIQUE(name)` even blocked two tenants from both naming a preset "default").
-- See `superpowers/specs/2026-08-14-S1b-tenant-scoped-audit-review-job-document-stores.md`.
--
-- One migration, not expand-then-contract across two files, for the same reason this
-- repo's own precedent (`0002_document_version_content.sql`) adds a column with a
-- default and drops the default in one migration: no rolling-upgrade window to protect
-- against *when the corresponding Rust code ships in the same deploy*.
--
-- That condition does NOT hold for every table below. Only `AuditStore` (S1b Task 2) and
-- `ReviewStore` (S1b Task 3) were actually updated to write `tenant_id` explicitly so far --
-- `JobStore`/`DocumentVersionStore`/`PresetStore`/`ApiKeyStore` remain unimplemented
-- (Tasks 4-6). Dropping the default on their columns here would make every existing,
-- un-updated `INSERT` on those tables fail its `NOT NULL` constraint the moment this
-- migration lands -- caught by `postgres-store-tests` failing on exactly that
-- (`null value in column "tenant_id" ... violates not-null constraint`) before this ever
-- reached a real deployment. Their `tenant_id` columns are added here (so the schema is
-- ready and the migration files stay batched) but keep `DEFAULT 'default'` until each
-- store's own Rust change ships and drops it -- the same expand/contract split this
-- spec's §3.2 originally called for, now applied per-table instead of per-migration-file
-- since this PR only finishes the Rust side for two of the six tables.
--
-- Every row written before this ships had no tenant of its own; it backfills to
-- `TenantId::default_tenant()`'s string form, `'default'`, matching every existing
-- `Caller::Trusted`/un-tenanted `Caller::Principal` call, which already resolves to that
-- same id via `Caller::tenant_ctx()` (shipped with S1, unchanged by this migration).

ALTER TABLE audit_segments ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
-- Safe to drop now: every AuditStore insert (all three backends) supplies tenant_id
-- explicitly as of this same change (S1b Task 2).
ALTER TABLE audit_segments ALTER COLUMN tenant_id DROP DEFAULT;
-- The open-segment lookup and node-scoped queries key on (node_id, tenant_id) now, not
-- node_id alone -- one open segment per node *per tenant*.
CREATE INDEX IF NOT EXISTS idx_audit_segments_node_tenant
    ON audit_segments (node_id, tenant_id);

-- The four tables below keep tenant_id's default until their own store gets the S1b
-- Task 4-6 treatment -- see the file-level comment above for why dropping it now would
-- break every existing insert.

ALTER TABLE review_items ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
-- Safe to drop now: every ReviewStore insert (all three backends) supplies tenant_id
-- explicitly as of this same change (S1b Task 3).
ALTER TABLE review_items ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX IF NOT EXISTS idx_review_items_tenant ON review_items (tenant_id, status);

ALTER TABLE jobs ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
CREATE INDEX IF NOT EXISTS idx_jobs_tenant ON jobs (tenant_id, owner);

ALTER TABLE document_versions ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
CREATE INDEX IF NOT EXISTS idx_document_versions_tenant
    ON document_versions (tenant_id, document_id);

ALTER TABLE presets ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
-- Two tenants could not previously both name a preset "default" -- a functional bug
-- independent of isolation, fixed by the same column. Safe to swap the constraint now
-- even though PresetStore doesn't write tenant_id explicitly yet: every row (old and
-- new, until PresetStore is updated) shares the column default, so this is behaviourally
-- identical to the old bare UNIQUE(name) until that changes.
ALTER TABLE presets DROP CONSTRAINT presets_name_key;
ALTER TABLE presets ADD CONSTRAINT presets_tenant_name_key UNIQUE (tenant_id, name);

ALTER TABLE api_keys ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
CREATE INDEX IF NOT EXISTS idx_api_keys_tenant ON api_keys (tenant_id, owner);
