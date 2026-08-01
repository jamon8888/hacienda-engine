# Vertical Model Specialisation, Steps 1–3 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make hacienda able to (a) *measure* NER quality per label on a labelled fr/en set, (b) ship **Tier 0 schema verticals** — a vertical is a label set, not a model — and (c) record which vertical produced a detection in the tamper-evident audit chain.

**Architecture:** Everything here is inside `hacienda-core` plus a CLI flag. No new crate, no upstream `xberg` change, no trained weights, no model reload. The whole Tier 0 mechanism is *arguments to an inference call*: `PipelineConfig.vertical.labels` → `Vec<EntityCategory::Custom(..)>` → `NerDetector::with_categories` → `CandleBackend::detect` → `category_to_label`'s `Custom(s) => s.as_str()` arm → GLiNER2 zero-shot.

**Tech Stack:** Rust 2024, existing workspace deps only (`serde`, `toml`, `blake3`, `clap` 4 in `hacienda-cli`). The evaluation harness is an `#[ignore]`d integration test, so `candle` stays behind the non-default `ner-candle` feature and out of CI's default path.

**Spec:** `superpowers/specs/2026-07-31-vertical-model-specialisation-design.md` §4.1, §7, §8 steps 1–3. Supersedes `docs/superpowers/plans/2026-07-28-gliner2-lora-hotswap-xberg-native.md`, which is rejected.

**Baseline:** recorded in Task 0 on 2026-07-31 against commit `1c9ae9c78de1f9464747b6726d6891c9bb9b5eb4` (rustc/cargo 1.97.1). `cargo clippy --all-targets --all-features -- -D warnings` **is** clean; `cargo fmt --check` is **not** clean (2 pre-existing files need reformatting, unrelated to this plan). Full detail below.

---

## Baseline (Task 0 results, recorded 2026-07-31)

Environment: commit `1c9ae9c78de1f9464747b6726d6891c9bb9b5eb4`, rustc/cargo 1.97.1, worktree at
`.claude/worktrees/vertical-model-specialisation`. Host has ~3.7 GB RAM + ~6.7 GB swap and is shared with other
concurrent processes (browser, other agent sessions) — see the "thrashing" note under item 4.

1. **`cargo test --workspace`** — **343 passed, 0 failed, 2 ignored**, 0 measured, across all workspace crates
   (hacienda, hacienda-api, hacienda-cli, hacienda-core, hacienda-wasm, incl. doctests). Wall time ~18m47s on
   this host from a cold-ish incremental cache. No failures, no `FAILED` lines.
2. **`cargo clippy --all-targets --all-features -- -D warnings`** — **clean**, exit 0, ~9m42s wall. (Note: the
   flag order matters — `cargo clippy --all-targets --all-features -D warnings` without the `--` separator is
   rejected by cargo itself with "unexpected argument '-D' found"; the correct invocation places `-D warnings`
   after `--`.) The only line printed was cargo's own `warning: ignoring 'resolver.feature-unification' without
   -Zfeature-unification` — a resolver notice, not a clippy lint.
   **`cargo fmt --check`** — **not clean**, exit 1, 2 files with diffs: `hacienda-cli/src/commands.rs` (around
   `build_vault_readme`, line 598) and `hacienda-cli/tests/extract.rs` (around line 195). Both are pre-existing
   long-line-wrapping drift unrelated to anything this plan touches. Not fixed as part of Task 0 (out of scope);
   flagged here so Task 2/3 diffs are not blamed for it.
3. **`grep -rn "^\[pii" --include=*.toml . ../../../../hacienda-private`** — **zero matches** in either repo.
   (The correct relative path from this worktree to the sibling `hacienda-private` repo is
   `../../../../hacienda-private` — four levels up, since this worktree lives at
   `hacienda-engine/.claude/worktrees/vertical-model-specialisation/`, not three levels as a naive guess would
   suggest. Verified with `realpath` before running.) No config file anywhere in either repo declares a `[pii]`
   section today, so a new `[pii.vertical]` key introduces no naming collision.
4. **`cargo check -p hacienda-core --features ner-candle`** — **succeeded**, exit 0, wall time **4m50s** (well
   under the 10-minute timeout). Pulled in and compiled `candle-core`, `candle-nn`, `candle-transformers`,
   `tokenizers`, `xberg-gliner`, and `xberg` itself. Memory stayed bounded throughout — `free -h` samples during
   the build showed available memory dip as low as ~180 MB with up to ~2 GB of swap in use, and the single
   largest observed process (`candle_transformers` / `xberg` rustc invocations) used ~19–21% RSS of total RAM —
   but the build made steady forward progress the whole time and did not stall or thrash. **No different plan is
   needed for model-touching tasks on this host**, though Tasks 1/2 should still expect multi-minute rebuilds
   and avoid running multiple heavy cargo invocations concurrently (a `cargo test` and `cargo clippy` run
   overlapped earlier in this same Task 0 session and serialized on cargo's own build-directory file lock rather
   than actually thrashing).
5. **Pre-change chain-hash literal for Task 3**, captured via a throwaway `#[test]` in
   `hacienda-core/src/audit/entry.rs` (added, run, then removed — the file is confirmed byte-identical to git
   `HEAD` via `git diff` after removal):

   ```
   AuditEntry::new(input("id-1"), "prev", 0).chain_hash
     = "72eb1d2f14c701e0c58280b2d7fc5132fdc0564a3ed42e7b0c4b84cfdd5a3ee4"
   ```

   Task 3's `should_verify_a_chain_written_before_the_vertical_field_existed` test must assert against this exact
   literal.

