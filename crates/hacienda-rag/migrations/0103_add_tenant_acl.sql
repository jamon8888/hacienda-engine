-- Adds per-row tenant ownership and ACL to `rag_documents`/`rag_chunks`.
-- NULL tenant_id means the DEFAULT tenant (rows written before scoping existed).
-- ACL is caller-defined JSON (conventionally {"tenants": ["acme", "globex"]});
-- NULL means owner-only access.
--
-- Numbered 0103 (next free slot in this crate's 0100-0199 block).

ALTER TABLE rag_documents
    ADD COLUMN IF NOT EXISTS tenant_id TEXT,
    ADD COLUMN IF NOT EXISTS acl JSONB;

ALTER TABLE rag_chunks
    ADD COLUMN IF NOT EXISTS tenant_id TEXT,
    ADD COLUMN IF NOT EXISTS acl JSONB;

CREATE INDEX IF NOT EXISTS idx_rag_documents_tenant_id ON rag_documents (tenant_id);
CREATE INDEX IF NOT EXISTS idx_rag_chunks_tenant_id ON rag_chunks (tenant_id);
