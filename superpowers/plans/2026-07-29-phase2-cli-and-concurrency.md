# Phase 2: `hacienda-cli`, Concurrency, and the Audit-Contention Measurement — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a `hacienda` binary with `extract`, `scan`, and `config show`; make the PII stage concurrent; and produce the measurement that decides whether Phase 6's audit work starts now or waits for Phase 4.

**Architecture:** A new `hacienda-cli` crate owning clap, config discovery, and provenance tracking. `hacienda-core` gains a bounded worker pool inside `process_batch` and enough instrumentation to say how much of a document's wall time was spent waiting on audit append. `hacienda` and `hacienda-core` gain no clap and no CLI-shaped API.

**Tech Stack:** Rust 2021, `clap` 4 (derive), `toml` 0.8 and `serde_json` (both already workspace deps), `tokio` (present), `futures` or `tokio::task::JoinSet` for the worker pool. `serde_yaml` **only if** YAML discovery survives Task 1 Step 2.

**Spec:** `superpowers/specs/2026-07-28-hacienda-cli-api-integration-design.md` §4, §6, §9 Phase 2, §12.5. Closes §8 Gap 3 (#30). Produces the gate for §8 Gap 4 (#31).

**Baseline:** 222 passing / 1 ignored across `hacienda-core` and `hacienda` at plan time; clippy `--all-targets --all-features -D warnings` clean; `cargo fmt --check` clean.

---

## Ground Truth — Verified vs Assumed

Every row below was read from source on 2026-07-29.

**Verified by reading source:**

| Fact | Location |
|---|---|
| Workspace members are `["hacienda", "hacienda-core"]`; `packages/*` excluded | `Cargo.toml:3-4` |
| **There is no `hacienda-cli` and no `hacienda-api` crate.** `hacienda/src/` is `lib.rs` alone | `ls hacienda/src` |
| **clap is not a dependency anywhere in the workspace**; `num_cpus` is not either | `grep` over root `Cargo.toml` |
| `toml = "0.8"` and `uuid` (v4, serde) are already workspace dependencies | `Cargo.toml:34,40` |
| `xberg` is a **git tag dependency** (`tag = "v1.0.2"`), not a path dep | `Cargo.toml:59` |
| `HaciendaConfig` derives `Serialize, Deserialize, Default` with `#[serde(default)]`; fields `extraction`, `pii`, `compliance`, `review`, `glossary` | `hacienda-core/src/config.rs:14-26` |
| `HaciendaResult` and `HaciendaMetadata` both derive `Serialize` — `--format json` needs no new serialisation | `facade.rs:34,51` |
| `process_batch` calls `extract_all(inputs, config)` **once**, then loops the PII pipeline over `extraction.results` **sequentially** | `facade.rs:289-306` |
| Per document the loop does: `pipeline.process`, `observe_glossary`, `record_audit(...).await`, `submit_for_review(...).await`, then assigns `document.content` | `facade.rs:296-304` |
| `extract_all` calls `xberg::extract` for one input and `xberg::extract_batch` for many | `facade.rs:422-431` |
| **xberg's batch extraction is already concurrent** — `extract_batch_concurrent` spawns onto a `tokio::task::JoinSet` under `feature = "tokio-runtime"` on non-wasm | `../xberg/crates/xberg/src/engine/extract_impl.rs:51-71,98-` |
| xberg has its **own** `ConcurrencyConfig { max_threads: Option<usize> }` reachable at `ExtractionConfig::concurrency` | `../xberg/.../core/config/concurrency.rs:24-31`, `.../config/extraction/core.rs:342` |
| xberg's auto-detected budget is `num_cpus::get().min(8)` — **capped at 8**, and it caps Rayon, ONNX intra-op, *and* the batch document budget together | `../xberg/.../config/concurrency.rs:50-54` |
| The audit serialisation point is `FileAuditStore::io_order`, a `tokio::sync::Mutex<()>` held across mutation **and** the `spawn_blocking` write+fsync | `audit/store_file.rs:132-143,271-` |
| `InMemoryAuditStore` serialises on a `std::sync::Mutex<State>` not held across `.await` | `audit/store.rs:102-106` |
| `RedactionMode` = `Mask` (default), `Hash`, `Pseudonymize`, `Remove`, `Custom` | `redaction/types.rs:8-28` |
| The glossary is a `Mutex<EntityGlossary>` on the facade, taken per document by `observe_glossary` | `facade.rs:29`, `facade.rs:299` |
| `HaciendaFacade::close()` is `async`, idempotent, and seals the open audit segment | `facade.rs:256` |
| Phase 1 measured `FileAuditStore` fsync at 934.2ms / 1000 entries in batches of 10 (`EveryBatch`) vs 308.4ms (`OnSeal`) | `superpowers/plans/2026-07-28-phase1-store-layer.md` Task 8 note |

**Assumed, to confirm during implementation:**

- That xberg's `ExtractionConfig` round-trips through `toml` cleanly. It is a large upstream
  struct and a single non-`Deserialize`-friendly field breaks the whole config file. **Task 1
  Step 1 proves this before any command is written**, because if it fails the config format is
  not a detail — it is the phase.
- That YAML discovery is worth a dependency. §6.3 names `hacienda.{toml,yaml,json}`; TOML and
  JSON cost nothing extra. Task 1 Step 2 decides.
- That the PII pipeline is `Send + Sync` enough to be shared across worker tasks behind an
  `Arc`. Task 4 Step 1 proves it by compiling a `JoinSet` spawn before any pool exists.

---

## Design Decisions

### D1. `--concurrency` means the PII stage, and it is not xberg's knob

This is the single most confusable point in the phase, so it is settled first.

There are **two** independent concurrency budgets:

| Budget | Owner | Reached via | Default |
|---|---|---|---|
| Extraction workers, Rayon pool, ONNX intra-op threads | xberg | `extraction.concurrency.max_threads` | `num_cpus::get().min(8)` |
| PII detect → redact → audit → review per document | hacienda | new | CPU count |

§8 gap 3 says "`process_batch` awaits each document in turn". Read against source that is
true of the **PII loop only** — `extract_all` already hands the whole batch to xberg, which
already fans out across a `JoinSet`. So the gap is narrower than its wording suggests, and
fixing it means parallelising `facade.rs:296-306`, not replacing `extract_all`.

Therefore:

- `--concurrency N` sets the **hacienda PII worker count**. `--help` says so in those words.
- It does **not** silently write `extraction.concurrency.max_threads`. Doing so would let a
  user asking for 16 PII workers also uncap xberg's Rayon pool on a 4-core box, and the
  resulting thrash would be attributed to the wrong subsystem.
- `config show` reports both budgets, separately, with their separate provenance. A user who
  sets one and wonders why the other did not move must be able to see both.
- xberg's `.min(8)` cap is **not** mirrored. Hacienda's default is CPU count per §12.5. Where
  they differ on a >8-core machine, `config show` will show two different numbers, which is
  correct and is exactly why both are printed.

### D2. The measurement is a deliverable, not a hope — so it is instrumented, not inferred

§9 requires two numbers: wall-clock at `--concurrency` 1/2/4/CPU-count, and *the fraction of
per-document wall time spent blocked on the audit mutex*. The first is a stopwatch. The second
cannot be inferred from the first.

The naive version — time the whole `record_audit(...).await` — measures **wait plus work**.
The fsync is the work, and Phase 1 already measured it at roughly 0.9ms per batch. Reporting
wait-plus-work as "contention" would cross the 20% threshold on fsync cost alone and unblock
Phase 6 for the wrong reason.

So instrument the wait specifically: inside `FileAuditStore::append`, time the
`io_order.lock().await` and nothing else. Report three numbers per run:

1. total per-document wall time,
2. time inside `append` (wait + write + fsync),
3. time waiting for `io_order` alone.

The §9 threshold is evaluated against (3)/(1). (2) is reported alongside so that a reader can
tell a contention problem from an fsync problem, because the two have opposite fixes:
contention wants more segments, fsync wants `SyncPolicy::OnSeal` or larger batches.

Instrumentation ships as `tracing` fields plus an opt-in counter, not as a permanent hot-path
`Instant::now()` pair on every append. Decide the mechanism in Task 5 Step 1.

### D3. Concurrency must not reorder the audit chain into meaninglessness

Running N documents in parallel against one `Arc<dyn AuditStore>` is safe — Phase 1's
`io_order` guarantees write order matches mutation order. What it does **not** guarantee is
that document order matches chain order, and it never could: that is what concurrency means.

Two consequences the implementation must not paper over:

- **`HaciendaResult.audit_entries` order becomes non-deterministic** across runs. Any test
  asserting on its order is asserting on scheduling. Task 4 must find and fix such tests
  rather than pinning concurrency to 1 in the test config to keep them green.
- **`pii: Vec<PipelineResult>` is documented as "one per document, in the same order"**
  (`facade.rs:39`). That contract survives only if results are collected back into input
  order. Collect into a pre-sized `Vec<Option<_>>` by index, exactly as xberg's own
  `extract_batch_concurrent` does (`extract_impl.rs:123-124`) — do not push as tasks complete.

The seal/segment model exists precisely so that concurrent writers need not share one
sequential chain. Phase 2 does **not** use it that way: one facade, one segment, N workers
queueing on `io_order`. That is deliberate — it is the configuration that produces the
measurement Phase 6 needs. Handing each worker its own segment here would fix gap 4 by
accident and leave gap 4's decision unmeasured.

### D4. `config show` prints provenance, and the key is not in the chain

§6.3: "`hacienda config show` must report **where each value came from**." That means the
output is not a serialisation of `HaciendaConfig` — it is a serialisation of
`(path, value, source)` triples, where source is one of `default`, `file:<path>`, `--config`,
`--config-json`, `flag`.

Two hard rules:

- **Key material never appears.** §6.3 puts the pseudonymisation key deliberately outside the
  precedence chain. `config show` prints the active `key_id` and the resolver's name. If a
  code path could ever print `PseudonymKey`, the type's hand-written redacting `Debug` from
  Phase 0 is the last line of defence, not the first — do not rely on it.
- **Provenance is recorded during merging, not reconstructed afterwards.** Reconstructing by
  diffing the merged config against the defaults cannot distinguish "the file set it to the
  default value" from "nobody set it", and those are different answers to the question the
  operator is actually asking, which is *"why is this not redacting?"*.

### D5. `--no-redact` refuses, and refusing is the feature

§6.2 marks `--no-redact` as refusing "without explicit acknowledgement". The inverted polarity
of the whole CLI (`hacienda extract` yields redacted text) is worth nothing if the escape
hatch is one flag. Require a second, explicit acknowledgement — an environment variable or a
paired `--i-understand-this-emits-unredacted-pii` flag — and make the error message state what
will be emitted, not merely that a flag is missing.

`scan` is the honest alternative and the error should name it: detect-only, no rewrite, no
unredacted text on stdout.

---

## Scope

**In:** `hacienda-cli` crate; `extract`; `scan`; `config show`; `--concurrency` end-to-end;
concurrent PII stage in `hacienda-core`; the §9 measurement and its written report.

**Out, with the phase that owns each:** `audit` (5), `review` (5), `compliance` (5),
`glossary` (5), `serve` (4), `config validate` (4 — it wants the API's schema), `completions`
(4), `xberg` passthrough (7), `--shard` / `--node-id` (6), `hacienda-api` (4).

`config show` is in scope while `config validate` is not, because `show` is what makes this
phase's own configuration debuggable and `validate` is what makes the *server's* startup
safe. Building `validate` now means building it against a config surface Phase 3 will extend.

---

## Tasks

### Task 1 — Prove the config file format before building anything on it

- [x] **Step 1 (red).** A test in a scratch integration test that round-trips a
      `HaciendaConfig` with a non-default `extraction` through `toml::to_string` and
      `toml::from_str` and asserts equality on the fields that matter. xberg's
      `ExtractionConfig` is large and upstream; if it does not round-trip, every later task
      is built on sand.

      **Result: it round-trips, stably.** `hacienda-core/tests/config_round_trip.rs`, 4 tests,
      all green: default config; all four optional stages enabled; xberg's concurrency budget;
      and a behaviour-recording test for unknown keys. No blocker — the config file format of
      §6.3 is buildable as specified.

      Two findings:

      - **`ConcurrencyConfig` is missing from xberg's crate-root re-export**, though every
        sibling config type is there (`../xberg/crates/xberg/src/lib.rs:189-198`). It is
        reachable as `xberg::core::config::ConcurrencyConfig` because `pub mod core` is
        public, so this needs no patch to xberg — but D1 requires constructing it, so the path
        is recorded here rather than rediscovered.
      - **Unknown keys are silently ignored.** `[pii]\nregex_frist = true` parses fine and the
        misspelt key vanishes. This is §6.3's "why is this not redacting" incident with a
        worse failure mode than precedence, and it reaches every embedder, not just the CLI.
        Filed as **#34**.
- [ ] **Step 2 (decide).** Settle the discovered-file formats. TOML and JSON are free. Record
      whether YAML earns `serde_yaml`, and if it does not, amend §6.3 to say
      `hacienda.{toml,json}` rather than leave the spec promising a format that does not exist.
- [ ] **Step 3.** Decide #34's resolution *before* Task 3, since `config show` is the command
      an operator runs when asking this exact question and the answer shapes its output. Note
      that `deny_unknown_fields` everywhere is the wrong fix — see the issue.

### Task 2 — The `hacienda-cli` crate exists and does nothing

- [x] **Step 1.** Create `hacienda-cli` with `[[bin]] name = "hacienda"`. Add to workspace
      members. Depend on `hacienda`, `clap` (derive), `tokio` (rt-multi-thread, macros),
      `anyhow`, `tracing-subscriber`. **Do not** add clap to `hacienda` or `hacienda-core` —
      §4 rejects feature-gating within one crate precisely because Cargo feature unification
      would pull clap into every library consumer.
- [x] **Step 2 (red).** An integration test asserting `hacienda --help` exits 0 and names
      `extract`, `scan`, and `config`. `hacienda-cli/tests/help.rs`, 3 tests. Confirmed red
      first — all three failed against a `fn main() {}`. No test-harness dependency was
      needed: cargo sets `CARGO_BIN_EXE_hacienda` for integration tests in the crate that
      defines the binary, so `assert_cmd` earned nothing.
- [x] **Step 3 (green).** Clap skeleton with the three subcommands, each `todo!()`-free and
      returning a clean "not implemented" exit code rather than panicking.

      Deviation: the §6.2 commands belonging to later phases are **absent**, not stubbed. A
      subcommand that parses and then apologises is indistinguishable from one that is
      broken, and `hacienda audit verify` existing-but-refusing is worse than not existing
      when the question is whether a chain is intact.

      `--concurrency` is defined once in a flattened `ConcurrencyArgs` so that D1's
      distinction from xberg's thread budget, and §6.3's requirement to name audit append as
      the ceiling, are stated identically everywhere the flag appears rather than copied and
      allowed to drift.
- [x] **Step 4.** Confirm `cargo build -p hacienda` still does not pull clap:
      `cargo tree -p hacienda -i clap` must report no match. Verified — *"package ID
      specification `clap` did not match any packages"* for both `hacienda` and
      `hacienda-core`. The §4 layering holds.
- [x] **Step 5.** Verify: 231 passing / 1 ignored / 0 failed across the workspace (222 core,
      4 config round-trip, 3 CLI help, 2 doc/integration) against the 222 baseline. Clippy
      `--workspace --all-targets -D warnings` clean; `cargo fmt --all --check` clean.

### Interlude — outstanding issues cleared before Task 3

Not in the original plan. Requested between Tasks 2 and 3, on the reasoning that Task 3 must
not build a `config show` on top of keys that do nothing. Closed: #11, #12, #23, #24, #25,
#26, #29, #34. Commented and left open: #19.

Three findings that change Task 3's inputs:

1. **`deny_unknown_fields` is now on every hacienda-owned config struct (#34).** The option
   the issue preferred — warn inside `extraction`, reject elsewhere — was not available:
   xberg's `ExtractionConfig` has declared `deny_unknown_fields` all along, verified by
   `should_already_reject_an_unknown_key_under_extraction`. The asymmetry ran the wrong way,
   with the one section hacienda does not own being the strict one. **Consequences for Task
   3:** `config show` does not need to surface unrecognised keys, because the load fails
   first and names the file and key; and `--strict-config` is unnecessary, since strict is
   the only mode.
2. **`AuditConfig` is down to `enabled` and `config_hash` (#23).** `log_path`, `format`,
   `max_files`, `max_file_size_mb`, and `include_span_hash` had no consumer anywhere.
   **Consequence for Task 3:** every key `config show` prints now has a reader. Task 6's
   `--audit-out` has no config-file counterpart yet; it gets one at the point the CLI reads
   it, not before, or the defect returns under a new name.
3. **`with_stores` honoured `config.review` over an explicitly passed store (#25).** Fixed
   to match the audit arm — explicit wins. Worth carrying into Task 3 as the same rule one
   layer up: an explicitly supplied layer beats an absent one, and "absent" must never
   cancel "explicit".

Verification after the interlude: 245 passing / 1 ignored / 0 failed across the workspace
(233 core lib, 7 config round-trip, 3 CLI help, 2 doc/integration) against the 222 baseline.
Clippy `--workspace --all-targets -D warnings` clean; `cargo fmt --all --check` clean.
`HaciendaFacade::close`'s new review arm was mutation-tested, not assumed.

### Task 3 — Config discovery, precedence, and provenance

- [ ] **Step 1 (red).** Table-driven tests over the §6.3 chain
      `defaults < discovered file < --config < --config-json < flags`. At least one case per
      adjacent pair, plus one where all five set the same value, asserting the winner **and**
      its recorded source.
- [ ] **Step 2 (green).** A `ConfigLoader` producing `(HaciendaConfig, Provenance)` together.
      Per D4, provenance is recorded as each layer is applied.
- [ ] **Step 3 (red).** `config show` output tests: every value carries a source; the
      pseudonymisation `key_id` is present; **no test fixture key material appears anywhere in
      the output** — assert on the absence explicitly, with a key set in the environment, so
      the test would fail if someone later serialised the resolver whole.
- [ ] **Step 4 (green).** `hacienda config show`, with `--format text|json`.
- [ ] **Step 5.** `config show` reports both concurrency budgets of D1, separately.

### Task 4 — Make the PII stage concurrent (closes #30)

- [ ] **Step 1 (spike).** Prove `Arc<PiiPipeline>` and `&HaciendaFacade` survive a
      `JoinSet` spawn — compile it before designing around it. If the pipeline is not `Sync`,
      the pool shape changes and the rest of this task is rewritten, so find out first.
- [ ] **Step 2 (red).** A test that N documents produce `pii` results **in input order** under
      concurrency > 1. Per D3 this is the contract at `facade.rs:39` and nothing currently
      protects it.
- [ ] **Step 3 (red).** A test that every document is audited exactly once under concurrency,
      reusing Phase 1's `CountingAuditStore`. A worker pool that drops a document on a full
      channel is a compliance defect, not a performance one.
- [ ] **Step 4 (green).** Bounded worker pool over `extraction.results` with a configurable
      limit. Collect into `Vec<Option<_>>` by index (D3). The glossary mutex and audit store
      are shared via the existing `&self`.
- [ ] **Step 5.** Find every existing test that asserts on `audit_entries` ordering. Fix them
      to assert on set membership or per-document identity. **Do not** pin test concurrency to
      1 to keep them green — that hides the exact regression the tests exist to catch.
- [ ] **Step 6 (mutation).** Remove the index-collection and push as tasks complete; the
      Step 2 test must fail. Revert. Then set the pool bound to `usize::MAX`; note what, if
      anything, fails — if nothing does, the bound is untested and Step 3 needs strengthening.
- [ ] **Step 7.** Wire `--concurrency` through the CLI to the pool. Default CPU count. The
      `--help` text names audit append as the ceiling, per §6.3's closing paragraph.

### Task 5 — The measurement that gates Phase 6 (#31)

- [ ] **Step 1 (decide).** Pick the instrumentation mechanism per D2. Constraint: it must
      distinguish *waiting for `io_order`* from *doing the fsync*, and it must not cost an
      `Instant::now()` pair per append when switched off.
- [ ] **Step 2 (green).** Instrument `FileAuditStore::append` around the `io_order` acquisition
      only.
- [ ] **Step 3 (red).** A test that the reported wait is near zero at concurrency 1 and
      materially above zero at concurrency 8 against a `FileAuditStore`. A contention metric
      that reads zero under contention is worse than no metric — it will be believed.
- [ ] **Step 4.** Fixed corpus, checked in or generated deterministically. Same corpus for
      every run; note its size and document count in the report.
- [ ] **Step 5.** Run at `--concurrency` 1, 2, 4, and CPU count. Record wall-clock, time in
      `append`, and `io_order` wait, per D2's three numbers. **Note the host constraint: this
      machine has 4 cores and roughly 3GB RAM, and builds here are I/O-bound.** A throughput
      curve measured under memory pressure measures the memory, so record RAM headroom
      alongside each run or the numbers cannot be defended.
- [ ] **Step 6 (evaluate).** Against §9's thresholds: throughput < 2x at CPU count, **or**
      audit-lock wait > 20% of per-document time → comment on #31 that Phase 6's audit work is
      unblocked immediately and is not held behind Phase 4. Otherwise comment with the numbers
      and leave it gated. Either way the numbers go in the issue, not only in this plan.
- [ ] **Step 7.** Update §6.3 and `hacienda extract --help` so the ceiling is documented
      rather than discovered, whichever way Step 6 goes.

### Task 6 — `extract` and `scan`

- [ ] **Step 1 (red).** `scan` on a fixture containing a known email: exits 0, reports the
      detection, and **emits no redacted or unredacted document text**. Assert the absence.
- [ ] **Step 2 (green).** `hacienda scan <input>...`, `--format text|json`.
- [ ] **Step 3 (red).** `extract` on the same fixture: the email does not appear in stdout in
      any mode. Assert per mode — `Mask`, `Hash`, `Pseudonymize` — because they fail
      differently and `Pseudonymize` without a key must error rather than degrade (Phase 0).
- [ ] **Step 4 (green).** `hacienda extract` with `--mode`, `--threshold`, `--format`,
      `--audit-out`, `--model-dir`, `--lora-dir`.
- [ ] **Step 5 (red).** `--no-redact` alone exits non-zero, and the message names both what
      would be emitted and `scan` as the alternative (D5).
- [ ] **Step 6 (green).** `--no-redact` plus its explicit acknowledgement.
- [ ] **Step 7.** `--audit-out` writes a verifiable chain; assert `verify_audit` passes on
      what was written, not merely that a file exists.
- [ ] **Step 8.** The CLI calls `facade.close()` before exit on every path, including the
      error paths. Phase 1 made `close` recommended rather than required, but a CLI that
      leaves a segment open on every error is what turns "recommended" into a recovery cost
      paid on every run.

### Task 7 — Documentation and close-out

- [ ] **Step 1.** CHANGELOG: the new binary, `--concurrency`, and any breaking change Task 4
      forced on `process_batch`'s ordering guarantees. If `audit_entries` ordering changed, it
      is **Breaking** and says so — a caller relying on it will not read a note filed under
      Added.
- [ ] **Step 2.** Strike §8 gap 3 in the spec with a "Closed 2026-07-__ by Phase 2" note, as
      gaps 1, 2, 5, 7 were struck. Close #30.
- [ ] **Step 3.** Post the measurement to #31 per Task 5 Step 6.
- [ ] **Step 4.** Verify: full suite, clippy `--all-targets --all-features -D warnings`,
      `cargo fmt --check`. Record the test count against the 222 baseline.
- [ ] **Step 5.** Completion note: what the measurement said, which mutations were run and what
      each proved, and every deviation from this plan. Per Phase 1's lesson, do **not** tick a
      test bullet without grepping for the test by name first.

---

## Risks

| Risk | Why it bites | Mitigation |
|---|---|---|
| xberg's `ExtractionConfig` does not round-trip through TOML | The whole config file format is unbuildable, and it is discovered in Task 3 after two tasks assume it | Task 1 is first and exists only to answer this |
| The measurement is taken on a 4-core, 3GB host | Contention curves under memory pressure measure the memory | Task 5 Step 5 records headroom; state the host in the report and in #31 |
| Concurrency exposes an ordering assumption in an existing test, and it gets "fixed" by pinning concurrency to 1 | The regression the test exists to catch becomes invisible | Task 4 Step 5 forbids it explicitly; Task 4 Step 6 mutates to confirm |
| `--concurrency` gets wired to xberg's `max_threads` because both are called concurrency | A user asking for PII workers uncaps Rayon on a small box | D1; Task 3 Step 5 prints both, which makes the conflation visible |
| `config show` leaks key material through a `Serialize` on something holding a resolver | It is the exact output an operator pastes into a support ticket | Task 3 Step 3 asserts absence with a real key in the environment |
