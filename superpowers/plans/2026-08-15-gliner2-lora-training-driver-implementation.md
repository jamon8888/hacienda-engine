# GLiNER2 Vertical LoRA Training Driver — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce a PEFT LoRA adapter for a vertical that **loads and merges** through
`NerDetector::from_candle_local`, and fail every way it can fail on CPU in seconds rather
than on GPU in hours.

**Architecture:** Three CPU-side Python modules under `training/` plus one Colab notebook.
No Rust change, no `xberg` change, no new crate. The runtime already merges adapters
(`CandleBackend::get_or_init`); this plan only feeds it a well-formed one.

**Tech Stack:** Python 3.10+, `gliner2` (its own `ExtractorTrainer`, not a hand-rolled PEFT
loop), `peft`, `safetensors`, `numpy`, `pytest`. Training runs on Colab because this host
has ~3 GB RAM and no GPU.

**Spec:** `superpowers/specs/2026-08-15-gliner2-lora-training-driver-plan.md`. Corrects
`2026-07-29-business-law-gliner2-lora-design.md` §8–§9. Respects
`2026-07-31-vertical-model-specialisation-design.md`: no hot-swap, no unmerged deltas,
Tier 2 shipping still gated on Tier 1 measurably failing.

---

## Baseline (recorded 2026-08-15)

Verified against GLiNER2 `main`, `xberg` rev `9bfbc10`, and both checkpoints:

1. **`training/adapter_export.py` exists and is green** — 15 tests. Casts to F32, pins
   `base_model_name_or_path` to the deployed directory basename, rejects `r=0`, rejects
   non-PEFT keys, and exact-matches module paths against base keys.
2. **`labeling/` (17 tests) and `dataset/` (6 tests) are green** and unchanged by this plan
   except `assemble.py`'s emitter (Task 1).
3. **The F16 blocker is resolved.** The upstream HF checkpoint
   `fastino/GLiNER2-Guardrails-PII-Multi` is **227 tensors, all F32** (verified by range-reading
   the safetensors header). The F16 artifact at
   `apps/hacienda-studio/public/models/gliner2/model.safetensors` is the *derived*
   distribution output of `scripts/convert_gliner2_f16.py`. **Train and merge against
   upstream F32; never against the local F16** — `decode_view` (`lora.rs:259-291`) decodes
   only F32/I64, so the F16 file cannot be a merge base.
4. **Module paths carry a doubled prefix**, identical upstream and locally:
   `encoder.encoder.layer.{0..11}.attention.self.{query,key,value}_proj.weight`.
5. **`peft`/`torch` are not installed on this host** and will not be. Anything touching them
   is Colab-only by construction.

---

## Task 1 — Emit GLiNER2's actual training format

`dataset/assemble.py` emits GLiNER **v1** (`{"tokenized_text", "ner"}`). GLiNER2's loader
raises `ValueError("Unknown dict format...")` on it.

- [x] Write failing tests in `dataset/test_emit.py`: accepted `(start, end, label)` spans for
      a chunk become `{"input": <chunk>, "output": {"entities": {label: [verbatim, ...]}}}`;
      multiple mentions of one type group into one list; mention strings are sliced from the
      chunk, never reconstructed from tokens.
- [x] Add `to_gliner2_record(chunk_text, char_spans, *, doc_id, source)` to `dataset/assemble.py`.
- [x] Keep `train_val_test_split` **unchanged** — document-boundary splitting and the
      human-reviewed-only test set are correct and format-independent.
- [x] Keep `to_word_span_record` for now but stop feeding the trainer from it; Task 2 re-points
      its round-trip discipline. Do not delete it in the same commit that adds the new emitter.
- [x] `cd dataset && ../.venv/bin/python -m pytest -q` green — **16 passed**.

Two rejections the emitter added beyond the plan, both silent-corruption paths: an
out-of-range `end` (Python slicing clamps it into a shorter mention) and an empty span
(matches at every position under `_find_sublist`). Repeated mention strings are deduped
per type — GLiNER2 resolves a mention to *every* occurrence, so listing it twice is inert.

**Verification:** a record emitted by `to_gliner2_record` round-trips through
`json.loads(json.dumps(...))` and every mention satisfies `mention in chunk_text`.

---

## Task 2 — Dataset preflight for the two silent-loss behaviours

Both destroy training signal with **no error**, so they must be measured before a run, not
inferred from bad metrics after one.

- [x] Write failing tests in `training/test_dataset_preflight.py`.
- [x] Create `training/dataset_preflight.py` with:
  - `width_violations(records, max_width=8)` — spans whose **word** width `>= max_width`.
    `model.py:594-604` only sets a label when `0 <= width < scores.shape[3]`, so wider spans
    are silently unlearnable. Report count **and** affected labels: a vertical whose key
    entity is routinely >8 words needs a different plan, not more epochs.
  - `unresolvable_mentions(records)` — mentions not findable under GLiNER2's own matching
    (lowercased word-split, `_find_sublist`). `sanitize()` drops the **entire entity type**
    for a record on a single miss (`data.py:756`, `:796`), so one bad mention silently
    deletes a whole type's supervision for that record.
  - `assert_split_integrity(splits)` — no `doc_id` in two splits.
- [x] Preflight **reports** rather than raises for width/resolvability (they are corpus facts
      needing a human decision), and **raises** for split leakage (always a bug).
- [x] `cd training && ../.venv/bin/python -m pytest -q` green — **34 passed** (15 export + 19
      preflight).

