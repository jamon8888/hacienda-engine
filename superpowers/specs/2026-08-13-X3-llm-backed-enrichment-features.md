# X3 — LLM-backed enrichment features

**Date:** 2026-08-13
**Status:** Proposed
**Program:** `2026-08-13-hacienda-xberg-capability-parity-program.md` §7
**Depends on:** `2026-08-13-P6-llm-call-enforcement-point.md` (the guard these five features
call through) **and** `2026-08-13-P7-structured-field-redaction-gap.md` (three of the five
populate `ExtractedDocument` fields P7 governs — see §3)
**Blocks:** nothing

---

## 1. Problem

Five xberg capabilities work only by calling an LLM through `liter-llm`: `translation`,
`summarization-llm` (abstractive, versus X1's extractive TextRank `summarization`),
`captioning` (image → text description), `classification` (document/text categorisation),
and `structured` (schema-guided structured extraction). Each sends document-derived content
to a third-party API — exactly the boundary P6 exists to guard, and (for the four that are
extraction-time, not chat-time) exactly the kind of `ExtractedDocument` field P7 exists to
guard once the response comes back.

## 2. Objectives / Non-objectives

**Objectives**

- Enable each feature's Cargo dependency only after P6 ships, routing its LLM call through
  `GuardedLlm` — never a direct `liter_llm`/`xberg::llm` call, per P6 §3's D-P6-1.
- Decide, **per feature**, whether the call is redact-first or raw-input-redact-output (§4),
  and announce that decision rather than leaving it implicit.
- For `translation`/`summarization-llm`/`classification` (they populate
  `ExtractedDocument.translation`/`.summary`/`.page_classifications`): confirm P7(b) covers
  those fields before shipping — same rule X1 applies to `keywords`/`summarization`.

**Non-objectives**

| Deferred | Reason |
| --- | --- |
| Image redaction for `captioning`'s input | A real, distinct gap (visual PII detection) — flagged in §5, not built here |
| Multi-turn LLM workflows, agentic tool use | These are single-call enrichment features, not a conversational surface |
| Provider selection/cost/rate-limiting policy | Infrastructure, not a redaction-guarantee question — P6's own non-objective, inherited here |

## 3. Which fields each feature touches

| Feature | `ExtractedDocument` field | P7 dependency |
| --- | --- | --- |
| `translation` | `translation: Option<Translation>` | Yes — separate field, same shape as `extracted_keywords` |
| `summarization-llm` | `summary: Option<DocumentSummary>` | Yes — same field X1's `summarization` uses; whichever backend runs first, P7(b) must already cover it |
| `classification` | `page_classifications: Option<Vec<PageClassification>>` | Yes |
| `structured` | `ExtractionConfig::structured_extraction`'s own output shape (not yet inventoried against `ExtractedDocument`'s fields — verify before enabling, don't assume it's covered) | To confirm |
| `captioning` | Image → text; lands wherever `ExtractedImage`'s caption field is (verify against `crates/xberg/src/types/extraction.rs`'s `ExtractedImage` definition before enabling) | To confirm, likely yes |

## 4. Redact-first vs. raw-input-redact-output: a per-feature decision, not a policy

**Redact-first (the safe default).** The LLM never receives raw PII — correct for data
minimisation (GDPR Art. 5) independent of what the feature does with the text, and needs no
additional review before shipping. Costs fidelity: a translation of "`[EMAIL]`" instead of an
address, a summary that says "`[PERSON]` agreed to `[ORG]`'s terms" instead of using names.

**Raw-input, redact-output.** Better fidelity — the LLM sees real names/values and produces
better prose — but means unredacted content left the process boundary toward a third party.
This is a materially different risk (I5) than a redacted call, not a smaller version of the
same risk, and needs an explicit sign-off per feature that ships it, not a blanket default.

**This spec does not pick one policy for all five.** Each feature's own PR states which it
uses and why — the same "degradation must be announced, never silent" rule the platform-parity
program already applies to E2's diff-mode fidelity (D3) and V2's adapter fallback. A silent
default, in either direction, is the failure mode this spec exists to prevent: silently
redact-first would surprise a customer expecting translation-quality fidelity; silently
raw-input would surprise a customer who chose this product specifically because it doesn't do
that.

**Recommendation where no product requirement says otherwise: redact-first.** It's the
option that needs no new review, and it's consistent with every other default in this
codebase (`RedactionConfig::default().mode = Mask`, not `Pseudonymize`; CLI `--no-redact`
requires explicit acknowledgement). Treat raw-input as an opt-in a specific feature earns by
stating why redact-first isn't good enough for it, not a starting position.

## 5. `captioning` is not exit-clean under redact-first, and may not be exit-clean at all yet

`captioning` takes an **image**, not text. Redacting "first" has no meaning for an image the
way it does for a text field — there is no text-PII pipeline step to run before the picture
reaches the LLM. If the image contains visible PII (a face, a name badge, a document
photographed in the background, a whiteboard with names on it), nothing in this stack detects
or redacts it today; the raw image bytes go to the captioning LLM regardless of which policy
§4 otherwise picks.

**`captioning` therefore does not ship as part of this spec** until one of:

- An image-redaction capability exists (a distinct scope — visual PII detection is a
  different problem from text PII detection, almost certainly needing its own model, not a
  regex/NER port) and is applied before the image reaches the LLM, or
- A product decision explicitly accepts the risk for a scoped use case (e.g. a deployment
  that only ever processes documents already known not to contain photographs of people) and
  states that acceptance in writing, the same way any other deliberate, narrower guarantee in
  this codebase is documented rather than silently assumed.

This is the one place in the xberg-capability-parity program where "additive and linked"
cannot currently be delivered — flagging it precisely, rather than shipping `captioning` with
a silent gap, is this spec's actual deliverable for this row.

## 6. Exit criteria

- P6 shipped; every one of the four text-based features (`translation`, `summarization-llm`,
  `classification`, `structured`) routes its LLM call through `GuardedLlm`, verified by the
  same "no unguarded `liter_llm`/`xberg::llm` call site" check P6's own exit criteria define.
- P7(b) shipped and covers `.translation`/`.summary`/`.page_classifications`/`structured`'s
  output shape before any of those four ship.
- Each shipped feature's PR states its redact-first/raw-input choice and reason (§4).
- `captioning` is either unshipped with the reason stated (§5), or shipped alongside a stated
  image-redaction answer — never shipped silently without one.

## 7. Sequencing

Blocked on P6 (all five) and P7(b) (`translation`, `summarization-llm`, `classification`,
`structured` — not `captioning`, which is blocked on the image-redaction question in §5
regardless of P7). Independent of X1/X2/X4.
