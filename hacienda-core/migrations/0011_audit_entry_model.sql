-- Persist model provenance introduced after the initial audit schema. Existing
-- rows remain NULL and continue to verify using the legacy hash shape.
ALTER TABLE audit_entries
    ADD COLUMN IF NOT EXISTS model TEXT;
