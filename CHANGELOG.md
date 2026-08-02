# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
