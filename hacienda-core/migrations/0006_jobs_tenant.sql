-- S1 (superpowers/specs/2026-08-01-S1-tenancy-and-projects.md): jobs belong to a
-- tenant (distinct from `owner`, the principal within that tenant — a job always
-- has exactly one tenant, but `owner` is nullable for trusted in-process callers).
--
-- DEFAULT 'default' backfills every pre-S1 row, same rationale as
-- 0005_audit_segments_tenant.sql.
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS tenant_id TEXT NOT NULL DEFAULT 'default';

-- Every tenant_id must name a real, admitted tenant (0004_tenants.sql, which seeds
-- 'default' so this backfilled value satisfies it immediately) — without this, the
-- database could persist an orphan tenant scope no `tenants` row ever admitted.
ALTER TABLE jobs
    ADD CONSTRAINT fk_jobs_tenant FOREIGN KEY (tenant_id) REFERENCES tenants (id);

CREATE INDEX IF NOT EXISTS idx_jobs_tenant ON jobs (tenant_id);
