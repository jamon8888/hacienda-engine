---
priority: high
---

- `hacienda` is a distribution facade re-exporting `hacienda-core` + `xberg` — put pipeline logic in `hacienda-core`, not in the facade or in `hacienda-api`/`hacienda-cli`.
- The REST API is Axum, not gRPC. Document input is base64 inline bytes only — never accept a wire-supplied path or URL (SSRF prevention).
- All routes except `/health`, `/version`, `/info`, `/openapi.json` require `Capability::DocumentsProcess` — new routes must be added to the guarded-routes reflection test, not just the route table.
- The job store is in-memory (durable backend is unbuilt, Phase 6) — jobs are lost on process restart. Don't design features that assume job durability until that phase lands.
- CLI `extract` takes `--mode {mask|hash|pseudonymize}` with no default — never silently pick a redaction mode for the user.
- CLI `scan` is detect-only and must not rewrite or leak entity text by default.
- CLI `serve` binds loopback-only by default and refuses non-loopback binding unless auth is enabled — do not weaken this without an explicit auth story.
- CLI has `hacienda audit verify <dir>` (verifies the `--audit-out` flat-JSON export only, not the full `GET /v1/audit` surface) and `hacienda pii reveal <token>`, plus a `--glossary-out <dir>` flag on `extract`/`scan` (not a subcommand — glossary state has no existence outside a live run). `review`, `compliance`, and the rest of `audit` remain deliberately absent (not stubbed) — do not add placeholder commands that appear to work but don't.
