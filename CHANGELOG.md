# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **`hacienda-studio`'s audio/video transcription now actually runs, instead of failing
  synchronously in every environment.** `worker/pipeline.ts` runs inside a Web Worker and
  constructed `@remotion/whisper-web`'s `WhisperBridge` directly; that package's own
  `canUseWhisperWeb()` checks `typeof window === "undefined"` and refuses to run otherwise —
  a Worker has `self`, not `window`, so every `WhisperBridge.load()`/`transcribeAudio()` call
  threw `"Whisper Web is not supported: `window` is not defined"` unconditionally, regardless
  of network access or which model was selected. Fixed architecturally, not patched:
  `WhisperBridge` (`lib/transcription/whisper-bridge.ts`) now runs only on the main thread
  (a `WhisperBridge` instance owned by `App.tsx`), and the worker requests a transcription
  over `postMessage` and awaits the reply — a small request/response correlation map
  (`worker/transcribe-bridge.ts`'s `TranscriptionRequestBridge`, unit-tested in isolation)
  keyed by `requestId`, with its own timeout so a lost or never-sent reply fails just that one
  file (through the existing per-file `try`/`catch` in `processFiles`) instead of hanging the
  whole batch — the same isolation guarantee a prior fix already gave the old, always-broken
  code path. `WhisperBridge.transcribeAudio()`'s resample/transcribe progress callbacks are
  now threaded into the existing per-file progress UI (a new `"transcribe"` `ProgressUpdate`
  stage) instead of only reaching `console.log`. `tests/e2e/audio.spec.ts`, which previously
  pinned the exact broken-in-every-environment error message as the expected outcome, now
  asserts the opposite: that message must never appear again, and (via a mocked model host,
  the same pattern already used for the NER model's real download) that the model download
  is actually attempted on the main thread — real inference against real model weights is
  intentionally left to a manual run, not this suite, for the same reason the NER model's
  real ~600MB download already is.
- **`postgres-store-tests` (new CI job) no longer fails on a connection-pool leak or a
  segment-creation race.** These `hacienda-core` Postgres-backed store tests were
  `#[ignore]`d and had never run in CI before; wiring them up (this changelog's "Postgres
  in CI" entry) surfaced two real bugs on first execution. First: the shared
  `PostgresFixture`'s `PgPool` (a static `OnceCell`) was created once but each
  `#[tokio::test]` ran on its own throwaway Tokio runtime — sqlx ties a connection's
  socket to the runtime that established it, so a connection handed across a test-runtime
  boundary lost its return-to-pool bookkeeping, permanently shrinking the pool's capacity
  test by test until every later test timed out on `pool.acquire()` regardless of
  complexity. Fixed by routing every fixture-touching test through one persistent runtime
  (`test_support::block_on_shared`) instead of a runtime per test. Second:
  `get_or_create_open_segment`'s `SELECT ... FOR UPDATE` only locks a row that already
  exists, so when no segment is open it locks nothing — two transactions could both see
  `None` and both insert their own "open" segment, splitting entries across two competing
  chains. Fixed with a `pg_advisory_xact_lock`, the same pattern `versions.rs`'s
  `create_version` already uses for its own check-then-insert race. Also fixed the same
  job's `postgres-integration-tests`: `sqlx::query!`/`query_as!` verify SQL against a live
  database at compile time whenever `DATABASE_URL` is set, but that job points
  `DATABASE_URL` at a database migrated only when the *test binary* runs, not at compile
  time — forced `SQLX_OFFLINE=true` so both jobs check queries against the committed
  `.sqlx` cache instead. Five further `hacienda-core::store::postgres::audit` tests still
  fail on an unrelated, pre-existing `SegmentIntegrity` bug (all five fail on the *same*
  corrupted seal regardless of what each individually exercises — see the comment above
  `postgres-store-tests` in `ci-postgres.yaml` and next to the audit test module) and are
  skipped from CI pending further investigation with a live Postgres session.
- **`POST /v1/documents` no longer silently drops every document when no PII pipeline is
  configured.** `process_documents` zipped `body.documents`, `result.extraction.results`,
  and `result.pii` three ways; `Iterator::zip` truncates to the shortest input, and
  `result.pii` is an empty `Vec` whenever `HaciendaFacade`'s PII pipeline is unset — so the
  whole zip produced zero results regardless of how many documents were submitted or
  extracted successfully. Fixed by iterating `result.extraction.results` (always one entry
  per submitted document) and pairing each with `result.pii`'s corresponding entry when one
  exists, defaulting to no entities otherwise — matching the facade-level contract already
  pinned by `should_extract_without_touching_pii_when_it_is_not_configured`
  (`hacienda-core/src/facade.rs`). Was previously flagged as a known, unfixed gap (see the
  "Known gap surfaced" note further down this changelog); the SDK tests that pinned the
  buggy `documents: []` response (`sdks/typescript/tests/extract.test.ts`,
  `sdks/python/tests/test_extract.py`) now assert the corrected behavior instead.
- **`main` build restored.** `hacienda-core` failed to compile (`page_from` re-exported at a
  wider visibility than its `pub(crate)` definition, plus two knock-on `E0282`s in the new
  `PostgresAuditStore::history()`), `hacienda-api` had two independent duplicate-definition
  errors (`AuditEntryDto` declared twice in `dto.rs`, `Query`/`From<QueryRejection>` declared
  twice in `extract.rs`) left over from the Phase 5/Phase 10 merge, three `AuditStore` test
  doubles in `hacienda-core/src/facade.rs` were missing the `history` method the trait gained,
  and the `.sqlx` offline query cache was missing entries for the new Postgres audit queries.
  CI (`ci-rust.yaml`) had been red on `main` since `00b210b`.
- **The full 5-endpoint audit API is live again**, not just documented in
  `hacienda-api/README.md`: `GET /v1/audit/{entries,verify,seals,export,tip}`, cursor-paginated,
  with the `audit:export` capability gating `/export`. This code
  (`hacienda-api/src/handlers/audit.rs`) existed since `93c6290` but was orphaned by a later
  merge — never declared in `handlers/mod.rs`, never wired into `ROUTE_TABLE` — leaving only the
  coarser Phase 10 `/v1/audit`/`/v1/audit/verify` (`audit_review.rs`) live. Restored
  `HaciendaFacade::{audit_history_with_auth, audit_seals_with_auth, audit_export_with_auth,
  audit_enabled}`, which the handlers call and which had been dropped along with the routes.
  `/v1/audit` stays on `audit_review::get_audit` for `sdks/typescript`'s hand-written
  `getAudit()`; `/v1/audit/verify` now serves the richer handler (broken-chain-as-200, names
  the offending entry/seal) — `sdks/typescript/src/client.ts`'s `verifyAudit()` return type
  updated to match (operation id kept as `verifyAudit` so `sdks/python`'s generated client
  still exposes a `verify_audit` function under the new handler), `audit_review::verify_audit`
  kept unrouted as a reference implementation. `/v1/audit/export`'s `json`/`jsonl` formats are a
  flat, chain-ordered export (verifiable via consecutive `chain_hash` recomputation) — not the
  segment-grouped, seal-embedding envelope the original design sketched; that richer shape is
  unimplemented, and the handler's doc comment and this entry now say so rather than overclaim.
  `PostgresAuditStore::history()` reads its extent counts, open-segment id, and entries inside
  one `REPEATABLE READ` transaction (previously separate queries, so a concurrent
  append/rotation could make a legitimate chain look like a `SegmentEntryCount` mismatch), and
  the RAG upsert path's server-side chunking (below) runs in `spawn_blocking` rather than
  inline on the async runtime.
- **README's bindings table no longer claims 14 language bindings that don't exist.** `alef.toml`
  references six Rust source files that were never written and a `packages/` output tree that
  was never created; the table's 14 "✅" rows described `cargo-alef`-generated FFI bindings that
  have never been generated. Replaced with what's real (the REST `sdks/python`/`sdks/typescript`
  clients, the hand-written `crates/hacienda-wasm`) and an honest note on what's aspirational.
  `.ai-rulez/skills/{crate-structure,alef-generated-bindings}/SKILL.md` flagged as copied from
  the upstream `xberg` repo — they describe a crate/package layout that doesn't exist here.
- **`GET /v1/review` required `review:decide` instead of the route table's declared
  `audit:read` (Phase 10).** `get_review` and `decide_review` both called
  `HaciendaFacade::review_queue_with_auth`, which unconditionally required
  `Capability::ReviewDecide` — so a caller with only `audit:read` passed the route-level
  guard on `GET /v1/review` and was then rejected by the facade. New
  `HaciendaFacade::review_queue_read_with_auth` requires `audit:read`, used by `get_review`
  only; `decide_review` keeps the original `review_queue_with_auth` (`review:decide`).
  Surfaced while closing a test-coverage gap noted in the Phase 10 implementation plan
  (no handler-level test exercised the two capabilities' distinction).

### Added

- **`hacienda review`, `hacienda compliance`, the rest of `hacienda audit`
  (`list`/`export`), and `hacienda completions`** — the CLI/API parity gap `cli.rs`'s
  header comment used to document as "deliberately absent" is closed now that the
  backing `hacienda-core` functionality (`ReviewQueue`/`FileReviewStore`,
  `ComplianceGenerator`, the segmented `FileAuditStore`) is real. `xberg` passthrough
  remains out of scope (a separate, larger design question).
  - `extract`/`scan` gain `--review-out <DIR>`: materialises this run's low-confidence
    detections into a durable review queue at `<DIR>/review.jsonl`, readable and
    actionable afterwards via the new `hacienda review` subcommand. Unlike
    `--audit-out`/`--glossary-out` (which overwrite on every run), `--review-out`
    *accumulates* — `FileReviewStore::open` replays what is already on disk before
    appending this run's submissions, so repeated runs build one durable reviewer inbox.
    Materialises `[review]` with defaults when none was configured, mirroring
    `--glossary-out`'s `[glossary]` materialisation. Refused when combined with
    `--no-redact`, same guard shape as `--audit-out`.
  - `hacienda review list|show|assign|decide|stats <DIR>` operate directly on a durable
    `FileReviewStore` — no facade, no capability check (the CLI is in-process and
    trusted, the same `Caller::Trusted` precedent `pii reveal` already documents).
    `decide`/`assign` surface the underlying `ReviewError` message (already decided,
    not found, invalid transition) rather than a generic wrapper — this also fixed
    `main.rs`'s error printing to show the *whole* anyhow context chain (`{:#}`) instead
    of only the outermost `.context(...)` layer, which was silently dropping exactly
    this kind of detail for every subcommand, not just the new ones.
  - `hacienda compliance dpia|model-card|checklist|dora|report` generate GDPR/AI-Act/DORA
    artefacts straight from `[compliance]` configuration — no facade or document
    involved, since these are pure functions of config (and, for `dora`, an
    `--incident <FILE>`-supplied `PiiIncident`). `compliance report` omits the DORA
    section when no `--incident` is given, exposing `ComplianceGenerator::report`'s
    existing "no incident, no DORA" behaviour as-is rather than forcing one.
  - `hacienda audit list|export <DIR> --node <ID>` read a durable, segmented
    `FileAuditStore` (`root/<node>/`, sealed and open segments) — distinct from
    `audit verify`'s flat `audit.json` export from `--audit-out`. Both reuse
    `HaciendaFacade::audit_history_with_auth`/`audit_export_with_auth` (built around a
    facade with every other subsystem switched off) rather than re-implementing paging
    or cross-segment chain reconstruction. `--node` has no default — segments are
    per-writer, so guessing one would silently open (and start writing an empty segment
    into) the wrong writer's history. Nothing in this CLI writes that layout yet
    (`extract --audit-out` and `serve` both stay on their existing, deliberately
    different audit paths), so this reads whatever a `FileAuditStore` another process —
    a library embedding `hacienda-core` directly, or a future `serve` enhancement —
    already wrote.
  - `hacienda completions bash|zsh|fish|powershell|elvish` prints a completion script
    generated by `clap_complete::generate` against the same `Cli` clap parses with, so it
    can never name a flag or subcommand this binary does not actually have. New
    `clap_complete` dependency.
- **Server-side chunking for RAG document upsert.** `POST /v1/rag/collections/{name}/documents`
  previously required the caller to submit pre-chunked, pre-embedded `chunks` — `full_text` was
  stored for search only, never chunked. When `chunks` is omitted, the server now splits
  `full_text` itself via a new `hacienda_rag::chunk_full_text` (wrapping xberg's `chunking`
  feature, pure Rust, no ONNX), producing chunks with `content` set and `embedding` empty for a
  caller to embed separately or leave unembedded. `hacienda-rag`'s new `chunking` feature is
  always-on in `hacienda-api` (unlike the ONNX-gated, opt-in `rag-embeddings`).
- **Python and TypeScript SDKs (`sdks/python`, `sdks/typescript`, Phase 14).** Client
  libraries for the hacienda-engine API, living in this repo (`sdks/`) rather than a
  separate `hacienda-sdks` repo — the session's GitHub App cannot create repositories, and
  the monorepo shape turned out simpler regardless: no cross-repo `spec-sync` workflow,
  since CI builds `hacienda-cli`, starts it, and fetches `/openapi.json` in the same job
  that generates and tests the client (`sdks/scripts/fetch-openapi.sh`), always against the
  exact commit under test. Neither package commits generated code (`_generated/` gitignored
  in both). Python: `HaciendaClient`/`AsyncHaciendaClient` (`openapi-python-client` +
  `httpx`, `uv`/`ruff`/`mypy`/`pytest`). TypeScript: `HaciendaClient`
  (`openapi-typescript` types + `openapi-fetch`, joined the existing npm/turbo workspace).
  Both cover all 44 operations across 14 OpenAPI tags via one namespace per tag
  (`client.pii.scan_text(...)` / `client.pii.scanText(...)`), a `target: "cloud"` axis
  (single-variant union, ready for Phase 15's `"device"`), retry-with-backoff on
  429/502/503/504, and a `.whoami()` shortcut. Every wrapper method routes through the
  `*_detailed` generated calls and raises a `HaciendaApiError` on any non-2xx response —
  the plain `sync`/`asyncio` (Python) and openapi-fetch's own `{data, error}` (TypeScript)
  both collapse errors, including documented 401/403/404/400, into an empty-looking
  result, which would make an API error indistinguishable from success. 12 pytest + 7
  vitest tests, all against a live `hacienda serve`, never a mock. New CI:
  `ci-sdk-python.yaml`, `ci-sdk-typescript.yaml` (triggered on `hacienda-api`/`hacienda-core`
  changes too, not just `sdks/`, since a schema change must re-run them). `publish-sdk.yaml`
  scaffolded (PyPI + npm, both OIDC trusted publishing) but not activated — needs
  org-level trusted-publishing configuration this session cannot provision.
- **`ReviewDecideRequest.decision` is now a closed enum** (`ReviewDecisionWire`:
  `approve`/`reject`/`modify`), not a free `String` — the generated OpenAPI schema lists
  the three valid values instead of an unconstrained string. Found during PR review while
  building the SDK's generated types.
- **Compliance/glossary responses use typed envelope DTOs**
  (`ComplianceDpiaResponse`, `ComplianceReportResponse`, `GlossaryResponse`) instead of
  bare `serde_json::Value`, so the OpenAPI schema names the stable `report`/`entries` +
  `audit_chain_tip` shape — the variable report/entry content itself stays opaque
  (`hacienda-core`'s `ComplianceReport`/`GlossaryEntry` are not `utoipa`-annotated).
- **Real OpenAPI 3.1 schema for `GET /openapi.json` (Phase 14 precondition).**
  `hacienda-api`'s OpenAPI document was previously a hand-built stub — one
  `{"description": "Access: ..."}` object per path, no HTTP methods, no request/response
  schemas, no `operationId`, nothing an SDK code generator could act on. Adopted `utoipa`
  (this crate only): `#[derive(ToSchema)]` on all 59 DTOs in `dto.rs`, `#[utoipa::path]`
  on all 44 route-table handlers, assembled by a new `ApiDoc` in `handlers/openapi.rs`.
  Verified end-to-end: `openapi-generator-cli generate -g python` and `-g
  typescript-fetch` against a live instance's `/openapi.json` both produce complete,
  syntactically valid typed clients (one API module per tag, one model per schema).
  Foreign types from `hacienda-core`/`hacienda-rag`/`xberg` (e.g. `PiiCategory`,
  `JobStatus`, `CollectionSpec`, `xberg::LlmConfig`) are represented via `#[schema(value_type
  = ...)]` overrides (`String` or `serde_json::Value`) rather than adding `utoipa` to those
  crates. New guard tests in `handlers/openapi.rs`:
  `openapi_path_set_equals_route_table_minus_openapi_json`,
  `every_openapi_path_has_at_least_one_typed_operation`,
  `every_declared_schema_is_present_in_components`.
- **`GET /v1/auth/whoami`.** Reports the *calling* principal's own granted capabilities.
  Corrects the platform-parity design spec's §8, which proposed adapting `xberg-sdks`'
  `_resolve_tier` into a capability probe against `GET /v1/auth/config` — that route
  requires `Capability::AuthManage`, which a normal SDK caller does not hold, so it
  cannot serve as a general capability probe. `whoami` is gated on
  `Capability::DocumentsProcess` instead and returns only the presented token's own
  grants, with no elevated privilege required to ask "what can I do."
- **CLI `--mode pseudonymize` now works.** `hacienda extract`/`hacienda scan` build their
  facade via `HaciendaFacade::with_key_resolver` with an `EnvKeyResolver` when the
  effective redaction mode is `pseudonymize`, instead of always calling
  `HaciendaFacade::new` (which never supplies a pseudonymiser and made
  `--mode pseudonymize` fail unconditionally — `RedactionEngine::new` returns
  `MissingPseudonymKey` whenever that mode has no pseudonymiser). Conditional on mode,
  not unconditional: `Pseudonymiser::new` resolves the active key eagerly, so wiring it
  in on every run would break every non-pseudonymize invocation on a host with no
  `HACIENDA_PSEUDONYM_ACTIVE_KEY` set. `hacienda serve` has the identical gap and is not
  fixed by this change — known, not yet addressed.
- **`hacienda pii reveal <token>` (CLI/API parity).** Reverses a pseudonym token via
  `HaciendaFacade::reveal_token_with_auth` as `Caller::Trusted` (the CLI's process
  boundary is the trust boundary, same precedent `serve` documents). Every
  `PseudonymError`/`PiiDisabled` from the reveal call collapses to one generic refusal,
  mirroring the HTTP API's `ApiError::from(HaciendaError::Pseudonym(_))` mapping, so CLI
  error text cannot be used to distinguish a wrong key from a malformed token.
  Facade-construction failure (no active key configured at all) is reported with its real
  message instead, since that is an operator misconfiguration, not a token probe.
- **`hacienda audit verify <dir>` (CLI/API parity).** Independently re-verifies the flat
  `audit.json` export written by `extract --audit-out` — the same one-shot-CLI shape, not
  the segmented `FileAuditStore` format `serve` uses for durable storage. Adds
  `hacienda_core::audit::verify_entries(&[AuditEntry]) -> Result<(), AuditError>`, a
  slice-only counterpart to `AuditChain::verify` for callers holding a deserialized entry
  vector without a chain's `config_hash`; `AuditChain::verify` now delegates to it.
- **`--glossary-out <dir>` on `extract`/`scan`.** Writes this run's entity glossary
  (`{category, term, count, mean_confidence}`, never document text) to
  `<dir>/glossary.json` via `HaciendaFacade::glossary_snapshot_with_auth`. Not a
  standalone `glossary` subcommand: glossary state (`EntityGlossary`) lives only inside a
  live run's facade, populated as documents are processed, with nothing durable to reread
  afterwards. Materialises a `[glossary]` config section when none was loaded, matching
  `--mode`'s existing materialisation of `[pii]` — without it, `--glossary-out` against a
  config with no `[glossary]` section would silently write an empty array.
- **`POST /v1/pii/reveal` endpoint (Phase 8).** Reverses a pseudonym token
  (`[CATEGORY:key_id:base32_ciphertext]`) back to its normalised plaintext.
  Requires `pii:reveal` capability. Writes a `Reveal` audit entry keyed by
  `blake3(plaintext)` so an auditor can join this call to the original redaction
  that minted the token by `span_hash`. Returns 400 for any malformed,
  unreadable, or unknown-key token — all token errors collapse to one status code
  to prevent probing key material.
- `HaciendaFacade::reveal_token_with_auth(caller, token)` — core method
  enforcing `Capability::PiiReveal`, delegating to `Pseudonymiser::reveal`, and
  recording the token reveal in the audit chain. `HaciendaFacade` now holds a
  cloned `Arc<Pseudonymiser>` so the de-pseudonymisation path is available
  outside a live pipeline run.
- **Postgres store backend (Phase 9).** `AuditStore`, `ReviewStore`, `JobStore`
  implementations backed by Postgres via `sqlx`, plus new stores for document
  versions, presets, and API keys. All stores share a single `PgPool` injected
  at process start (no global state). Schema includes `audit_segments`,
  `audit_entries`, `review_items`, `jobs`, `document_versions`, `presets`,
  `api_keys` tables with indexes. Migrations run explicitly via `--migrate`
  flag — never implicitly in library code.
- `HaciendaFacade::with_stores` extended with optional parameters for the
  three new store types (`DocumentVersionStore`, `PresetStore`, `ApiKeyStore`).
- `hacienda-core` gains a `postgres` feature flag (off by default) to gate
  `sqlx` dependency for wasm32 and non-Postgres consumers.
- **Phase 5 routes: audit, review, compliance, glossary (Phase 10).** 7 new
  endpoints: `GET /v1/audit`, `GET /v1/audit/verify`, `GET /v1/review`,
  `POST /v1/review/{id}/decide`, `GET /v1/compliance/dpia`,
  `GET /v1/compliance/report`, `GET /v1/glossary`. All guarded by
  `audit:read` except review decide which requires `review:decide`.
- `HaciendaFacade::glossary_snapshot_with_auth` and
  `HaciendaFacade::compliance_report_with_auth` — new facade accessors
  exposing existing core logic (previously write-only from the route layer).
- `ApiError::from` now maps `ReviewError` variants to appropriate HTTP codes.
- **API key generation and Argon2id hashing (Phase 11 Task 1).** `hacienda_core::auth::keys`
  gains `generate_key()` and `ApiKeyPair { raw_key, key_hash }`. Keys are 256 bits of
  `OsRng` entropy, base62-encoded and prefixed (`hcd_live_<43 chars>`) for identification
  in logs and support requests without revealing the key itself. Only `key_hash` — an
  Argon2id hash with a unique per-key salt — is ever persisted; `raw_key` is returned once
  at issuance and cannot be recovered from the hash. `ApiKeyPair`'s hand-written `Debug`
  redacts `raw_key` so an accidental `{:?}` in a log line cannot leak it. `verify_key`
  compares in constant time via `argon2::verify_password`, avoiding a timing oracle on
  key validity. Design Decision D7 considered reusing `aes-siv` — already a workspace
  dependency for pseudonymisation — but rejected it: AEAD ciphers are fast and
  deterministic by design, so using one for key hashing would make stolen hashes
  brute-forceable at AEAD speed rather than Argon2id speed. `argon2 = "0.5"` was added
  as a new dependency instead, per OWASP guidance to hash credentials slowly.
  **Breaking (unreleased):** `Capability` gains a new variant, `AuthManage`. `Capability`
  is not `#[non_exhaustive]`, so an embedder who matches on it exhaustively (no wildcard
  arm) fails to compile until they handle the new variant — flagged per this crate's own
  semver policy regardless of the fact that nothing has shipped yet.
- **Deterministic API key lookup and `ApiKeyTokenResolver` (Phase 11 Task 2).**
  Argon2id salts every hash it produces, so `key_hash` alone could never be used to
  look up a stored `ApiKey` from a freshly presented key — only to verify one already
  found by some other means. `hacienda_core::auth::keys` gains `lookup_key(raw_key)`,
  a deterministic BLAKE3 digest computed once at issuance and again on every
  presented key; `ApiKeyPair` and `ApiKey` both gain a `lookup_hash` field alongside
  `key_hash`. **Breaking (unreleased):** `ApiKeyStore::create` now takes both
  `key_hash` and `lookup_hash`, and `get_by_hash` is renamed `get_by_lookup_hash` to
  describe what it actually searches on; `api_keys.lookup_hash TEXT NOT NULL UNIQUE`
  was added to the (unshipped) `0001_init.sql` migration. `key_hash` still does all
  verification via `verify_key` — `lookup_hash` never gates access on its own.
  `hacienda_core::auth::authn::ApiKeyTokenResolver` is a new `authn::TokenResolver`
  (the trait the Axum auth middleware actually calls) that authenticates a bearer
  token against any `Arc<dyn ApiKeyStore>`: look up by `lookup_key(token)`, reject if
  revoked, confirm with `verify_key` against the stored `key_hash`, then map
  `owner`/`capabilities` onto a `Token`. It takes the store directly via
  `ApiKeyTokenResolver::new` rather than through `TokenResolverType`/
  `build_token_resolver`, since it needs a live store handle that `AuthConfig`'s
  serializable fields cannot carry — the same reasoning `HaciendaFacade::with_stores`
  already uses for its optional stores.
- **API key management routes (Phase 11 Task 3).** 3 new endpoints, all guarded by
  `auth:manage`: `POST /v1/auth/keys` (issue — the raw key appears exactly once, in
  this response, and is never logged or echoed elsewhere), `DELETE /v1/auth/keys/{id}`
  (revoke, 204, idempotent so a missing/already-revoked id cannot be probed for),
  and `GET /v1/auth/config` (reports `enabled`/`resolver`, never key material).
  `hacienda-api/src/handlers/auth.rs` follows the existing `handlers/audit_review.rs`
  pattern: extract the `Caller` from request extensions, let
  `HaciendaFacade::{issue_key_with_auth, revoke_key_with_auth}` enforce the
  capability, map errors through `ApiError::from`. `get_auth_config` has no facade
  method behind it (there is nothing to call — the fields come straight off
  `HaciendaFacade::config()`), so it asserts `auth:manage` itself rather than relying
  solely on the route table's declared requirement, matching every other handler's
  defense-in-depth. **Known limitation, tested rather than hidden:**
  `resolver` reports `HaciendaConfig::auth.resolver` — the *declared* configuration —
  not necessarily the resolver instance actually wired into the live `AuthState`,
  which exposes no introspection getter for which resolver it holds. An embedder who
  wires a store-backed `ApiKeyTokenResolver` in directly (the documented reason that
  type bypasses `TokenResolverType`) makes this field stale; the test
  `auth_config_resolver_reflects_declared_config_not_the_live_resolver` demonstrates
  this rather than papering over it. Fixing it needs an `AuthState` introspection API
  in `hacienda-core`, out of scope here.
- **`GET /v1/jobs`, `GET /v1/jobs/{id}/result` (Phase 13 Task 1).** Both guarded by
  `documents:process`, same as the existing `GET /v1/jobs/{id}`. `list_jobs` filters by
  `?status=`, is tenant-scoped (a principal sees only jobs it owns; a trusted in-process
  caller sees all), and paginates in-process over `JobStore::list`'s full result via
  `?limit=`/`?offset=` (default 50, capped at 200) — the store's `list` signature has no
  pagination parameters, so `total` reflects the caller-visible count before slicing rather
  than a store-level count. `get_job_result` is deliberately distinct from `get_job`: a
  polling loop that only needs job status should not pay to deserialize a potentially large
  `result` on every poll. It always returns 200 regardless of job state (`result`/`error`
  simply absent until the job settles) and applies the same 404-not-403 ownership check as
  `get_job` to avoid turning job ids into a membership oracle (OWASP A01). New
  `extract::Query<T>` extractor mirrors the existing `Json<T>` pattern so a malformed query
  string fails into the `{"error": {...}}` envelope rather than axum's default plain-text
  rejection.
- **`GET/POST /v1/presets`, `GET/DELETE /v1/presets/{id}` (Phase 13 Task 2).** Thin CRUD
  routes over Phase 9's `PresetStore`, guarded by `documents:process` — presets are inert
  config, not part of the audit-bearing pipeline. Opt-in via a new
  `ApiState::with_preset_store(Arc<dyn PresetStore>)` builder (mirrors `with_rag_store`
  exactly); routes 400 (`ApiError::invalid_request`) when no store is attached. Unlike
  `RagStore`, `PresetStore` has no in-memory backend (Postgres-only), so `hacienda-api` now
  depends on `hacienda-core`'s `postgres` feature unconditionally rather than gating it
  behind a second opt-in Cargo feature — the runtime opt-out already exists via the
  `Option<Arc<dyn PresetStore>>` field.
- Fixed a latent Phase 9 defect this uncovered: `hacienda-core`'s `postgres` feature could
  not compile in any environment (including CI's `--all-features`/`--each-feature` jobs)
  because sqlx's compile-time `query!` macros need either a live `DATABASE_URL` or a
  committed offline query cache, and neither existed. Generated `.sqlx/` (44 cached queries)
  via `cargo sqlx prepare --workspace -- --features postgres -p hacienda-core` against a
  real Postgres instance; sqlx picks this up automatically with no `DATABASE_URL` or
  `SQLX_OFFLINE` needed. This was never previously exercised by CI on this branch, so it is
  a fix to a pre-existing gap, not a regression.
- **`GET /v1/documents/{id}/versions`, `GET /v1/documents/{id}`, `GET /v1/documents/{id}/diff`,
  `GET /v1/documents/{id}/diff/{diff_job_id}` (Phase 13 Task 3).** Document versioning and
  diffing over Phase 9's `DocumentVersionStore`, guarded by `documents:process` since a
  version's content is exactly `/v1/documents`' own redacted output. `POST /v1/documents`
  gains an optional `document_id` field on each input; supplying it stores that document's
  redacted output as a new version (idempotent on identical content, sequence increments on
  change) and echoes `document_id`/`version_sequence` back in the response. Opt-in via a new
  `ApiState::with_version_store(Arc<dyn DocumentVersionStore>)` builder (mirrors
  `with_preset_store`); routes 400 when no store is attached, except
  `/diff/{diff_job_id}`, which polls `JobStore` directly (same shape as
  `GET /v1/jobs/{id}/result`) and is unaffected by whether versioning is configured.
  `/diff` is synchronous by default under a 2-second wall-clock budget; past the budget it
  hands the still-running computation off to a detached task and returns `202 Accepted` with
  a `diff_job_id` to poll, reusing `JobStore` rather than a second async mechanism. The diff
  itself is a hand-written LCS-based line diff (`str::lines()`) — no diff crate exists in the
  workspace, and a byte-level diff is sufficient per the platform-parity spec.
- **Breaking schema change (unreleased): `document_versions` gains `content`/`entities_json`
  columns; `DocumentVersionStore::create_version` gains matching parameters.** The table
  previously stored only `content_hash`, making `GET /v1/documents/{id}` and `/diff`
  impossible to implement — there was nowhere to read the actual redacted text back from.
  Migration `0002_document_version_content.sql` adds `content TEXT NOT NULL` and
  `entities_json JSONB NOT NULL`; both are the pipeline's redacted output, never raw input,
  consistent with Decision 1 (content-addressing by hash of redacted output, not raw input).
- Known gap surfaced (not fixed here, out of scope for this task): `POST /v1/documents`
  silently returns `{"documents":[]}` for any batch when no PII pipeline is configured
  (`result.pii` is empty, so zipping it with `result.extraction.results` yields zero
  entries regardless of extraction success). Pre-existing before this task's changes.
- **`POST /v1/uploads/presign`, `POST /v1/uploads/confirm` (Phase 13 Task 4).** Presigned
  uploads for large documents that should not transit the API server as a base64 body.
  `hacienda_core::store::object::ObjectStore` trait (`presign_put`, `head`) with an
  `S3ObjectStore` implementation behind a new `s3` Cargo feature, working against AWS S3 or
  any S3-compatible endpoint (MinIO, R2, GCS S3-compat) via `S3Config::endpoint`. Uses
  `rusty-s3` for pure-computation request signing and `reqwest` only for the `HEAD` call
  `confirm_upload` issues. Both routes require `documents:process`, since a presigned upload
  is a precursor step to `/v1/documents` and carries the same class of content; opt-in via a
  new `ApiState::with_object_store(Arc<dyn ObjectStore>)` builder (mirrors
  `with_version_store`), 400 when no store is attached. `confirm_upload`'s storage key is
  derived solely from the server-issued `upload_id`, never from the client-supplied
  `filename` — the SSRF-safety argument this reopens (spec §5) holds because no
  client-supplied URL or path ever reaches `ObjectStore::head`/`presign_put`.
- **`GET /v1/usage` (Phase 13 Task 5).** Per-principal usage metering, derived from the audit
  chain rather than tracked separately (Decision 3): entity count (row count) and byte count
  (`SUM(span_length)`), optionally windowed by `?since=`/`?until=` on `created_at`.
  Document count is deliberately not reported — `AuditEntry` carries no `document_id`, so it
  cannot be derived without a schema change or silently mis-counting zero-redaction documents.
  New `hacienda_core::store::postgres::usage::{UsageStore, PostgresUsageStore}` queries
  `audit_entries` directly rather than going through `AuditStore::entries`, which is scoped to
  only the currently-open segment — a usage read-model needs sealed history too, or a billing
  window would under-report every time a segment rotates. Guarded by `audit:read`, the same
  capability as `/v1/audit` and `/v1/compliance/*`, since this is a read-model over that same
  data. Opt-in via a new `ApiState::with_usage_store(Arc<dyn UsageStore>)` builder (mirrors
  `with_object_store`), 400 when no store is attached. Fixed a bug surfaced by the live
  integration test: `span_length` is `BIGINT`, and Postgres's `SUM(BIGINT)` returns `NUMERIC`,
  not `BIGINT`, which failed the runtime column-type check — fixed with an explicit
  `::BIGINT` cast on the aggregate.
- **`hacienda-rag` crate: `RagStore` trait, backend-agnostic IR, in-memory backend (Phase 12
  Task 1).** New workspace member `crates/hacienda-rag`, recovered near-verbatim from xberg's
  own MIT-licensed `xberg-rag` crate (removed from that workspace pre-1.0 in commit
  `77e2fd3d711ecf6a673ff1ceb0a17abc9a2c4e64`). `RagStore` (renamed from `VectorStore` to read
  alongside `AuditStore`/`ReviewStore`/`JobStore`) is an object-safe, `async_trait` vector-store
  contract, backed by a neutral type/filter/query intermediate representation
  (`CollectionSpec`, `DocumentRecord`, `ChunkRecord`, `Filter`, `RetrieveQuery`,
  `RetrieveMode::{Vector, FullText, Hybrid, Sparse, LateInteraction}`) and a `Capabilities`
  negotiation type so callers can discover what a given backend actually supports.
  `InMemoryVectorStore` is the pure-Rust reference backend. The upstream crate's process-global
  registry was dropped in favor of constructor injection (no global state, matching this
  codebase's own convention); registry-only error variants (`AlreadyRegistered`,
  `NotRegistered`, `InvalidName`) were dropped from `RagError` accordingly. 49 tests,
  `cargo clippy -p hacienda-rag --all-targets -- -D warnings` clean.
- **`PgVectorStore`: durable `pgvector`-backed `RagStore` (Phase 12 Task 2).** New
  `crates/hacienda-rag/src/backends/pgvector.rs`, gated behind a new `postgres` Cargo feature
  (off by default, matching `hacienda-core`'s own `postgres` gating) so in-memory-only
  consumers never pull `sqlx`/`pgvector`/`uuid`/`chrono`. Migrations
  (`crates/hacienda-rag/migrations/0001_init.sql`) create `rag_collections`, `rag_documents`,
  `rag_chunks` and are run explicitly, never implicitly from a constructor. Distance metrics
  map to pgvector operators (`Cosine` → `<=>`, `L2` → `<->`, `InnerProduct` → `<#>`);
  `IndexMethod::Diskann` has no pgvector equivalent and is substituted with HNSW at
  index-build time with a `tracing::warn!`, and `capabilities()` never advertises `Diskann` so
  callers can discover the substitution in advance rather than being surprised by it.
  `RetrieveMode::Hybrid` fuses vector and full-text candidates via reciprocal rank fusion
  (k=60); `Sparse`/`LateInteraction` are stored as JSONB pass-through, not yet scoreable by
  this backend. Uses `sqlx::query`/`QueryBuilder` (not the `query!`/`query_as!` compile-time
  macros) because `retrieve`/`delete_by_filter` compile arbitrary `Filter` trees to SQL at
  runtime and because the macros need a live, already-migrated `DATABASE_URL` at `cargo build`
  time. **Known deployment issue:** `hacienda-rag`'s and `hacienda-core`'s migrations are both
  numbered `0001`; sqlx's `_sqlx_migrations` table is one-per-physical-database, so running
  both crates' migrations against the same database produces `VersionMismatch(1)` — a
  production deployment must point `hacienda-rag` at a distinct database from
  `hacienda-core`'s stores until this is resolved. 57 non-live tests plus 19 `#[ignore]`d
  live tests (`DATABASE_URL`-gated, run against a real `pgvector/pgvector` Postgres instance,
  mirroring `hacienda-core`'s own live-Postgres test convention rather than testcontainers).
  Route existence for `/v1/rag/*` was confirmed against the real, CI-synced `xberg-sdks`
  OpenAPI spec (Phase 12 Task 3 Step 1); answer-synthesis (streaming LLM answers over
  retrieved chunks) was explicitly scoped out of Phase 12 (Task 3 Step 4) — no upstream route
  exists to build a contract against.
- **`/v1/rag/*` HTTP routes (Phase 12 Task 3).** `POST /v1/rag/collections`,
  `GET`/`DELETE /v1/rag/collections/{name}`, `POST /v1/rag/collections/{name}/documents`, and
  `POST /v1/rag/collections/{name}/retrieve` — the 5 of 8 confirmed `/v1/rag/*` routes that map
  onto an existing `RagStore` method. List-collections, list-documents, per-document reindex,
  and migrate-embeddings (plus the RAG-specific use of the jobs-poll route) have no trait
  primitive to serve them and are not built; adding them needs new `RagStore` methods first,
  which is a trait-design task of its own. All 5 routes require `Capability::DocumentsProcess`
  (no new capability variant — a RAG collection carries the same class of redacted content
  `/v1/documents` already gates) and are 400 (`ApiError::invalid_request`) rather than 404 or
  500 when `ApiState::rag_store` is `None`, since RAG is opt-in per deployment. Handlers call
  `Arc<dyn RagStore>` directly with no facade wrapper (mirrors `JobStore`, already held
  directly on `ApiState`); `retrieve` fetches the `CollectionSpec` first and calls
  `RetrieveQuery::validate` against it before the query reaches the store. Audit-logging parity
  with `/v1/documents` was considered and explicitly deferred: `HaciendaFacade::record_audit`
  is keyed to entity spans from the redaction pipeline, and a RAG upsert has none to attribute
  without inventing a fictitious `PiiOutput` or a new audit-event shape. `hacienda serve`
  attaches an `InMemoryVectorStore` by default, the same in-memory-by-default precedent as its
  job store; an embedder wanting durability constructs a `PgVectorStore` and passes it to
  `ApiState::with_rag_store` instead. Writing the round-trip route test surfaced a real,
  pre-existing bug: `PrimaryScore`'s `Vector`/`FullText`/`Sparse`/`LateInteraction` variants
  were tuple newtypes under `#[serde(tag = "kind")]`, which `serde_json` cannot serialize
  (internally-tagged enums require variant content to be a map, not a bare scalar) — every real
  `retrieve` call over HTTP 500'd. Fixed by giving each variant a named `score` field; no
  existing test had round-tripped a `RetrieveOutput` through `serde_json` before this route
  test did.
- **Real pseudonymisation.** `RedactionMode::Pseudonymize` now emits a keyed, deterministic,
  reversible token — `[EMAIL:k1:MZXW6YTB...]` — built with AES-256-SIV (RFC 5297) over the
  NFKC-normalised value, with the PII category as authenticated associated data. Equal
  values yield equal tokens across processes and runs, so a reader can follow one
  pseudonymous subject through a corpus; a key holder can reverse it.
- `Pseudonymiser::{new, with_active, token, reveal}`, `KeyResolver`, `EnvKeyResolver`,
  `KeyId`, and `PseudonymKey` in `hacienda_core::redaction`. Keys are read from
  `HACIENDA_PSEUDONYM_ACTIVE_KEY` and `HACIENDA_PSEUDONYM_KEY_<ID>` (64 bytes of hex), or
  from any store an embedder supplies via `EnvKeyResolver::with_lookup`.
- Key rotation is additive: the key id travels in the token, so tokens minted under a
  retired key stay revealable once that id is listed as retired.
- `PiiPipeline::with_pseudonymiser`, `PiiPipeline::with_detector_and_pseudonymiser`, and
  `HaciendaFacade::with_key_resolver`.
- `RedactionConfig::key_id` — an *identifier*, never key material. Key material never
  enters config and so never reaches `config show`, logs, or support bundles.
- **Durable store layer (Phase 1).** `AuditStore`, `ReviewStore`, and `JobStore` traits in
  `hacienda_core::{audit, review, jobs}`, each with an in-memory backend for tests and for
  embedders who want no filesystem. All methods are async and return `Result`; a backend
  that cannot be reached now says so instead of returning an empty answer.
- **Segmented audit chains.** `Segment`, `SegmentSeal`, `NodeId`, `compute_seal_hash`, and
  `verify_seal_chain` in `hacienda_core::audit`. A blake3 entry chain is inherently
  sequential, so one global chain would serialise every writer forever. Each writer instead
  owns a segment numbered from zero, and segments are linked by a second chain over their
  seals. Verification is three independent checks: entries within a segment, the seal chain
  across segments, and each seal's recorded tip against the segment it seals.
- `SegmentSeal` carries `entry_count` and a `seal_hash` over the previous seal. The design's
  original two-check scheme could not detect truncation of a segment's tail; `sealed_tip`
  catches it, and the count exists so recovery can report *"holds 37 entries, seal records
  40"* rather than an opaque hash mismatch.
- `FileAuditStore` — a durable audit backend writing JSON-lines segments under a per-node
  directory, with `SyncPolicy::{EveryBatch, OnSeal}`. On `open` it replays and seals a
  segment left open by a previous run, so an unclean shutdown costs no entries.
- `InMemoryJobStore` with compare-and-swap `transition(id, from, to)`. Exactly one worker
  must claim each queued job; a get-then-put API loses that property while passing every
  sequential test.
- `FileReviewStore` — a durable review backend. It logs `Submitted`/`Assigned`/`Decided`
  *events* and replays them on open, rather than rewriting a state file per mutation: a
  crash mid-rewrite loses the whole file, while a crash mid-append loses at most the record
  being written.
- `HaciendaFacade::with_stores(config, audit_store, review_store)` and
  `HaciendaFacade::close()`. `close` seals the open audit segment; it is idempotent and
  recommended rather than required, because opening a store already recovers a segment left
  open by a previous run. There is deliberately no `Drop` impl — `Drop` cannot await, and
  `block_on` inside it panics when called from within a Tokio runtime.
- **`hacienda` binary (`hacienda-cli` crate).** `extract`, `scan`, and `config show` are
  implemented: `extract` redacts by default and refuses `--no-redact` without
  `--i-accept-unredacted-pii` (naming `scan` as the honest, no-leak alternative); `scan`
  detects and reports without ever emitting document text. `clap` is a dependency of
  `hacienda-cli` alone, so library consumers of `hacienda` and `hacienda-core` do not
  acquire it through Cargo feature unification.
- `--concurrency` on `extract` and `scan` bounds documents in flight through hacienda's own
  PII stage (`PiiPipeline::process`), default CPU count. It is a separate budget from
  xberg's `extraction.concurrency.max_threads`, which caps Rayon, ONNX intra-op, and
  batch-document parallelism together, and it does not write it. Measured against a
  300-document fixed corpus (issue #31): raising it past 1 did not reach 2x throughput on a
  4-core/~3GB host, and the audit store's `io_order` lock is not why — measured wait stayed
  under 0.2% of wall time at every level tested. `hacienda extract --help` states this.
- `extract` and `scan` call `HaciendaFacade::close()` before exit on every path, error
  paths included, so a durable audit store's segment is always sealed cleanly rather than
  left for the next process's recovery step.
- `ReviewStore::close`, with a default no-op body, plus `ReviewQueue::close`.
  `HaciendaFacade::close` now closes the review store as well as the audit store.
  Both closes are attempted even if the first fails, so one broken backend cannot make
  the other leak. Today's backends hold nothing open — a `FileReviewStore` opens,
  appends, `sync_data`s, and closes within each append — but the facade previously had
  no call site for cleanup at all.
- Initial hacienda distribution crate
- hacienda-core with PII pipeline, redaction, compliance, audit, review, glossary
- xberg integration via PostProcessor and NerBackend traits
- Full polyglot bindings (14 languages) via alef
- CLI with pii, compliance, review, audit subcommands
- REST API with PII, compliance, review, audit endpoints
- Feature flags: xberg-full, pii, compliance, audit, review, glossary

### Changed

- **Breaking:** `HaciendaFacade::process_batch`'s per-document work (PII detection) now
  runs concurrently over a bounded worker pool. `HaciendaResult.pii` still holds one result
  per document in input order — that contract (`facade.rs:39`) is preserved by collecting
  into a pre-sized slot per index rather than pushing as tasks complete. Audit-entry chain
  order is a separate guarantee and is **not** preserved across concurrently-processed
  documents: when `hacienda extract` processes more than one input under `--concurrency` >
  1 (the default is CPU count), the order entries land in the audit chain — and so in
  `--audit-out`'s `audit.json` — now reflects completion order, not the order the input
  files were given in. `AuditEntry` carries no document identifier today (`id` is a random
  UUID minted at record time), so a caller needing document-to-entry correspondence has no
  way to recover it from a concurrent run; that gap is not new here, but concurrency is
  what turns "unused" into "needed."
- **Breaking:** unknown keys in a config file are now rejected instead of discarded.
  `HaciendaConfig` and every hacienda-owned config below it carry `deny_unknown_fields`.
  A misspelt key previously parsed cleanly and left the control at its default, so the
  file said a control was configured and the process disagreed. The forward-compatibility
  cost is already paid regardless: xberg's `ExtractionConfig` has declared
  `deny_unknown_fields` all along, so `[extraction]` was strict while the sections
  hacienda owns were not.
- **Breaking:** `AuditConfig` loses `log_path`, `format`, `max_files`, `max_file_size_mb`,
  and `include_span_hash`, along with `max_file_size_bytes()`. Nothing read any of them.
  An operator who set `log_path` and enabled auditing got no file and no warning. They
  were also wrong-shaped for what replaced them — `FileAuditStore` writes a directory of
  segments rather than one rotating file, and rotation is caller-driven, not
  size-triggered. Point at a durable store with `HaciendaFacade::with_stores`.
- **Breaking:** `RedactionAuditEntry::action` is a `RedactionAction`, was a
  `RedactionMode`. A mode says a custom template was configured; the action carries the
  template that was actually applied, which is the question an auditor is asking. The
  `From<RedactionMode> for RedactionAction` impl is removed: the conversion cannot be
  total, and it had been filling the `Custom(String)` gap with the literal `"template"`,
  making every custom redaction in the chain identical.
- `HaciendaFacade::with_stores` now honours an explicitly supplied review store even when
  `config.review` is `None`, defaulting the threshold and deadline. It previously dropped
  the store and returned a facade with no queue, so every decision recorded against it
  went nowhere. This matches the audit arm, where an explicit store already won.
- `Pseudonymize` now validates configured NER labels at pipeline construction rather than
  failing partway through a document.
- Migrated from xberg-pii-ecosystem to hacienda distribution
- **Breaking:** `RedactionConfig::default().mode` is now `Mask`, was `Pseudonymize`.
  Pseudonymisation requires a secret the default path has no way to obtain, so it cannot
  be the out-of-the-box mode.
- **Breaking:** `RedactionEngine::new` takes a second argument,
  `Option<Arc<Pseudonymiser>>`, and returns `Result<Self, RedactionError>`. It returns
  `MissingPseudonymKey` for `Pseudonymize` without a key rather than silently masking:
  masking and pseudonymisation are different controls under GDPR Art. 4(5), and quietly
  applying the weaker one would leave an operator believing they hold reversible output.
- **Breaking:** `RedactionEngine::redact` returns `Result<RedactionResult, RedactionError>`.
  It fails the whole call rather than emitting a document that looks redacted and still
  contains the span.
- **Breaking:** every `ReviewQueue` method is now `async` and returns `Result`. `list` in
  particular no longer collapses a backend failure into an empty vector. An empty list is a
  meaningful answer — *nothing awaits review* — and a store that cannot be read must not be
  able to impersonate it. `get` returns `Result<Option<_>>`: `Ok(None)` means the store was
  read and holds no such item, `Err(_)` means it could not be read at all.
- **Breaking:** `ReviewQueue::new` takes an `Arc<dyn ReviewStore>`. `ReviewQueue` is now a
  policy wrapper over a backend rather than the storage itself.
- **Breaking:** `HaciendaFacade::audit_entries` and `verify_audit` are now `async` and
  return `Result`. The facade holds an `Arc<dyn AuditStore>` in place of the old in-process
  `Mutex<AuditChain>`, so reading the chain can now involve I/O and can now fail.
- **Breaking:** an audit or review write failure fails the whole `process`/`process_batch`
  call. Previously a failed write was discarded and the caller received a
  `HaciendaResult` reporting zero audited entries — indistinguishable from a document that
  contained no PII.

### Security

- **`Pseudonymize` output from any earlier version is not pseudonymised data.** It was the
  per-category constant `[EMAIL:****]` — masked data with a misleading label, and it was
  the *default* mode. It carries no key, distinguishes no subjects, and cannot be
  reversed. Any compliance claim resting on that output as pseudonymisation under GDPR
  Art. 4(5) does not hold, and re-processing from source is the only remedy.
- Key material is held in `Zeroizing` and wiped on drop. `PseudonymKey` has a hand-written
  `Debug` that redacts it and no `Display`, `Clone`, or `Serialize` impl.
- Token length is padded to 16-byte buckets. Deterministic encryption necessarily
  discloses equality; bucketing keeps it from also disclosing exact plaintext length.
- Reveal failures are deliberately indistinguishable across wrong key, altered category
  label, and forged body, so the error is not an oracle.
- **Audit and review logs survive a crash mid-append.** Both file stores treat the newline
  as the record terminator: an unterminated tail is removed from the file at open, with a
  `warn`, and every terminated line must still parse. Skipping the tail without removing it
  is not enough — appends land at EOF, so a fragment left in place is welded to the next
  record on the same physical line, and that line then looks truncated too. One power cut
  would otherwise have made every subsequent write silently unrecoverable. The surviving
  prefix is a valid hash-chain prefix and still verifies; no entry a caller was told was
  durable is lost.
- **A failed disk write now poisons `FileAuditStore` and `FileReviewStore`.** Both apply
  the mutation in memory and then write, so a write failure leaves memory ahead of disk.
  A caller retrying a failed `decide` would then receive `AlreadyDecided` for a decision
  that never reached the disk — a durable-looking answer for data that was lost. After
  any write failure every subsequent call, read or write, returns `Io` until the store is
  reopened; reopening replays from disk, which is the source of truth and was never
  updated.
- Hash-chained audit log with blake3
- JWT authentication for API

## [0.1.0] - 2026-07-28

### Added

- First public release
- GDPR/DORA/AI Act compliance features
- 42 PII regex patterns + GLiNER2 ML backend
- 5 redaction modes (Mask, Hash, Pseudonymize, Remove, Custom)
- DPIA, Model Card, DORA, AI Act report generators
- Human review queue with Approve/Reject/Modify
- Hash-chained audit log (blake3) with CSV/JSON export
- Entity glossary with Markdown/HTML/Wiki link injection
- PCI-DSS, HIPAA, GDPR, Custom redaction profiles
- CLI, REST API, MCP server, 14 language bindings

[Unreleased]: https://github.com/jamon8888/hacienda-engine/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jamon8888/hacienda-engine/releases/tag/v0.1.0
