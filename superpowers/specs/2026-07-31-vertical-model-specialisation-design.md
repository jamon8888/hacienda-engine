# Per-Vertical Model Specialisation and Model Footprint

**Date:** 2026-07-31
**Status:** Proposed
**Supersedes:** `2026-07-28-gliner2-lora-hotswap-xberg-native.md` (branch
`claude/gliner2-lora-hotswap-xberg-native`, never merged)
**Builds on:** `2026-07-27-vertical-ner-architecture-design.md` (Approved)

---

## 1. Problem Statement

hacienda needs per-vertical PII detection — finance, healthcare, legal/business law — where each vertical
recognises entity types the base model was not obviously trained to emphasise (`iban`, `mrn`, `docket_number`).

The 2026-07-28 plan answered this with LoRA adapters hot-swapped at runtime. That plan is the wrong shape. It
assumed a cost structure that does not hold, proposed a mutation that is unsound against the cache it builds on,
and — most importantly — skipped the question of whether weight specialisation is needed at all.

This spec answers three questions in order:

1. What is the cheapest mechanism that produces a working vertical? (Section 4)
2. If weights are eventually required, how are they delivered without N copies of the base model? (Section 5)
3. What actually blocks shipping any of this? (Section 6)

---

## 2. Measured Ground Truth

Everything in this section was measured on 2026-07-31, not quoted from planning documents. Prior docs have
repeatedly overstated or understated these figures.

### 2.1 Model composition

`fastino/GLiNER2-Guardrails-PII-Multi`, read from the safetensors header (227 tensors, 307,098,645 params,
100% F32):

| Component                       | Params      | Share | F32     | F16     |
| ------------------------------- | ----------- | ----- | ------- | ------- |
| `encoder.embeddings.word_embeddings` `[250112, 768]` | 192,086,016 | 62.5% | 768 MB | 384 MB |
| 12 transformer layers           | 85,054,464  | 27.7% | 340 MB  | 170 MB  |
| Heads (`span_rep`, `count_*`, `classifier`) + rel. embeddings | 29,958,165 | 9.8% | 120 MB | 60 MB |
| **Total**                       | 307,098,645 | 100%  | 1.23 GB | 614 MB  |

Architecture is `deberta-v2` (`microsoft/mdeberta-v3-base`): hidden 768, 12 layers, 12 heads, intermediate 3072,
**vocab 250112**, max positions 512.

The single most important number here: **62.5% of the model is a multilingual token embedding table, and LoRA
never touches it.** The entire LoRA-addressable surface is the 85 M transformer-layer parameters.

### 2.2 Adapter cost vs. carrying cost

Computed from the real tensor shapes, targeting every 2D linear in all 12 layers
(`query_proj`, `key_proj`, `value_proj`, `attention.output.dense`, `intermediate.dense`, `output.dense`):

| Rank | Adapter params | Adapter size (F16) | Share of base |
| ---- | -------------- | ------------------ | ------------- |
| r=8  | 1,327,104      | **2.65 MB**        | 0.43%         |
| r=16 | 2,654,208      | **5.31 MB**        | 0.86%         |

Under merge-at-load — what xberg implements today — carrying 2.65 MB of vertical-specific information costs a
**full 614 MB (F16) or 1.23 GB (F32) duplicate of the base model**. That is a 232x overhead on the thing that
actually differs. This ratio is the central fact of this design.

### 2.3 Hard limits

| Limit                                    | Value                | Source                                            |
| ---------------------------------------- | -------------------- | ------------------------------------------------- |
| API container memory                     | 4 GB                 | `docker-compose.yml:27`                           |
| jsDelivr per-file cap / current wasm size | 50 MB / 48.06 MB     | Track L; ~1.9 MB headroom                         |
| Browser model delivery                   | **Currently broken** | B3: IndexedDB `QuotaExceededError`, renderer crash |
| Trained adapters in existence            | **Zero**             | No `adapter_config.json` anywhere on disk         |

