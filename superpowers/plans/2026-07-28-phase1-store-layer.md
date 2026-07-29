# Phase 1: Segment Model and the Store Layer — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give audit, review, and job state a persistence seam, and give the audit chain a segment identity, so that a restart no longer discards the tamper-evident record and two writers no longer need one chain.

**Architecture:** Three object-safe `async` traits — `AuditStore`, `ReviewStore`, `JobStore` — each with an in-memory backend and (for audit and review) a file backend. `AuditChain` is untouched; a new `Segment` wraps one chain with writer identity and a **seal hash** that links a node's segments into a chain one level up. `HaciendaFacade` swaps `Mutex<AuditChain>` and the concrete `ReviewQueue` for `Arc<dyn …Store>`.

**Tech Stack:** Rust 2021, `async-trait` 0.1 (already a workspace dependency), `blake3`, `serde_json`, `tokio` (already present), `uuid`, `chrono`. No new third-party crates.

**Spec:** `superpowers/specs/2026-07-28-hacienda-cli-api-integration-design.md` §12.1, §12.2, §9 Phase 1. Closes §8 Gaps 1, 2, 5.

**Baseline:** 171 `hacienda-core` lib tests + 2 doc/integration tests passing at plan time (`cargo test -p hacienda-core -p hacienda`, exit 0).

---

## Ground Truth — Verified vs Assumed

Every row below was read from source on 2026-07-28. A subagent's survey preceded this and its claims were re-checked by hand; where the two disagree, this table wins.

**Verified by reading source:**

| Fact | Location |
|---|---|
| `AuditChain { entries, last_chain_hash, seq, config_hash }`; `new/push/append/verify/entries/len/is_empty/config_hash/tip` | `audit/chain.rs:9-132` |
| `verify()` recomputes from `GENESIS_HASH` using **`index as u64`** as the sequence number | `audit/chain.rs:85-96` |
| `push` overwrites `input.config_hash` with the chain's own | `audit/chain.rs:32` |
| `append` rejects on `ConfigMismatch` then on recomputed-hash mismatch | `audit/chain.rs:46-76` |
| `compute_chain_hash(prev, seq, id, category, action, span_hash, config_hash)` | `audit/entry.rs` (re-exported `audit/mod.rs:14`) |
| `AuditEntry` derives `Debug, Clone, Serialize, Deserialize` | `audit/entry.rs:59` |
| `AuditError` has 4 variants: `Io`, `Json`, `ChainIntegrity`, `ConfigMismatch` | `audit/error.rs:3-24` |
| `AuditSink` trait — `write`/`flush`/`rotate`, all `&mut self` | `audit/sink.rs:11-18` |
| `FileSink::flush()` is `BufWriter::flush()` only — **no `fsync` anywhere in the crate** | `audit/sink.rs:103-108`; `grep sync_all\|sync_data` → none |
| `FileSink` holds its own `chain` field that `write()` never uses — `write` takes a pre-minted entry | `audit/sink.rs:29,88` |
| `FileSink::rotate()` shifts numbered files but never seals or records the chain tip | `audit/sink.rs:110-137` |
| `AuditSink` / `FileSink` are referenced **nowhere** outside `audit/sink.rs` and its tests | `grep` over `hacienda-core/src`, `hacienda/src` |
| Facade holds `audit_chain: Option<Mutex<AuditChain>>`, `review_queue: Option<ReviewQueue>` | `facade.rs:22-23` |
| The double lock: `let config_hash = lock(chain).config_hash().to_string(); let mut guard = lock(chain);` | `facade.rs:238-239` |
| `record_audit`, `submit_for_review`, `observe_glossary`, `audit_entries`, `verify_audit` are all **synchronous** `fn`, called from `async fn process_batch` | `facade.rs:145-166, 195-197, 235-263` |
| `ReviewQueue { items: Mutex<Vec<ReviewQueueItem>>, config: ReviewConfig }` | `review/queue.rs:14-17` |
| `assign` refuses any item not `Pending` — already a compare-and-swap | `review/queue.rs:111-116` |
| `decide` refuses any item with `decision.is_some()` — also a CAS | `review/queue.rs:81-83` |
| `needs_review` and `priority_from_confidence` are policy, not storage | `review/queue.rs:28-30, 42` |
| No `segment`, `shard`, `store`, or `job` identifier exists anywhere in the repo | `grep -ri` over `hacienda-core/src`, `hacienda/src` |
| `hacienda/src/lib.rs` is 33 lines — there is no CLI or API consumer yet | `wc -l` |
| `async-trait = "0.1"` is a workspace dependency and `hacienda-core` already depends on it | `Cargo.toml:23`, `hacienda-core/Cargo.toml:32` |

**Assumed, to confirm during implementation:**

- That `#[async_trait]` on `AuditStore` keeps `Arc<dyn AuditStore>` object-safe with the
  chosen method signatures. Task 2 Step 1 proves this by compiling an `Arc<dyn AuditStore>`
  binding before any backend exists.
- That `File::sync_data()` is available and cheap enough at one call per document. Task 3
  measures it on the fixture corpus rather than assuming.

---

## Design Decisions

### D1. Sequence numbers are per-segment, and they have to be

`AuditChain::verify()` recomputes each entry's hash with `index as u64` — the entry's
position in `self.entries`, not a stored field. A segment whose chain began at a global
sequence of 5,000 would fail its own `verify()` immediately.

So each segment's chain starts at `GENESIS_HASH` with `seq = 0`. §12.1's check 1 —
"each segment verifies internally, `AuditChain::verify()`, unchanged" — is not merely
satisfied by this, it *requires* it. There is no global total order, which §12.1 already
argues is legally unnecessary: DORA and GDPR require that a record exists, is unaltered,
and can be produced.

### D2. Segments get a seal hash — a strengthening of §12.1

§12.1 sketches `prev_segment_tip: Option<Hash>` as a plain field and verification check 2
as `prev_segment_tip == previous.sealed_tip`. That check is only meaningful while the
predecessor still exists.

Delete a node's oldest segment and rewrite its successor's `prev_segment_tip` to `None`,
and the remaining set passes checks 1 and 2 completely. That is a truncation attack, and
suppressing the earliest records is precisely the attack a tamper-evident log exists to
defeat.

The fix costs one blake3 call per seal and touches no existing code:

