# GLiNER2 Vertical LoRA — Training Driver Plan

**Date:** 2026-08-15
**Status:** Proposed
**Corrects:** `2026-07-29-business-law-gliner2-lora-design.md` §8 and §9
**Respects:** `2026-07-31-vertical-model-specialisation-design.md` (tier gating, no hot-swap)

Every claim below was verified against source: the GLiNER2 upstream at
`github.com/fastino-ai/GLiNER2@main`, the pinned runtime at `xberg` rev `9bfbc10`
(`crates/xberg-gliner`), and the deployed checkpoint header at
`apps/hacienda-studio/public/models/gliner2/model.safetensors`. Where a prior spec
disagrees, the prior spec is wrong and is corrected here.

---

## 1. Seven corrections that change the build

**1.1 The training format in `dataset/assemble.py` is the wrong format.**
GLiNER2 does not consume `{"tokenized_text": [...], "ner": [[start, end, label]]}` —
that is GLiNER **v1**. GLiNER2 consumes verbatim mention strings:

```json
{"input": "John Smith works at OpenAI in San Francisco.",
 "output": {"entities": {"person": ["John Smith"], "location": ["San Francisco"]}}}
```

`ExtractorDataset.__getitem__` accepts `input`/`output` or `text`/`schema` and
`data.py::_load_dict_list` raises `ValueError("Unknown dict format...")` on anything
else. GLiNER2 resolves mentions to word spans internally via
`SchemaTransformer._find_sublist` over lowercased word-split text.

Consequence: `assemble.py`'s char→word-span conversion and its round-trip assertion are
solving a problem GLiNER2 already solves internally, in a format it rejects. The
round-trip assertion is still valuable — but as a *pre-flight on mention resolvability*
(§3.3), not as a span emitter.

**1.2 `target_modules` in `adapter_config.json` is dead code for the runtime.**
`lora.rs:56` marks it `#[allow(dead_code)]`. Merge matching is an **exact** `HashMap`
lookup of each base key minus `.weight` (`lora.rs:224-238`). Only *tensor key paths*
decide what merges. The previous spec's emphasis on getting `target_modules` "exactly
right" targets the wrong field.

**1.3 Module paths carry a doubled `encoder.encoder.` prefix.**
Verified in the checkpoint header: `encoder.encoder.layer.0.attention.self.query_proj.weight`.
GLiNER2 nests HF's `DebertaV2Model` under its own `encoder` attribute. DeBERTa-v2 also
names projections `query_proj`/`key_proj`/`value_proj`, not BERT's `query`/`key`/`value`.
A bare `query_proj` never matches the exact-lookup merge.

**1.4 The deployed checkpoint is F16 and cannot be merged into.**
All 227 tensors are F16. `decode_view` (`lora.rs:259-291`) decodes only F32 and I64.
**Merging requires an F32 base.** This is the same blocker recorded as "F16 merge
support upstream" in the 2026-07-31 spec §5.1, now confirmed from both directions.

**1.5 GLiNER2 ships first-class LoRA — do not hand-roll PEFT.**
`SpanExtractorModel.apply_lora()` builds a `peft.LoraConfig` and calls `get_peft_model`.
`TrainingConfig` exposes `use_lora`, `lora_r=16`, `lora_alpha=32.0`, `lora_use_dora`,
`lora_target_modules`, `save_adapter_only=True`. The driver configures this; it does not
reimplement it.

**1.6 Three GLiNER2 defaults produce an adapter the runtime rejects.**

| Default | Effect at load | Required setting |
|---|---|---|
| `fp16=True` | adapter saved F16; `lora.rs:127` hard-rejects non-F32 | train F32, or post-cast on export |
| `lora_use_dora` | emits `lora_magnitude_vector` keys; `lora.rs:172-185` rejects | keep `False` |
| `base_model_name_or_path` = HF slug | fails the substring guard vs. dir basename | rewrite to deployed basename |

**1.7 Two silent data-loss behaviours must be measured, not assumed away.**
`max_width = 8` words: `model.py:594-604` only sets a label when
`0 <= width < scores.shape[3]`, so longer spans are **silently dropped** with no error.
Multi-word PII (addresses, long entity names) is directly exposed to this.
`sanitize()` (`data.py:756`, `:796`) drops the **entire entity type** for a record if
*any one* of its mentions is not found verbatim in the text.

---

## 2. Scope and gate

This plan describes the driver only. It does **not** authorise a training run.
Per 2026-07-31 §8, Tier 2 is entered only when Tier 1 calibration measurably fails on a
held-out set. Building the driver ahead of that gate is justified because the driver is
also what produces the *evidence* — but the gate on shipping an adapter stands.

Non-goals: hot-swap (rejected, §3.1 of that spec), unmerged runtime deltas (§5.2,
explicitly "do not build speculatively"), browser delivery.

---

## 3. Driver design

Five stages. Stages 3.1–3.3 run on CPU in seconds and exist to make stage 3.4 — the only
expensive, GPU-bound step — fail before it starts rather than after.

### 3.1 Preflight: base checkpoint contract