Consequence of the 4 GB cap: with the F32 base at 1.23 GB resident, plus transient duplication during merge,
**two merged verticals is already marginal and three does not fit**. Merge-at-load multi-adapter fails on the
server too, not just in the browser.

### 2.4 The capability that is already paid for and unused

`xberg::text::ner::NerBackend` exposes `detect_with_custom(text, categories, custom_labels)` — GLiNER2's
zero-shot path, where entity types are supplied as *text labels at inference time*.

- `detect_with_custom` is **never called anywhere in hacienda** (verified: zero hits across all `.rs` files).
- `NerDetector::with_categories` is called **only from tests** (`pipeline.rs:448`, `pipeline.rs:493`).
- Production therefore always runs the hardcoded `DEFAULT_CATEGORIES` in `hacienda-core/src/pii/ner.rs:14-20`:
  Person, Organization, Location, Email, Phone. Five categories. No configuration path reaches them.

**The 2026-07-28 plan proposed training adapters so the model could detect entity types that the model can
already be asked for, and that hacienda simply never asks for.** This is the finding that reorders the roadmap.

---

## 3. Why the Hot-Swap Plan Is Rejected

Beyond being unnecessary as a first step, it has three defects.

**3.1 The swap is unsound.** `CandleBackend::get_or_init` (candle.rs:111) returns `Arc<CandleBackend>` from a
process-global `LazyLock<RwLock<AHashMap>>` keyed `(model_dir, Option<adapter_dir>)`, and
`CandleBackend { model: Mutex<Gliner2Candle> }`. Option B's `&self` hot-swap would mutate a shared, globally
cached instance: every other `Arc` holder silently changes adapter mid-flight, and the cache key becomes a lie
about the object it names. In a system that must attribute redactions to a model, this is a correctness bug, not
a performance trade-off.

**3.2 Option A was already built.** The proposed `AdapterRegistry` (~180 lines) is a re-implementation of
xberg's existing cache. Keying on `(model_dir, adapter_dir)` already yields N resident backends and O(1)
selection. The plan budgeted 6.5 days for work that is a config-shape change.

**3.3 It is gated on annotation, not code.** No adapter exists. The sibling training effort
(`claude/gliner2-lora-business-law-ibfru1`) is blocked on a human legal annotator and GPU training. Every
acceptance criterion in the old plan ("Finance adapter detects `credit_card`") is unverifiable today.

---

## 4. Architecture: Three Tiers of Vertical Specialisation

A vertical is a **bundle**, not a model. Weights are the last tier, entered only when the tier below is
measurably insufficient. This extends the taxonomy model already approved in
`2026-07-27-vertical-ner-architecture-design.md`, whose roadmap already placed "Fine-tuned GLiNER2 per vertical"
at Q4 2026 gated on "when training data available" — a gate the hot-swap plan jumped.

```text
Tier 2  Adapted      LoRA delta, ~2.65 MB     requires annotation + eval proof     NOT YET
Tier 1  Calibrated   thresholds + gazetteers, ~KB   requires a labelled dev set    NEXT
Tier 0  Schema       label set + regex packs, 0 B   requires nothing               SHIP NOW
```

### 4.1 Tier 0 — Schema verticals (zero weights)

A vertical declares what it wants detected; the label set reaches GLiNER2 as zero-shot text labels. Nothing is
loaded, nothing is trained, nothing is duplicated.

The mechanism is verified end-to-end and needs **no upstream change**: `CandleBackend::detect` maps each
requested category through `category_to_label`, whose `Custom` arm is `EntityCategory::Custom(s) => s.as_str()`,
and hands the resulting `&[&str]` straight to `model.extract_ner(text, &labels, threshold)`. Populating
`NerDetector`'s existing `categories` vector with `EntityCategory::Custom(label)` entries is therefore
sufficient. (`detect_with_custom` is equivalent today — `CandleBackend` does not override the trait default,
which simply appends the labels as `Custom` categories and calls `detect`.)