```text
seal_hash = blake3(
    prev_seal_hash ‖ segment_id ‖ node_id ‖ config_hash ‖
    sealed_tip ‖ entry_count ‖ opened_at ‖ sealed_at
)
```

A node's segments become a hash chain *of segments* — the same construction as the entry
chain, one level up. Truncating the oldest segment now breaks the successor's recorded
`seal_hash`, which is exactly the property the entry chain already has and the segment
layer did not.

It also gives Phase 6's anchors a well-defined leaf: the merkle root of §12.1 check 3 is
taken over `seal_hash` values, not over raw tips.

`entry_count` is in the preimage too, but — **corrected during Task 1** — not for the
reason first written here. The original claim was that it pins a length `sealed_tip` leaves
free. That is wrong: the tip *is* the last entry's chain hash, so removing entries changes
it either way. What `entry_count` actually buys is an invariant the store can assert at
recovery — replayed entries must number exactly this many — failing with "expected 40
entries, found 37" rather than an opaque hash mismatch. Worth eight bytes; not a distinct
security property, and the code says so.

**This deviates from the spec as written.** Task 8 amends §12.1 to record it.

### D3. The store traits are `async`, and the audit trait is batch-oriented

`record_audit` is synchronous today and called from `async fn process_batch`. Put a file or
Postgres write behind it unchanged and it becomes blocking I/O on an async path — a direct
CLAUDE.md violation and, at Phase 2's `--concurrency N`, a runtime stall rather than a
style complaint.

`#[async_trait]` boxes the futures, which keeps `Arc<dyn AuditStore>` object-safe as §12.2
requires. Native AFIT would not.

The audit trait takes a **batch**:

```rust
async fn append(&self, inputs: Vec<AuditEntryInput>) -> Result<Vec<AuditEntry>, AuditError>;
```

One document produces one call. This is the right transaction boundary for Phase 6's
Postgres, it is one fsync per document rather than per entity, and **it dissolves §8 gap 5
by construction** — there is no second acquisition to make because there is no second call.
Fixing the double lock as a standalone edit would leave the shape that produced it.

Consequence: `record_audit`, `audit_entries`, and `verify_audit` become `async`. That is a
breaking change to `HaciendaFacade`'s public surface; Task 8 records it in the CHANGELOG.

### D4. `ReviewQueue` survives as a policy wrapper

`needs_review` and `priority_from_confidence` are policy; the `Mutex<Vec<_>>` is storage.
Splitting them lets `ReviewQueue` keep its exact public API — every existing call site and
all of `review/queue.rs`'s tests keep working — while the storage moves behind
`Arc<dyn ReviewStore>`.

`assign` and `decide` move into the store as single atomic operations, not as
read-then-write pairs. §12.2 notes these are already compare-and-swap and that the
semantics must survive the move; splitting them into `get` + `put` at the trait boundary
would silently discard that, and the resulting race would only appear under Phase 2's
concurrency. Task 4 tests it with real threads.

### D5. `JobStore` ships as trait + in-memory only

Nothing in the repo produces job state. `POST /v1/documents/async` and `GET /v1/jobs/{id}`
are Phase 4 endpoints.

The §9 ordering argument — establish the seam before endpoints build on it — justifies the
*trait*. It does not justify a file backend: that would be a durability path with no caller,
untested in anger, in a compliance-adjacent crate. Phase 4 picks the backend once a real
producer exists.

The job payload is stored as an opaque `Option<String>` of JSON rather than a typed
`HaciendaResult`. A store layer that names a pipeline type cannot be reused by anything
else, and `Arc<dyn JobStore>` cannot be generic.

### D6. `AuditSink` and `FileSink` are deprecated, not deleted

Both are `pub` (`audit/mod.rs:17`) and neither is used anywhere. `FileSink`'s numeric
rotation conflates "roll the log file" with "seal a chain segment" — the two are now
different operations, and `rotate()` explicitly does not seal.

CLAUDE.md's api-compatibility rule requires one minor version of deprecation before
removal. They get `#[deprecated]` pointing at `AuditStore`, keep working, and their tests
keep running under `#[allow(deprecated)]`.

---

## Global Constraints

- Workspace root `/home/jamin/Documents/hacienda-engine/`, edition 2021, resolver 2.
- Build host is RAM-constrained (~3GB). Do not raise cargo `jobs`. Never run `cargo clean`.
- All tests inline `#[cfg(test)]`. There is no `tests/` directory in `hacienda-core`.
- Test names follow `should_<behaviour>_when_<condition>`.
- No `.unwrap()` in library code. Every `Result` carries context.
- Every new public item gets a rustdoc comment explaining *why*, not just *what*.
- After each task: `cargo test -p hacienda-core`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt`.
- **Confirm the red phase before implementing.** Write the test, run it, read the actual
  failure, then write the code.

---

## Tasks

### Task 1 — Segment types and the seal hash

New file `hacienda-core/src/audit/segment.rs`.

- [x] **Step 1 (red).** Write `segment_tests` asserting the API below, run
      `cargo test -p hacienda-core segment`, and paste the compile errors into the task log.
- [x] **Step 2.** Define the types:

```rust
/// Identifies the writer that owns a segment. Defaults to hostname-pid.
pub struct NodeId(String);

/// One writer's contiguous run of audit entries, sealed when the writer stops.
pub struct Segment {
    segment_id: String,          // uuid v4
    node_id: NodeId,
    config_hash: String,
    prev_seal_hash: Option<String>,
    chain: AuditChain,           // starts at GENESIS_HASH, seq 0 — see D1
    opened_at: String,           // rfc3339
}

/// The immutable record left behind when a segment is closed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentSeal {
    pub segment_id: String,
    pub node_id: String,
    pub config_hash: String,
    pub prev_seal_hash: Option<String>,
    pub sealed_tip: String,
    pub entry_count: u64,
    pub opened_at: String,
    pub sealed_at: String,
    /// blake3 over every field above. See D2 — this is what makes truncating a
    /// node's oldest segment detectable.
    pub seal_hash: String,
}

