-- Adds a progress-reporting column to `jobs` (Phase 12 close-out, Track 2):
-- long-running jobs such as `POST /v1/rag/collections/{name}/migrate-embeddings`
-- report incremental progress (e.g. documents re-embedded so far) via
-- `JobStore::update_progress`, polled through `GET /v1/jobs/{id}` alongside
-- the existing `status`/`result_json`/`error` fields. Stored as opaque JSON,
-- same rationale as `result_json`: the store layer does not depend on any
-- particular pipeline's progress shape.
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS progress_json TEXT;
