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

- [x] Create `fixtures/ner-eval/README.md` stating provenance, licence, and that no real client data may enter this directory.
- [x] Create `fixtures/ner-eval/base-fr.json` and `base-en.json`, shape:
      `{ "cases": [ { "id": "...", "lang": "fr", "text": "...", "entities": [ { "start": 0, "end": 5, "label": "person" } ] } ] }`
      Offsets are **byte** offsets into the UTF-8 text, matching `Entity { start: u32, end: u32 }`. All offsets
      were generated (not hand-computed) by a small Python helper that encodes each text as UTF-8 and indexes the
      byte string, then self-checks by decoding the slice back and comparing to the expected substring — see
      `ANNOTATION.md` rule 6.
- [x] Seed with at least 40 cases per language covering the five base categories, deliberately including accented and multi-part French surnames, particles (`de la`, `d'`), and organisation names that overlap person names. → **45 cases in `base-fr.json` (55 entities), 42 in `base-en.json` (52 entities)**, verified by `should_have_at_least_forty_cases_per_base_language`.
- [x] Create `fixtures/ner-eval/vertical-finance.json` with the same shape, labelled with the finance vertical's labels. → **18 cases**, labels `iban` (aliased by `to_pii_category` onto `PiiCategory::Iban`), `swift_code` and `account_number` (both fall through to `PiiCategory::Custom`, deliberately exercising the alias-table footgun called out in Task 2.3/§4.2's risk table — see `README.md`'s "Label vocabulary" section).
- [x] Add a schema-validity test in `hacienda-core/tests/ner_eval.rs` that deserialises every file and asserts each `entities[i]` slices `text` without panicking and yields non-empty content. **This test is not `#[ignore]`d** — the fixture stays valid in CI even though the model does not run there. → `should_slice_every_fixture_entity_at_valid_utf8_boundaries_with_nonempty_content`, plus three supporting schema tests (case-count floor, label-vocabulary allowlist, unique case ids). All pass under default `cargo test -p hacienda-core --test ner_eval`.

- [x] **Write `fixtures/ner-eval/ANNOTATION.md` before annotating anything.** Exact-match F1 measures agreement with a convention; an unstated convention makes the number meaningless. It must rule on at least: titles (`M.`, `Me`, `Dr`) in or out of a person span; particles (`de la`, `d'`) in or out; whether a legal-entity suffix (`SARL`, `SA`) is part of the organisation span; nested spans (a person inside an organisation name); and byte-vs-char offsets on accented text. Every ruling gets an example. → Also rules on leading definite articles (`Le`/`La`/`The`) in organisation spans and on address/email/phone span boundaries, since those came up while writing cases; every ruling in the file is applied consistently across all three fixtures.
- [ ] Have a second person (or a second independent pass) re-annotate a 20-case sample and record inter-annotator agreement. **Not done — needs a human.** These fixtures are the output of one annotator (this agent, in a single pass) and have not been independently re-annotated or checked for inter-annotator agreement. Do not treat any F1 this harness reports as a validated measurement until that review happens — see `README.md`'s "Known limitation" section and `ANNOTATION.md`'s closing section, both of which restate this so it cannot be missed by only reading one of the two files. This mirrors the caveat already present in PR #42's own description.

### 1.2 Metrics

- [x] In `hacienda-core/tests/ner_eval.rs`, implement span matching and scoring:
      - `fn score(gold: &[Span], pred: &[Span], mode: MatchMode) -> Metrics` returning per-label and micro-averaged precision, recall, F1, plus raw TP/FP/FN counts.
      - `enum MatchMode { Exact, Overlap { min_ratio: f32 } }`. Report **both**; exact-boundary F1 and overlap F1 diverge sharply on names, and reporting only one hides the failure mode.
      - Matching is a greedy one-to-one assignment per label — a predicted span may satisfy at most one gold span, so duplicate predictions count as FP.
- [x] Unit-test the metric math with hand-computed cases in the same file: perfect match, all-miss, off-by-one boundary, duplicate prediction, label mismatch at identical offsets, zero-gold-zero-pred (define F1 as 1.0, and assert it). → all six in `metric_unit_tests`, plus an instance-count test; all pass.
- [x] **Report a bootstrap confidence interval on every F1, not a bare point estimate.** With ~40 cases per language a per-label F1 rests on tens of instances; two point estimates differing by 0.03 is noise. Spec step 4's gate ("no F1 regression beyond agreed tolerance") is unenforceable against point estimates — resample cases with replacement (1000 draws) and report the 95% interval alongside. The report must also print the instance count per label so a reader can see when a number is not worth quoting. → `bootstrap_micro_f1_ci` (1000 resamples, case-level, dependency-free xorshift64 PRNG since the workspace has no `rand` crate), unit-tested in `bootstrap_unit_tests`; `LabelMetrics` carries `gold_count`/`predicted_count` per label and for the micro-average.

