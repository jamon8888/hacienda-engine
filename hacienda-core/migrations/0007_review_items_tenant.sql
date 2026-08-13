-- S1 (superpowers/specs/2026-08-01-S1-tenancy-and-projects.md): review items belong
-- to a tenant. DEFAULT 'default' backfills every pre-S1 row, same rationale as
-- 0005_audit_segments_tenant.sql / 0006_jobs_tenant.sql.
ALTER TABLE review_items ADD COLUMN IF NOT EXISTS tenant_id TEXT NOT NULL DEFAULT 'default';

-- Every tenant_id must name a real, admitted tenant (0004_tenants.sql, which seeds
-- 'default' so this backfilled value satisfies it immediately) — without this, the
-- database could persist an orphan tenant scope no `tenants` row ever admitted.
ALTER TABLE review_items
    ADD CONSTRAINT fk_review_items_tenant
    FOREIGN KEY (tenant_id) REFERENCES tenants (id);

CREATE INDEX IF NOT EXISTS idx_review_items_tenant ON review_items (tenant_id);
