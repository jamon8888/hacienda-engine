# Business-Law GLiNER2 LoRA: Corpus, Auto-Labeling & Training Design

**Date:** 2026-07-29
**Status:** Proposed
**Author:** hacienda-core team
**Depends on:** `superpowers/specs/2026-07-27-vertical-ner-architecture-design.md` (fills its
"Fine-tuned GLiNER2 per vertical — Q4 2026, when training data available" row),
`superpowers/specs/2026-07-28-hacienda-cli-api-integration-design.md` §11–12.4 (LoRA load
path, adapter-aware routing)

---

## 1. Problem Statement

hacienda already has a working LoRA **merge** path — it has never had a **training** path.

- `hacienda-core/src/pii/config.rs`'s `ModelConfig.lora_adapter_dir` and
  `hacienda-cli/src/cli.rs`'s `--lora-dir` flag exist today.
- `NerDetector::from_candle_local` (`hacienda-core/src/pii/ner.rs:69`) calls
  `xberg::text::ner::candle::CandleBackend::get_or_init(model_dir, lora_adapter_dir)`,
  which loads a PEFT-format adapter and merges it into the base GLiNER2 weights.
- There is no adapter that exists to put in that directory for business law, and no
  pipeline that produces one.

The 2026-07-27 vertical NER design added an M&A/Corporate-Law and Financial-Services
taxonomy but scoped fine-tuning out ("when training data available"). This spec is that
missing half: how a `business_law` LoRA adapter gets *trained*, using LLM-assisted
auto-labeling over `harveyai/harvey-labs` (MIT-licensed) plus other corpora, in a shape that
plugs into the loader above **unchanged**.

## 2. Goals / Non-Goals

**Goals**

- Produce one PEFT LoRA adapter directory (`adapter_config.json` + `adapter_model.safetensors`)
  that `CandleBackend::get_or_init` loads with zero changes to this repo's Rust code.
- Add a `business_law` vertical taxonomy, registered the same way `m&a` and
  `financial_services` already are.
- Define a repeatable, auditable data pipeline — not a one-off script — since the taxonomy
  and label set will drift as verticals are added.

**Non-Goals**

| Deferred | Reason |
|---|---|
| Training code living in this Cargo workspace | hacienda does not implement inference or training (`hacienda-cli-api-integration-design.md` §10); the pipeline is Python and lives in a separate repo/directory, out of scope here. |
| Modifying `xberg-gliner`'s Candle backend or `LoraAdapter` parsing | We do not patch xberg (same policy as §11.2 of the integration design). The adapter must conform to the existing parser, not the other way round. |
| Multi-tenant adapter routing infrastructure | Already designed in §12.4 of the integration design; this spec only flags where a `business_law` adapter becomes one more entry in that routing problem. |
| Real (non-synthetic) client documents | Out of scope for licensing and privilege reasons; see §6. |

## 3. Architecture Overview

```text
┌─────────────────────────────────────────────────────────────────────────────┐
│                     BUSINESS-LAW LoRA TRAINING PIPELINE  (Python, external)  │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  Corpus Sources          Chunking          LLM Auto-Labeler     QC Gate      │
│  ┌────────────┐        ┌──────────┐       ┌──────────────┐   ┌──────────┐  │
│  │ harvey-labs│───────▶│ sentence-│──────▶│ Claude, tool  │──▶│ offset + │  │
│  │ (synthetic)│        │ aligned, │       │ call, 3x      │   │ label    │  │
│  │ CUAD       │        │ overlap  │       │ self-consist. │   │ whitelist│  │
│  │ EDGAR      │        │ windows  │       │ sampling      │   │ validator│  │
│  └────────────┘        └──────────┘       └──────────────┘   └────┬─────┘  │
│                                                                     │        │
│                     ┌───────────────────────────────────────────────┘        │
│                     ▼                                                       │
│         Human Spot-Check (stratified 5%)  ──▶  Active-Learning Loop         │
│                     │                              (round 2+)               │
│                     ▼                                                       │
│         Dataset Assembly (word-token spans, GLiNER2 JSONL, train/val/test)  │
│                     │                                                       │
│                     ▼                                                       │
│         PEFT LoRA fine-tune (fastino-ai/GLiNER2 + peft, base frozen)        │
│                     │                                                       │
│                     ▼                                                       │
│      adapter_config.json + adapter_model.safetensors  (PEFT standard shape)│
└──────────────────────────────────────┬────────────────────────────────────┘
                                        │  (this is the only artifact hacienda consumes)
                                        ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                    HACIENDA (Rust, unchanged by this spec)                  │
│                                                                              │
│  --lora-dir <path>  ──▶  ModelConfig.lora_adapter_dir                       │
│                     ──▶  NerDetector::from_candle_local(model_dir, dir)     │
│                     ──▶  CandleBackend::get_or_init(model_dir, dir)         │
│                     ──▶  LoraAdapter::from_bytes  (parses adapter_config.json│
│                          + adapter_model.safetensors, merges into base)     │
└─────────────────────────────────────────────────────────────────────────────┘
```

