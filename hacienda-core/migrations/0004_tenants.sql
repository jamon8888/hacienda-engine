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