Width is counted in **GLiNER2 tokens, not whitespace words**, and the module exports
`gliner2_tokens` — a verbatim copy of `WhitespaceTokenSplitter._PATTERN`
(`processor.py:231-238`). A `text.lower().split()` stand-in was tried first and produced
false positives on real data (`inc.` vs `inc`), which is worse than no preflight: it
trains the reader to ignore the report. The notebook carries the same pattern inline
because it must run on Colab without the repo; `test_dataset_preflight.py` is the source
of record for its behaviour.

---

## Task 3 — Colab training notebook

`notebooks/gliner2_vertical_lora_colab.ipynb` (delivered with this plan). **Training happens
on Colab; this repo only produces the notebook and the exporter it imports.**

Two Colab-specific constraints shape it:

- **GPU quota is the scarce resource**, not GPU speed. §1–§5 therefore cost none of it —
  ~2 KB downloaded, under a minute, any runtime — and they carry every check that can
  invalidate the approach. Discovering a malformed corpus or a naming mismatch *after*
  committing a session is the failure to avoid.
- **Sessions are ephemeral** (~90 min idle, 12 h hard cap). §0 mounts Drive and puts the
  work dir, the HF cache, the corpus, and the final adapter there. Writing to `/content`
  loses the adapter on disconnect *and* re-downloads 1.2 GB next session.

Mixed precision stays off (`fp16=False`, `bf16=False`) because the adapter must be F32
(`lora.rs:127`), so the GPU runs full precision — still far faster than CPU, just without
the tensor-core speedup an fp16 run would get.

- [ ] §2 **PEFT key-naming proof** — the one assumption in the spec that was read from PEFT
      source and never executed. Answered against a structural stub reproducing GLiNER2's
      real module path (`encoder.encoder.layer.0.attention.self.query_proj`), because the
      question is about PEFT's convention over a module tree, not about GLiNER2's weights.
      No download, no training, one second. Fails loudly if saved keys are not exactly
      `base_model.model.<base key minus .weight>.lora_{A,B}.weight`. **If this fails, stop** —
      merge-at-load cannot work and GPU time will not change that.
- [ ] §3 **Base checkpoint contract** via a ranged read of the safetensors header (~2 KB, not
      1.2 GB): asserts all-F32 and captures the real key names the exact-match merge uses.
- [ ] §6–§7 Run end to end on the tiny synthetic vertical, then re-check adapter keys against
      the base. §2 proves PEFT's *naming*; §7 is what proves the *set of modules* GLiNER2's
      own `apply_lora` targeted is mergeable — and asserts the adapter is non-empty, since
      `lora_target_modules=['encoder', 'span_rep']` are GLiNER2 module *groups*, not PEFT
      suffixes. `FQ_TARGETS` (the 36 fully-qualified attention projections) is the fallback.
- [ ] §8 Confirm the exported `adapter_model.safetensors` is F32 and `adapter_config.json` sets
      `base_model_name_or_path` to the **deployed** basename (`gliner2`), not the HF slug.

Non-default settings the notebook must pin, each because a default breaks the loader or the
data (spec §1.6, §1.7):

| Setting | Default | Required | Why |
|---|---|---|---|
| `fp16` | `True` | `False` | adapter saved F16; `lora.rs:127` hard-rejects |
| `bf16` | `False` | `False` | same dtype constraint |
| `lora_use_dora` | `False` | `False` | DoRA emits `lora_magnitude_vector`; parser rejects |
| `synthetic_entity_label_prob` | `0.2` | `0.0` | relabels 20% to "entity 1"/"entity 2" |
| `save_adapter_only` | `True` | `True` | we want the delta, not a 1.2 GB fork |

Target modules: the 36 encoder attention projections, optionally `span_rep.*` and
`count_embed.projector.*`. **Exclude `count_pred` and `classifier`** — zero gradient on
entity-only data, and `classifier` is not ported to the Rust runtime.
`count_embed.gru`/`pos_embedding` cannot be LoRA'd (PEFT targets `nn.Linear` only).

---

## Task 4 — Merge smoke test

Driver correctness is "it loads and changes behaviour", which is a separate question from
metric quality.

- [ ] Place the exported adapter and an **F32** base locally.
- [ ] `cargo test -p hacienda-core --features ner-candle --test lora_adapter_contract` still green.
- [ ] Add an `#[ignore]`d integration test asserting `from_candle_local(f32_base, Some(adapter))`
      returns `Ok` and produces output differing from the base model on a fixture.
- [ ] `#[ignore]` because it needs real weights, matching the existing `ner_eval.rs` pattern.

---

## Sequencing and gates

| # | Task | Gate | State |
|---|---|---|---|
| 1 | Emitter | none — CPU, local | **done**, 16 tests |
| 2 | Dataset preflight | none — CPU, local | **done**, 19 tests |
| 3a | Notebook §1–§5 | **no GPU quota**; proves key naming | notebook written, unrun |
| 3b | Notebook §6–§8 | Colab GPU session + 1.2 GB base | blocked on 3a |
| 4 | Merge smoke test | needs 3b + F32 base on disk | blocked on 3b |
| 5 | Real vertical training | **Tier 1 must have measurably failed first** | not authorised |

Tasks 1–4 are safe to build now: they are also what produces the evidence the Tier 2 gate
needs. Task 5 is not authorised by this plan.

---

## Open, resolve before spending real GPU time

- **PEFT key naming** — read from source, not executed. Task 3 §2 is exactly this, and needs
  no GPU, no download, and no training to answer.
- **`save_embedding_layers="auto"`** should resolve to `False` (no `vocab_size` on
  `ExtractorConfig`). If it emits `embed_tokens` keys, `lora.rs:246`'s coverage check fails
  loudly — a safe failure.
- **Whether `span_rep`/`count_embed` LoRA beats encoder-only** is unmeasured. Treat the
  target-module set as a hyperparameter, not a settled choice.
- **PEFT version** GLiNER2 is tested against is unpinned.