## 4. Business-Law Vertical Taxonomy

`m&a.yaml` covers deal mechanics; `business_law` covers the general commercial-contract
layer underneath it (the entities present in *any* commercial agreement, not just
M&A transactions) — deliberately non-overlapping so a document can carry both verticals'
tags without duplicate entity types.

**New file:** `apps/hacienda-studio/lib/verticals/business_law.yaml`

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

> **Review note (2026-07-29):** `governing_law`, `jurisdiction`, and `regulator` were
> deliberately dropped from this list — they already exist verbatim in `m&a.yaml`
> (`regulator` also in `shared.yaml`), and `VerticalDictionary`'s lookup map
> (`apps/hacienda-studio/lib/verticals/dictionary.ts:16`) is a flat `Map` keyed by
> lowercased entity name: whichever taxonomy is loaded last in `index.ts`'s array wins
> silently for a shared key. Redefining them here would make their vertical attribution
> depend on load order instead of being correct. They remain reachable through the
> existing taxonomies without duplication. `indemnification_clause` was kept distinct
> from m&a's `indemnification` rather than merged, since collapsing them would attribute
> every business-law indemnification clause to the `m&a` vertical — a real gap (business
> law and M&A doc taxonomies duplicate a concept with no shared owner), but resolving it
> properly is cross-vertical entity resolution, already scoped as future work in §15, not
> this spec.

**Registration** (mirrors the existing pattern in
`apps/hacienda-studio/lib/verticals/index.ts`, which bundles taxonomies at build time
rather than fetching them — see the comment there about the SPA-fallback bug this avoids):

```ts
import businessLawYaml from "./business_law.yaml?raw";
// ...
const RAW_TAXONOMIES: Record<string, string> = {
  "m&a": maYaml,
  financial_services: financialServicesYaml,
  business_law: businessLawYaml,
  shared: sharedYaml,
};
```

`entityTypes` and `relationships` must be flat string lists — `requireStringList` in
`index.ts` rejects anything else, which is exactly the bug class (`sectors: []` parsed as
the literal string `"[]"`) that comment documents. Any generated YAML must round-trip
through that same validator before being committed, not just through a YAML parser.

## 5. Corpus Sources & Contamination Discipline

| Source | Role | Caveat |
|---|---|---|
| `harveyai/harvey-labs` (`tasks/<practice-area>/*/documents/*`) | Primary raw text — synthetic `.docx`/`.pdf` business documents, lawyer-reviewed, MIT-licensed, no privilege/PII exposure. 1,671 tasks across 24+ practice areas. | Synthetic-only: templated phrasing and repeated entity names will bias a model trained on it alone. Must be mixed with real corpora below. |
| CUAD (510 commercial contracts, 41 clause types) | Real, already span-labeled — use directly for a subset of `business_law` labels that map onto CUAD's clause taxonomy (license grants, non-competes, indemnification, IP assignment), bypassing LLM labeling for that slice. | CUAD's label set doesn't cover the full `business_law` taxonomy (no `contract_type`/`effective_date`-style spans); it supplements, doesn't replace, the LLM-labeled set. |
| SEC EDGAR filings | Real-world governing-law / jurisdiction / party-role density, public domain. | Needs its own extraction step (already-structured HTML, not `.docx`/`.pdf`), and its own chunking pass. |

**Contamination rule:** partition `harvey-labs` **by task ID**, not by document, into a
training pool and a held-out pool, before any labeling starts. The held-out pool is never
touched by this pipeline. This exists so that a later "does entity-enriched extraction
improve agent performance on LAB tasks" evaluation has a pool that was genuinely never
seen by anything upstream of it — not a legal requirement (MIT permits training on it
outright), a methodological one.

Extraction from `.docx`/`.pdf` to plain text reuses `xberg`'s document conversion (the
same code path `apps/hacienda-studio`'s worker pipeline already uses for 97+ formats)
rather than a second bespoke parser.

## 6. Auto-Labeling Pipeline

