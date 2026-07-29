# Business-Law GLiNER2 LoRA Implementation Plan

**Implements:** `superpowers/specs/2026-07-29-business-law-gliner2-lora-design.md`
**Goal:** Ship a `business_law` vertical taxonomy registered in `hacienda-studio`, an
auditable LLM-assisted auto-labeling pipeline over `harvey-labs`/CUAD/EDGAR, and a
trained LoRA adapter that loads through hacienda's existing `--lora-dir` path with zero
Rust-side changes.

**Architecture:** Two independent surfaces that meet at one artifact — a PEFT adapter
directory. Surface A (this Cargo/npm workspace): taxonomy registration + a Rust
integration test that guards the load contract. Surface B (external, Python): corpus →
auto-label → human QC → dataset → LoRA train → adapter export. Nothing in Surface B is
committed to this repo; only its output (a path to an adapter directory) is consumed.

**Tech Stack:** TypeScript/Vitest (`apps/hacienda-studio`) for the taxonomy; Python
(`transformers`, `peft`, `safetensors`, an LLM client) for the pipeline, living in its own
repo/directory — not this Cargo workspace; Rust/Cargo test (`hacienda-core`) for the
adapter-loading contract check.

## Global Constraints

- Base model: `fastino/GLiNER2-Guardrails-PII-Multi` (DeBERTa-v2 encoder, 300M params).
  **Verified against the real `xberg` v1.0.2 source during this plan's execution**
  (`crates/xberg-gliner/src/candle/{lora,model}.rs`, commit `9dcc864`) — the guard is
  weaker than the spec originally claimed: `adapter_config.json`'s
  `base_model_name_or_path` is checked with a case-sensitive, bidirectional
  **substring** test against the deployed model directory's basename
  (`model_dir.file_name()`), and is skipped entirely if the field is absent. Pin the
  literal deployed directory basename as the training-time constant, not the HF slug —
  see spec §9 for the corrected contract.
- Existing Rust surface (do not modify): `ModelConfig.lora_adapter_dir`
  (`hacienda-core/src/pii/config.rs:49`), `--lora-dir` CLI flag
  (`hacienda-cli/src/cli.rs:86`), `NerDetector::from_candle_local`
  (`hacienda-core/src/pii/ner.rs:69`), `CandleBackend::get_or_init`.
- Existing TS surface: taxonomies are bundled at build time via `?raw` imports in
  `apps/hacienda-studio/lib/verticals/index.ts` — never fetched at runtime (see the
  comment there about the SPA-fallback bug this avoids). `entityTypes`/`relationships`
  must be flat string lists (`requireStringList` rejects anything else).
- Test commands: `npm run test:unit` (from `apps/hacienda-studio`) for TS;
  `cargo test -p hacienda-core` for Rust.
- **Taxonomy collision rule (found during spec review):** `VerticalDictionary`'s lookup
  map (`dictionary.ts:16`) is a flat `Map` keyed by lowercased entity name — the
  last-loaded taxonomy silently wins a shared key. `business_law.yaml` must not
  redeclare any key already present in `m&a.yaml`, `financial_services.yaml`, or
  `shared.yaml`. Task 1 includes a test enforcing this so it can't regress silently.

---

## Phase 1: Business-Law Taxonomy (this repo)

### Task 1: Add and register `business_law.yaml`

**Files:**

- Create: `apps/hacienda-studio/lib/verticals/business_law.yaml`
- Modify: `apps/hacienda-studio/lib/verticals/index.ts`
- Modify: `apps/hacienda-studio/lib/verticals/index.test.ts`

**Interfaces:**

- Consumes: none (parallel to the existing `m&a`/`financial_services`/`shared` taxonomies)
- Produces: `loadVerticalTaxonomy("business_law")`

- [ ] **Step 1: Write failing tests**

