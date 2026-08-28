-- Add model provenance column to audit_entries for GLiNER2 taxonomy alignment
ALTER TABLE audit_entries ADD COLUMN IF NOT EXISTS model TEXT;

-- Also add vertical column for completeness, though audit chain hashing already supports it
ALTER TABLE audit_entries ADD COLUMN IF NOT EXISTS vertical TEXT;
