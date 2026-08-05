# `fixtures/ner-eval/`

Span-level NER evaluation fixtures for the harness in `hacienda-core/tests/ner_eval.rs`.
See `superpowers/specs/2026-07-31-vertical-model-specialisation-design.md` §8 step 1 and
`superpowers/plans/2026-07-31-vertical-model-specialisation-implementation.md` Task 1 for
why this exists.

This is a **different corpus from `fixtures/pii-corpus.json`** at the repo root, and
deliberately so: `pii-corpus.json` is category-set-only and shared with Studio's `vitest`
suite ("one corpus, two runners"); this directory is span-level (byte offsets, one
entity at a time) and model-dependent (only `hacienda-core`'s Candle backend consumes
it), so there is no JS-side runner to keep in sync.

## Provenance and licence

Every sentence in `base-fr.json`, `base-en.json`, and `vertical-finance.json` was
**hand-written for this fixture set** by the engineer who authored this harness, as a
single annotation pass (see "Known limitation" below). Names, addresses, phone numbers,
emails, IBANs, SWIFT/BIC codes, and account numbers are **entirely fabricated** — chosen
to be structurally realistic (valid-looking French/international formats) but do not
correspond to any real person, company, or account. IBAN/SWIFT checksums are not
guaranteed to validate; these are NER training/eval fixtures, not format-validation
fixtures (`hacienda-core/src/pii/patterns.rs` owns checksum-validated matching).

**No real client data may ever enter this directory.** Not a redacted excerpt, not a
"just the shape, not the content" paraphrase, not a fixture derived from a support
ticket. If a case is needed that resembles a real scenario, write a new fabricated
sentence with the same structure rather than adapting real text. This rule exists
because these files are committed to the repository and, unlike the redaction pipeline
itself, are never redacted.

Licence: these fixtures are original content authored for this repository and are
covered by the repository's own licence (Apache-2.0, per the workspace `Cargo.toml`).
No third-party corpus, scrape, or dataset was used as a source.

## Files

| File | Shape | Purpose |
|---|---|---|
| `base-fr.json` | `{ "cases": [...] }`, `lang: "fr"` | Base five categories (Person, Organization, Location/Address, Email, Phone), French |
| `base-en.json` | same shape, `lang: "en"` | Base five categories, English |
| `vertical-finance.json` | same shape, mixed `lang` | Finance-vertical zero-shot labels: `iban`, `swift_code`, `account_number` |
| `ANNOTATION.md` | — | The annotation guideline. Read this before adding or judging any case; exact-match F1 is only meaningful against a stated convention. |

Case shape (all three files):

```json
{
  "cases": [
    {
      "id": "fr-001",
      "lang": "fr",
      "text": "...",
      "entities": [
        { "start": 35, "end": 60, "label": "person" }
      ]
    }
  ]
}
```

`start`/`end` are **UTF-8 byte offsets** into `text` (Rust `str` byte indices — the same
representation as `xberg::types::entity::Entity { start: u32, end: u32 }`), **not**
character or codepoint indices. See `ANNOTATION.md` for a worked example of why this
matters on accented text.

### Label vocabulary

The `label` field is written as the canonical `snake_case` string that
`hacienda_core::pii::PiiCategory`'s own `Serialize` implementation produces for the
category the model's output is expected to land in **after** hacienda's
`to_pii_category` mapping (`hacienda-core/src/pii/ner.rs`), not the raw label text sent
to the model. This keeps the harness's scoring code (`hacienda-core/tests/ner_eval.rs`)
a straight comparison with no separate alias table to keep in sync:

- Base files use `person`, `organization`, `address`, `email`, `phone_number`. Note
  `address` (not `location`): the model's raw `Location` category is mapped onto
  `PiiCategory::Address` by `to_pii_category`, and `phone_number` for the same reason
  (`Phone` → `PiiCategory::PhoneNumber`).
- `vertical-finance.json` uses `iban`, `swift_code`, `account_number` — the finance
  vertical's zero-shot label set, matching the example in the design spec (§4.1). Of
  these, only `iban` is aliased by `to_pii_category`'s alias table
  (`ner.rs`'s `to_pii_category` `Custom` arm) onto an existing `PiiCategory::Iban`
  variant; `swift_code` and `account_number` are **not** in that table and therefore
  arrive as `PiiCategory::Custom("swift_code")` / `PiiCategory::Custom("account_number")`
  rather than the more specific `PiiCategory::SwiftBic` / `PiiCategory::BankAccount`
  variants that exist in the taxonomy. This is a real, pre-existing footgun (see the
  implementation plan's Task 2.3 `should_map_an_aliased_vertical_label_to_its_taxonomy_category`
  test and the "Alias-table collision" risk row) — the fixture deliberately exercises
  both the aliased and non-aliased cases side by side so the harness's output makes the
  asymmetry visible rather than hiding it.

## Known limitation — single annotator

These fixtures were produced by **one annotation pass** and have **not** had a second,
independent re-annotation or an inter-annotator-agreement check. The implementation
plan's §1.1 checklist item for a second annotator pass is intentionally left unchecked.
**Treat every number this harness reports as provisional until a second person with
French/English legal-document experience has reviewed and, ideally, independently
re-annotated a sample.** This mirrors the caveat already carried in PR #42's own
description.

## Running the evaluation

The scoring math and fixture schema validity are covered by a normal (non-`#[ignore]`d)
`cargo test` and need no model. Running the model itself requires a local GLiNER2
model directory and the `ner-candle` feature, and is gated `#[ignore]` because CI does
not have a model available:

```sh
HACIENDA_EVAL_MODEL_DIR=~/model_f16 cargo test -p hacienda-core --features ner-candle --test ner_eval -- --ignored --nocapture
```

`HACIENDA_EVAL_MODEL_DIR` must point at a local model directory in the shape
`from_candle_local` expects (`config.json`, `encoder_config/`, `tokenizer.json`,
`tokenizer_config.json`, `model.safetensors`). If the variable is unset, the ignored
test prints a message and passes trivially rather than failing the build — the model
dependency is optional, never required for `cargo test -p hacienda-core --features
ner-candle --tests` to *compile*.

The report is written to `HACIENDA_EVAL_OUT` (default `target/ner-eval/<timestamp>.json`)
and records the model directory, the `model.safetensors` blake3 digest, the label set,
the threshold, and the full metrics table (see `ANNOTATION.md` and `ner_eval.rs`'s
module docs for the metric definitions, including why reported recall is labelled
"truncated recall at 0.5").