---

## Ground Truth — Verified vs Assumed

Every row below was read from source on 2026-07-31, on `main`-equivalent state, with `xberg` claims read from the **pinned tag** (`git show v1.0.2:...`) and **not** from the local `../xberg` working tree, which sits at `1.0.0-rc.37` / `492cc21` and is *not* what this workspace builds.

**Verified by reading source:**

| Fact | Location |
|---|---|
| `xberg` is a git **tag** dependency, `tag = "v1.0.2"` (commit `9dcc864`) | root `Cargo.toml` |
| `category_to_label`'s custom arm is `EntityCategory::Custom(s) => s.as_str()` — labels pass through as raw text | `xberg v1.0.2 crates/xberg/src/text/ner/candle.rs` |
| `impl NerBackend for CandleBackend` implements **only** `detect`; it does not override `detect_with_custom` | same |
| `CandleBackend::detect` passes its own `const DEFAULT_THRESHOLD: f32 = 0.5` to `extract_ner`, ignoring any caller threshold | same |
| Backends are cached process-wide by `(PathBuf, Option<PathBuf>)` via `get_or_init` | same, `:111` |
| `NerDetector { backend: Arc<dyn NerBackend>, categories: Vec<EntityCategory>, threshold: f32 }` | `hacienda-core/src/pii/ner.rs:23-27` |
| `DEFAULT_CATEGORIES` = Person, Organization, Location, Email, Phone | `ner.rs:14-20` |
| `with_categories(Vec<EntityCategory>)` exists and is public; its **only** callers today are two tests | `ner.rs:40`, `pipeline.rs:448,493` |
| `NerDetector::detect` filters by `self.threshold`, keeping entities with no confidence | `ner.rs:102-105` |
| `to_pii_category` already maps `EntityCategory::Custom(label)` through an alias table (`ssn`, `iban`, `credit_card`, `passport`, `address`, `full_name`, …) and falls back to `PiiCategory::Custom(label)` | `ner.rs:136-145` |
| `load_detector` builds the detector from `config.model.model_dir` + `lora_adapter_dir` and applies `model_threshold_default` | `pipeline.rs:229-238` |
| `PiiPipeline::assemble` calls `validate_ner_labels_for_pseudonymize(detector)` when mode is `Pseudonymize` | `pipeline.rs:131` |
| That validation iterates `detector.configured_categories()` and runs each through `to_pii_category` → `category_label` | `pipeline.rs:259-271` |
| `category_label` rejects an empty rendered label or one containing `[`, `:`, `]` | `redaction/pseudonym.rs:392-405` |
| **Therefore vertical labels are already validated for pseudonymisation for free**, provided they are placed in `categories` | composition of the two rows above |
| `PipelineConfig` is `#[serde(default, deny_unknown_fields)]`; adding a field is backward-compatible for reading old TOML but **rejects unknown keys**, so config files may not carry a `vertical` key before the field exists | `pii/config.rs:11` |
| `AuditEntry.principal: Option<String>` is the precedent for adding a chain-hashed field; `None` hashes as `""` so pre-existing chains still verify | `audit/entry.rs:78-86`, `:186-188` |
| `ChainHashFields` is a named struct, deliberately so that adding a field is "a deliberate, reviewable act" | `audit/entry.rs:156-172` |
| `config_hash` is an **operator-supplied opaque string** (`AuditConfig::default() == "default"`), not derived from config, and `AuditChain::append` **rejects** any entry whose `config_hash` differs from the chain's | `audit/mod.rs:66-82`, `chain.rs:33,48-52` |
| Consequently the vertical id **cannot** ride on `config_hash` if it may ever vary within one chain — it needs its own field | composition of the two rows above |
| `facade.rs` builds `AuditEntryInput` in two places (reveal path ~`:678`, redaction path ~`:735`), both setting `config_hash: String::new()` because the store owns it | `facade.rs:678-690, 735-750` |
| `fixtures/pii-corpus.json` has 25 cases of shape `{id, text, categories}` and asserts **set equality of categories**, regex-only, mirrored by `apps/hacienda-studio/lib/pii-engine.test.ts` | `fixtures/pii-corpus.json`, `hacienda-core/tests/pii_corpus.rs` |
| There is **no** span-level fixture, no P/R/F1 code, and no `benches/` directory anywhere in the workspace | `ls fixtures/ hacienda-core/tests/`, `grep criterion` |
| `DEFAULT_CATEGORIES` is a **private** `const`, not `pub(crate)` | `ner.rs:14` |
| The non-`ner-candle` / wasm32 `load_detector` returns `PiiError::ModelUnavailable` on its first line, so nothing placed inside it runs on those builds; and `load_detector` is only called when `model.enabled` is true | `pipeline.rs:239-247`, `:59-61` |
| `export_csv` writes a bare literal header and declares **no** format version | `audit/export.rs:41-46` |
| `crates/hacienda-wasm/src/lib.rs:25,34` hardcodes `PipelineConfig::default()` — the browser has no PII config surface at all | `crates/hacienda-wasm/src/lib.rs` |
| On wasm32, `hacienda-core` pulls xberg with `ner-candle-wasm` (a **different** feature from hacienda's own `ner-candle`), so xberg's Candle backend *is* compiled in | `hacienda-core/Cargo.toml:56-64`; xberg `Cargo.toml:417-424` |
| …but `NerDetector::from_candle_local` and the real `load_detector` are both gated `all(feature = "ner-candle", not(target_arch = "wasm32"))`. **On wasm32 `load_detector` always returns `ModelUnavailable`** — hacienda-core cannot load a model in the browser at all, with any feature set | `ner.rs:67`, `pipeline.rs:228,239` |
| `with_categories`'s two call sites are `pipeline.rs:448,493`, both inside `#[cfg(test)]`; `ner.rs:40` is the definition, not a caller | `ner.rs:40`, `pipeline.rs:448,493` |
| The existing CSV test asserts the header with `starts_with("id,timestamp,category")`, so appending a column does **not** break it — the break is invisible to the suite and lands on downstream parsers | `audit/export.rs:141` |
| `hacienda-api/src/dto.rs:199` exposes `PipelineConfig` through an **explicit field allowlist**, not a derived `Serialize` | `hacienda-api/src/dto.rs` |
| Nothing in `hacienda-core` is `#[non_exhaustive]` | `grep -rn non_exhaustive hacienda-core/src` |
| `ner-candle` is a non-default feature (`ner-candle = ["xberg/ner-candle"]`); default is `["jobs"]` | `hacienda-core/Cargo.toml:8-13` |
| candle **has been built** in this workspace's debug profile (12 artifacts in `target/debug/deps`), so the feature compiles here | `ls target/debug/deps` |
| `ExtractArgs` already carries `--mode/--threshold/--model-dir/--lora-dir`, merged by `apply_cli_overrides` | `hacienda-cli/src/cli.rs:85-107`, `config.rs:162-191` |
| An F16 model dir exists locally at `/home/jamin/model_f16/` with `config.json`, `encoder_config/`, `tokenizer.json`, `tokenizer_config.json`, `model.safetensors` | `ls ~/model_f16/` |

**Assumed, to confirm during implementation:**

| Assumption | How to confirm | If false |
|---|---|---|
| GLiNER2 returns usable spans for arbitrary French labels (`numero_de_dossier`, `code_swift`) | Task 1's harness on the fr split | Tier 0 is English-label-only; record it and re-scope |
| Adding ~5–8 labels to the request does not materially change latency | time the harness with 5 vs 13 labels | cap vertical label count in config validation |
| Adding labels does not degrade the *base* five categories (label interference) | harness reports base-category F1 with and without the vertical | verticals must run as a second pass, not an extended set — a significant re-scope |
| `PiiCategory::Custom(label)`'s uppercase rendering is stable enough for pseudonym tokens across runs | `category_label` round-trip test in Task 2 | normalise labels at config-load time |
| No committed `hacienda.toml` in this repo or `hacienda-private` sets an unknown `[pii]` key that a new field would collide with | `grep -rn "\[pii" --include=*.toml` in both repos | rename the field |
| `fixtures/ner-eval/` is deliberately **not** subject to "one corpus, two runners" | it is model-dependent and the model does not run in Studio's vitest; `fixtures/pii-corpus.json` keeps its mirror | if wrong, the fixture shape must be JS-consumable from the start — cheaper to decide now than to reshape later |

---

## Explicit Non-Goals

Stated so no task drifts into them. Each is a *later* step in spec §8 with its own gate.

- **No LoRA, no adapter, no training.** Spec step 7, gated on step 5 being exhausted *and* annotation capacity existing.
- **No hot-swap, no mutation of a cached backend.** Rejected outright in spec §3.
- **No vocabulary pruning.** Spec step 4, gated on Task 1's harness existing first.
- **No unmerged runtime LoRA delta / `DebertaV2Model` fork.** Spec step 8.
- **No model-identity field in `AuditEntry`.** Spec step 6, a prerequisite for step 7, not for step 3. Task 3 adds the *vertical* id only.
- **No per-request vertical switching.** v1 binds one vertical per `PiiPipeline`, chosen at construction. See Task 3's audit-chain constraint for why this is not merely a simplification.
- **No `[[pii.verticals]]` registry array.** The spec sketches one; with N=1 it is premature abstraction. Introduce it when a second vertical exists.
- **No fix to Studio's asset loader or the F16 SHA mismatch.** Real bugs (spec §6.2), but separate work with a separate owner.

---

## Task 0 — Record the baseline

- [x] Run `cargo test --workspace` and record the exact passed/failed/ignored counts in this file under **Baseline**. → **343 passed, 0 failed, 2 ignored** (see Baseline section above).
- [x] Run `cargo clippy --all-targets --all-features -D warnings` and `cargo fmt --check`; record clean or fix first. → clippy (`-- -D warnings`) **clean**; fmt **not clean** (2 files: `hacienda-cli/src/commands.rs`, `hacienda-cli/tests/extract.rs` — see Baseline section above; not fixed here, out of Task 0's scope).
- [x] Run `grep -rn "^\[pii" --include=*.toml . ../hacienda-private` and record every config file that would be affected by a new `[pii]` key. → ran as `grep -rn "^\[pii" --include=*.toml . ../../../../hacienda-private` (correct path from this worktree, verified with `realpath`); **zero matches**, no colliding config files in either repo.
- [x] Confirm `cargo check -p hacienda-core --features ner-candle` succeeds and record wall time (the host has ~3 GB RAM; if this thrashes, every model-touching task needs a different plan). → **succeeded**, exit 0, **4m50s**, no thrashing observed (see Baseline section above).
- [x] **Capture the pre-change chain-hash literal Task 3 needs.** Run the existing `audit::entry` tests with a temporary `dbg!` (or a throwaway test) to print `AuditEntry::new(input("id-1"), "prev", 0).chain_hash` for the fixture in `entry.rs:192`, and paste the hex here. Task 3's backward-compatibility test is worthless without a literal captured *before* the field is added. → `"72eb1d2f14c701e0c58280b2d7fc5132fdc0564a3ed42e7b0c4b84cfdd5a3ee4"` (see Baseline section above); throwaway test added, run, and removed — `entry.rs` is unchanged from `HEAD`.

**Acceptance:** the four numbers and the chain-hash literal exist in this document. No code changed.

---

## Task 1 — Evaluation harness (spec §8 step 1)

**Why first:** every claim downstream — "zero-shot is good enough", "pruning costs no F1", "Tier 1 closes the precision gap" — is unmeasurable today. This task has no gate because it *is* the gate.

### 1.1 Span-level fixture

- [x] Create `fixtures/ner-eval/README.md` stating provenance, licence, and that no real client data may enter this directory. → written, including the "DRAFT — not yet human-validated" status section and the 0.5 recall-ceiling note.
- [x] Create `fixtures/ner-eval/base-fr.json` and `base-en.json`, shape:
      `{ "cases": [ { "id": "...", "lang": "fr", "text": "...", "entities": [ { "start": 0, "end": 5, "label": "person" } ] } ] }`
      Offsets are **byte** offsets into the UTF-8 text, matching `Entity { start: u32, end: u32 }`. → both files created; byte offsets were computed and independently re-verified by a script that slices every entity out of its source text (not hand-counted), so accented cases (`Élodie`, `Nguyễn`, `Saint-Étienne`) are correct.
- [ ] Seed with at least 40 cases per language covering the five base categories, deliberately including accented and multi-part French surnames, particles (`de la`, `d'`), and organisation names that overlap person names. → **not met at the stated scale.** `base-fr.json` has 12 cases, `base-en.json` has 13. Per explicit direction for this pass, these are draft scaffolding cases (10–15/language) covering all five categories, particles, accents, and the person/org-overlap pattern (see `ANNOTATION.md` rule 4) at least once each — but they are roughly a third of the plan's 40-case target. Expanding to 40+ per language is real annotation work for a human who knows French business documents (see the plan's own risk table), not mechanical scaffolding, and is left for that pass.
- [x] Create `fixtures/ner-eval/vertical-finance.json` with the same shape, labelled with the finance vertical's labels. → 10 cases covering `iban`, `swift_code`, `account_number`, `routing_number`, `card_number` (2–3 instances of each label).
- [x] Add a schema-validity test in `hacienda-core/tests/ner_eval.rs` that deserialises every file and asserts each `entities[i]` slices `text` without panicking and yields non-empty content. **This test is not `#[ignore]`d** — the fixture stays valid in CI even though the model does not run there. → `should_have_schema_valid_entities_in_every_fixture`, passes in default `cargo test -p hacienda-core --test ner_eval` (verified below).

- [x] **Write `fixtures/ner-eval/ANNOTATION.md` before annotating anything.** Exact-match F1 measures agreement with a convention; an unstated convention makes the number meaningless. It must rule on at least: titles (`M.`, `Me`, `Dr`) in or out of a person span; particles (`de la`, `d'`) in or out; whether a legal-entity suffix (`SARL`, `SA`) is part of the organisation span; nested spans (a person inside an organisation name); and byte-vs-char offsets on accented text. Every ruling gets an example. → written with all five rulings plus a worked example each, marked DRAFT, with an explicit inter-annotator-agreement placeholder and an "open questions for the second annotator" section.
- [ ] Have a second person (or a second independent pass) re-annotate a 20-case sample and record inter-annotator agreement. If it is low, the guideline is the bug, not the model. → **not done.** This requires an actual second annotator (or a genuinely independent second pass, not the same agent re-reading its own guideline); it is out of scope for this pass and is called out as unresolved in both `README.md` and `ANNOTATION.md`.

### 1.2 Metrics

- [x] In `hacienda-core/tests/ner_eval.rs`, implement span matching and scoring:
      - `fn score(gold: &[Span], pred: &[Span], mode: MatchMode) -> Metrics` returning per-label and micro-averaged precision, recall, F1, plus raw TP/FP/FN counts.
      - `enum MatchMode { Exact, Overlap { min_ratio: f32 } }`. Report **both**; exact-boundary F1 and overlap F1 diverge sharply on names, and reporting only one hides the failure mode.
      - Matching is a greedy one-to-one assignment per label — a predicted span may satisfy at most one gold span, so duplicate predictions count as FP. → implemented exactly as specified.
- [x] Unit-test the metric math with hand-computed cases in the same file: perfect match, all-miss, off-by-one boundary, duplicate prediction, label mismatch at identical offsets, zero-gold-zero-pred (define F1 as 1.0, and assert it). → six tests in `metrics_tests`, all passing (verified below).
- [x] **Report a bootstrap confidence interval on every F1, not a bare point estimate.** With ~40 cases per language a per-label F1 rests on tens of instances; two point estimates differing by 0.03 is noise. Spec step 4's gate ("no F1 regression beyond agreed tolerance") is unenforceable against point estimates — resample cases with replacement (1000 draws) and report the 95% interval alongside. The report must also print the instance count per label so a reader can see when a number is not worth quoting. → `bootstrap_micro_f1_ci` (dependency-free splitmix64 PRNG, 1000 draws, 95% interval); `LabelMetricsReport` carries `support_gold`/`predicted` per label in the emitted report. Three unit tests in `bootstrap_tests` cover the degenerate (all-identical), empty, and disagreeing-cases behaviour.

### 1.3 Runner

- [x] Add `#[ignore]`d, `#[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]` tests that read `HACIENDA_EVAL_MODEL_DIR` (skip with a clear message if unset), build a `NerDetector` directly via `from_candle_local`, and score each fixture. → `runner::should_produce_an_evaluation_report_against_a_local_model`; compiles clean under `cargo check -p hacienda-core --features ner-candle --tests` and `cargo clippy` with the same flags (verified below), **not executed** — see 1.4/1.5 notes.
- [x] Emit a report to `HACIENDA_EVAL_OUT` (default `target/ner-eval/<timestamp>.json`) containing: model dir, its `model.safetensors` blake3 digest, label set, threshold, and the full metrics table. **The digest is the point** — a report that does not identify the weights it scored cannot support the step-4 pruning comparison. → `EvalReport` includes all of these plus the label-interference tables and threshold sweep; code path compiled, not run.
- [x] Document the invocation in `fixtures/ner-eval/README.md`:
      `HACIENDA_EVAL_MODEL_DIR=~/model_f16 cargo test -p hacienda-core --features ner-candle --test ner_eval -- --ignored --nocapture` → present verbatim in the README's "Running the harness" section.

### 1.4 The recall ceiling this harness cannot see

`CandleBackend::detect` ignores the caller's threshold and passes its own hardcoded
`const DEFAULT_THRESHOLD: f32 = 0.5` to `extract_ner` (verified in `v1.0.2`). `NerDetector::threshold` then
filters *what the backend already returned*. So hacienda can only ever raise the effective threshold above 0.5,
never lower it, and **the harness physically cannot observe any entity scored below 0.5.**

Consequences that must be stated in the report rather than discovered later:

- Reported recall is **truncated recall at 0.5**, not recall. Label it as such in the report schema.
- A precision/recall curve is unobtainable. Any Tier 1 calibration is therefore precision-side only — spec §4.2.
- Do **not** work around this by patching the vendored dependency. Instead:
  - [ ] Measure how much sits at the boundary: sweep `NerDetector::with_threshold` over 0.5…0.95 and report the metrics at each step. A recall curve that is still climbing steeply *at* 0.5 is evidence that material recall lies below it. → **code written; execution attempted twice and did not complete on this host.** See "Attempted execution" below.
  - [ ] If it is material, that evidence — not an assumption — justifies asking xberg to thread a threshold through `NerBackend::detect`. Record the finding here and open the upstream issue; do not block Tasks 2–3 on it. → blocked; no finding to record yet.

**Attempted execution, 2026-08-01, on this host (~3.7 GB RAM, ~6.7 GB swap, shared with other processes):**

1. **First attempt** (`#[tokio::test]`, default current-thread runtime): failed after 134.65s with a panic, not a timeout —
   `can call blocking only when running on the multi-threaded runtime`, at
   `xberg v1.0.2 crates/xberg/src/text/ner/candle.rs:169`. Root cause: `CandleBackend::detect` calls
   `tokio::task::block_in_place`, which requires `flavor = "multi_thread"`. **Fixed** by changing the attribute to
   `#[tokio::test(flavor = "multi_thread")]` (the workspace's `tokio` already has `rt-multi-thread` enabled, so no
   dependency change was needed). This fix is a genuine, permanent correction to the harness, not an
   execution-environment workaround — it is now in `hacienda-core/tests/ner_eval.rs:919-923` with a comment
   explaining why.
2. **Second attempt**, after the fix: ran **71 minutes** of wall-clock time (09:55–11:06) without completing.
   `ps` showed steady CPU accumulation (up to 93 CPU-minutes across worker threads, consistent with a
   multi-threaded runtime under memory pressure rather than a hang) but `free -h` showed 3.1–3.3 GiB of 3.7 GiB
   used throughout and 2.3–3.6 GiB of swap in active use. This is **not the harness thrashing** in the sense of a
   bug — the run performs one real model load (cached by `get_or_init` on `(model_dir, None)`, so all 13
   `NerDetector` constructions after the first share one resident model) followed by roughly 310 individual
   `extract_ner` forward passes (35 base-only + 25 extended + 10 threshold-sweep steps × 25 texts each). On a
   host already at its RAM ceiling, each forward pass competing for resident pages against the OS pushing other
   allocations to swap is enough to make this take an unreasonable amount of time. Killed (`kill -9`) rather than
   let it run indefinitely.

**Per the plan's own stated mitigation for this exact risk (see the Risks table): run on a different host, do not
weaken the harness to force a result out of an inadequate one.** No fixture count was reduced, no threshold-sweep
step was cut, and no shortcut was taken to produce a number. The harness is code-complete and the one real defect
found (`block_in_place`/runtime flavor) is fixed. Executing it to get real numbers is unfinished and needs either
more RAM or a smaller model directory to complete in reasonable time on this host — flagged as the actionable
item for whoever picks this back up.

**Acceptance:** the non-ignored schema test passes in a default `cargo test` (✅ verified: `cargo test -p hacienda-core --test ner_eval` → 10 passed, 0 failed, 0 ignored); the ignored runner is code-complete, compiles under `cargo check -p hacienda-core --features ner-candle --tests`, and its one real defect (tokio runtime flavor) has been found and fixed by actually attempting to run it — but it has **not completed** a run on this host (see "Attempted execution" above); the metric unit tests pass (✅ verified, 6/6 in `metrics_tests`). Base-category baseline numbers are **not recorded** — this remains the actionable next step, on a host with more headroom.

### 1.5 Gate into Task 2

Task 1 is not merely a prerequisite; it can **invalidate Task 2's design**. Run this comparison before writing Task 2 code:

- [ ] Score the base five categories **twice**: once with `DEFAULT_CATEGORIES` alone, once with `DEFAULT_CATEGORIES` + the finance vertical labels. Same fixture, same model, same threshold. → **code written; execution attempted 2026-08-01 and did not complete in 71 minutes on this host (RAM-constrained, swap-heavy).** See 1.4's "Attempted execution" note — same runner invocation. `runner::should_produce_an_evaluation_report_against_a_local_model` builds `base_only_cases` (base-fr + base-en scored with `base_categories()`) and `extended_cases` (the same two fixtures scored with `base_plus_finance_categories()`), reporting both as `LabelInterferenceReport { base_only, base_plus_finance_vertical }` in both match modes.
- [ ] If base-category F1 drops beyond the interval from 1.2, **label interference is real** and "extend the base set" (Task 2.2) is the wrong design — verticals would have to run as a second detection pass and merge, which is a materially larger change. Stop and re-scope rather than shipping the extension. → cannot be evaluated without a completed run. **This is the hard gate from the plan's own framing (line 153: "this task has no gate because it *is* the gate") — Task 2 must not start until someone runs `HACIENDA_EVAL_MODEL_DIR=~/model_f16 cargo test -p hacienda-core --features ner-candle --test ner_eval -- --ignored --nocapture` on a host with enough headroom to finish, and reads `label_interference` out of the resulting JSON report. Confirmed not yet possible on this host — two attempts, one fixed a real bug, neither produced a report.**
- [ ] Record the two tables side by side here regardless of outcome. → pending a successful run on adequate hardware.

---

## Task 2 — Tier 0 schema verticals (spec §8 step 2)

### 2.1 Config

- [ ] Add to `hacienda-core/src/pii/config.rs`:
      ```rust
      #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
      #[serde(default, deny_unknown_fields)]
      pub struct VerticalConfig {
          /// Stable identifier recorded in the audit chain.
          pub id: String,
          /// Zero-shot labels handed to the NER backend in addition to the base categories.
          pub labels: Vec<String>,
      }
      ```
      and `pub vertical: Option<VerticalConfig>` on `PipelineConfig` (default `None`).
- [ ] Add `pub fn validate(&self) -> Result<(), PiiError>` on `VerticalConfig`: non-empty `id`; non-empty `labels`; every label non-empty after trim; no label containing `[`, `:`, `]` (mirroring `category_label`); no duplicates after case-folding. Return a new `PiiError::InvalidVertical { id, reason }` variant.
- [ ] Do **not** add a `verticals` array or a selector. One optional vertical, bound at construction.

### 2.2 Threading

**Validation must not live in `load_detector`.** Verified: `pipeline.rs:239-247` has a
`#[cfg(not(all(feature = "ner-candle", not(target_arch = "wasm32"))))]` stub that returns
`PiiError::ModelUnavailable` on its first line, and `load_detector` is only reached when
`config.model.enabled` is true. Putting validation there means a malformed vertical is silently accepted in the
**default regex-only configuration** and on every wasm build. Validate in `assemble`, unconditionally, next to
the existing `validate_ner_labels_for_pseudonymize` call at `pipeline.rs:131`.

- [ ] In `PiiPipeline::assemble`, before building the detector, call `config.vertical.as_ref().map(VerticalConfig::validate).transpose()?`. A bad vertical is a configuration error in every build profile.
- [ ] `DEFAULT_CATEGORIES` at `ner.rs:14` is a **private** `const` today. Change it to `pub(crate)` as part of this task — the plan's earlier draft assumed it was already visible; it is not.
- [ ] In the `ner-candle` arm of `load_detector`, extend categories when a vertical is configured:
      ```rust
      let mut detector = NerDetector::from_candle_local(model_dir, config.model.lora_adapter_dir.as_deref())?
          .with_threshold(config.model_threshold_default);
      if let Some(vertical) = &config.vertical {
          let mut categories = crate::pii::ner::DEFAULT_CATEGORIES.to_vec();
          categories.extend(vertical.labels.iter().cloned().map(EntityCategory::Custom));
          detector = detector.with_categories(categories);
      }
      Ok(detector)
      ```
      The base five are **extended, not replaced** — a finance vertical must still find people.
- [ ] No change to `NerDetector::detect`, `PiiPipeline::detect`, or any call site. That is the design.

### 2.3 Tests

- [ ] `should_extend_base_categories_with_vertical_labels` — build a pipeline with a stub backend and a vertical, assert `configured_categories()` contains all five base variants plus each label as `EntityCategory::Custom`.
- [ ] `should_reject_a_vertical_label_containing_a_token_delimiter` — assert `PiiError::InvalidVertical`, and assert it fails **regardless of redaction mode** (validation is not pseudonymise-only).
- [ ] `should_reject_an_invalid_vertical_in_a_regex_only_pipeline` — `model.enabled = false`, bad vertical, still `PiiError::InvalidVertical`. This is the test that pins the validation site; without it a future refactor will quietly move validation back into `load_detector`.
- [ ] `should_map_a_vertical_label_to_a_custom_pii_category` — stub backend returns `EntityCategory::Custom("swift_code")`, assert the emitted `ModelEntity.category == PiiCategory::Custom("swift_code")`.
- [ ] `should_map_an_aliased_vertical_label_to_its_taxonomy_category` — label `"iban"` must arrive as `PiiCategory::Iban`, not `Custom("iban")`, because `to_pii_category`'s alias table already claims it. This is a real footgun: a vertical author choosing `iban` gets different downstream behaviour than one choosing `iban_number`. Assert it and document it in the config rustdoc.
- [ ] `should_pseudonymise_a_vertical_detection_into_a_parsable_token` — end-to-end with a stub backend and `RedactionMode::Pseudonymize`, assert the token is `[SWIFT_CODE:...]` and that `reveal` round-trips it.
- [ ] `should_reject_a_pipeline_whose_vertical_label_cannot_be_pseudonymised` — confirm the existing `validate_ner_labels_for_pseudonymize` path now covers vertical labels with no new code.
- [ ] Config round-trip: extend `hacienda-core/tests/config_round_trip.rs` so a TOML file with `[pii.vertical]` survives serialise → deserialise, and so a file **without** it still parses (the `deny_unknown_fields` direction is the risk).

### 2.4 Surface exposure

- [ ] **Do not add a `--vertical` flag.** With one `Option<VerticalConfig>` the flag could only confirm or reject the id already in the config file — a flag that cannot change behaviour. Adding it now would contradict this plan's own refusal of the `[[pii.verticals]]` registry as premature. The flag arrives with the registry, or not at all.
- [ ] Extend `config show` (`hacienda-cli/src/commands.rs:169-181`) to print the active vertical id and label set with provenance, matching the existing `model_dir` lines. This is the whole CLI surface for v1, and it is enough: the operator can see what is active.
- [ ] **`hacienda-api`:** `dto.rs:199` is an explicit allowlist of `PipelineConfig` fields, not a derived `Serialize`. Decide whether the vertical is exposed over HTTP and act deliberately — leaving it out is defensible, leaving it out *by forgetting the file exists* is not. Add the field to the allowlist and a DTO test either way, or add a comment recording the decision not to.
- [ ] **wasm — amend the spec, do not build anything.** Verified: `hacienda-core` on wasm32 *does* compile xberg's Candle backend (via `ner-candle-wasm`), but both `NerDetector::from_candle_local` and the real `load_detector` are gated `not(target_arch = "wasm32")`, so `load_detector` on wasm always returns `ModelUnavailable`. hacienda-core cannot run a model in the browser under any feature combination today; Studio reaches `xberg-wasm`'s `NerModel` directly instead (commit `c322cad`). Spec §4.1's "works identically on native and wasm32" is therefore false as plumbing — Tier 0 is native-only until that gate is opened. Correct §4.1 and add it to spec §9 Open Questions. **Do not** widen the cfg as part of this plan: making hacienda-core load a 614 MB model in a browser is spec §6's blocker, not a checkbox here.

### 2.5 API compatibility

- [ ] `VerticalConfig` and `PipelineConfig::vertical` are **public API additions**. Add them to `CHANGELOG.md` under `[Unreleased] / Added`, per the repo's api-compatibility rule.
- [ ] `PipelineConfig` has public fields and no `#[non_exhaustive]` (there is none anywhere in `hacienda-core`). Adding a field breaks any external struct-literal construction. Either mark it `#[non_exhaustive]` now — cheap, and it makes every future field additive — or accept and document the semver consequence. Decide explicitly; do not add the field silently.

**Acceptance:** all new tests pass; `cargo test --workspace` count equals Task 0's baseline plus the new tests, with **zero** pre-existing failures; `clippy -D warnings` clean; `hacienda config show` displays the vertical; CHANGELOG updated.

---

## Task 3 — Vertical provenance in the audit chain (spec §8 step 3)

**Constraint discovered while planning, and the reason this task exists:** `config_hash` is operator-supplied and *chain-scoped* — `AuditChain::append` refuses any entry whose `config_hash` differs from the chain's. The vertical therefore cannot be folded into `config_hash` unless it is guaranteed constant for the life of a chain. Following the `principal` precedent instead gives a per-entry, chain-hashed field with backward-compatible verification.

**Decision, made here rather than deferred:** the recorded value is `"<id>@<first 8 hex of blake3 over the
sorted, case-folded label set>"`, e.g. `finance@3f9a1c02`. An id alone is a false provenance claim — the same id
with different labels detects different things, and the audit record's job is to say what *was* detectable. The
digest makes a silently-edited label set visible. Every step below uses this composed value; `None` when no
vertical is configured.

- [ ] Add `#[serde(default)] pub vertical: Option<String>` to `AuditEntry` and `pub vertical: Option<String>` to `AuditEntryInput`, documented like `principal`, with a rustdoc note that the value is `id@digest` and why.
- [ ] Add `VerticalConfig::provenance_id(&self) -> String` producing that value, unit-tested for stability under label reordering and case changes, and for *instability* when a label is added.
- [ ] Add `pub vertical: Option<&'a str>` to `ChainHashFields`, populate it in `AuditEntry::new` and `chain_hash_fields`, and hash it in `compute_chain_hash` as `fields.vertical.unwrap_or("")` — **appended after `principal`**, so chains written before this field existed hash byte-for-byte identically.
- [ ] Populate it at both `AuditEntryInput` construction sites in `facade.rs` from the pipeline's configured vertical id.
- [ ] Add the column to `audit/export.rs`'s CSV header and row. **This is a breaking change to the export format, and the export declares no version** — verified: `export_csv` writes a bare literal header at `export.rs:44-46` with nothing to bump. Append the column last, state the break in `CHANGELOG.md`, and check any downstream parser before merging.
- [ ] Tests:
      - `should_verify_a_chain_written_before_the_vertical_field_existed` — construct entries with `vertical: None` and assert the chain hash equals a **hard-coded literal captured before the change**. Capture that literal in Task 0. Without it this test proves nothing.
      - `should_change_the_chain_hash_when_the_vertical_changes` — same inputs, different vertical id, different hash.
      - `should_reject_a_tampered_vertical_id` — rewrite `vertical` on a serialised entry, assert verification fails.
      - Round-trip an entry with `vertical: Some(..)` through JSON and CSV.
      - `should_change_the_provenance_id_when_a_label_is_added` — same id, one extra label, different digest.
- [ ] `ChainHashFields` is `pub` with `pub` fields and no `#[non_exhaustive]`; adding a field breaks external construction. Same decision as Task 2.5, and here the case for `#[non_exhaustive]` is stronger, because this struct is *designed* to grow (its own rustdoc calls adding a field "a deliberate, reviewable act"). Add it, and note the semver impact in CHANGELOG.
- [ ] Add `AuditEntry.vertical` and the CSV column to `CHANGELOG.md`.

**Acceptance:** old-chain verification test passes against the literal captured in Task 0; tamper test fails verification; export round-trips; CHANGELOG updated.

---

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Zero-shot custom labels perform badly enough to be useless | Tier 0 ships but nobody should enable it | Task 1 runs **before** Task 2 ships; if F1 is unusable, the plan's outcome is a measurement and a documented "no", which is still the right outcome |
| Adding labels degrades the base five (label interference) | **Task 2.2's design is wrong**, not merely regressed | Promoted out of this table into a hard gate — Task 1.5. Task 2 does not start until the two-table comparison is recorded |
| Annotation is the real cost and is not engineering work | Plan looks like three weeks of Rust and is mostly a week of careful labelling by someone who knows French legal documents | Stated here explicitly. Task 1.1 is ~80% of this plan's effort; staff it accordingly or the harness will be built and never populated |
| `deny_unknown_fields` on `PipelineConfig` | An older binary rejects a config carrying `[pii.vertical]` | Task 0 inventories config files; note in CHANGELOG that new config requires the new binary |
| Host RAM (~3 GB) makes `ner-candle` builds thrash | Task 1 stalls | measured in Task 0; if it thrashes, run the harness on a different host and record results here rather than weakening the harness |
| Alias-table collision (`iban` → `PiiCategory::Iban`) | Vertical author gets surprising downstream behaviour | explicit test + rustdoc in Task 2.3 |
| Export CSV column addition breaks a downstream parser | Silent data corruption in someone's pipeline | called out explicitly in Task 3 rather than shipped quietly |

---

## Sequencing

Task 0 → Task 1 → **gate 1.5** → (Task 2 ‖ Task 3) → integration.

The gate is real. If Task 1.5 shows label interference, or if custom-label F1 on the fr split is unusable, the
correct outcome of this plan is a recorded measurement and a documented "no" — not a shipped Tier 0. That is a
success, and it costs one task instead of a quarter spent on adapters for a capability the model already had.

Tasks 2 and 3 touch disjoint files (`pii/*` + `hacienda-cli/*` vs `audit/*`) except for the two `AuditEntryInput` construction sites in `facade.rs`, which Task 3 owns. They can run in parallel if Task 2 lands first or the facade edit is coordinated.

Task 1 must not be deferred "until there is time". It is the only task here that changes what the project *knows*.
