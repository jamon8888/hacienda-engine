# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Tier 0 schema verticals.** `hacienda_core::pii::VerticalConfig` (`id`, `labels`) and
  `PipelineConfig::vertical: Option<VerticalConfig>` (default `None`). A configured
  vertical's labels are handed to the NER backend as additional zero-shot
  `EntityCategory::Custom` categories, *extending* the base five categories rather than
  replacing them — a finance vertical still finds people. No new weights, no model
  reload; see `superpowers/specs/2026-07-31-vertical-model-specialisation-design.md`
  §4.1. `VerticalConfig::validate` rejects an empty id, an empty label set, an empty or
  `[`/`:`/`]`-containing label, or case-insensitive duplicate labels, and runs
  unconditionally in `PiiPipeline::assemble` (every build profile, including
  regex-only and wasm) — not only when a model is actually loaded. `hacienda config
  show` prints the active vertical's id and labels. `GET /v1/pii/config` now also
  reports `vertical_id`/`vertical_labels` (deliberately exposed; unlike `model_dir`
  these are not host-identifying).
  **Compatibility:** `PipelineConfig` gained a new public field (`vertical`) and is
  **not** `#[non_exhaustive]` — no type in `hacienda-core` is today, so this follows
  existing precedent rather than introducing a new one, but it does mean external
  struct-literal construction of `PipelineConfig` (`PipelineConfig { .. }` without
  `..Default::default()`) breaks on upgrade, as it would for any field addition to this
  struct. `Default` is unaffected: `PipelineConfig::default()` and `..Default::default()`
  keep working, and `vertical` defaults to `None` — a config with no `[pii.vertical]`
  section behaves exactly as before.
- `hacienda-cli` and `hacienda-api` now compile `hacienda-core` with the `ner-candle`
  feature, so `--model-dir`/`--lora-dir` (already accepted by the CLI) actually reach a
  Candle-backed `NerDetector` instead of always failing with `ModelUnavailable`.
- **Vertical provenance in the audit chain.** `AuditEntry.vertical: Option<String>` and
  `AuditEntryInput.vertical: Option<String>` record which Tier 0 schema vertical (if
  any) was active when an entity was detected. The recorded value is
  `"<id>@<first 8 hex of blake3 over the sorted, case-folded label set>"` (e.g.
  `finance@3f9a1c02`), via the new `VerticalConfig::provenance_id`, not the bare
  vertical id — an id alone would be a false provenance claim, since the same id with a
  different label set detects different things. `HaciendaFacade` populates it from the
  pipeline's configured vertical at both audit-entry construction sites. The field is
  covered by `compute_chain_hash`, appended immediately after `principal`, so it cannot
  be rewritten after the fact without breaking verification; `None` hashes as the empty
  string, so audit chains written before this field existed continue to verify
  byte-for-byte (`should_verify_a_chain_written_before_the_vertical_field_existed`
  pins this against the exact pre-change chain-hash literal).
  **Breaking change:** `audit::export::export_csv`'s header and every row gain a
  `vertical` column, appended last. The CSV export format is unversioned, so this is a
  silent shape change to any downstream parser that reads it positionally or by
  fixed column count — check any such parser before upgrading.
  **Compatibility:** `ChainHashFields` is now `#[non_exhaustive]` — its own rustdoc
  already called adding a field "a deliberate, reviewable act", so this formalises
  that: external struct-literal construction of `ChainHashFields` (there are no known
  external constructors of it outside this crate today) now requires going through a
  crate-provided constructor rather than a bare literal, and will break on every future
  field addition regardless. `AuditEntry` and `AuditEntryInput` are not marked
  `#[non_exhaustive]` — consistent with the Task 2.5 decision for `PipelineConfig`, the
  new `vertical` field is additive and defaults to `None`/absent via `#[serde(default)]`
  on `AuditEntry`, but external struct-literal construction of `AuditEntryInput` (which
  has no `Default` impl) breaks on upgrade, as it would for any field addition to that
  struct.
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
