-- S1b (superpowers/specs/2026-08-14-S1b-tenant-scoped-audit-review-job-document-stores.md
-- Task 6): presets belong to a tenant. DEFAULT 'default' backfills every pre-S1 row,
-- same rationale as 0005_audit_segments_tenant.sql / 0006_jobs_tenant.sql /
-- 0007_review_items_tenant.sql / 0009_document_versions_tenant.sql. The default is
-- dropped in this same migration: `PresetStore` (Postgres-only backend) already
-- supplies `tenant_id` explicitly as of this change.
ALTER TABLE presets ADD COLUMN IF NOT EXISTS tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE presets ALTER COLUMN tenant_id DROP DEFAULT;

-- Every tenant_id must name a real, admitted tenant (0004_tenants.sql, which seeds
-- 'default' so this backfilled value satisfies it immediately) — without this, the
-- database could persist an orphan tenant scope no `tenants` row ever admitted.
ALTER TABLE presets
    ADD CONSTRAINT fk_presets_tenant
    FOREIGN KEY (tenant_id) REFERENCES tenants (id);

-- Two tenants could not previously both name a preset "default" — a functional bug
-- independent of isolation, fixed by the same column. `name` is caller-supplied, the
-- identical situation `document_versions_tenant_document_version_key`
-- (0009_document_versions_tenant.sql) already fixed for `document_id` — the constraint
-- must include tenant_id, not just the column.
ALTER TABLE presets DROP CONSTRAINT presets_name_key;
ALTER TABLE presets ADD CONSTRAINT presets_tenant_name_key UNIQUE (tenant_id, name);