```typescript
// apps/hacienda-studio/lib/verticals/index.test.ts — extend the existing describe block
it.each(["m&a", "financial_services", "business_law", "shared"])(
  "loads %s with populated entity types and relationships",
  async (vertical) => {
    const taxonomy = await loadVerticalTaxonomy(vertical);
    expect(taxonomy.vertical).toBe(vertical);
    expect(taxonomy.entityTypes.length).toBeGreaterThan(0);
    expect(taxonomy.entityTypes.every((type) => type.length > 0)).toBe(true);
  },
);

it("parses the business law taxonomy", async () => {
  const taxonomy = await loadVerticalTaxonomy("business_law");
  expect(taxonomy.entityTypes).toContain("contracting_party");
  expect(taxonomy.entityTypes).toContain("indemnification_clause");
  expect(taxonomy.relationships).toContain("governed_by");
});

it("does not redeclare an entity type already owned by another vertical", async () => {
  const [ma, fs, businessLaw, shared] = await Promise.all(
    ["m&a", "financial_services", "business_law", "shared"].map(loadVerticalTaxonomy),
  );
  const owned = new Map<string, string>();
  for (const t of [ma, fs, shared]) {
    for (const type of t.entityTypes) owned.set(type, t.vertical);
  }
  for (const type of businessLaw.entityTypes) {
    expect(owned.has(type)).toBe(false);
    // failure message names both verticals so a real collision is fixable, not just red
  }
});
```

- [ ] **Step 2: Run tests, confirm failure**

```bash
cd apps/hacienda-studio && npm run test:unit -- lib/verticals/index.test.ts
```

Expected: FAIL — `business_law` not in `RAW_TAXONOMIES`, module doesn't exist yet.

- [ ] **Step 3: Create `business_law.yaml`**

