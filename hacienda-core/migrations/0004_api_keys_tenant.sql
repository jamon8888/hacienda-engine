-- Adds tenant scoping to `api_keys` (S1 tenancy, item 4.1+4.3 — see
-- superpowers/specs/2026-08-01-S1-tenancy-and-projects.md §8): every existing key
-- belongs to the single pre-existing deployment, which becomes tenant `default`
-- (matches `tenancy::DEFAULT_TENANT` in `hacienda-core/src/tenancy.rs`).
--
-- `ApiKeyStore::create`/`list` now take a `tenant: &TenantId` alongside `owner` — a key
-- is never visible outside its own tenant, even if `owner` collides across tenants.
ALTER TABLE api_keys ADD COLUMN IF NOT EXISTS tenant_id TEXT NOT NULL DEFAULT 'default';

-- Replaces `idx_api_keys_owner`: every current query filters by tenant *and* owner
-- together (`ApiKeyStore::list`), never by owner alone.
DROP INDEX IF EXISTS idx_api_keys_owner;
CREATE INDEX IF NOT EXISTS idx_api_keys_tenant_owner ON api_keys (tenant_id, owner);
