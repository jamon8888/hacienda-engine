-- S1 (superpowers/specs/2026-08-01-S1-tenancy-and-projects.md): registered tenants.
--
-- A row here is what makes a TenantId real — admitted, with a display name and an
-- admission timestamp. The pre-S1, single-tenant deployment is migrated onto id
-- 'default' (spec §8); every other tenant-scoped table (audit segments, jobs, etc.)
-- gains its own tenant_id column in later migrations as those stores are scoped.
CREATE TABLE IF NOT EXISTS tenants (
    id              TEXT PRIMARY KEY,
    display_name    TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_tenants_created ON tenants (created_at);

-- Seed the 'default' tenant itself — every table backfilled with tenant_id = 'default'
-- in migrations 0005-0008 needs this row to exist for the foreign keys those
-- migrations add to be satisfiable, and a fresh database must have an admitted
-- 'default' tenant from the start regardless (spec §8's migration path assumes one).
INSERT INTO tenants (id, display_name) VALUES ('default', 'Default')
ON CONFLICT (id) DO NOTHING;