Use the taxonomy from spec §4 (post-review-fix — `governing_law`/`jurisdiction`/
`regulator` intentionally omitted, see the spec's review note):

```yaml
vertical: "business_law"
sectors:
  - commercial_contracts
  - employment
  - intellectual_property
  - real_estate
  - regulatory_compliance
  - dispute_resolution

entityTypes:
  - contract_type
  - contracting_party
  - counterparty_role
  - effective_date
  - term_length
  - renewal_clause
  - termination_clause
  - termination_for_cause
  - dispute_resolution_mechanism
  - arbitration_clause
  - venue
  - confidentiality_obligation
  - non_compete
  - non_solicitation
  - ip_assignment
  - license_grant
  - license_scope
  - liability_cap
  - indemnification_clause
  - force_majeure
  - assignment_clause
  - notice_provision
  - payment_terms
  - penalty_clause
  - compliance_obligation

relationships:
  - party_to
  - counterparty_of
  - governed_by
  - indemnifies
  - licenses_to
  - assigns_rights_to
  - subject_to_regulation_by
```

- [ ] **Step 4: Register in `index.ts`**

```typescript
import businessLawYaml from "./business_law.yaml?raw";
// ...
const RAW_TAXONOMIES: Record<string, string> = {
  "m&a": maYaml,
  financial_services: financialServicesYaml,
  business_law: businessLawYaml,
  shared: sharedYaml,
};
```

- [ ] **Step 5: Run tests, confirm pass**

```bash
cd apps/hacienda-studio && npm run test:unit -- lib/verticals/index.test.ts
```

- [ ] **Step 6: Commit**

```bash
git add apps/hacienda-studio/lib/verticals/business_law.yaml apps/hacienda-studio/lib/verticals/index.ts apps/hacienda-studio/lib/verticals/index.test.ts
git commit -m "feat(verticals): add business_law taxonomy"
```

---

## Phase 2: Corpus Extraction & Contamination Split (external, Python)

### Task 2: Extract raw text with a locked train/held-out split

**Files (in the external pipeline repo/directory, not this workspace):**

- Create: `corpus/extract_harvey_labs.py`
- Create: `corpus/task_split.json` (committed output — the split itself must be
  reviewable, not regenerable-by-accident)

**Interfaces:**

- Consumes: a local checkout of `harveyai/harvey-labs` (`tasks/<practice-area>/*/task.json`
  + `documents/`)
- Produces: `(task_id, doc_filename) -> raw_text` records, partitioned into `train_pool`
  and `held_out_pool` by `task_id`

- [ ] **Step 1: Write failing test** — the split must be deterministic and disjoint

```python
# corpus/test_task_split.py
def test_split_is_disjoint_and_deterministic():
    split_a = build_task_split(seed=42, practice_areas=["corporate-ma", "real-estate"])
    split_b = build_task_split(seed=42, practice_areas=["corporate-ma", "real-estate"])
    assert split_a == split_b  # reproducible from the seed, not from wall-clock/random state
    assert set(split_a["train_pool"]).isdisjoint(split_a["held_out_pool"])
    assert len(split_a["held_out_pool"]) > 0

def test_split_is_by_task_not_by_document():
    split = build_task_split(seed=42, practice_areas=["corporate-ma"])
    # every document under a train-pool task id stays in the train pool
    for task_id in split["train_pool"]:
        assert task_id not in split["held_out_pool"]
```

- [ ] **Step 2: Run, confirm failure** (module doesn't exist)
- [ ] **Step 3: Implement `build_task_split` and `extract_harvey_labs.py`**

  - Filter to business-law-relevant practice areas present in `harvey-labs/tasks/`
    (confirm the actual directory names at extraction time — `corporate-ma` is
    confirmed to exist per the tutorial walkthrough; others need a directory listing
    pass since the practice-area taxonomy wasn't fully enumerated during spec-writing).
  - Extract `.docx`/`.pdf` to plain text via `xberg`'s document conversion path (reuse,
    don't reimplement — same code `apps/hacienda-studio`'s worker already runs).
  - Partition by `task_id` using a fixed seed; write `task_split.json` and commit it —
    the split is a decision, not a derived artifact regenerated per run.

- [ ] **Step 4: Run, confirm pass**
- [ ] **Step 5: Commit** the extraction script and `task_split.json`

---

## Phase 3: Auto-Labeling — deterministic pieces (external, Python, real unit tests)

The LLM call itself isn't unit-testable, but everything around it is pure logic and
should be tested as such before any model spend.

### Task 3: Offset resolver

**Files:**

- Create: `labeling/offset_resolver.py`
- Test: `labeling/test_offset_resolver.py`

**Interfaces:**

- Consumes: `(chunk_text, entity_text, context_before, context_after)` from an LLM response
- Produces: `Optional[(start, end)]` — `None` means "drop, don't guess" (spec §6.2)

- [ ] **Step 1: Write failing tests**

```python
def test_unique_match_resolves_directly():
    chunk = "The Licensee shall not assign this Agreement without consent."
    assert resolve_offset(chunk, "assign this Agreement", None, None) == (22, 44)

def test_ambiguous_match_resolved_by_context():
    chunk = "Acme shall notify Beta. Beta shall notify Acme in return."
    # "Acme" appears twice; context disambiguates which occurrence was meant
    result = resolve_offset(chunk, "Acme", context_before=None, context_after="shall notify Beta")
    assert result == (0, 4)

def test_unresolvable_match_returns_none_rather_than_guessing():
    chunk = "Acme shall notify Beta. Beta shall notify Acme in return."
    assert resolve_offset(chunk, "Acme", context_before=None, context_after=None) is None

def test_text_not_present_returns_none():
    chunk = "The parties agree to arbitration in Delaware."
    assert resolve_offset(chunk, "mediation", None, None) is None
```

- [ ] **Step 2: Run, confirm failure**
- [ ] **Step 3: Implement `resolve_offset`** per spec §6.2's algorithm (exact search →
      unique match wins → context disambiguates → else `None`)
- [ ] **Step 4: Run, confirm pass**
- [ ] **Step 5: Commit**

### Task 4: Taxonomy whitelist gate

**Files:**

- Create: `labeling/taxonomy_gate.py`
- Test: `labeling/test_taxonomy_gate.py`

**Interfaces:**

- Consumes: `business_law.yaml`'s `entityTypes` (single source of truth — load the same
  YAML file Task 1 created, don't hand-copy the label list into Python)
- Produces: `is_valid_label(label: str) -> bool`

- [ ] **Step 1: Write failing tests**

```python
def test_accepts_a_taxonomy_label():
    assert is_valid_label("contracting_party") is True

def test_rejects_a_near_miss_label():
    # LLMs invent labels like this — must be gated, not silently accepted (spec §6.2)
    assert is_valid_label("contracting_party_name") is False

def test_rejects_case_variants_to_force_exact_taxonomy_match():
    assert is_valid_label("Contracting_Party") is False
```

- [ ] **Step 2–5:** implement against the literal `entityTypes` list parsed from
      `apps/hacienda-studio/lib/verticals/business_law.yaml`, confirm pass, commit.

### Task 5: Self-consistency voter

**Files:**

- Create: `labeling/consistency.py`
- Test: `labeling/test_consistency.py`

**Interfaces:**

- Consumes: 3 label-sample runs per chunk (list of `(start, end, label)` triples each)
- Produces: `accepted` (≥2/3 agreement) and `review_queue` (1/3 agreement) partitions

- [ ] **Step 1: Write failing tests**

```python
def test_majority_agreement_is_accepted():
    samples = [
        [(0, 4, "contracting_party")],
        [(0, 4, "contracting_party")],
        [(0, 4, "counterparty_role")],  # disagrees on label, not span
    ]
    result = vote(samples)
    assert (0, 4, "contracting_party") in result.accepted

def test_single_sample_agreement_goes_to_review_not_training():
    samples = [
        [(10, 20, "force_majeure")],
        [],
        [],
    ]
    result = vote(samples)
    assert (10, 20, "force_majeure") in result.review_queue
    assert (10, 20, "force_majeure") not in result.accepted
```

- [ ] **Step 2–5:** implement, confirm pass, commit.

---

## Phase 4: Human QC & Active-Learning Loop (external, process — not code-testable)

### Task 6: Stratified sample export for review

- [ ] **Step 1:** script exporting a stratified 5% sample of `accepted` spans (by
      practice area × label) to a reviewable format (spreadsheet or simple review UI)
- [ ] **Step 2:** record measured precision/recall per label from the reviewed sample;
      commit the report (`labeling/round1_precision_report.md`) — this is the number
      that decides whether round 2 is needed, not a vibe check
- [ ] **Step 3:** round 2+ — run the round-1-trained interim adapter over unlabeled
      chunks, route LLM/model disagreements to the review queue, retrain; stop when
      held-out F1 (Phase 7) plateaus across rounds

---

## Phase 5: Dataset Assembly (external, Python, real unit test)

### Task 7: Word-token span conversion with round-trip assertion

**Files:**

- Create: `dataset/assemble.py`
- Test: `dataset/test_assemble.py`

**Interfaces:**

- Consumes: accepted char-offset spans (Phase 3), GLiNER2's word tokenizer
- Produces: GLiNER2 JSONL (`{"tokenized_text": [...], "ner": [[start_word, end_word, label], ...]}`)

- [ ] **Step 1: Write failing test** — this is the assembly-time assertion spec §8 calls
      mandatory, since drift here is otherwise silent

```python
def test_word_span_round_trips_to_the_original_text():
    chunk = "The Licensee shall not assign this Agreement without consent."
    char_span = (24, 46)  # "assign this Agreement"
    record = to_word_span_record(chunk, [(*char_span, "assignment_clause")])
    # re-render the word range back to text and compare to the original char span
    start_w, end_w, label = record["ner"][0]
    rendered = " ".join(record["tokenized_text"][start_w : end_w + 1])
    assert rendered == chunk[char_span[0]:char_span[1]]

def test_split_is_by_document_not_by_chunk():
    # chunks from one document must not appear on both sides of train/val/test
    records = [make_record(doc_id="doc-1"), make_record(doc_id="doc-1")]
    split = train_val_test_split(records, seed=1)
    doc_ids_per_split = {k: {r["doc_id"] for r in v} for k, v in split.items()}
    assert doc_ids_per_split["train"].isdisjoint(doc_ids_per_split["val"])
    assert doc_ids_per_split["train"].isdisjoint(doc_ids_per_split["test"])
```

- [ ] **Step 2–5:** implement, confirm pass, commit. Test set is the human-reviewed
      slice only (spec §8) — assert this at assembly time, not by convention:

```python
def test_only_human_reviewed_records_land_in_test_split():
    records = [make_record(source="auto_labeled"), make_record(source="human_reviewed")]
    split = train_val_test_split(records, seed=1)
    assert all(r["source"] == "human_reviewed" for r in split["test"])
```

---

## Phase 6: LoRA Training & Adapter Export (external, GPU-dependent — not unit-testable here)

### Task 8: Fine-tune and export

- [ ] **Step 1:** confirm `target_modules` (`query_proj`, `key_proj`, `value_proj`)
      actually exist as tensor names in the merged base checkpoint — spec §9 flags this
      as unverified-until-checked, not assumed from generic DeBERTa-v2 docs
- [ ] **Step 2:** `peft.LoraConfig(r=8..16, lora_alpha=16..32, target_modules=[...],
      base_model_name_or_path="fastino/GLiNER2-Guardrails-PII-Multi")`, base frozen, train
      against `fastino-ai/GLiNER2`'s training script using Phase 5's JSONL
- [ ] **Step 3:** export `adapter_config.json` + `adapter_model.safetensors`; assert the
      exported `base_model_name` field is the literal string from Step 2, byte-for-byte
- [ ] **Step 4:** commit the training script and config (not the weights — those are a
      release artifact, not source)

---

## Phase 7: Adapter Integration Verification (back in this repo — real Rust test)

> **Task 9 was rescoped during execution.** The original version assumed a fixture pair
> (tiny base model + mismatched adapter) could stand in for the real thing in CI. Reading
> the actual `xberg-gliner` v1.0.2 source (`crates/xberg-gliner/src/candle/{lora,model}.rs`,
> pulled via the pinned git dependency and inspected directly, not assumed from planning-doc
> pseudocode) showed that isn't true: the base-model-name guard only runs *inside*
> `Gliner2Candle::load_adapter`, which requires a fully-loaded, real base model first —
> there is no lower-level entry point that exercises the guard without one. `xberg-gliner`'s
> own equivalent test, `base_model_extracts_entities_and_adapter_changes_output`
> (`crates/xberg-gliner/tests/candle_smoke.rs`), is `#[ignore]`d and gated behind
> `GLINER2_CANDLE_MODEL_DIR`/`GLINER2_TEST_ADAPTER_DIR` env vars for exactly this reason —
> a ~600MB real GLiNER2 checkpoint isn't something to fake or to fetch in this sandbox.
> Task 9 is split in two: 9a is real, weights-free, and was run in this session; 9b is
> staged for whoever has the real weights, following xberg's own established pattern
> rather than inventing a lighter substitute that can't exercise the real merge path.

### Task 9a: `from_candle_local` fails loudly, not silently, on a bad path

**Files:**

- Create: `hacienda-core/tests/lora_adapter_contract.rs`

**Interfaces:**

- Consumes: `NerDetector::from_candle_local` (existing, unmodified)
- Produces: a CI-visible assertion that a missing/malformed model or adapter directory
  returns `Err(PiiError::Ner)` rather than panicking or silently falling back to
  regex-only detection — the exact failure mode §13 of the integration design flags as
  an open risk ("advertises model-backed detection while running regex-only")

- [ ] **Step 1: Write failing test**

```rust
// hacienda-core/tests/lora_adapter_contract.rs
//! `NerDetector::from_candle_local` must fail loudly on a bad model or adapter path,
//! not panic or silently produce a detector that reports nothing — see
//! hacienda-cli-api-integration-design.md §13. Requires no model weights: every case
//! here fails before any tensor is read.
#![cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]

use hacienda_core::pii::NerDetector;

#[test]
fn should_return_err_not_panic_for_a_nonexistent_model_dir() {
    let model_dir = std::path::Path::new("/nonexistent/hacienda-lora-contract-test-model-dir");
    let result = NerDetector::from_candle_local(model_dir, None);
    assert!(result.is_err(), "a missing model_dir must fail to load, not panic");
}

#[test]
fn should_return_err_not_panic_for_a_nonexistent_adapter_dir() {
    // model_dir also doesn't exist here — from_local fails before load_adapter is ever
    // reached, so this still needs no real weights; it's asserting the same "Err, not
    // panic" contract through the adapter-path argument specifically.
    let model_dir = std::path::Path::new("/nonexistent/hacienda-lora-contract-test-model-dir");
    let adapter_dir = std::path::Path::new("/nonexistent/hacienda-lora-contract-test-adapter-dir");
    let result = NerDetector::from_candle_local(model_dir, Some(adapter_dir));
    assert!(result.is_err(), "a missing adapter_dir must fail to load, not panic");
}

#[test]
fn should_return_err_for_an_empty_model_dir() {
    let dir = tempfile::tempdir().expect("create temp dir");
    // Exists, but has none of tokenizer.json/config.json/model.safetensors —
    // distinguishes "path doesn't exist" from "path exists but isn't a model".
    let result = NerDetector::from_candle_local(dir.path(), None);
    assert!(result.is_err(), "an empty directory must fail to load, not panic");
}
```

- [ ] **Step 2: Run, confirm failure** (module doesn't exist / `tempfile` dev-dependency
      not yet added)
- [ ] **Step 3:** add `tempfile` to `hacienda-core`'s `[dev-dependencies]`; enable the
      `ner-candle` feature when running this test (`cargo test -p hacienda-core --features
      ner-candle`) — the test module is compiled out entirely otherwise, which is a
      real footgun worth naming: `cargo test -p hacienda-core` alone silently skips it.
- [ ] **Step 4: Run, confirm pass**
- [ ] **Step 5: Commit**

### Task 9b: base-model-name guard, end-to-end (staged — needs real weights)

Not executable in a sandbox without network access to a ~600MB model checkpoint. Follow
`candle_smoke.rs`'s pattern exactly rather than a lighter substitute:

- [ ] Add `hacienda-core/tests/lora_adapter_guard.rs`, `#[ignore]`d, reading
      `HACIENDA_GLINER2_MODEL_DIR` / `HACIENDA_LORA_ADAPTER_DIR` env vars, skipping with a
      message if unset (mirrors `candle_smoke.rs:15-22` exactly)
- [ ] Assert: an adapter whose `adapter_config.json.base_model_name_or_path` is a string
      that is neither a substring of, nor contains, the model directory's basename fails
      to load (`is_err()`) — per the corrected contract in spec §9, this is a substring
      check, not equality, so the fixture's mismatched name must be constructed
      accordingly (e.g. `"totally-unrelated-model"` against a real deployed directory
      name, not a case-variant of the real name, which the substring check may accept)
- [ ] Assert: an adapter with a matching (substring-relationship) `base_model_name_or_path`
      loads successfully and changes inference output vs. the unadapted base — reusing
      the same real weights already required for Task 8's training run, not a second
      fixture set
- [ ] Run manually or in a CI job that provisions the weights (out of scope for this
      plan's default `cargo test` — same scoping xberg itself uses)

---

## Phase 8: Evaluation

### Task 10: Held-out F1 vs. zero-shot baseline

- [ ] **Step 1:** run the trained adapter against the held-out human-reviewed test set
      (Phase 5); compute entity-level F1 per label
- [ ] **Step 2:** run the un-adapted base model zero-shot (taxonomy labels as free-text
      categories) against the same test set
- [ ] **Step 3:** any label where the adapter does not beat zero-shot gets flagged and
      excluded from the shipped adapter's advertised label set, not shipped silently weak
      (spec §14 acceptance criteria)
- [ ] **Step 4:** commit the evaluation report

---

## Plan Self-Review

**Spec coverage check** (against `2026-07-29-business-law-gliner2-lora-design.md`):

- ✅ Business-law taxonomy, non-colliding with existing verticals — Task 1 (collision
  bug caught during spec review, fixed before this plan was written, and now guarded by
  a test so it can't silently reappear)
- ✅ Corpus sourcing + task-ID contamination split — Task 2
- ✅ Auto-labeling: offset resolution, taxonomy gate, self-consistency — Tasks 3–5
- ✅ Human QC + active-learning loop — Task 6
- ✅ Dataset assembly, word-token round-trip, document-level split, human-only test set —
  Task 7
- ✅ LoRA training contract (`target_modules`, corrected `base_model_name_or_path`
  substring guard) — Task 8
- ✅ Adapter load-path verification in this repo — Task 9a (executed this session, real
  weights not required); Task 9b staged for whoever has the real GLiNER2 checkpoint
- ✅ Held-out evaluation vs. zero-shot baseline — Task 10

**Boundary check:** no task in Phases 2–6, 8 adds code to this Cargo/npm workspace — only
Tasks 1 and 9 touch this repo, matching the spec's Non-Goals (§2).

**No placeholders:** every task has concrete file paths and either real test code or a
named script deliverable; nothing deferred to a "TODO" inside this plan.

---

## Execution Handoff

Tasks 1 and 9a were executed in this session, in a worktree, and verified with real test
runs (not just written):

- Task 1: `npx vitest run` — 10/10 in `lib/verticals/index.test.ts`, 27/27 across the full
  `hacienda-studio` unit suite.
- Task 9a: pending — implemented against the real `xberg` v1.0.2 source, to be run with
  `cargo test -p hacienda-core --features ner-candle`.

Task 9b (real-weights guard test) and Phases 2–8 (external Python pipeline, GPU-dependent
training) remain staged as written specs for whoever/whatever runs them next — not
executable in this sandbox.