### 6.1 Chunking

Sentence-aligned windows of ~300 tokens with 50-token overlap, tracking byte offset into
the source document. GLiNER2's DeBERTa-v2 encoder has a real context ceiling; overlap is
what catches entities that would otherwise straddle a window boundary, deduplicated by
offset after labeling.

### 6.2 Labeling request/response schema

Structured tool-call output, not freeform JSON-in-prose:

```json
// Response schema per chunk
{
  "entities": [
    {
      "text": "<verbatim substring — must appear exactly in the chunk>",
      "label": "<one of business_law.yaml entityTypes — rejected otherwise>",
      "context_before": "<3-5 words preceding, for disambiguation>",
      "context_after": "<3-5 words following>"
    }
  ]
}
```

The model is never asked for character offsets — it confabulates them past a sentence or
two. Offset resolution is deterministic and downstream of generation:

1. Search the chunk for exact occurrences of `text`.
2. Unique match → that's the offset. Multiple matches → disambiguate via
   `context_before`/`context_after`. Neither resolves it → drop the span; do not guess.
3. Reject any `label` outside the `business_law.yaml` whitelist. LLMs invent near-miss
   labels (`target_company_name` for `contracting_party`) that fragment the class
   distribution if not gated here.

### 6.3 Self-consistency and negative sampling

- Sample each chunk 3x (temperature ~0.3–0.5). Keep spans with ≥2/3 (offset, label)
  agreement for training; 1/3-agreement spans go to the human review queue, not into the
  training set directly.
- Deliberately retain a fraction of chunks labeled "no entities" (business-law documents
  are mostly boilerplate — notices, signature blocks, defined-terms sections) so the
  adapter doesn't learn to always predict something.

## 7. Human QC & Active-Learning Loop

- Round 1: stratified 5% sample of accepted spans (across practice area and label) reviewed
  by hand; report measured precision/recall per label before trusting the set. Agreement
  across samples reduces hallucination, it does not eliminate labels the LLM is
  *confidently and consistently* wrong about.
- Round 2+: train an interim adapter on round-1 data, run it over unlabeled chunks, and
  route chunks where the interim model disagrees with (or is low-confidence against) the
  LLM's labels into the human review queue. Add reviewed chunks to the training set and
  retrain. Stop when held-out F1 (§10) plateaus across rounds.

## 8. Dataset Assembly

GLiNER2 trains on **word-token spans**, not character offsets:

```json
{"tokenized_text": ["The", "Licensee", "shall", "not", "assign", "..."],
 "ner": [[1, 1, "contracting_party"], [4, 4, "assignment_clause"]]}
```

Char-offset spans from §6 must be remapped through the same word-splitter GLiNER2's own
preprocessing uses. This step has no error signal when it silently drifts by one token —
it just corrupts a slice of the training set — so a round-trip check (re-render each
span's word range back to text, compare to the original `text` field) is a required
assembly-time assertion, not an optional one.

Split: train/val/test by **document**, never by chunk (chunks from the same document leak
across a split otherwise). Test set is the human-reviewed slice only — never auto-labeled
data — since that's the only slice with a trustworthy label.

## 9. LoRA Training Contract — matching hacienda's existing loader

This is the section that has to be exactly right, because nothing downstream validates it
except a load-time failure.

> **Correction (2026-07-29, verified against source):** the original version of this
> section described a `base_model_name` field and an equality-based load-time guard,
> sourced from `superpowers/plans/01-core-pipeline.md`'s pseudocode — that file documents
> the superseded `pii-fastino` crate, not what `hacienda-core` actually calls today. The
> real contract, read directly from the `xberg` git dependency pinned at tag `v1.0.2`
> (`crates/xberg-gliner/src/candle/lora.rs` and `candle/model.rs`, commit `9dcc864`), is
> materially different and weaker than originally written here. Corrected below.

`xberg-gliner`'s `LoraAdapter::load` (`candle/lora.rs:84`) deserializes `adapter_config.json`
into:

```rust
pub struct LoraConfig {
    pub r: usize,
    pub lora_alpha: f64,
    pub target_modules: Option<Vec<String>>,
    pub base_model_name_or_path: Option<String>,  // note: _or_path, not base_model_name
    pub fan_in_fan_out: bool,
}
```

**The base-model guard is a bidirectional substring check against a directory basename, and
it is optional.** `Gliner2Candle::load_adapter` (`candle/model.rs:137`):

```rust
if let Some(adapter_base) = adapter.config.base_model_name_or_path.as_deref()
    && !self.model_id.contains(adapter_base)
    && !adapter_base.contains(&self.model_id)
{
    return Err(/* "adapter trained on '{adapter_base}', current model is '{model_id}'.
                   Refusing to merge; remove base_model_name_or_path ... to bypass." */);
}
```

Three consequences that change how training must set this field:

- **`model_id` is the base model directory's basename** (`model_dir.file_name()`,
  `candle/model.rs:199`), not a HuggingFace-style slug. A model loaded from
  `/models/gliner2-guardrails-pii-multi/` has `model_id =
  "gliner2-guardrails-pii-multi"` — nothing containing a `/` will ever match it exactly;
  containment is what has to hold, not equality.
