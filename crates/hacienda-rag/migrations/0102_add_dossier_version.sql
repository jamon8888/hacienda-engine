-- Adds legal retrieval scoping to `rag_documents`/`rag_chunks` (P1 dossier/version scope).
-- `dossier_id` (folder) and `version_ref` (document version) are carried on both
-- documents and chunks (denormalized on chunks, mirroring `collection`/`document_id`)
-- so retrieval can scope by dossier/version without joining the document table.
--
-- Numbered 0102 (this crate's next free slot in its 0100-0199 block — see
-- `hacienda-core/migrations/README.md`).

ALTER TABLE rag_documents
    ADD COLUMN IF NOT EXISTS dossier_id TEXT,
    ADD COLUMN IF NOT EXISTS version_ref TEXT;

ALTER TABLE rag_chunks
    ADD COLUMN IF NOT EXISTS dossier_id TEXT,
    ADD COLUMN IF NOT EXISTS version_ref TEXT;

CREATE INDEX IF NOT EXISTS idx_rag_documents_dossier_id ON rag_documents (dossier_id);
CREATE INDEX IF NOT EXISTS idx_rag_documents_version_ref ON rag_documents (version_ref);
CREATE INDEX IF NOT EXISTS idx_rag_chunks_dossier_id ON rag_chunks (dossier_id);
CREATE INDEX IF NOT EXISTS idx_rag_chunks_version_ref ON rag_chunks (version_ref);
