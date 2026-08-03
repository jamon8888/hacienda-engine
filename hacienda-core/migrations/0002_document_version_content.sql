-- Store the actual redacted-output content alongside the existing content_hash so
-- versions and diffs are retrievable, not just identifiable. content_hash remains
-- the idempotency/identity key (blake3 of redacted output, per Decision 1 of the
-- integration spec: hacienda never versions raw input, only its own redacted
-- output); content/entities_json are the retrievable payload for
-- `GET /v1/documents/{id}` and `GET /v1/documents/{id}/diff`.
ALTER TABLE document_versions ADD COLUMN content TEXT NOT NULL DEFAULT '';
ALTER TABLE document_versions ADD COLUMN entities_json JSONB NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE document_versions ALTER COLUMN content DROP DEFAULT;
ALTER TABLE document_versions ALTER COLUMN entities_json DROP DEFAULT;