### 1.3 Runner

- [x] Add `#[ignore]`d, `#[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]` tests that read `HACIENDA_EVAL_MODEL_DIR` (skip with a clear message if unset), build a `NerDetector` directly via `from_candle_local`, and score each fixture. → `runner::should_score_ner_eval_fixtures_against_a_local_model`. Compiles cleanly under `cargo check -p hacienda-core --features ner-candle --tests` and is clippy-clean under `-D warnings`; **not executed** in this sandbox, which has no local model directory (see Acceptance note below).
- [x] Emit a report to `HACIENDA_EVAL_OUT` (default `target/ner-eval/<timestamp>.json`) containing: model dir, its `model.safetensors` blake3 digest, label set, threshold, and the full metrics table. **The digest is the point** — a report that does not identify the weights it scored cannot support the step-4 pruning comparison. → `EvalReport`/`FixtureReport` in the `runner` module; digest via the workspace's existing `blake3` dependency.
- [x] Document the invocation in `fixtures/ner-eval/README.md`:
      `HACIENDA_EVAL_MODEL_DIR=~/model_f16 cargo test -p hacienda-core --features ner-candle --test ner_eval -- --ignored --nocapture`

### 1.4 The recall ceiling this harness cannot see

`CandleBackend::detect` ignores the caller's threshold and passes its own hardcoded
`const DEFAULT_THRESHOLD: f32 = 0.5` to `extract_ner` (verified in `v1.0.2`). `NerDetector::threshold` then
filters *what the backend already returned*. So hacienda can only ever raise the effective threshold above 0.5,
never lower it, and **the harness physically cannot observe any entity scored below 0.5.**

Consequences that must be stated in the report rather than discovered later:

