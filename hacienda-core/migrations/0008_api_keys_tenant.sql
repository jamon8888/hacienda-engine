-- S1 (superpowers/specs/2026-08-01-S1-tenancy-and-projects.md): API keys belong to a
-- tenant. DEFAULT 'default' backfills every pre-S1 row, same rationale as
-- 0005_audit_segments_tenant.sql / 0006_jobs_tenant.sql / 0007_review_items_tenant.sql.
ALTER TABLE api_keys ADD COLUMN IF NOT EXISTS tenant_id TEXT NOT NULL DEFAULT 'default';

-- Every tenant_id must name a real, admitted tenant (0004_tenants.sql, which seeds
-- 'default' so this backfilled value satisfies it immediately) — without this, the
-- database could persist an orphan tenant scope no `tenants` row ever admitted.
ALTER TABLE api_keys
    ADD CONSTRAINT fk_api_keys_tenant
    FOREIGN KEY (tenant_id) REFERENCES tenants (id);

CREATE INDEX IF NOT EXISTS idx_api_keys_tenant ON api_keys (tenant_id);
