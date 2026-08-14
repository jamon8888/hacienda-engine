-- Tenant scoping (S1b): `TenantCtx`/`Caller::tenant_ctx()` (S1) resolve a tenant for
-- every caller, but no store actually scoped by it -- verified directly against every
-- table below before writing this migration. In a shared deployment, two tenants sharing
-- one process saw each other's audit history, review-queue content (including
-- unredacted `text_snippet`, by design), document versions/diffs, and preset names
-- (a bare `UNIQUE(name)` even blocked two tenants from both naming a preset "default").
-- See `superpowers/specs/2026-08-14-S1b-tenant-scoped-audit-review-job-document-stores.md`.
--
-- One migration, not expand-then-contract across two files: this repo's own precedent
-- (`0002_document_version_content.sql`) already adds a column with a default and drops
-- the default in the same migration, because migrations and the code that requires the
-- new column ship together in one deploy here -- there is no rolling-upgrade window to
-- protect against. Every row written before this ships had no tenant of its own; it
-- backfills to `TenantId::default_tenant()`'s string form, `'default'`, matching every
-- existing `Caller::Trusted`/un-tenanted `Caller::Principal` call, which already resolves
-- to that same id via `Caller::tenant_ctx()` (shipped with S1, unchanged by this
-- migration).

ALTER TABLE audit_segments ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE audit_segments ALTER COLUMN tenant_id DROP DEFAULT;
-- The open-segment lookup and node-scoped queries key on (node_id, tenant_id) now, not
-- node_id alone -- one open segment per node *per tenant*.
CREATE INDEX IF NOT EXISTS idx_audit_segments_node_tenant
    ON audit_segments (node_id, tenant_id);

ALTER TABLE review_items ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE review_items ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX IF NOT EXISTS idx_review_items_tenant ON review_items (tenant_id, status);

ALTER TABLE jobs ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE jobs ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX IF NOT EXISTS idx_jobs_tenant ON jobs (tenant_id, owner);

ALTER TABLE document_versions ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE document_versions ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX IF NOT EXISTS idx_document_versions_tenant
    ON document_versions (tenant_id, document_id);

ALTER TABLE presets ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE presets ALTER COLUMN tenant_id DROP DEFAULT;
-- Two tenants could not previously both name a preset "default" -- a functional bug
-- independent of isolation, fixed by the same column.
ALTER TABLE presets DROP CONSTRAINT presets_name_key;
ALTER TABLE presets ADD CONSTRAINT presets_tenant_name_key UNIQUE (tenant_id, name);

ALTER TABLE api_keys ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE api_keys ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX IF NOT EXISTS idx_api_keys_tenant ON api_keys (tenant_id, owner);
