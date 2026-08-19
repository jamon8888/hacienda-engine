-- S1b (superpowers/specs/2026-08-14-S1b-tenant-scoped-audit-review-job-document-stores.md
-- Task 5): document versions belong to a tenant. DEFAULT 'default' backfills every
-- pre-S1 row, same rationale as 0005_audit_segments_tenant.sql / 0006_jobs_tenant.sql /
-- 0007_review_items_tenant.sql. The default is dropped in this same migration:
-- `DocumentVersionStore` (Postgres-only backend) already supplies `tenant_id` explicitly
-- as of this change.
ALTER TABLE document_versions ADD COLUMN IF NOT EXISTS tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE document_versions ALTER COLUMN tenant_id DROP DEFAULT;

-- Every tenant_id must name a real, admitted tenant (0004_tenants.sql, which seeds
-- 'default' so this backfilled value satisfies it immediately) — without this, the
-- database could persist an orphan tenant scope no `tenants` row ever admitted.
ALTER TABLE document_versions
    ADD CONSTRAINT fk_document_versions_tenant
    FOREIGN KEY (tenant_id) REFERENCES tenants (id);

CREATE INDEX IF NOT EXISTS idx_document_versions_tenant
    ON document_versions (tenant_id, document_id);

-- `document_id` is caller-supplied (the client picks the UUID via `POST
-- /v1/documents?document_id=...`), so two tenants choosing the same id is entirely
-- plausible — not a hypothetical edge case. The old bare UNIQUE(document_id,
-- version_sequence) would let a second tenant's first version (version_sequence=1)
-- collide with a first tenant's own version 1 for the "same" document_id, failing the
-- constraint on an operation that has nothing to do with the first tenant. Each tenant's
-- version_sequence must be free to start at 1 independently, so the constraint moves to
-- (tenant_id, document_id, version_sequence) — the same fix 0010_presets_tenant.sql
-- applies for the identical caller-supplied-identifier reason.
ALTER TABLE document_versions DROP CONSTRAINT document_versions_document_id_version_sequence_key;
ALTER TABLE document_versions ADD CONSTRAINT document_versions_tenant_document_version_key
    UNIQUE (tenant_id, document_id, version_sequence);
