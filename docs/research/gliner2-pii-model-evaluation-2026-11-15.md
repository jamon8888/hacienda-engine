# GLiNER2 PII model evaluation — 2026-11-15

## Scope

The official Fastino GLiNER2 local model was evaluated with the repository's synthetic
French/English finance fixture. This run validates Hacienda's model-backed NER path and
its category normalization. It does not establish a legal-grade guarantee or measure
recall over real client files.

- Model weights (`model.safetensors`) BLAKE3:
  `7663861192857d820a2b6f12e8d8d365a669fa458c0132fcf211dd2dec2194fd`
- Fixture: `vertical-finance.json`, 18 synthetic cases, one annotated span per case.
- Default configured threshold: `0.5`.
- Strict exact micro score: precision **0.944**, recall **0.944**, F1 **0.944**
  (17 true positives, 1 false positive, 1 false negative).
- Exact per-label F1: `bank_account` **1.000** (6/6), `swift_code` **1.000** (6/6),
  `iban` **0.833** (5/6; one false positive and one false negative).
- Exact-match case-bootstrap 95% interval: **[0.833, 1.000]** (1,000 resamples).

GLiNER returns the requested `account_number` label, and Hacienda maps it to the
canonical `bank_account` category before scoring. The fixture now uses that canonical
category for gold spans; before the correction, all six correctly detected accounts
were incorrectly scored as category mismatches.

The overlap score is **0.833**. Its percentile-bootstrap interval was **[0.889, 1.000]**,
which excludes the point estimate. With only 18 cases this interval is unstable and is
not used as an acceptance result; the bootstrap implementation or interval method
needs further statistical review before relying on overlap intervals.

## Limits and next work

1. These are synthetic fixtures from a single annotation pass. A second independent
   annotation of representative French and English legal documents is required before
   interpreting the scores as product-quality estimates.
2. `xberg v1.0.2`'s Candle detector applies a hard internal threshold of `0.5`. Raising
   Hacienda's threshold cannot reveal predictions below that value, so the reported
   recall is truncated at `0.5`.
3. The model's `iban` category misses one of six spans in this fixture. Investigate the
   missed span and false positive, then validate against a larger, independently
   annotated corpus.
4. `swift_code` remains a custom Hacienda category. Mapping it to the existing
   `SwiftBic` taxonomy variant is a separate product decision.
5. This evaluation covers local GLiNER inference and category normalization only. It
   does not certify end-to-end document ingestion, metadata redaction, token storage,
   rehydration, Claude/Codex proxy egress, UI display, or team deployment.

## Reproduction

```sh
HACIENDA_EVAL_MODEL_DIR="$HOME/Library/Application Support/io.hacienda.secrecy-client/models/gliner2-pii" \
HACIENDA_EVAL_FIXTURE=vertical-finance.json \
HACIENDA_EVAL_OUT="$HOME/.cargo/target/ner-eval/gliner2-pii-finance-canonical-2026-11-15.json" \
cargo test --release -p hacienda-core --features ner-candle --test ner_eval \
  should_score_ner_eval_fixtures_against_a_local_model -- --ignored --nocapture
```

The JSON report is a local build artifact and is intentionally not committed because it
contains a machine-specific model path. The model digest and summarized metrics above
are sufficient to identify this run without exposing that path.
