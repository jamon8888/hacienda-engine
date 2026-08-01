# `fixtures/ner-eval/` — span-level NER evaluation fixtures

Built for Task 1 of `superpowers/plans/2026-07-31-vertical-model-specialisation-implementation.md`
(spec §8 step 1). This directory is the evaluation corpus for hacienda's statistical NER
detector (`hacienda_core::pii::NerDetector`), scored by `hacienda-core/tests/ner_eval.rs`.

## Status: DRAFT — not yet human-validated

**Every case in `base-fr.json`, `base-en.json`, and `vertical-finance.json` was drafted by
an engineering agent as scaffolding for the harness, not annotated or reviewed by a human
who knows French legal/business documents.** The plan's own risk table calls annotation
"the real cost" and states Task 1.1 is "~80% of this plan's effort" — that effort has not
happened yet. Byte offsets are mechanically verified (a script sliced every entity out of
its source text and confirmed the slice matches the intended span — see the schema-validity
test in `ner_eval.rs`), so the fixture is *structurally* valid, but the *labelling
decisions* (span boundaries, particle handling, legal-suffix handling) are the drafting
agent's own reading of `ANNOTATION.md`, applied by that same agent, not cross-checked by a
second annotator. Per the plan (§1.1), do not treat any F1 number computed against these
files as a validated baseline until:

1. A second person (or a second independent pass) re-annotates a sample and inter-annotator
   agreement is recorded in `ANNOTATION.md`.
2. The case count is checked against real business-document phrasing, not invented sentences.

## Provenance

All text in this directory is **synthetic**, written for this fixture. Every person name,
organisation, address, email address, phone number, IBAN, SWIFT/BIC code, account number,
routing number, and card number is fictional or drawn from well-known public placeholder
conventions (e.g. `example.com`/`example.org`/`example.fr` email domains, IBAN check-digit
patterns that are structurally valid but not issued to any real account). **No real client
data, no data from any hacienda deployment, and no data copied from a real document of any
kind may ever be added to this directory.** If a future contributor is tempted to paste in
a real email thread or contract excerpt "because it's realistic", redact and rewrite it
from scratch instead — this fixture ships in the repository history forever.

## Licence

Original content, contributed under the same licence as the rest of this repository. No
third-party corpus, dataset, or scraped text was used.

## Files

- `README.md` — this file.
- `ANNOTATION.md` — the annotation guideline (also DRAFT — needs second-annotator sign-off).
- `base-fr.json`, `base-en.json` — the five base categories (`person`, `organization`,
  `location`, `email`, `phone`), French and English, byte-offset spans.
- `vertical-finance.json` — the finance vertical's five labels (`iban`, `swift_code`,
  `account_number`, `routing_number`, `card_number`), mixed fr/en text.

## Schema

```json
{
  "cases": [
    {
      "id": "fr-001",
      "lang": "fr",
      "text": "...",
      "entities": [
        { "start": 0, "end": 5, "label": "person" }
      ]
    }
  ]
}
```

`start`/`end` are **byte** offsets into the UTF-8 `text`, matching xberg's
`Entity { start: u32, end: u32, .. }`. They are *not* character offsets — several cases
deliberately include accented French text (`Élodie`, `Nguyễn`, `Saint-Étienne`) where byte
and character offsets diverge, exercising that distinction on purpose.

## Running the harness

The non-ignored schema-validity test and the metric unit tests run in a default
`cargo test`:

```
cargo test -p hacienda-core --test ner_eval
```

The model-backed runner is `#[ignore]`d and gated behind the `ner-candle` feature (native
only, not wasm32). It needs a local GLiNER2 model directory (`config.json`,
`encoder_config/`, `tokenizer.json`, `tokenizer_config.json`, `model.safetensors`):

```
HACIENDA_EVAL_MODEL_DIR=~/model_f16 cargo test -p hacienda-core --features ner-candle --test ner_eval -- --ignored --nocapture
```

Optional: set `HACIENDA_EVAL_OUT=/path/to/report.json` to control where the JSON report is
written (default `target/ner-eval/<timestamp>.json`).

The runner reports, for both `base-fr.json` and `base-en.json`:

- Per-label and micro-averaged precision/recall/F1, in both `Exact` and `Overlap` match
  modes, with 95% bootstrap confidence intervals and per-label instance counts.
- The same metrics with `DEFAULT_CATEGORIES` alone, and again with `DEFAULT_CATEGORIES`
  plus the finance vertical's labels added — the §1.5 label-interference comparison.
- A threshold sweep from 0.5 to 0.95 (see the note below on why 0.5 is a floor, not the
  harness's own default).

## The 0.5 recall ceiling

`xberg`'s `CandleBackend::detect` (pinned `v1.0.2`) hardcodes
`const DEFAULT_THRESHOLD: f32 = 0.5` and ignores any threshold the caller passes in.
`NerDetector::threshold` can only ever raise the *effective* threshold above 0.5 by
filtering what the backend already returned — it can never lower it. **This harness
physically cannot observe any entity the backend scored below 0.5.** Any recall number this
harness reports is *truncated recall at 0.5*, not recall, and a precision/recall curve
below 0.5 is unobtainable without patching the vendored dependency (which this plan
explicitly declines to do — see §1.4 of the implementation plan). Read every recall number
in a generated report with that ceiling in mind.