```toml
[[pii.verticals]]
id = "finance"
name = "Financial Services"
labels = ["iban", "swift_code", "account_number", "routing_number", "card_number"]
patterns = ["iban", "luhn_card"]        # deterministic regex, already in patterns.rs
[pii.verticals.thresholds]
default = 0.5
iban = 0.7
```

Properties that make this the right default:

- **Cost:** one base model, resident once, regardless of vertical count.
- **Switching:** replacing a `Vec<String>` on the request path. No model reload, no lock, no cache mutation,
  no aliasing hazard.
- **Concurrency:** two requests can use different verticals against the same `Arc<CandleBackend>`
  simultaneously, because nothing is mutated. Merge-at-load cannot do this at any memory budget.
- **Surface parity of the *mechanism*:** it is inference-time arguments, not weights, so nothing about it is
  native-specific. The *plumbing* is another matter, and the distinction must not be blurred: `hacienda-core`
  on wasm32 compiles xberg's Candle backend (via the `ner-candle-wasm` feature) but gates both
  `NerDetector::from_candle_local` and the real `load_detector` on `not(target_arch = "wasm32")`, so
  `load_detector` in the browser always returns `ModelUnavailable`. Studio reaches `xberg-wasm`'s `NerModel`
  directly instead. **Tier 0 is native-only until that gate is opened**, which is Section 6's problem, not
  this section's.
- **Determinism:** regex/gazetteer hits for structured identifiers (IBAN, NPI, docket formats) are exact and
  auditable. Structured PII should be matched, not predicted, wherever a checksum or format exists.

Required work is small and entirely inside hacienda: a `VerticalConfig` in `pii/config.rs`, and threading its
label set from config through `PiiPipeline::load_detector` into `NerDetector::with_categories`. The detection
call site does not change.

### 4.2 Tier 1 — Calibrated verticals

Zero-shot recall is usually acceptable; zero-shot *precision at a fixed threshold* usually is not, and it varies
per label. Tier 1 adds per-label thresholds and optional gazetteers/negative lists, fitted on a small labelled
dev set. Kilobytes, no GPU, no change to the inference path beyond filtering.

This is where most of the realisable accuracy gain lives, and it is reachable with roughly two orders of
magnitude less annotation than a LoRA.

**Constraint, verified in `v1.0.2`:** `CandleBackend::detect` ignores the caller's threshold and passes its own
`const DEFAULT_THRESHOLD: f32 = 0.5` to `extract_ner`; `NerDetector` then filters the returned entities against
its own threshold. hacienda can therefore only ever *raise* the effective threshold above 0.5, never lower it.
Any calibration that wants a label admitted below 0.5 — plausible for high-recall categories — needs
`NerBackend::detect` to carry a threshold, which is an upstream xberg change. Tier 1 must be scoped to
precision-side (raising) calibration until that lands, and step 5 of Section 8 should measure whether the recall
left on the table below 0.5 is material before requesting the change.

### 4.3 Tier 2 — Adapted verticals (LoRA)

Entered **only** on evidence: a held-out set where Tier 1 misses a target metric that a trained adapter is
plausibly able to close. Section 5 covers delivery. Section 8 covers the gate.

---

## 5. Adapter Delivery, When Tier 2 Arrives

Two mechanisms, chosen by surface. Neither mutates a shared cached backend.

### 5.1 Native — merged, bounded, distinct cache entries

Reuse `get_or_init` exactly as designed: each `(model_dir, adapter_dir)` pair is a *separate* backend. Selection
picks a different `Arc`; nothing is swapped in place. This is sound and needs no upstream change.

It needs two things to be practical:

- **F16 merge support upstream.** The merge path is F32-only today
  (`model.rs`: native pins F32 "for LoRA merge compatibility"), so each resident vertical costs 1.23 GB rather
  than 614 MB. Fixing this roughly doubles how many verticals fit the 4 GB container. Small, well-scoped
  upstream change.