- **The check is case-sensitive `str::contains` in both directions.** `adapter_config.json`
  must set `base_model_name_or_path` to a string that is a substring of the deployed model
  directory's basename, or vice versa — e.g. `"gliner2-guardrails-pii-multi"` (matching
  case, matching substring) works; the HF-style mixed-case slug
  `"fastino/GLiNER2-Guardrails-PII-Multi"` does **not** contain, and is not contained by,
  a lowercase directory name — it would be rejected as a false mismatch. Pin the exact
  deployed directory basename as the training-time constant, not the HF slug.
- **The guard is skipped entirely if the field is absent.** The error message itself says
  "remove `base_model_name_or_path` from `adapter_config.json` to bypass" — this is a
  documented, intentional escape hatch, not a hole. The training pipeline must set the
  field deliberately; omitting it by oversight silently disables the only cross-model
  sanity check that exists.

The one check that is unconditional, not opt-in, is structural: `merge_into_base`
(`candle/lora.rs:246`) rejects an adapter if any of its target modules has no matching key
in the base `model.safetensors` — "adapter targets module '{path}' but no matching key
found in base safetensors." This is what actually catches "trained against a different
architecture"; the name guard above only catches "pointed at a differently-named directory
with the same architecture."

Remaining requirements, unchanged from the original draft:

- **`target_modules`** must name modules that actually exist in the DeBERTa-v2 encoder
  (attention projections: `query_proj`, `key_proj`, `value_proj`) — verified against the
  real module names in the merged base checkpoint before training, not assumed from HF's
  generic DeBERTa docs. `merge_into_base`'s structural check (above) is what turns a wrong
  guess into a load-time error instead of a silent no-op.
- Weights export as standard PEFT-format `adapter_model.safetensors`, keys following
  `base_model.model.<module_path>.lora_{A,B}.weight`, **F32 only** — `lora.rs`'s loader
  rejects any other dtype outright (`candle/lora.rs:127`).
- Rank/alpha are training hyperparameters, not architectural constraints from hacienda's
  side — pick from standard ranges (`r=8–16`, `alpha=16–32`) and tune against §11's held-out
  F1, not against convention. `r=0` is rejected at load (`candle/lora.rs:94`).

Training itself: `peft.LoraConfig` against `fastino-ai/GLiNER2`'s own training script, base
weights frozen, only the LoRA A/B matrices updated.

## 10. Multi-Adapter Serving

Once `business_law` exists alongside future verticals, it becomes one more entry in the
problem `hacienda-cli-api-integration-design.md` §12.4 already designed for:
`CANDLE_BACKEND_CACHE` has no eviction, so every `(model, adapter)` pair merged stays
resident for the life of the process. Nothing in this spec changes that design — rendezvous
routing and admission control there already account for "more adapters than `m&a` +
`financial_services`." This spec just confirms `business_law` fits the same
`(model_dir, lora_adapter_dir)` cache key shape without a new case.

## 11. Evaluation Strategy

- **Primary:** entity-level F1 per label on the held-out human-reviewed test set (§8) —
  never on auto-labeled data, which would just measure agreement with the labeler.
- **Comparison:** same test set run through the un-adapted base
  `GLiNER2-Guardrails-PII-Multi` in zero-shot mode with `business_law.yaml`'s label
  names as free-text categories, to quantify the LoRA's actual lift over prompting the base
  model with the taxonomy.
- **Exploratory, secondary:** using the held-out `harvey-labs` task pool (§5) to check
  whether entity-enriched extraction changes agent performance on those tasks. This is an
  agentic-pipeline question, not an NER-accuracy one, and is not a gate for shipping the
  adapter.

## 12. Testing Strategy