Read the base `model.safetensors` header and assert, before anything else:

- dtype is F32 (else fail with the F16 blocker, §1.4 — do not silently proceed)
- every planned adapter module path has an exact `<path>.weight` key
- record the deployed directory basename as the `base_model_name_or_path` constant

Already implemented and tested: `training/adapter_export.py::verify_module_paths`,
`base_model_name_for`, `passes_base_model_guard`.

### 3.2 Dataset emission (`{"input", "output"}`)

Replace `assemble.py`'s emitter. Source of record stays the accepted spans from
`labeling/consistency.py`; the emitter groups accepted `(start, end, label)` spans per
chunk into `{"entities": {label: [text[start:end], ...]}}`.

Retain from `assemble.py`, unchanged, because they are correct and orthogonal to format:
`train_val_test_split`'s document-boundary splitting and human-reviewed-only test set.

### 3.3 Dataset preflight (the two silent-loss checks)

Fail or report loudly rather than discovering loss in the metrics:

- **Width:** count spans whose word-width `>= max_width` (8). These can never be
  learned. Report the count and the affected labels; a vertical whose key entity is
  routinely >8 words needs a different plan, not a longer run.
- **Resolvability:** for each mention, confirm it is findable verbatim in the
  lowercased word-split text under GLiNER2's own matching. Any miss drops the whole
  entity type for that record (§1.7). This is where `assemble.py`'s round-trip
  discipline should be re-pointed.
- **Split integrity:** re-assert no `doc_id` spans two splits.

### 3.4 Training

`train_gliner2(...)` / `ExtractorTrainer` with, explicitly and non-defaulted:

```
use_lora=True, lora_use_dora=False
fp16=False, bf16=False              # F32 — see §1.6
lora_r=16, lora_alpha=32.0          # scale = alpha/r (lora.rs:214); tune vs. §4, not convention
save_adapter_only=True
synthetic_entity_label_prob=0.0     # default 0.2 relabels to "entity 1"/"entity 2"
encoder_lr=1e-5, task_lr=5e-4
lora_target_modules=[...]           # see below
```

**Target modules for a NER-only vertical.** Include the 36 encoder attention
projections (`encoder.encoder.layer.{0..11}.attention.self.{query,key,value}_proj`), and
optionally `span_rep.span_rep_layer.{project_start,project_end,out_project}.{0,3}` and
`count_embed.projector.{0,2}`.

**Exclude `count_pred` and `classifier`:** on entity-only data the count loss path is
not exercised for `task_type == "entities"`, so they receive no gradient — LoRA on them
is dead weight, and `classifier` is not ported to the Rust runtime at all.
**`count_embed.gru` and `pos_embedding` cannot be LoRA'd** — PEFT targets `nn.Linear`
only, and the GRU is not one.

New labels *are* learnable through the encoder alone: GLiNER2 has no per-label embedding
table. Label semantics are encoder hidden states read at `[E]` marker positions, and the
gather/scorer stages are parameter-free. This is what makes a merge-only runtime viable —
the loader can add deltas to existing weights but could never add new head parameters.

### 3.5 Export

`training/adapter_export.py::write_adapter` — already built and tested (15 tests). Casts
to F32, always writes `base_model_name_or_path` as the deployed basename, rejects `r=0`,
rejects non-PEFT keys, and runs the §3.1 coverage check when given base keys.

---

## 4. Acceptance

The driver is correct when a smoke run on a tiny synthetic vertical produces an adapter
that **loads and merges** through `NerDetector::from_candle_local` against an F32 base,
and measurably changes output versus the base model. Metric quality is a separate
question from driver correctness and belongs to the Tier 1/2 gate.

---

## 5. Sequencing

| # | Step | Gate |
|---|---|---|
| 1 | Obtain/convert an F32 base checkpoint | blocks all merging (§1.4) |
| 2 | PEFT key-naming smoke run: `apply_lora` + `save_pretrained`, diff adapter keys vs. base keys | unverified assumption (§6) |
| 3 | Dataset emitter + preflight (§3.2, §3.3) | none |
| 4 | Training driver (§3.4) | needs GPU |
| 5 | Merge smoke test (§4) | needs 1–4 |
| 6 | Real vertical training | Tier 1 must have measurably failed |

---

## 6. Unverified — resolve before spending GPU time

- **PEFT key naming was read, not executed.** `peft`/`torch` are not installed here.
  Step 5.2 above is specifically to confirm that saved keys are exactly
  `base_model.model.<base key minus .weight>.lora_{A,B}.weight` before a real run.
- **`save_embedding_layers="auto"`** should resolve to `False` (no `vocab_size` on
  `ExtractorConfig`), so no `embed_tokens` keys. If it does emit them, the coverage
  check at `lora.rs:246` fails loudly — a safe failure, not a silent one.
- **Whether `span_rep`/`count_embed` LoRA beats encoder-only** is unmeasured. Treat the
  target-module set as a hyperparameter, not a settled choice.
- **PEFT version** GLiNER2 is tested against is unpinned here.