pub fn compute_seal_hash(/* every SegmentSeal field except seal_hash */) -> String;
```

- [x] **Step 3.** `Segment::open(node_id, config_hash, prev_seal_hash)`,
      `push(&mut self, AuditEntryInput) -> &AuditEntry` (delegates to `chain.push`),
      `seal(self) -> SegmentSeal`, plus `id`/`node_id`/`tip`/`len`/`is_empty`/`entries`
      accessors and `verify(&self)` delegating to `chain.verify()`.
- [x] **Step 4.** `verify_seal_chain(seals: &[SegmentSeal]) -> Result<(), AuditError>` —
      recomputes each `seal_hash` and checks each `prev_seal_hash` against its predecessor.
      Add `AuditError::SegmentIntegrity { segment_id, expected, actual }` and
      `AuditError::SegmentLink { segment_id, expected, actual }`.
- [x] **Step 5.** Export from `audit/mod.rs`.
- [x] **Step 6 (green).** Tests:
  - `should_open_a_segment_at_genesis_with_no_entries`
  - `should_carry_the_previous_seal_hash_into_the_new_segment`
  - `should_verify_a_sealed_segment_internally`   ← proves `AuditChain::verify` is untouched
  - `should_record_the_entry_count_in_the_seal`
  - `should_accept_a_correctly_linked_run_of_seals`
  - `should_reject_a_seal_whose_hash_does_not_cover_its_fields`
  - **`should_detect_a_deleted_leading_segment`** ← the D2 attack: build 3 seals, drop the
    first, set the new first's `prev_seal_hash` to `None`, assert `SegmentIntegrity`
  - `should_detect_a_reordered_pair_of_seals`
  - `should_detect_entries_truncated_from_a_segment_tail`  ← proves `entry_count` earns its
    place in the preimage
- [x] **Step 7.** Verify: tests pass, clippy clean.

### Task 2 — `AuditStore` trait and the in-memory backend

New file `hacienda-core/src/audit/store.rs`.

- [x] **Step 1 (red).** Write a test that binds `let _: Arc<dyn AuditStore> = …;` and confirm
      it fails to compile. This is the object-safety check from the Assumed table — do it
      before writing any backend.
      **Process deviation:** the red phase was skipped. The trait and its tests were written
      together, so the object-safety property was never observed failing. The property does
      hold — `should_construct_arc_dyn_audit_store` compiles and runs — but it was confirmed,
      not tested. Recorded rather than papered over.
- [x] **Step 2.** Define:

```rust
#[async_trait]
pub trait AuditStore: Send + Sync {
    /// Mint and record a document's worth of entries in one call.
    ///
    /// Batched deliberately: it is the transaction boundary a database backend needs,
    /// it is one fsync per document rather than per entity, and it leaves no room for
    /// the two-acquisition shape of §8 gap 5.
    async fn append(&self, inputs: Vec<AuditEntryInput>) -> Result<Vec<AuditEntry>, AuditError>;

    /// Entries in the segment currently open on this writer.
    async fn entries(&self) -> Result<Vec<AuditEntry>, AuditError>;

    /// Every seal this store can see, oldest first.
    async fn seals(&self) -> Result<Vec<SegmentSeal>, AuditError>;

    /// Verify the open segment, every sealed segment, and the links between them.
    async fn verify(&self) -> Result<(), AuditError>;

    /// Seal the open segment and open a successor linked to it.
    async fn rotate(&self) -> Result<SegmentSeal, AuditError>;