- **Unit:** offset-resolution golden cases (unique match, ambiguous match resolved by
  context, unresolvable match dropped); taxonomy whitelist rejection; word-token
  round-trip assertion (§8); `business_law.yaml` passes the same `requireStringList`
  validation `index.test.ts` already runs for `m&a`/`financial_services`.
- **Integration (cheap, real weights not required):** `NerDetector::from_candle_local`
  must return `Err(PiiError::Ner)`, not panic, for a nonexistent or malformed
  `model_dir`/`lora_adapter_dir` — the load-failure-must-not-look-like-success concern
  §13 of the integration design flags as an open risk. This is checkable with no model
  weights at all and belongs in this repo's default `cargo test`.
- **Integration (requires real weights, not CI-default):** actually exercising
  `base_model_name_or_path`'s substring guard (§9) end-to-end needs a loaded ~600MB
  GLiNER2 base model plus a real PEFT adapter — `xberg-gliner`'s own equivalent test,
  `base_model_extracts_entities_and_adapter_changes_output`
  (`crates/xberg-gliner/tests/candle_smoke.rs`), is `#[ignore]`d and driven by
  `GLINER2_CANDLE_MODEL_DIR`/`GLINER2_TEST_ADAPTER_DIR` env vars for exactly this reason.
  hacienda should follow the same pattern rather than inventing a lighter-weight
  substitute that can't actually exercise the real merge path — see the implementation
  plan's Task 9 for the concrete split between what runs in default CI and what needs a
  weights-provisioned job.
- **Golden files:** a small hand-labeled document set per practice area, held out entirely
  from the auto-labeling loop, used as the stable regression target across training
  iterations.

## 13. Risks & Open Questions

| Risk | Mitigation |
|---|---|
| Synthetic-only training data biases the adapter toward `harvey-labs`'s templated phrasing | Mix with CUAD (real contracts) and EDGAR; do not ship an adapter trained on `harvey-labs` alone. |
| LLM labeler invents labels or hallucinates spans | Whitelist gate + self-consistency voting (§6); measured, not assumed, precision from human spot-check (§7). |
| Word-token offset drift during dataset assembly | Mandatory round-trip assertion (§8); silent by default otherwise. |
| `target_modules` naming mismatch against the real DeBERTa-v2 checkpoint | Verify against the actual merged base checkpoint's tensor names before training, not HF's generic docs. |
| Label taxonomy drift between `business_law.yaml` and future verticals | New entity types get a relationships/overlap review against `m&a.yaml`/`financial_services.yaml` before merge, same as this spec did in §4. |
| Adapter cache growth once more than 2 verticals exist | Already owned by `hacienda-cli-api-integration-design.md` §12.4; not re-solved here. |
| `base_model_name_or_path`'s guard is a case-sensitive substring check against a directory basename, and is skipped entirely if the field is omitted (§9) — weaker than "exact identifier match" | Pin the deployed model directory's literal basename as the training-time constant (not the HF slug); never omit the field from `adapter_config.json`; the structural module-match check in `merge_into_base` is the one guard that isn't optional. |

## 14. Acceptance Criteria

- [ ] `business_law.yaml` added and passing `index.test.ts`'s taxonomy validation
- [ ] Auto-labeling pipeline produces a JSONL dataset in GLiNER2's word-token span format
- [ ] Held-out human-reviewed test set exists, disjoint from all auto-labeled/training data
- [ ] Trained adapter's `adapter_config.json` sets `base_model_name_or_path` to the deployed
      model directory's literal basename (not the HF slug), verified by loading it through
      `LoraAdapter::load` against a real (non-CI, weights-provisioned) base model
- [ ] `NerDetector::from_candle_local` returns `Err(PiiError::Ner)` rather than panicking for
      a malformed model/adapter directory — verified in default CI with no model weights
      required (see implementation plan Task 9)
- [ ] Held-out F1 exceeds the zero-shot base-model baseline (§11) for every label in the
      taxonomy, or the label is flagged and excluded rather than shipped silently weak
- [ ] `harvey-labs` train/held-out task-ID split is documented and enforced by the pipeline,
      not by convention

## 15. Future Extensions

| Feature | Timeline | Notes |
|---|---|---|
| Real-estate / employment-law verticals reusing this same pipeline | Q1 2027 | Pipeline (§6–9) is taxonomy-agnostic; only §4's YAML and label set change per vertical. |
| Active-learning round automation (no manual retrain trigger) | Q1 2027 | Currently a manual loop per §7. |
| Cross-vertical entity resolution (`business_law` term also tagged `m&a`) | Q1 2027 | Needs the batch entity registry from the 2026-07-27 vertical NER design. |