- Reported recall is **truncated recall at 0.5**, not recall. Label it as such in the report schema. → `EvalReport::recall_caveat`, populated on every run with the "truncated recall at 0.5" wording verbatim.
- A precision/recall curve is unobtainable. Any Tier 1 calibration is therefore precision-side only — spec §4.2.
- Do **not** work around this by patching the vendored dependency. Instead:
  - [x] Measure how much sits at the boundary: sweep `NerDetector::with_threshold` over 0.5…0.95 and report the metrics at each step. A recall curve that is still climbing steeply *at* 0.5 is evidence that material recall lies below it. → `threshold_sweep_values()` (0.50, 0.55, …, 0.95) and `FixtureReport::threshold_sweep`, code-complete and type-checked; **not run**, so no sweep data exists yet (see below).
  - [ ] If it is material, that evidence — not an assumption — justifies asking xberg to thread a threshold through `NerBackend::detect`. Record the finding here and open the upstream issue; do not block Tasks 2–3 on it. **Not done — cannot be done from this sandbox.** No model directory exists here, so the ignored runner has never actually executed and there is no threshold-sweep data to read a finding off of. This checkbox needs a run against a real model (e.g. `~/model_f16` per the plan's own baseline row) before anyone can say whether the recall left below 0.5 is material.

**Acceptance:** the non-ignored schema test passes in a default `cargo test` — **confirmed**: `cargo test -p hacienda-core --test ner_eval` under default features (no `ner-candle`) is **14 passed, 0 failed, 0 ignored**, covering the schema tests and all metric/bootstrap unit tests. `cargo check -p hacienda-core --features ner-candle --tests` succeeds and `cargo clippy -p hacienda-core --all-targets --features ner-candle -- -D warnings` is clean, so the ignored runner compiles and does not bit-rot silently. The ignored runner itself, the base-category baseline numbers, and the 0.5…0.95 threshold sweep data **have not been produced** — this sandbox has no `~/model_f16` (or any other local GLiNER2 model directory), so `HACIENDA_EVAL_MODEL_DIR` was never set and the runner has only ever taken its "skip, print a message" branch. Running it against a real model directory, and recording the resulting base-category baseline numbers here, is the next action before Task 1.5 or Task 2 can start.

### 1.5 Gate into Task 2

Task 1 is not merely a prerequisite; it can **invalidate Task 2's design**. Run this comparison before writing Task 2 code:

- [ ] Score the base five categories **twice**: once with `DEFAULT_CATEGORIES` alone, once with `DEFAULT_CATEGORIES` + the finance vertical labels. Same fixture, same model, same threshold.
- [ ] If base-category F1 drops beyond the interval from 1.2, **label interference is real** and "extend the base set" (Task 2.2) is the wrong design — verticals would have to run as a second detection pass and merge, which is a materially larger change. Stop and re-scope rather than shipping the extension.
- [ ] Record the two tables side by side here regardless of outcome.

**Status: not run — cannot be run from this environment.** No agent session working this plan so far (Task 0 through this point) has had a local GLiNER2 model directory available; every ignored-runner invocation has taken the "skip, print a message" branch. This gate is therefore **not empirically satisfied**. Task 2 below was implemented anyway, at explicit user direction to continue the plan end-to-end, on the following basis:

- Task 2's code *is* exactly the design the plan specifies (extend `DEFAULT_CATEGORIES`, do not replace it) — nothing about proceeding without the gate changes what gets built, only whether it has been shown safe to enable.
- The risk this gate protects against is **runtime behaviour** (does adding vertical labels degrade base-category recall in the deployed model), not a compile-time or type-level property. Shipping the code with the gate outstanding is safe *precisely because* nothing downstream defaults it to on — see Task 2.1's `vertical: Option<VerticalConfig>` defaulting to `None`, and Task 2.4's decision not to add a `--vertical` CLI flag. An operator must opt in by writing `[pii.vertical]` into a config file.
- The unresolved risk is now **operational, not architectural**: do not enable a `[pii.vertical]` config in any environment that has real traffic until this comparison has actually been run against the target model and recorded here. That is a deployment gate, not a code-review gate, and it is documented as such in this file, in `README.md`, and in the PR description so it cannot be missed by reading only one of the three.
- If this comparison is later run and shows material interference, Task 2's "extend, don't replace" design is wrong and needs the second-detection-pass re-scope the original text above describes — that would be a breaking change to `VerticalConfig`'s semantics, not a patch. Flagging it here so the next person who *can* run the harness knows what a failing result implies.

---

## Task 2 — Tier 0 schema verticals (spec §8 step 2)

### 2.1 Config

- [x] Add to `hacienda-core/src/pii/config.rs`:
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
      → confirmed: `VerticalConfig` added verbatim (plus rustdoc, including the alias-table footgun note) at
      `hacienda-core/src/pii/config.rs`; `PipelineConfig::vertical: Option<VerticalConfig>` added, `None` in
      `PipelineConfig::default()`; re-exported as `hacienda_core::pii::VerticalConfig`.
- [x] Add `pub fn validate(&self) -> Result<(), PiiError>` on `VerticalConfig`: non-empty `id`; non-empty `labels`; every label non-empty after trim; no label containing `[`, `:`, `]` (mirroring `category_label`); no duplicates after case-folding. Return a new `PiiError::InvalidVertical { id, reason }` variant.
      → confirmed: implemented exactly as specified; `PiiError::InvalidVertical { id, reason }` added to
      `hacienda-core/src/pii/mod.rs`; 6 unit tests in `config.rs` cover empty id, empty label set, blank label,
      each of `[`/`:`/`]`, and case-folded duplicates.
- [x] Do **not** add a `verticals` array or a selector. One optional vertical, bound at construction.
      → confirmed: `PipelineConfig` carries a single `Option<VerticalConfig>`, nothing else.

### 2.2 Threading

**Validation must not live in `load_detector`.** Verified: `pipeline.rs:239-247` has a
`#[cfg(not(all(feature = "ner-candle", not(target_arch = "wasm32"))))]` stub that returns
`PiiError::ModelUnavailable` on its first line, and `load_detector` is only reached when
`config.model.enabled` is true. Putting validation there means a malformed vertical is silently accepted in the
**default regex-only configuration** and on every wasm build. Validate in `assemble`, unconditionally, next to
the existing `validate_ner_labels_for_pseudonymize` call at `pipeline.rs:131`.

- [x] In `PiiPipeline::assemble`, before building the detector, call `config.vertical.as_ref().map(VerticalConfig::validate).transpose()?`. A bad vertical is a configuration error in every build profile.
      → confirmed: added as the first statement in `assemble`, ahead of the pseudonymize-mode check, using
      `if let Some(vertical) = &config.vertical { vertical.validate()?; }` (equivalent to the sketch's
      `.map(...).transpose()?`, chosen for a clearer error site in a stack trace). Pinned by
      `should_reject_an_invalid_vertical_in_a_regex_only_pipeline` (`model.enabled = false` still rejects).
- [x] `DEFAULT_CATEGORIES` at `ner.rs:14` is a **private** `const` today. Change it to `pub(crate)` as part of this task — the plan's earlier draft assumed it was already visible; it is not.
      → confirmed: changed to `pub(crate)`.
- [x] In the `ner-candle` arm of `load_detector`, extend categories when a vertical is configured:
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
      → confirmed, with one deliberate deviation from the literal sketch: the `DEFAULT_CATEGORIES.to_vec()` +
      `.extend(...)` logic lives in a new `pub(crate) fn categories_with_vertical` in `ner.rs` rather than
      inline in `load_detector`, so it is unit-testable (`should_extend_base_categories_with_vertical_labels`,
      `should_return_the_base_categories_unchanged_when_no_vertical_is_configured`) without a real model
      directory — `load_detector`'s `ner-candle` arm cannot itself be exercised in this sandbox (needs real
      GLiNER2 weights on disk; see Task 1.5's status). `load_detector` now calls that shared function; behaviour
      is unchanged from the sketch.
- [x] No change to `NerDetector::detect`, `PiiPipeline::detect`, or any call site. That is the design.
      → confirmed: neither function's body changed.

### 2.3 Tests

- [x] `should_extend_base_categories_with_vertical_labels` — build a pipeline with a stub backend and a vertical, assert `configured_categories()` contains all five base variants plus each label as `EntityCategory::Custom`.
      → confirmed, in `hacienda-core/src/pii/ner.rs`: asserts on `categories_with_vertical`'s output directly
      (see 2.2's deviation note) rather than reaching into a `PiiPipeline`'s private `ner_detector` field —
      equivalent coverage, since that function is exactly what `load_detector` now calls.
- [x] `should_reject_a_vertical_label_containing_a_token_delimiter` — assert `PiiError::InvalidVertical`, and assert it fails **regardless of redaction mode** (validation is not pseudonymise-only).
      → confirmed, in `pipeline.rs`: loops over `[Mask, Hash]` plus a separate `Pseudonymize` case, all three
      rejecting with `PiiError::InvalidVertical` and no detector configured at all (so pseudonymize-specific
      validation cannot be the thing catching it).
- [x] `should_reject_an_invalid_vertical_in_a_regex_only_pipeline` — `model.enabled = false`, bad vertical, still `PiiError::InvalidVertical`. This is the test that pins the validation site; without it a future refactor will quietly move validation back into `load_detector`.
      → confirmed, in `pipeline.rs`, using `PiiPipeline::new` (not `with_detector`) so `load_detector` really is
      skipped end-to-end.
- [x] `should_map_a_vertical_label_to_a_custom_pii_category` — stub backend returns `EntityCategory::Custom("swift_code")`, assert the emitted `ModelEntity.category == PiiCategory::Custom("swift_code")`.
      → confirmed, in `ner.rs`.
- [x] `should_map_an_aliased_vertical_label_to_its_taxonomy_category` — label `"iban"` must arrive as `PiiCategory::Iban`, not `Custom("iban")`, because `to_pii_category`'s alias table already claims it. This is a real footgun: a vertical author choosing `iban` gets different downstream behaviour than one choosing `iban_number`. Assert it and document it in the config rustdoc.
      → confirmed, in `ner.rs`; the footgun is documented on `VerticalConfig`'s rustdoc in `config.rs` under a
      "Footgun: the alias-table collision" heading.
- [x] `should_pseudonymise_a_vertical_detection_into_a_parsable_token` — end-to-end with a stub backend and `RedactionMode::Pseudonymize`, assert the token is `[SWIFT_CODE:...]` and that `reveal` round-trips it.
      → confirmed, in `pipeline.rs`, **with one change from the plan's example token**: the label used is
      `docket_number` (text `DOC-REF-48213`), not `swift_code` (text `BOFAUS3N`) — the latter combination was
      tried first and failed, because `BOFAUS3N` also matches hacienda's own built-in SWIFT/BIC *regex* pattern
      (`patterns.rs`), and with the default `regex_first: true` merge priority the regex detection pre-empted
      the model/vertical one, producing a `[SWIFTBIC:...]` token instead — a real merge-priority interaction,
      not a bug, but the wrong thing for this test to demonstrate. `docket_number`/`DOC-REF-48213` isolates the
      vertical path from the regex engine. Also note `reveal` returns the *normalised* (lowercased) value, per
      `normalize`'s existing documented behaviour — asserted as `text.to_lowercase()`, not `text`.
- [x] `should_reject_a_pipeline_whose_vertical_label_cannot_be_pseudonymised` — confirm the existing `validate_ner_labels_for_pseudonymize` path now covers vertical labels with no new code.
      → confirmed, in `pipeline.rs`: builds a detector via `categories_with_vertical` (so its categories are
      shaped exactly as `load_detector` would build them for a vertical) but leaves `config.vertical` itself
      `None`, so `VerticalConfig::validate` cannot be what catches the bad label — only the pre-existing,
      unmodified `validate_ner_labels_for_pseudonymize` can, and does (`PiiError::InvalidEntityLabel`).
- [x] Config round-trip: extend `hacienda-core/tests/config_round_trip.rs` so a TOML file with `[pii.vertical]` survives serialise → deserialise, and so a file **without** it still parses (the `deny_unknown_fields` direction is the risk).
      → confirmed: `should_round_trip_a_configured_vertical_through_toml` and
      `should_parse_a_pii_section_with_no_vertical_key_at_all`, both passing.

### 2.4 Surface exposure

- [x] **Do not add a `--vertical` flag.** With one `Option<VerticalConfig>` the flag could only confirm or reject the id already in the config file — a flag that cannot change behaviour. Adding it now would contradict this plan's own refusal of the `[[pii.verticals]]` registry as premature. The flag arrives with the registry, or not at all.
      → confirmed: no `--vertical` flag added; `hacienda-cli/src/cli.rs` untouched.
- [x] Extend `config show` (`hacienda-cli/src/commands.rs:169-181`) to print the active vertical id and label set with provenance, matching the existing `model_dir` lines. This is the whole CLI surface for v1, and it is enough: the operator can see what is active.
      → confirmed: a `[pii.vertical]` block added right after `[pii.model]` in `print_config_text`, printing
      `id`/`labels` with `(from: config)` when set and `(not configured — no vertical is active)` otherwise.
      Manually verified against a `[pii.vertical]` TOML file — see the "Verification" notes below.
- [x] **`hacienda-api`:** `dto.rs:199` is an explicit allowlist of `PipelineConfig` fields, not a derived `Serialize`. Decide whether the vertical is exposed over HTTP and act deliberately — leaving it out is defensible, leaving it out *by forgetting the file exists* is not. Add the field to the allowlist and a DTO test either way, or add a comment recording the decision not to.
      → **decision: expose it.** Unlike `model_dir`/`lora_adapter_dir` (filesystem paths, host topology) a
      vertical's id and labels describe *what the pipeline is configured to detect*, which an API client
      integrating against `/v1/pii/redact` benefits from knowing, and neither is secret. Added
      `vertical_id: Option<String>` and `vertical_labels: Vec<String>` to `PiiConfigResponse`
      (`hacienda-api/src/dto.rs`), populated in `handlers/pii.rs::pii_config`, with two new HTTP-level tests in
      `handlers/pii.rs` (`should_report_no_active_vertical_when_none_is_configured`,
      `should_report_the_active_vertical_id_and_labels`) that build a real `axum::Router` via
      `routes::build_router` and assert on the JSON response — both pass.
- [x] **wasm — amend the spec, do not build anything.** Verified: `hacienda-core` on wasm32 *does* compile xberg's Candle backend (via `ner-candle-wasm`), but both `NerDetector::from_candle_local` and the real `load_detector` are gated `not(target_arch = "wasm32")`, so `load_detector` on wasm always returns `ModelUnavailable`. hacienda-core cannot run a model in the browser under any feature combination today; Studio reaches `xberg-wasm`'s `NerModel` directly instead (commit `c322cad`). Spec §4.1's "works identically on native and wasm32" is therefore false as plumbing — Tier 0 is native-only until that gate is opened. Correct §4.1 and add it to spec §9 Open Questions. **Do not** widen the cfg as part of this plan: making hacienda-core load a 614 MB model in a browser is spec §6's blocker, not a checkbox here.
      → confirmed: spec §4.1 already carried the corrective language (verified byte-for-byte present before
      this task started — likely written alongside this plan document itself). Added a new §9 Open Questions
      item 6 ("When does Tier 0 actually reach the browser?") cross-referencing §4.1's correction, per this
      checklist's explicit instruction to add it there too. No `cfg` gate touched; no wasm code touched.

### 2.5 API compatibility

- [x] `VerticalConfig` and `PipelineConfig::vertical` are **public API additions**. Add them to `CHANGELOG.md` under `[Unreleased] / Added`, per the repo's api-compatibility rule.
      → confirmed: entry added to `CHANGELOG.md`'s `[Unreleased] / Added`, describing the new types, the
      extend-not-replace behaviour, the unconditional validation site, the CLI/API surface, and the semver
      decision below.
- [x] `PipelineConfig` has public fields and no `#[non_exhaustive]` (there is none anywhere in `hacienda-core`). Adding a field breaks any external struct-literal construction. Either mark it `#[non_exhaustive]` now — cheap, and it makes every future field additive — or accept and document the semver consequence. Decide explicitly; do not add the field silently.
      → **decision: do not mark it `#[non_exhaustive]`.** Kept consistent with the rest of `hacienda-core`,
      where the attribute is used nowhere today — introducing it on exactly this one struct, in this one PR,
      would be a bigger and less-reviewed precedent-setting change than the field addition itself, and Task
      2.5 asks for a decision on *this* field addition, not a crate-wide policy change. The semver consequence
      (external `PipelineConfig { .. }` struct-literal construction without `..Default::default()` breaks on
      upgrade) is accepted and documented in the CHANGELOG entry above. `PipelineConfig::default()` and
      `..Default::default()` usage — the pattern used throughout this codebase's own tests — are unaffected.

**Acceptance:** all new tests pass — confirmed: `cargo test -p hacienda-core` (default features) is 298 passed
/ 4 failed / 2 ignored in the lib target (the 4 failures are the pre-existing `audit::store_file`/
`review::store_file` chmod-under-root failures named in this task's brief, not new; every other target —
`config_round_trip` 9/9, `ner_eval` 14/14 (+1 ignored), `pii_corpus` 1/1 — passes clean), and the same holds
under `--features ner-candle`. `cargo test -p hacienda-cli` (11+8+3+2 = 24/24) and `cargo test -p hacienda-api`
(20/20 + 3/3 safety) both pass, including the new CLI-adjacent and DTO tests. `cargo clippy -p hacienda-core
--all-targets --features ner-candle -- -D warnings` is clean. `hacienda config show` displays the vertical
(manually verified). CHANGELOG updated. The plan's literal phrase "`cargo test --workspace` count equals Task
0's baseline plus the new tests, with zero pre-existing failures" is **not achievable as written** — Task 0's
baseline of 0 failures predates the 4 known chmod-under-root failures, which are a sandbox artifact unrelated to
this task and were already present (and separately documented as expected) before Task 2 started; "zero
*new* failures" is what was actually verified and is true.

---

## Task 3 — Vertical provenance in the audit chain (spec §8 step 3)

**Constraint discovered while planning, and the reason this task exists:** `config_hash` is operator-supplied and *chain-scoped* — `AuditChain::append` refuses any entry whose `config_hash` differs from the chain's. The vertical therefore cannot be folded into `config_hash` unless it is guaranteed constant for the life of a chain. Following the `principal` precedent instead gives a per-entry, chain-hashed field with backward-compatible verification.

**Decision, made here rather than deferred:** the recorded value is `"<id>@<first 8 hex of blake3 over the
sorted, case-folded label set>"`, e.g. `finance@3f9a1c02`. An id alone is a false provenance claim — the same id
with different labels detects different things, and the audit record's job is to say what *was* detectable. The
digest makes a silently-edited label set visible. Every step below uses this composed value; `None` when no
vertical is configured.

- [x] Add `#[serde(default)] pub vertical: Option<String>` to `AuditEntry` and `pub vertical: Option<String>` to `AuditEntryInput`, documented like `principal`, with a rustdoc note that the value is `id@digest` and why.
      → confirmed: both added in `hacienda-core/src/audit/entry.rs`, doc comment mirrors `principal`'s
      structure (what it is, that it's covered by `compute_chain_hash`, that `None` hashes as `""` so old
      chains still verify) and additionally explains why the value is `id@digest` rather than the bare id.
- [x] Add `VerticalConfig::provenance_id(&self) -> String` producing that value, unit-tested for stability under label reordering and case changes, and for *instability* when a label is added.
      → confirmed, in `hacienda-core/src/pii/config.rs`, next to `validate`: lowercases and sorts labels,
      hashes each with a `\0` separator via `blake3::Hasher`, formats `"{id}@{first 8 hex}"`. Four unit tests:
      `should_produce_a_stable_provenance_id_regardless_of_label_order`,
      `..._regardless_of_label_case`, `should_change_the_provenance_id_when_a_label_is_added` (the plan's
      required name), and `should_prefix_the_provenance_id_with_the_vertical_id` (format sanity: `finance@`
      prefix, 8 lowercase hex digest chars).
- [x] Add `pub vertical: Option<&'a str>` to `ChainHashFields`, populate it in `AuditEntry::new` and `chain_hash_fields`, and hash it in `compute_chain_hash` as `fields.vertical.unwrap_or("")` — **appended after `principal`**, so chains written before this field existed hash byte-for-byte identically.
      → confirmed: field added; populated in both `AuditEntry::new`'s `ChainHashFields` literal and
      `chain_hash_fields()`; `compute_chain_hash` hashes `fields.vertical.unwrap_or("")` as the very next
      `hasher.update` call after `fields.principal.unwrap_or("")`, with a comment warning never to reorder the
      two. Pinned by `should_hash_an_absent_vertical_as_no_bytes` (mirrors the existing principal test) and,
      decisively, by the literal-backed backward-compatibility test below.
- [x] Populate it at both `AuditEntryInput` construction sites in `facade.rs` from the pipeline's configured vertical id.
      → confirmed: added `pub(crate) fn vertical_provenance_id(&self) -> Option<String>` on `PiiPipeline`
      (`hacienda-core/src/pii/pipeline.rs`, next to the existing `pub fn config(&self) -> &PipelineConfig`
      accessor — which, contrary to this task's brief text, already existed on `PiiPipeline`; the new method is
      a thin, testable wrapper around it: `self.config.vertical.as_ref().map(VerticalConfig::provenance_id)`).
      Both `record_reveal` (~facade.rs:678) and `record_audit` (~facade.rs:735) now compute
      `self.pii_pipeline.as_ref().and_then(|p| p.vertical_provenance_id())` once per call and clone it into
      every `AuditEntryInput` in their respective loops.
- [x] Add the column to `audit/export.rs`'s CSV header and row. **This is a breaking change to the export format, and the export declares no version** — verified: `export_csv` writes a bare literal header at `export.rs:44-46` with nothing to bump. Append the column last, state the break in `CHANGELOG.md`, and check any downstream parser before merging.
      → confirmed: `vertical` appended as the last CSV column in both the header literal and the row's field
      slice, with a comment explaining why "appended last" matters for positional parsers. Grepped the whole
      workspace for CSV consumers of `export_csv`/`audit::export::export`: none exist outside `export.rs`'s own
      tests, so there is no in-repo downstream parser to update. Breaking change stated in `CHANGELOG.md`.
- [x] Tests:
      - `should_verify_a_chain_written_before_the_vertical_field_existed` — construct entries with `vertical: None` and assert the chain hash equals a **hard-coded literal captured before the change**. Capture that literal in Task 0. Without it this test proves nothing.
        → confirmed, in `entry.rs`, using the original `input("id-1")` fixture unmodified (only extended with
        `vertical: None`) and asserting `entry.chain_hash == "72eb1d2f14c701e0c58280b2d7fc5132fdc0564a3ed42e7b0c4b84cfdd5a3ee4"` — Task 0's exact literal. **Passes.**
      - `should_change_the_chain_hash_when_the_vertical_changes` — same inputs, different vertical id, different hash.
        → confirmed, in `entry.rs`: three-way comparison (no vertical vs `finance@3f9a1c02` vs
        `finance@aaaaaaaa`), all pairwise distinct.
      - `should_reject_a_tampered_vertical_id` — rewrite `vertical` on a serialised entry, assert verification fails.
        → confirmed, but placed in `chain.rs` rather than `entry.rs`, mirroring the existing
        `should_detect_a_tampered_entry` chain-level test exactly (push an entry with a vertical, mutate
        `chain.entries[0].vertical` directly, assert `chain.verify()` returns `AuditError::ChainIntegrity` at
        index 0) — this exercises the real, user-facing verification API rather than manually recomputing a
        hash, which is a more faithful test of "verification fails."
      - Round-trip an entry with `vertical: Some(..)` through JSON and CSV.
        → confirmed: `should_round_trip_an_entry_with_a_vertical_through_json` (`entry.rs`) and
        `should_round_trip_an_entry_with_a_vertical_through_csv` (`export.rs`), plus
        `should_deserialize_an_entry_with_no_vertical_key_at_all` (`entry.rs`, the `#[serde(default)]`
        direction) and `should_export_an_absent_vertical_as_an_empty_csv_field` /
        `should_append_the_vertical_column_last_in_the_csv_header` (`export.rs`).
      - `should_change_the_provenance_id_when_a_label_is_added` — same id, one extra label, different digest.
        → confirmed, in `config.rs` next to `provenance_id`'s other unit tests (see above).
- [x] `ChainHashFields` is `pub` with `pub` fields and no `#[non_exhaustive]`; adding a field breaks external construction. Same decision as Task 2.5, and here the case for `#[non_exhaustive]` is stronger, because this struct is *designed* to grow (its own rustdoc calls adding a field "a deliberate, reviewable act"). Add it, and note the semver impact in CHANGELOG.
      → confirmed: `#[non_exhaustive]` added to `ChainHashFields` with a rustdoc paragraph explaining why
      (contrast with the Task 2.5 `PipelineConfig` decision: nothing outside this crate constructs
      `ChainHashFields` today — grepped and confirmed — so the attribute costs nothing now and forces every
      future external caller through a crate-provided constructor). Semver impact noted in `CHANGELOG.md`.
- [x] Add `AuditEntry.vertical` and the CSV column to `CHANGELOG.md`.
      → confirmed: one `[Unreleased] / Added` entry covering `AuditEntry.vertical` /
      `AuditEntryInput.vertical`, `provenance_id`'s format, the hash-order/backward-compatibility guarantee,
      the CSV breaking change, and the `ChainHashFields` `#[non_exhaustive]` semver note.

**Acceptance:** old-chain verification test passes against the literal captured in Task 0 — **confirmed**:
`should_verify_a_chain_written_before_the_vertical_field_existed` passes, asserting exactly
`"72eb1d2f14c701e0c58280b2d7fc5132fdc0564a3ed42e7b0c4b84cfdd5a3ee4"`. Tamper test
(`should_reject_a_tampered_vertical_id`) fails verification as expected (`AuditError::ChainIntegrity` at index
0). Export round-trips (JSON and CSV, both directions, including the no-vertical-key-at-all and
absent-vertical-as-empty-CSV-field cases). CHANGELOG updated. `cargo test -p hacienda-core` (default features)
is 311 passed / 4 failed (the same four pre-existing chmod-under-root `audit::store_file`/`review::store_file`
failures named in Task 2's acceptance, not new) / 2 ignored in the lib target, with `config_round_trip` 9/9,
`ner_eval` 14/14 (+1 ignored), and `pii_corpus` 1/1 all clean — 311 is exactly Task 2's 298 plus this task's 13
new lib-level tests. Identical result under `--features ner-candle`. `cargo clippy -p hacienda-core
--all-targets --features ner-candle -- -D warnings` is clean. `cargo test -p hacienda-cli -p hacienda-api` is
20+3+11+8+3+2 = 47/47 passing (facade.rs's two call sites compile and exercise cleanly through both crates).
Also updated the seven other in-crate `AuditEntryInput` struct-literal fixtures (`chain.rs`, `segment.rs`,
`sink.rs`, `store.rs`, `store_file.rs`, plus `export.rs`'s own) and the three `hacienda-wasm` construction sites
(`crates/hacienda-wasm/src/lib.rs`, `tests/wasm.rs`, `tests/idb.rs`) to carry the new field — `AuditEntryInput`
has no `Default` impl, so every base fixture needed the field added explicitly; call sites using
`..base_fixture()` struct-update syntax needed no change. The `hacienda-wasm` edits could not be verified by
`cargo check`/`cargo test` in this sandbox (no `wasm32-unknown-unknown` target installed, and the requested
verification matrix does not include this crate), but the edits are the same mechanical one-field addition
made everywhere else, following the file's own existing comment style.

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