- **An explicit residency budget.** Config declares how many adapters may be resident; exceeding it is a
  configuration error at startup, not an OOM at request time. With the F32 base today the honest limit is one.

### 5.2 Browser and high-N — unmerged runtime delta

The memory-correct form: keep one frozen base, keep adapters as deltas, and compute
`y = xW^T + (alpha/r)(xA^T)B^T` in the forward pass. Cost becomes `base + N x 2.65 MB` instead of `N x base`,
and per-request vertical selection becomes possible.

Cost and risk, stated honestly: `xberg-gliner`'s encoder wraps
`candle_transformers::models::debertav2::DebertaV2Model`, which exposes no seam for intercepting its `Linear`
layers. Realising this requires wrapping or forking that model inside xberg — a substantial upstream change to a
git-tag dependency (`v1.0.2`), not a local patch. Candle 0.11 has every operation required; the obstacle is
structural, not numerical.

**Recommendation: do not build 5.2 speculatively.** It is the correct end state at high N, but N is currently
zero, and the browser cannot deliver even one base model today (Section 6). Revisit when Tier 2 is justified
*and* delivery is fixed.

---

## 6. The Actual Blocker: Model Footprint

Verticals are not what stands between hacienda and shipping. Model delivery is, and it is the same blocker on
every surface. Track B3 already measured the browser failing on the 1.23 GB artifact with IndexedDB
`QuotaExceededError` and a renderer crash — failures of the single-shot fetch/put design that a smaller model
alone does not fix, but that a smaller model makes far less likely to recur.

### 6.1 Vocabulary pruning — the largest available win

62.5% of this model is a 250,112-token multilingual embedding table inherited from mDeBERTa-v3, which covers
100+ languages. hacienda processes French and English legal and business documents.

Pruning the vocabulary to the tokens actually exercised by that corpus, and remapping the tokenizer:

| Vocab   | Embedding params | Embedding F16 | Total model F16 |
| ------- | ---------------- | ------------- | --------------- |
| 250,112 | 192.1 M          | 384 MB        | 614 MB (today)  |
| 64,000  | 49.2 M           | 98 MB         | ~328 MB         |
| 32,000  | 24.6 M           | 49 MB         | ~279 MB         |

A ~2x reduction on top of the F16 conversion already done — roughly **4.4x smaller than the artifact Studio
downloads today** — with no annotation, no training, and no accuracy cost on tokens that remain. It also shrinks
the 16 MB `tokenizer.json`.

This must be gated on measurement, for a reason specific to this workload: PII detection is disproportionately
about proper nouns. SentencePiece degrades unseen tokens into more subword pieces rather than failing, so the
risk is longer sequences and softer span boundaries on unusual names, not breakage. That is exactly what the
Section 8 evaluation harness is for. The pruning corpus must include names, not just legal prose.

### 6.2 Live discrepancies to correct

- `apps/hacienda-studio/lib/asset-loader.ts:21` still defaults to
  `fastino/GLiNER2-Guardrails-PII-Multi` — the **1.23 GB F32** artifact. The F16 model is reachable only by
  setting `VITE_MODEL_BASE_URL`. The program plan's claim that the loader was repointed is false; Studio
  downloads the large model today.
- `hacienda-private/scripts/models/gliner2-guardrails-pii-f16.sha256` records 614,224,538 bytes / `53c73fff…`;
  the artifact on disk is 614,224,466 bytes / `7dd22d08…`. Two different conversion runs. Recorded provenance
  does not match the published artifact — a supply-chain integrity gap that must be closed before the file is a
  distributed dependency.
- `scripts/convert_gliner2_f16.py` documents itself as streaming but accumulates every tensor in a dict before
  writing (peak ~1.8 GB on a ~3 GB host), and copies only `tokenizer.json` and `encoder_config/config.json` —
  not `config.json` or `tokenizer_config.json`.
