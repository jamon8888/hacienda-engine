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
  - governing_law
  - jurisdiction
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
  - regulator
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
except a load-time failure. `xberg-gliner`'s `LoraAdapter::from_bytes`
(`superpowers/plans/01-core-pipeline.md:1026`) deserializes `adapter_config.json` into:

```rust
pub struct LoraConfig {
    pub r: usize,
    pub lora_alpha: f64,
    pub target_modules: Option<Vec<String>>,
    pub fan_in_fan_out: bool,
    pub base_model_name: Option<String>,
}
```

Training-side requirements this implies:

- **`base_model_name`** must equal the exact identifier of the base weights hacienda loads
  (`fastino/GLiNER2-Guardrails-PII-Multi`) — this is "the base-model-name guard" referenced
  in `hacienda-cli-api-integration-design.md` §11.1; a mismatch is a deliberate load-time
  rejection, not a bug to work around.
- **`target_modules`** must name modules that actually exist in the DeBERTa-v2 encoder
  (attention projections: `query_proj`, `key_proj`, `value_proj`). This must be verified
  against the real module names in the merged base checkpoint before training, not assumed
  from HF's generic DeBERTa docs — a mismatched name fails silently at merge time in some
  PEFT-adjacent tooling rather than erroring.
- Weights export as standard PEFT-format `adapter_model.safetensors`, with LoRA A/B tensor
  keys following the `module_path.lora_A.weight` / `.lora_B.weight` convention that
  `parse_lora_key` (same file) expects.
- Rank/alpha are training hyperparameters, not architectural constraints from hacienda's
  side — pick from standard ranges (`r=8–16`, `alpha=16–32`) and tune against §10's held-out
  F1, not against convention.

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
- **Integration:** load a fixture adapter directory through the existing `--lora-dir` CLI
  flag / `ModelConfig.lora_adapter_dir` path in CI, asserting the `base_model_name` guard
  actually rejects a mismatched adapter — this is the one behavior from §9 the Rust side
  can and should verify in CI, since a silent regression there is exactly the "advertises
  model-backed detection while running regex-only" failure mode §13 of the integration
  design already flags as an open risk.
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

## 14. Acceptance Criteria

- [ ] `business_law.yaml` added and passing `index.test.ts`'s taxonomy validation
- [ ] Auto-labeling pipeline produces a JSONL dataset in GLiNER2's word-token span format
- [ ] Held-out human-reviewed test set exists, disjoint from all auto-labeled/training data
- [ ] Trained adapter's `adapter_config.json` round-trips through `LoraAdapter::from_bytes`
      with the correct `base_model_name` guard behavior verified in CI
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
