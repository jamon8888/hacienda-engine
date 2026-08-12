-- S1 (superpowers/specs/2026-08-01-S1-tenancy-and-projects.md §5): audit segments
-- named (tenant, node), not just node.
--
-- DEFAULT 'default' backfills every pre-S1 row onto the tenant the migration path
-- (spec §8) is built around, and lets existing INSERT statements that don't yet name a
-- tenant keep working unchanged until the store layer threads a real TenantCtx through
-- (tracked separately — see the Vague 2 plan's S1 task notes).
ALTER TABLE audit_segments ADD COLUMN IF NOT EXISTS tenant_id TEXT NOT NULL DEFAULT 'default';

CREATE INDEX IF NOT EXISTS idx_audit_segments_tenant ON audit_segments (tenant_id);
