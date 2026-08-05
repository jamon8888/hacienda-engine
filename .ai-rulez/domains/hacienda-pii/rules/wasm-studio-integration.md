---
priority: medium
---

- `crates/hacienda-wasm` exposes the same pipeline (`process`, `scan`, redaction modes including AES-SIV pseudonymize) to the browser — Studio calls WASM directly, not the REST API. Don't add hacienda-api dependencies to Studio.
- `apps/hacienda-studio` is React 18 + Vite + shadcn + CodeMirror 6 — CodeMirror decorations must survive user edits to the document, not just render once.
- The audit trail is an IndexedDB store (`idb` npm package) wrapped by `AuditHandle` (wasm32-only) — it persists across reloads but is lost on profile/storage clear, and does not currently export into the vault. Don't imply it does in UI copy or docs.
- The NER model asset (GLiNER2 weights, ~1.2 GB unquantized) is not currently invoked from the browser bundle — treat browser-side NER as unresolved until model delivery is solved, don't assume it's wired in.
- Keep business logic in `hacienda-core`/`hacienda-wasm` — Studio is a thin client; don't reimplement pipeline logic in TypeScript.
- Folder ingest, per-document PII editing, and vault export all preserve relative document links and path structure — don't flatten paths when adding new import/export flows.