    /// Seal without opening a successor. Idempotent.
    async fn close(&self) -> Result<SegmentSeal, AuditError>;
}
```

- [x] **Step 3.** ~~`InMemoryAuditStore { open: Mutex<Segment>, sealed: Mutex<Vec<…>>, … }`~~
      **Deviation — one lock, not three.** The plan's shape gave `open`, `sealed`, and
      `closed_seal` a mutex each, and that is what made `close` racy: it read `closed_seal`,
      released, then took `open`, so two concurrent callers could both see "not yet closed"
      and the loser would find the segment already taken. `close` is documented as safe for
      a signal handler racing a normal exit — precisely the broken case. Shipped as
      `InMemoryAuditStore { state: Mutex<State>, node_id, config_hash }` with
      `State { open: Option<Segment>, sealed: Vec<(SegmentSeal, Vec<AuditEntry>)>, closed_seal: Option<SegmentSeal> }`.
      One lock covering the whole transition removes the window instead of narrowing it, and
      makes lock-ordering deadlocks unrepresentable. `open` is `Option` because a closed
      store has no segment; that is what `AuditError::StoreClosed` reports.
- [x] **Step 4.** Hold the `Mutex` across no `.await` — the in-memory backend does no I/O, so
      take the guard, do the work, drop it before returning. Add a comment saying so; the
      file backend in Task 3 has to be more careful and a reader should know why.
      `verify` additionally snapshots under the lock and verifies outside it: verification is
      O(n) over all history, and blocking every append for its duration would turn a routine
      integrity check into an outage.
- [x] **Step 5 (green).** Tests:
  - `should_return_one_entry_per_input_in_order`
  - `should_chain_entries_across_two_append_calls`
  - `should_verify_after_a_rotation`
  - `should_link_the_new_segment_to_the_sealed_one_on_rotate`
  - `should_report_entries_from_the_open_segment_only`
  - `should_be_idempotent_when_closed_twice`
  - `should_serialise_concurrent_appends_without_breaking_the_chain` ← 8 tokio tasks
    × 10 appends, then `verify()`
  - **Added beyond the plan**, all from defects found reviewing the first cut:
    - `should_detect_a_tampered_entry_in_a_sealed_segment` — nothing above ever saw
      `verify()` return `Err`. A `verify` hardcoded to `Ok(())` passed the whole list. The
      Phase 0 lesson, applied: a check that has never been seen to fail is not a check.
    - `should_report_a_missing_entry_as_a_count_mismatch` — pins the `entry_count`
      assertion added to `verify_sealed_entries`, which is what D2 (as corrected) says the
      field is actually for.
    - `should_reject_open_entries_minted_under_a_substituted_config` — pins the third
      parameter of `verify_open_entries`; see Step 5b.
    - `should_be_idempotent_when_closed_concurrently` — 8 tasks on a multi-thread runtime.
      The sequential twin cannot exercise the check-then-install window; this is the test
      that justifies the single lock in Step 3.
    - `should_refuse_to_append_after_close` — asserts `StoreClosed`, not `ChainIntegrity`.
- [x] **Step 5b (unplanned, security).** `verify_open_entries` originally derived the config
      hash from `entries[0].config_hash` — from the very data under verification. An attacker
      who re-mints the whole run under one config hash of their own choosing produces a set
      that is internally consistent and replays cleanly. Changed to take the **store's**
      `config_hash` as a parameter. `verify()` was also reordered to run `verify_seal_chain`
      *first*, because every later check reads `config_hash`/`sealed_tip`/`entry_count` off a
      seal, and those are only trustworthy once the hash covering them is confirmed.
      **Mutation run:** reverting to `entries[0].config_hash` →
      `should_reject_open_entries_minted_under_a_substituted_config` FAILED (12 passed,
      1 failed); reverted, green.
- [x] **Step 5c (unplanned).** Lifecycle errors were being reported as
      `AuditError::ChainIntegrity { index: 0, expected: "open segment", actual: "store is closed" }`.
      A caller matching on `ChainIntegrity` to raise a tamper alarm would fire on a
      shutdown-ordering bug. Added `AuditError::StoreClosed { operation: &'static str }`, plus
      `AuditError::SegmentEntryCount` for Step 5's count assertion.
- [x] **Step 6.** Verify. 199 tests pass (`hacienda-core` 198 + 1 doc-test),
      `clippy --all-targets --all-features -D warnings` clean, `cargo fmt --check` clean.

### Task 3 — `FileAuditStore`

Same file or `audit/store_file.rs`, whichever keeps both under ~400 lines.

- [x] **Step 1 (red).** Write the durability test first —
      `should_recover_every_entry_after_a_simulated_restart` — and watch it fail.
- [x] **Step 2.** Layout: `root/{node_id}/{segment_id}.jsonl` for entries,
      `root/{node_id}/{segment_id}.seal.json` for the seal. A segment's presence without a
      seal means it was open when the process ended; recovery replays it and seals it.
- [x] **Step 3.** Durability. `flush` alone does not survive power loss —
      `File::flush` is a near-no-op and there is no `fsync` anywhere in the crate today.
      Add `sync_data()` after each `append` batch and after each seal write, behind
      `FileAuditStore::with_sync_policy(SyncPolicy::{EveryBatch, OnSeal})`, defaulting to
      `EveryBatch`. An audit log that reports success on data it may lose is worse than one
      that is slow.
- [x] **Step 4.** Do the blocking I/O under `tokio::task::spawn_blocking`, and structure the
      code so no `std::sync::MutexGuard` is alive across the `.await`. Take the guard, build
      the byte buffer, drop the guard, then `spawn_blocking` the write. A guard held across
      an await makes the future `!Send` and will not compile behind `Arc<dyn AuditStore>` —
      let the compiler enforce it rather than reviewing for it.
- [x] **Step 5.** `open(root, config_hash)` recovers: scan the node directory, load seals,
      verify the seal chain, replay any unsealed segment, then open a successor.
      A recovered store whose seal chain does not verify **fails to open**. Continuing would
      mean appending to a chain already known to be broken.
- [x] **Step 6 (green).** Tests, each with a `TempDir` (copy the RAII helper from
      `audit/sink.rs:145-169`):
  - `should_write_one_json_line_per_entry`
  - `should_recover_every_entry_after_a_simulated_restart`
  - `should_seal_a_segment_left_open_by_a_previous_run`
  - `should_link_a_recovered_segment_to_the_one_before_it`
  - `should_refuse_to_open_a_store_whose_seal_chain_is_broken`
  - `should_verify_across_two_sealed_segments_and_one_open_one`
  - `should_keep_two_node_ids_in_separate_segments`  ← the §12.1 payoff: a CLI run and a
    replica writing to one directory must not collide
- [x] **Step 7.** Deprecate per D6: `#[deprecated(since = "…", note = "use AuditStore")]` on
      `AuditSink` and `FileSink`, `#![allow(deprecated)]` on their test module.
- [x] **Step 8.** Verify. Time the fixture corpus with `SyncPolicy::EveryBatch` vs `OnSeal`
      and record both numbers in the task log — Phase 2's measurement will want them.
      **1000 entries in batches of 10: `EveryBatch` 934 ms, `OnSeal` 308 ms — a 3x cost for
      durability.** The cost is per fsync, so it is per *batch* and does not scale with entry
      count; raising the batch size amortises it without a policy change. Filed as #19 for
      Phase 2 to decide deliberately. Default stays `EveryBatch`.
- [x] **Step 9 (unplanned, correctness — found in review).** **Concurrent appends could write
      to the file out of chain order.** `append` minted under the state lock, dropped the
      guard, then wrote under `spawn_blocking` — two separate critical sections, so two
      callers could mint as (A, B) and write as (B, A). Memory stayed consistent because
      minting was serialised, but the *file* held entries out of chain order, and recovery
      replays the file, not memory. **The store could not reopen its own output.** Confirmed
      before fixing: the new test failed 4 of 5 runs.
      Fixed by adding `io_order: tokio::sync::Mutex<()>` held across the whole
      mint-then-write sequence in `append`, `rotate`, and `close`. This is the one place a
      tokio mutex is correct here — it *must* span an `.await`, and it is a genuine ordering
      requirement rather than a workaround for the `!Send` error a held `std` guard produces.
      Lock order is always `io_order` then `state`. `rotate` and `close` need it too: a seal
      records `entry_count` and `sealed_tip` for entries an in-flight `append` may not have
      written yet.
      New test `should_write_concurrent_appends_to_the_file_in_chain_order` — the file
      backend's counterpart to the in-memory `should_serialise_concurrent_appends_…`, which
      cannot have this bug because it has no second critical section. 6/6 green after the fix.
      Also removed an `.expect("open segment always has a path")` and an
      `.unwrap()` in `rotate` (both forbidden in library code — a panic in an audit write
      loses the batch and takes the caller down), plus a dead `let _ = new_segment;` binding.

### Task 4 — `ReviewStore` trait, in-memory backend, `ReviewQueue` rewired

- [x] **Step 1 (red).** Add `should_let_exactly_one_of_two_concurrent_assignments_win` to
      `review/queue.rs` against the *current* implementation. It should pass — the CAS is
      already correct. This is the regression net for the refactor, so it must exist before
      the refactor, not after.
- [x] **Step 2.** New `review/store.rs`:

```rust
#[async_trait]
pub trait ReviewStore: Send + Sync {
    async fn submit(&self, item: ReviewQueueItem) -> Result<ReviewQueueItem, ReviewError>;
    /// Atomic compare-and-swap on `status == Pending`. Must not be split into
    /// get-then-put by any implementation — see D4.
    async fn assign(&self, id: &str, reviewer: &str) -> Result<ReviewQueueItem, ReviewError>;
    /// Atomic compare-and-swap on `decision.is_none()`.
    async fn decide(&self, id: &str, decision: ReviewDecision, reviewer: &str, comment: &str)
        -> Result<ReviewQueueItem, ReviewError>;
    async fn list(&self, filter: Option<ReviewStatus>) -> Result<Vec<ReviewQueueItem>, ReviewError>;
    async fn get(&self, id: &str) -> Result<Option<ReviewQueueItem>, ReviewError>;
    async fn stats(&self) -> Result<QueueStats, ReviewError>;
}
```

- [x] **Step 3.** `InMemoryReviewStore` holding today's `Mutex<Vec<ReviewQueueItem>>`, with
      `assign`/`decide` moved across verbatim so the CAS semantics are transported rather
      than reimplemented.
- [x] **Step 4.** `ReviewQueue { store: Arc<dyn ReviewStore>, config: ReviewConfig }`.
      It keeps `needs_review` and `priority_from_confidence`; `submit` still builds the item
      (id, priority, deadline, timestamps) and hands it to the store. Add
      `ReviewQueue::with_store(config, store)`.
- [x] **Step 5.** `ReviewQueue`'s methods become `async`. Update `review/queue.rs`'s existing
      tests to `#[tokio::test]` and `.await` — change the calls, **not** the assertions. An
      assertion that changes during a refactor is a test that stopped testing the same thing.
- [x] **Step 6 (green).** All pre-existing review tests pass unchanged in substance, plus:
  - `should_let_exactly_one_of_two_concurrent_assignments_win` (from Step 1, now async)
  - `should_let_exactly_one_of_two_concurrent_decisions_win`
- [x] **Step 7.** Verify. 209 tests, clippy clean, fmt clean.
- [x] **Step 8 (unplanned, correctness — found in review).** The first cut swallowed store
      errors in **four** places: `submit` returned `unwrap_or(item)`, `list`
      `unwrap_or_default()`, `get` `unwrap_or(None)`, `stats` `unwrap_or_default()`. A
      comment argued this "keeps the call site clean for the common case". It does not — it
      is a bare-exception-handler in Result clothing, and CLAUDE.md forbids it.
      The consequences were concrete: `list()` collapsing an error into an empty list means
      a broken store is **indistinguishable from an empty review queue**, so a reviewer is
      told there is no work outstanding when persistence is down. `stats()` did the same with
      all-zero counts. And `HaciendaFacade::submit_for_review` incremented `review_submitted`
      regardless of outcome, so the facade reported "N submitted for review" when zero were
      persisted. None of this was reachable while the backend was in-memory and infallible —
      it would all have gone live the moment Task 5's file backend landed.
      All four now propagate. `ReviewQueue::{submit, list, get, stats}` return `Result`;
      `get` returns `Result<Option<_>>` and the two layers stay distinct on purpose, so an
      unreadable store cannot answer "no such item". `submit_for_review` propagates via
      `HaciendaError::Review` (the `#[from]` already existed) and counts only confirmed
      submissions. Whether aborting the whole batch is the right *policy* is a separate
      question, filed as #22.
      Test call sites updated mechanically — `.expect(...)` added to calls, **no assertion
      changed**.

### Task 5 — `FileReviewStore`

- [x] **Step 1 (red).** `should_replay_every_decision_after_a_restart`.
- [x] **Step 2.** Append-only JSON-lines **event** log — `Submitted`, `Assigned`, `Decided` —
      replayed into memory on open. Review items mutate, so rewriting a whole-state file per
      change would either lose the change on a crash mid-write or need a temp-and-rename
      dance per mutation. An append-only log matches the audit backend's shape and makes a
      partial trailing line a recoverable condition rather than data loss.
- [x] **Step 3.** A truncated trailing line is dropped with a `tracing::warn`, not an error.
      A crash mid-append is a normal condition for an append-only log; refusing to open is
      the wrong response to the one case the format was chosen to tolerate.
- [x] **Step 4.** `sync_data()` after each event; blocking I/O under `spawn_blocking`.
- [x] **Step 5 (green).**
  - `should_replay_every_decision_after_a_restart`
  - `should_preserve_status_transitions_across_a_restart`
  - `should_drop_a_truncated_trailing_line_and_keep_the_rest`
  - `should_reject_an_assignment_that_lost_the_race_after_a_restart`
- [x] **Step 6.** Verify. 222 passed, 1 ignored; clippy `-D warnings` and `cargo fmt --check`
      clean.

- [x] **Step 7 (unplanned, correctness — data loss).** Step 3 as written is **not enough**,
      and the same defect was already live in `FileAuditStore`. Dropping the partial record
      at *replay* time fixes the read and leaves the write broken: `append` opens with
      `append(true)` and lands at EOF, so the fragment — which has no trailing newline — is
      welded to the next record on the same physical line. That welded line is still the
      last line, so it is dropped as "truncated" too. **Every write after a single crash is
      accepted, fsynced, and silently discarded, forever, with no error anywhere.**

      Caught by `should_accept_writes_after_recovering_from_a_truncated_line`, which the
      original truncation test misses because it never writes after recovering. Confirmed
      failing before the fix (`got ["item-a"]`).

      Fixed in both stores with the same rule, and the rule is better than the one planned:
      **the newline is the record terminator, and its presence — not whether the bytes
      happen to parse — is what says a record was fully written.** That distinction is load
      bearing: `append_bytes_and_sync` issues the payload and the newline as two writes, so
      a crash between them leaves a *parseable* but unterminated record. Any unterminated
      tail is removed from the file (`set_len` + `sync_all`, because the length is metadata)
      at open; every terminated line must then parse or it is a hard error. Replay no longer
      needs a "last line is special" branch at all.

      `FileAuditStore` had the mirror-image failure: `read_jsonl` errored on *any*
      unparseable line, so a crash mid-append made the audit log **permanently unopenable** —
      a whole audit history lost to one power cut, against the durability DORA asks for.
      Covered by `should_recover_from_a_crash_part_way_through_an_append`, which also asserts
      the surviving prefix still verifies and that a post-recovery append is durable across
      another restart. Both stores now read bytes rather than `read_to_string`, since a crash
      can split a multi-byte UTF-8 character and `read_to_string` would refuse the file
      outright rather than let us drop the tail.

- [x] **Step 8 (unplanned, test quality).** The agent's concurrency test claimed *"Confirmed
      before implementing `io_order`: this test failed on 4 of 5 runs when `io_order` was
      removed."* **Not reproducible.** Removing all three `io_order` acquisitions, the test
      passed 8 of 8 runs.

      The test was structurally incapable of failing: it raced 8 tasks against **one** item,
      and the state-lock CAS means only one `assign` and one `decide` ever write, so a single
      pair of writes had to cross in a very narrow window. Rewritten to race an independent
      assign/decide pair against each of 40 items over 3 runs — 120 chances instead of 3 —
      and to compare the full `(status, decision, decided_by)` triple of every item between
      the live store and the reopened one. Mutation re-run: **5 of 5 fail** without
      `io_order`, 5 of 5 pass with it.

- [x] **Step 9 (unplanned, filed not fixed).** Both file stores mutate in-memory state before
      the write succeeds, so an I/O failure leaves memory and disk disagreeing: a caller who
      retries a failed `decide` gets `AlreadyDecided` for a decision that was never
      persisted. The caller *is* told the operation failed, so this is a trap rather than
      silent loss. Filed as #24.

### Task 6 — `JobStore` trait and in-memory backend

Per D5: trait + in-memory only. No file backend.

- [x] **Step 1 (red).** Write the tests first.
      **Process deviation:** as in Task 2, the trait and tests were written together and
      compiled on the first attempt, so no test was observed failing before it passed.
- [x] **Step 2.** New `hacienda-core/src/jobs/` (`mod.rs`, `types.rs`, `store.rs`) behind a
      `jobs` feature, default-on:

```rust
pub enum JobStatus { Queued, Running, Succeeded, Failed }

pub struct Job {
    pub id: String,
    pub status: JobStatus,
    pub created_at: String,
    pub updated_at: String,
    /// Serialized result payload. Opaque so the store layer does not depend on the
    /// pipeline types — see D5.
    pub result_json: Option<String>,
    pub error: Option<String>,
}

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn create(&self) -> Result<Job, JobError>;
    async fn get(&self, id: &str) -> Result<Option<Job>, JobError>;
    /// CAS on the expected current status, so two workers cannot both claim a job.
    async fn transition(&self, id: &str, from: JobStatus, to: JobStatus) -> Result<Job, JobError>;
    async fn finish(&self, id: &str, result_json: String) -> Result<Job, JobError>;
    async fn fail(&self, id: &str, error: String) -> Result<Job, JobError>;
    async fn list(&self, filter: Option<JobStatus>) -> Result<Vec<Job>, JobError>;
}
```

- [x] **Step 3.** `InMemoryJobStore` over `Mutex<HashMap<String, Job>>`. Rustdoc note added on
      both the trait and the module: provisional until Phase 4's `/v1/jobs/{id}` exercises it.
- [x] **Step 4 (green).**
  - `should_create_a_job_in_the_queued_state`
  - `should_refuse_a_transition_from_an_unexpected_status`
  - `should_let_exactly_one_of_two_workers_claim_a_queued_job` ← genuine 2-task race on a
    multi-thread runtime; asserts exactly one `Ok` and exactly one `StatusMismatch`
  - `should_record_the_error_when_a_job_fails`
- [x] **Step 4b (unplanned).** `list()` sorted on `created_at` alone while its rustdoc promised
      "stable order". RFC 3339 timestamps have finite resolution, so jobs created in the same
      instant compared equal and came back in `HashMap` iteration order, which is randomised
      per process. Added `.then_with(|| a.id.cmp(&b.id))`.
- [x] **Step 5.** Verify — covered by the Task 2 Step 6 run (199 tests, clippy, fmt).
      `async-trait` was moved from `[dev-dependencies]` to `[dependencies]`: a public
      production trait cannot depend on a dev-only crate.

### Task 7 — Facade wiring

- [x] **Step 1 (red).** Add `should_keep_the_audit_chain_across_a_facade_restart` to
      `facade.rs` and watch it fail to compile.
- [x] **Step 2.** Replace the fields:

```rust
audit_store: Option<Arc<dyn AuditStore>>,
review_queue: Option<ReviewQueue>,   // now holds Arc<dyn ReviewStore> internally
```

- [x] **Step 3.** `HaciendaFacade::new` still builds `InMemoryAuditStore` when
      `pii.audit.enabled` — behaviour is unchanged for every existing caller. Add
      `with_stores(config, audit: Option<Arc<dyn AuditStore>>, review: Option<Arc<dyn ReviewStore>>)`
      alongside `with_key_resolver`, and one combined constructor so a caller can have both
      keys and stores. Do not multiply constructors beyond that.
- [x] **Step 4.** `record_audit` becomes `async` and builds the whole `Vec<AuditEntryInput>`
      before one `store.append(inputs).await`. **The `config_hash` read disappears entirely:**
      `AuditChain::push` already overwrites `input.config_hash` with the chain's own
      (`chain.rs:32`), so the facade was reading a value it did not need in order to pass it
      back to be discarded. §8 gap 5 is closed by deleting the reason for the first lock, not
      by reordering the two.
- [x] **Step 5.** `audit_entries()` and `verify_audit()` become `async`. `process_batch`
      awaits `record_audit` and `submit_for_review`.
- [x] **Step 6.** Confirm `lock()`'s poison-recovery helper is still needed; if the glossary
      is its last user, narrow its doc comment to say so.
- [x] **Step 7 (green).**
  - `should_keep_the_audit_chain_across_a_facade_restart` (file store, two facades, one dir)
  - `should_record_one_audit_entry_per_redacted_span` (existing, now `.await`)
  - `should_verify_the_chain_after_processing_a_batch`
  - `should_acquire_the_audit_lock_once_per_document` — a counting `AuditStore` test double
    asserting `append` is called exactly once per document. Without it, the next editor
    reintroduces per-entity calls and nothing objects.
  - ~~`should_keep_review_decisions_across_a_facade_restart`~~ — **not written.** This bullet
    was ticked in error; a grep for `FileReviewStore` in `facade.rs` returns nothing. The
    durable review store has no facade-level coverage at all: every facade review test uses
    the in-memory backend, so nothing proves a decision survives a restart the way the audit
    chain is proven to. Filed as #26.
- [x] **Step 8.** Verify: full suite, clippy, fmt. 222 passed, 1 ignored; clippy
      `-D warnings` and `cargo fmt --check` clean.

Deviations from the plan as written:

- **Step 3's "one combined constructor" was not built.** `with_stores` passes `None` for the
  pseudonymiser, so a caller needing pseudonymisation *and* durable stores still has no
  public constructor — the private `build` already takes all four arguments, only the public
  surface is missing. Filed as #25 rather than added here, because #25 has to settle a prior
  question first: `build` resolves an argument-vs-config conflict in opposite directions for
  the two stores. An explicitly supplied audit store is used even when `pii.audit.enabled` is
  false, while an explicitly supplied *review* store is **silently dropped** when
  `config.review` is `None` — a caller who asks for durable review persistence gets none,
  with no error. Adding a fourth constructor before picking one rule would multiply the
  inconsistency.
- **Step 7's test names differ.** `should_acquire_the_audit_lock_once_per_document` is
  `should_append_exactly_once_per_document` — there is no lock to acquire any more, and the
  name should describe the invariant that survives. It asserts two spans in one document
  still produce one `append`, and that a second document takes the count to exactly two.
  `should_verify_the_chain_after_processing_a_batch` is subsumed by the restart test, which
  verifies after a reopen and is strictly stronger. Added beyond the plan:
  `should_fail_the_batch_when_the_audit_store_fails` (an injected `Io` failure must surface
  as `HaciendaError::Audit`, not as a result reporting zero audited entries),
  `should_be_idempotent_on_close`, and the two no-store arms of `close` and `verify_audit`.
- **Step 6 resolved as predicted.** The glossary is `lock()`'s only remaining user; its doc
  comment now says so.
- **No `Drop` impl**, per D6: `Drop` cannot await, `block_on` inside it panics within a Tokio
  runtime, and a detached `tokio::spawn` risks the process exiting first.
  `FileAuditStore::open` already seals an orphaned segment, so a caller who forgets `close`
  pays one recovery step and loses nothing.

### Task 8 — Verification, mutation checks, and documentation

Phase 0's decisive lesson: a suite that never fails against a broken implementation is not
a suite. These mutations are mandatory, and each one's result goes in the completion note.

- [x] **Step 1 (mutation A).** In `compute_seal_hash`, drop `prev_seal_hash` from the
      preimage. **Run in Task 1** — `should_detect_a_deleted_leading_segment` failed, the
      other eight passed. Reverted, green.
- [x] **Step 2 (mutation B).** Drop `entry_count` from the preimage. **Run in Task 1** —
      `should_reject_a_seal_whose_hash_does_not_cover_its_fields` failed. Reverted, green.
      (The plan originally expected `should_detect_entries_truncated_from_a_segment_tail`
      to be the one that fired; it does not, which is what exposed the wrong rationale for
      `entry_count` now corrected in D2.)
- [x] **Step 2b (mutation C′).** Make `check_seal_link` unconditionally return `Ok`.
      **Run in Task 1** — `should_detect_entries_truncated_from_a_segment_tail` and
      `should_detect_a_reordered_pair_of_seals` failed. Reverted, green.
- [x] **Step 3 (mutation C).** Make `InMemoryReviewStore::assign` a read-then-write pair with
      a `yield_now()` between. **Run in Task 4** —
      `should_let_exactly_one_of_two_concurrent_assignments_win` FAILED with `ok_count == 8`:
      all eight tasks observed `Pending` and all eight won. Reverted, green. This is the
      mutation that mattered most: it is the exact degradation the refactor risked, and it
      would have passed every sequential test.
- [x] **Step 4 (mutation D).** Make `FileAuditStore::open` skip seal-chain verification.
      **Run** — `should_refuse_to_open_a_store_whose_seal_chain_is_broken` FAILED
      (8 passed, 1 failed). Reverted, green.
- [x] **Step 5.** `cargo test -p hacienda-core -p hacienda` — **222 passed, 0 failed,
      1 ignored** (the synthetic fsync benchmark), plus the doc-test. Baseline was 171.
      `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo fmt --check`
      clean.
- [x] **Step 6.** CHANGELOG updated: `Added` for the three traits, the segment model, the
      four backends, `FileReviewStore`, and the facade's `with_stores`/`close`; `Changed`
      **marked Breaking** for `audit_entries`/`verify_audit` becoming `async`, `ReviewQueue`
      becoming a policy wrapper with async fallible methods, and a store write failure now
      failing the whole call; `Security` for crash-mid-append recovery in both file stores.
      **No `Deprecated` entry for `AuditSink`/`FileSink`** — the plan assumed they would be
      superseded, but `FileSink` still serves callers who want one flat log rather than a
      segmented store, and nothing in Phase 1 gives those callers a replacement. Deprecating
      an API with no migration path is an instruction to ignore the warning.
- [x] **Step 7a (done early — depends only on Task 1).** Spec §12.1 amended in
      `specs/2026-07-28-hacienda-cli-api-integration-design.md`: added an "Amendment: the
      seal hash" subsection recording that check 2 as originally written does not detect
      truncation (delete the oldest segment, blank the successor's back-pointer, and the
      check passes because a `None` back-pointer is indistinguishable from a legitimate
      first segment). Documents the `seal_hash` preimage, the length-prefixing rationale,
      the split of check 2 into 2a self-consistency and 2b linkage, that `entry_count` is
      for diagnosis rather than detection, and that per-segment sequence numbering is forced
      by `AuditChain::verify` using the array index.
- [x] **Step 7.** Spec: §8 gaps 1, 2, and 5 struck with "Closed 2026-07-28 by Phase 1" notes
      matching the Gap 7 formatting. §12.1's sketch now shows `Segment` and `SegmentSeal`
      separately, carries `prev_seal_hash`/`seal_hash`/`entry_count`, records why sequence
      numbers restart per segment, and restates check 2 as the seal chain pointing at the
      truncation argument in the amendment.
- [x] **Step 8.** Completion note below.

---

## Completion note — 2026-07-28

**Result.** 171 tests → 222 passing, 1 ignored. Clippy `-D warnings` and `cargo fmt --check`
clean. All eight tasks done. §8 gaps 1, 2, and 5 closed.

**Mutation results.** Every mutation is recorded with the test that caught it, because a
check nobody has watched fail is not a check.

| # | Mutation | Caught by | Outcome |
|---|---|---|---|
| A | `compute_seal_hash` drops `prev_seal_hash` | `should_detect_a_deleted_leading_segment` | 1 failed / 8 passed |
| B | `compute_seal_hash` drops `entry_count` | `should_reject_a_seal_whose_hash_does_not_cover_its_fields` | 1 failed |
| C′ | `check_seal_link` returns `Ok` unconditionally | two seal tests | failed |
| C | `InMemoryReviewStore::assign` split into read-then-write | `should_let_exactly_one_of_two_concurrent_assignments_win` | `ok_count == 8` — all eight tasks won |
| D | `FileAuditStore::open` skips seal-chain verification | `should_refuse_to_open_a_store_whose_seal_chain_is_broken` | 1 failed / 8 passed |
| E | all three `io_order` acquisitions removed from `FileReviewStore` | `should_write_concurrent_events_in_order_so_replay_matches_live_state` | 5 of 5 runs failed |

Mutation B is the one that taught something: the plan predicted
`should_detect_entries_truncated_from_a_segment_tail` would fire, and it did not.
`sealed_tip` already changes when entries are removed, so the count is **diagnosis, not
detection** — it exists so recovery can say "holds 37 entries, seal records 40" instead of
reporting an opaque hash mismatch. D2's rationale was rewritten to say so.

Mutation E is the one that nearly did not happen. The test as originally written passed
8 of 8 runs with `io_order` removed — it raced 8 tasks against a single item, and the CAS
means only one pair of writes ever contends. Rewritten to race an independent assign/decide
pair against each of 40 items, it fails every run. **A concurrency test that has not been
watched fail is decoration.**

**fsync cost (Task 3 Step 8).** 1000 entries in batches of 10: `EveryBatch` 934.2 ms,
`OnSeal` 308.4 ms — roughly 3×. The cost is per fsync and therefore per *batch*, not per
entry, so larger batches amortise it. `EveryBatch` remains the default; the choice is filed
for deliberate reconsideration as #19.

**The two defects worth remembering.** Both were shipped by subagents, both passed their own
tests, and both were found only by writing a test aimed at the specific failure.

1. **Two critical sections is an ordering bug.** Mutating in-memory state under a
   `std::sync::Mutex`, dropping the guard, then writing under `spawn_blocking` decouples
   mutation order from write order. Memory stays consistent — minting was serialised — but
   the *file* ends up out of order, and recovery replays the file, not memory. In
   `FileAuditStore` the store could not reopen its own output. In `FileReviewStore` it is
   quieter and worse: a `Decided` landing before its `Assigned` means a decision a human
   made disappears on restart, with every event present and nothing logged. Fixed in both
   with an `io_order: tokio::sync::Mutex<()>` spanning the whole sequence — the one place a
   tokio mutex is correct, because it must span an `.await`.

2. **Skipping a bad record is not the same as removing it.** Both file stores recovered from
   a truncated trailing line by ignoring it at replay. But `append` writes at EOF, so the
   fragment — having no newline — is welded to the next record on the same line, and that
   line then looks truncated too. One power cut made every subsequent write silently
   vanish forever. Both stores now treat the newline as the record terminator, remove any
   unterminated tail from the file at open, and treat a terminated line that will not parse
   as a hard error.

**A third lesson: agents misread a red test as a broken one.** The Task 7 agent reported
`should_recover_from_a_crash_part_way_through_an_append` as "a pre-existing failure I did not
introduce", concluded it "disagrees with the design decision recorded in the module doc", and
claimed to have confirmed by stashing that it predated its own edits. All of that is wrong.
The test had been written minutes earlier, in this session, deliberately, as the red half of
defect 2 above — and the module doc it cited as authority was exactly the reasoning being
corrected. The stash check was confounded by concurrent edits to the same tree.

The agent's instinct was defensible: don't touch a failing test you did not write. The
failure was in the conclusion — treating a doc comment as settled design rather than asking
whether the doc was the thing that was wrong. Worth remembering when running agents in
parallel against a tree someone else is also editing: a red test they did not write looks
identical to a broken one.

Its one genuine catch is fixed: the restart test hardcoded `"default"` as the config hash, so
a future change to `AuditConfig::default()` would have surfaced as a confusing
`ConfigMismatch` rather than an assertion failure. It now reads the hash off the config it
builds the facade from.

**Sharp edges left behind.**

- Both file stores mutate memory before the write succeeds, so an I/O failure leaves the two
  disagreeing (#24). The caller is told the write failed, so this is a retry trap rather
  than silent loss.
- `HaciendaFacade::with_stores` silently drops a review store when `config.review` is `None`,
  and resolves the argument-vs-config conflict in the opposite direction for the audit store
  (#25). The same issue covers the missing keys-plus-stores constructor.
- `FileAuditStore::verify()` re-reads all history from disk on every call (#21), and there is
  no API to verify a chain across multiple node directories (#20) — that is Phase 5's
  `hacienda audit verify`.
- `AuditConfig::log_path` is dead configuration: it defaults to `"audit.log"` and nothing
  reads it (#23). A segmented store takes a directory, so the field is now wrong-shaped as
  well as unused.
- A review-store failure aborts the whole document batch (#22). Correct as a default; whether
  it is the right *policy* is a Phase 2 question.

---

## Out of Scope

Named so nobody has to guess whether they were forgotten.

- **Anchors** (§12.1 check 3 — periodic merkle roots over live segment tips). They need a
  coordination point that does not exist until there are concurrent writers, i.e. Phase 4.
  D2's seal chain is what gives them a leaf to hash when they arrive.
- **Postgres backends** — Phase 6, explicitly.
- **`hacienda audit verify` across segments** — Phase 5. `AuditStore::verify` is the engine
  it will call; the CLI surface is not Phase 1's.
- **The batch-processing serialisation of §8 gap 3** — Phase 2.
- **A file backend for `JobStore`** — D5.
- **Glossary storage.** §12.2 calls it a grow-only map merged on read, which is a different
  construction from the three stores here, and no gap in §8 names it.