- F16 weights and the F32-only merge path are mutually incompatible today. Whichever of 5.1 or 6.1 lands first
  forces this to be resolved.

---

## 7. Provenance and Audit

`AuditEntry` records `source: EntitySource` as `Regex | Model`. It does not record **which** model or adapter
produced a detection, and `config_hash` covers PII config, not a model manifest.

For a product whose claim is secret professionnel and DORA/DPIA-grade evidence, "a model detected this" is not a
sufficient record once more than one model can be active. Before any Tier 2 adapter or runtime vertical
selection ships:

- `AuditEntry` gains a model identity: base model digest, adapter identity (or explicit `None`), and vertical id.
- That identity enters the chain hash, so provenance is tamper-evident like the rest of the entry.
- The model card records the active base+adapter set rather than a static template.

Tier 0 needs a weaker version of the same thing — the vertical id and label set must be recorded, because the
label set changes what is detectable and therefore what absence of a finding means.

---

## 8. Sequencing and Decision Gates

| Step | Work                                                              | Gate to proceed                                     |
| ---- | ----------------------------------------------------------------- | --------------------------------------------------- |
| 1    | Evaluation harness: labelled fr/en dev set, per-label P/R/F1       | none — prerequisite for every claim below            |
| 2    | Tier 0 schema verticals: config, threading, `detect_with_custom`   | none — no new dependency, no model change            |
| 3    | Record vertical id + label set in audit entries                    | ships with step 2                                    |
| 4    | Vocabulary pruning experiment, measured on step 1's harness        | no F1 regression beyond agreed tolerance             |
| 5    | Tier 1 calibration: per-label thresholds, gazetteers               | step 1 shows precision gap at fixed threshold        |
| 6    | Model identity in `AuditEntry` + chain hash                        | prerequisite for step 7                              |
| 7    | Tier 2 LoRA, native, merged, residency-bounded (5.1) + F16 merge   | step 5 exhausted **and** annotation capacity exists  |
| 8    | Unmerged runtime delta upstream (5.2)                              | N>=3 verticals justified, or browser delivery fixed  |

Steps 1 and 2 are independent of every unresolved question in this document and can start immediately. Nothing
below step 4 requires a trained adapter, a GPU, or an upstream xberg release.

---

## 9. Open Questions

1. **Is zero-shot good enough?** Unanswerable until step 1. It is the question that decides whether steps 7-8
   ever happen, and it has never been measured.
2. **Do the verticals need disjoint label sets?** Studio's `VerticalDictionary` currently keys a flat map by
   lowercased entity name, so the last-loaded taxonomy silently wins a shared key. Tier 0 makes label sets
   load-bearing, which turns that latent collision into a correctness bug.
3. **Where is the vertical selected?** Per-request (API), per-invocation (CLI), or per-workspace (Studio). This
   determines whether vertical state can live on the pipeline at all, or must be a request parameter. Tier 0
   makes per-request cheap; Tier 2 merged does not.
4. **Who annotates?** Steps 1, 5 and 7 all need labelled French legal text. This is the binding resource
   constraint on the whole roadmap and it is not an engineering one.
5. **Is mdeberta-v3-base the right base at all?** If pruning to fr/en is acceptable, a monolingual or smaller
   base may dominate it on both size and accuracy. Worth one experiment before investing in adapters for this
   specific base.

---

## 10. Summary

The vertical problem was framed as a model-weights problem. Measurement says it is not:

- A vertical's learned difference is 2.65 MB; merge-at-load makes it cost 614 MB. The delivery mechanism, not
  the adapter, is the expense.
- 62.5% of the model is a multilingual embedding table irrelevant to both verticals and this corpus. That is the
  largest single win available, and it is unrelated to LoRA.
- The model already accepts arbitrary entity labels at inference time. hacienda has never used this. Verticals
  are reachable now, at zero model cost, on every surface.

Ship Tier 0 and an evaluation harness. Let measurement decide whether weights are ever needed.
