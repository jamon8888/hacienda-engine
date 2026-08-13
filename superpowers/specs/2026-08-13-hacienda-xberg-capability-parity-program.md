# Programme — Parité des capacités pipeline xberg, liées à la preuve

**Date:** 2026-08-13
**Status:** Proposed — decomposition to validate before child specs are treated as final
**Scope:** `hacienda-engine`. This document specifies nothing itself: it cuts the program
into independent child specs, the same way `2026-08-01-hacienda-platform-parity-program.md`
does for the API/CLI/SDK/proof-layer program.
**Relationship to the existing program:** sibling, not a subsection. That program's **Piste
E** is REST-surface parity with Xberg Enterprise's *product* (`/v1/extract`, `/v1/rag/*`,
presets, uploads). This program is about a layer underneath that: how much of the
**upstream `xberg` crate's own capability** — OCR, embeddings, reranking, transcription,
summarization, translation, captioning, keyword extraction, layout detection — hacienda's
pipeline actually exercises today, versus what the crate ships. The two meet at one seam:
once a capability is enabled here, `2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md`'s
tool-inventory mechanism and the existing `/v1/documents` route give it API/CLI/MCP exposure
without further work — `HaciendaFacade::process` is the single call site every extraction
path already goes through.

---

## 0. Urgent finding, discovered while writing this program: read before anything else

Investigating X1 (below) surfaced a **live gap in already-shipped code**, verified by
reproduction, not inferred: `POST /v1/documents`, `hacienda extract`, and `hacienda-mcp`'s
`documents_process` all redact `ExtractedDocument.content` correctly but **leave `.tables`,
`.pages[].content`, and other structured fields completely unredacted** in the same
response. A control-corpus spreadsheet with a name/email/IBAN in one row redacts correctly in
`content` and appears in plaintext, twice more, in `tables` and `pages`. Full reproduction,
root cause, and fix design: `2026-08-13-P7-structured-field-redaction-gap.md`.

This is not a hypothetical risk this program is choosing to guard against — it is active in
the extraction-format features (`pdf`/`office`/`excel`/`email`/`hwp`/`hwpx`/`iwork`/`archives`)
already merged. **P7 is the actual highest-priority item across both this program and the
existing platform-parity program**, ahead of P6 despite its later spec number, and every
capability in this program that would add a new structured field
(`extracted_keywords`, `.summary`, `.translation`, `.page_classifications`, layout's
structure tree) is blocked on it, not merely related to it.

---

## 1. Objective and where it came from

**Objective, as given:** hacienda should offer the same capability set xberg exposes, plus
hacienda's own layer (pseudonymisation, audit, review, compliance) — and that layer must be
**additive and linked**, not a parallel, bypassable path. Every new capability enabled by
this program must arrive already wearing hacienda's redaction/audit discipline, never as a
second, unguarded way to get document content out of the system.

**What "linked" already means in this codebase, proven once.** `hacienda-api/src/handlers/rag_stream.rs`
redacts the caller's prompt and every retrieved chunk before calling
`hacienda_rag::answer_stream` — the one existing call into an LLM — with the comment
*"Mandatory PII redaction gate... [the LLM] never sees them."* That is the correct shape.
It is also, today, **discipline in one handler**, not a structural guarantee: `answer_stream`
itself documents *"this module performs no redaction itself"* and trusts the caller. Compare
to storage, where P1 makes the raw, unredacted store type **impossible to obtain from outside
the crate** — a future integration cannot forget the guard because there is nothing to call
that skips it. There is no equivalent for "text about to leave the process toward an LLM
API," and five of the capabilities below only work by making exactly that call. Closing that
gap (**P6**, §5) is this program's hard prerequisite for X3 — and, discovered while writing
this program, **P7** (§4) is a second, more urgent one already violated in shipped code.

**Everything else is comparatively mechanical.** Verified directly against the pinned xberg
commit's source (`crates/xberg/Cargo.toml`'s feature graph, `crates/xberg/src/*`), not against
documentation: most of xberg's capability surface is either already reachable for free
(pure-Rust features, same shape as the `pdf`/`office`/`excel`/... win already shipped) or
gated behind infrastructure decisions (ONNX Runtime, a native library, an LLM API key) that
are provisioning questions, not code-design questions.

---

## 2. Program invariants

Reuses the existing program's I1–I4 (`2026-08-01-hacienda-platform-parity-program.md` §2)
verbatim — nothing in this program relaxes "no unredacted text crosses a persistence
boundary," "every content operation writes an audit entry first," tenant isolation, or
"the route table is the one source of truth." One addition:

| # | Invariant | Test negatif attendu |
| --- | --- | --- |
| **I5** | **No unredacted text crosses a process boundary toward a third-party service**, LLM APIs included. Equivalent in kind to I1, extended from storage to network egress. | A call into an LLM client constructed from raw, undredacted input → refused at compile time or by a test asserting no such call site exists outside the guard (P6). |
| **I6** | **Redaction covers every text-bearing field of a response, not only the primary `content` field.** Found violated in shipped code (§0) — recorded as an invariant so it has a name and cannot be quietly re-violated the next time a new field is added. | A control-corpus value present in any `ExtractedDocument` field (table cell, page content, metadata, keyword, summary, ...) → walk the *entire* response value, not just top-level string fields, and assert absence (P7). |

---

## 3. Map of the program

```text
┌──────────────────────────────────────────────────────────────────────┐
│  P7 — Structured-field redaction gap (URGENT — live in shipped code,  │
│        blocks any new ExtractedDocument field: keywords/summary/...)  │
└────────────────────────────┬───────────────────────────────────────--┘
                             │
┌────────────────────────────┴───────────────────────────────────────--┐
│  P6 — LLM call enforcement point (blocks X3, extends P1's guard)      │
└────────────────────────────┬───────────────────────────────────────--┘
                             │ required before any capability calls an LLM
┌────────────────────────────┴──────────────────────────────────────────┐
│  X1 — Pure-Rust enrichment            X2 — ONNX/native-backed         │
│  (no new infra, keywords/summary      (infra provisioning decision,   │
│   gated on P7)                         transcription/layout gated too)│
├─────────────────────────────────────────────────────────────────────--┤
│  X3 — LLM-backed enrichment (needs P6 and, for 4 of 5 features, P7)   │
├─────────────────────────────────────────────────────────────────────--┤
│  X4 — Presets & diff engine: reuse-vs-reinvent decision                │
└─────────────────────────────────────────────────────────────────────--┘
```

X1/X2/X3/X4 do not depend on each other. **P7 gates every row that populates a new
`ExtractedDocument` field** (X1's `keywords`/`summarization`, X2's `transcription`/
`layout-*`, X3's `translation`/`summarization-llm`/`classification`/`structured`) — see each
child spec's own dependency line; P6 gates only X3 (the LLM-call boundary, a different
concern from P7's field-coverage one). Rows that don't touch either boundary (X1's
`qr-codes`/`tree-sitter`/`static-embeddings`/`candle-ocr`, X2's retrieval-time
`embeddings`/`reranker`/`sparse-embeddings`/`late-interaction`, all of X4) are unblocked and
can proceed immediately.

---

## 4. P7 — Structured-field redaction gap

**Full spec:** `2026-08-13-P7-structured-field-redaction-gap.md`. Summarised here because of
where it sits in the delivery order (§10), not restated in full — see the child spec for the
reproduction, root cause, and fix design.

**Problem, in one sentence.** `HaciendaFacade::process_batch_with_auth` redacts exactly one
field (`ExtractedDocument.content`); every other text-bearing field xberg populates —
`tables`, `pages` (which nests a *second* copy of both `content` and `tables`),
`formatted_content`, `metadata.authors`/`created_by`/`additional`, and (once X1/X2/X3 ship)
`extracted_keywords`, `summary`, `translation`, `page_classifications` — passes through
unredacted, in every transport (REST, CLI, MCP) that shares `HaciendaResult`.

**Fix, in one sentence.** Redact every string-bearing field on `ExtractedDocument`
generically (walk the structure, don't name each field), immediately preceded by a hotfix
that strips what isn't yet provably safe rather than emitting it — see the child spec §5 for
the (a)/(b) two-speed design.

---

## 5. P6 — LLM call enforcement point

**Problem.** `liter_llm::LlmClient` is called directly from `hacienda-rag`'s `answer_stream`,
with redaction performed by the caller (`hacienda-api`) as a matter of discipline, not
structure. `translation`, `summarization-llm`, `captioning`, `classification`, and
`structured` (xberg's `structured` feature, LLM-driven structured extraction) each need
their own call into an LLM. Under the current architecture, each of those five integrations
is a fresh opportunity to forget the redaction step `rag_stream.rs` got right — and unlike a
store write, there is no compile-time backstop.

**Scope.** A guard analogous to P1's `Guard<S>`, generalised from storage to LLM calls:

```text
GuardedLlm
  complete(prompt, context)  → detect → redact/pseudonymise → THEN call the LLM client
  stream(prompt, context)    → same, then stream tokens back
```

The raw, ungated call path (`liter_llm::LlmClient::chat_stream` today; any future direct use
of `xberg::llm`'s `complete_text`/`complete_with_json_schema` if X3 ever needs whole-response
rather than streaming calls) is not exported outside the crate that owns the guard. Where
this lives is an open decision this spec must settle before X3 starts: `hacienda-core` (so
every consumer — `hacienda-rag`, a future captioning/translation module — depends on it the
same way they depend on `PiiPipeline`) is the leading candidate, matching P1's own placement
rationale.

**Non-objectives.** Choosing which capabilities call an LLM (→ X3). Multi-provider routing,
cost controls, rate limiting — infrastructure concerns orthogonal to the redaction guarantee.

**Migration for the existing call site.** `hacienda_rag::answer_stream` keeps its documented
"performs no redaction itself" contract in this crate map, but `hacienda-api`'s
`rag_stream::answer` handler is refactored to call through `GuardedLlm` instead of calling
`redact_text_with_auth` twice by hand and then `answer_stream` directly — same behaviour,
now backed by the same structural guarantee every future caller gets, not a precedent every
future caller has to remember to copy.

**Exit criteria.** A test asserts no path from `HaciendaFacade` (or anything it exposes) into
`liter_llm`/`xberg::llm` exists that has not passed through `GuardedLlm` — the same "cannot
compile a bypass" property P1's own exit criterion asserts for stores. `rag_stream::answer`'s
existing control-corpus test (values never appear in the response) continues to pass after
the refactor, unchanged in what it proves.

---

## 6. X1 — Pure-Rust enrichment features

**Problem.** Six xberg capabilities need zero new infrastructure — no ONNX Runtime, no
native system library, and in most cases no model download — confirmed against
`crates/xberg/Cargo.toml`'s own feature definitions:

| Feature | What it does | Extra deps beyond what's already vendored |
| --- | --- | --- |
| `keywords` (`keywords-yake` + `keywords-rake`) | Keyword/keyphrase extraction | `stopwords` (already pure Rust) |
| `summarization` | Extractive TextRank summarisation | `stopwords` |
| `qr-codes` | QR code decoding from images | `rqrr`, `image` (already pulled in by `pdf`) |
| `tree-sitter` | Syntax-aware code file parsing | `tree-sitter-language-pack` |
| `static-embeddings` | Dense embeddings via model2vec — no ORT | `model2vec-rs`, `tokenizers`, `ndarray`, plus a one-time HF model download (network, not a system install) |
| `candle-ocr` (+ `candle-trocr`/`candle-paddleocr-vl`/`candle-glm-ocr`/`candle-deepseek-ocr`) | OCR via pure-Rust candle transformer inference — no Tesseract, no ORT | `xberg-candle-ocr`, HF model download. hacienda already vendors candle for its own GLiNER2 NER backend, so this adds no new inference runtime to the deployment, only new model weights |

**Scope.** Enable these on the workspace `xberg` dependency (same Cargo.toml entry the
`pdf`/`office`/... win already touched). For every feature whose output is new document text
(`keywords`, `summarization`, `candle-ocr`'s transcribed text, `tree-sitter`'s parsed
symbols if surfaced as text) — verify it flows through `xberg::extract_batch`'s existing
extraction path so `HaciendaFacade::process`'s PII stage sees it automatically, the same way
enabling `pdf` required no `hacienda-core` code change. `qr-codes` decodes to structured data
(a URL/string payload), not document prose, but the decoded string is still scanned by the
PII stage on the same principle — a QR code can encode a phone number or an email.
`static-embeddings` is not document text and is scoped by **I5's storage analogue, I1**: it
must only ever embed already-redacted/pseudonymised text — this is X1's one point of
contact with P1, not with P6 (no LLM call involved), and the existing E4 vector-store guard
already states the requirement generally.

**Non-objectives.** ONNX-backed variants of any of the above (→ X2). Wiring `candle-ocr`
into a first-class "OCR enabled" product surface with configuration knobs (backend choice,
per-request override) — this spec only gets the capability compiling and flowing through the
existing pipeline; a configuration UI is separate.

**Exit criteria.** `cargo check` clean with all six features added, mirroring the
`pdf`/`office`/... validation already done. A control-corpus test: a PII value embedded in a
scanned image (via `candle-ocr`) or a document whose keywords/summary would otherwise surface
it is redacted in every output path, the same test class `hacienda-api/src/handlers/audit.rs`
already runs for the REST surface.

**Sequencing.** No dependency on P6 or X2. Highest value-to-cost ratio in this program —
start here.

---

## 7. X2 — ONNX and native-backed enrichment features

**Problem.** The remaining extraction/retrieval capabilities need real infrastructure, not
just a Cargo feature:

| Feature | Infra needed | Notes |
| --- | --- | --- |
| `embeddings`, `reranker`, `sparse-embeddings`, `late-interaction` | ONNX Runtime (`ORT_LIB_LOCATION` or `ort-bundled`) + HF model download | Full-fidelity alternatives to X1's `static-embeddings`/no reranker equivalent |
| `layout-detection` | ONNX Runtime + HF model | Table/reading-order detection |
| `layout-tract` | Pure-Rust `tract` engine + HF model, **no ORT** | Cheaper infra than `layout-detection`, but xberg's own comment notes it loses TATR/SLANeXT table-structure recognition and PP-DocLayout-V3 (a `tract` op-translation bug) — prefer this only once that limitation is checked against hacienda's actual document mix |
| `transcription` | ONNX Runtime (Whisper) + HF model + `symphonia`/`lofty` (pure-Rust audio decode) | Audio/video → text; spoken PII (names, phone numbers) is a real risk, same PII-stage requirement as any other extractor |
| `ocr` (Tesseract) | Native Tesseract binary | The heavier-infra sibling of X1's `candle-ocr`; only worth adding if candle's backends don't cover a needed format/language |

**Scope.** For each row: a provisioning decision (which ORT strategy — bundled download vs.
system-linked, matching the existing `.ai-rulez` OCR/ONNX skill docs' guidance for xberg
itself), then the same Cargo-feature-plus-PII-stage-verification pattern as X1. `transcription`
and `layout-detection`/`layout-tract` feed into `xberg::extract_batch` the same way `pdf` does,
so once enabled they get audit/redaction coverage automatically; `embeddings`/`reranker`/
`sparse-embeddings`/`late-interaction` are retrieval-time, not extraction-time, and fall under
E4's existing vector-store guard (embed/rerank only redacted text) rather than a new one.

**Non-objectives.** Choosing a default embedding/reranking model or exposing model selection
as a product-facing config surface — infra enablement only, product decisions separate.
GPU acceleration passthroughs (`candle-cuda`/`candle-metal`/etc.) — deployment-target
decisions, not part of this spec's exit criteria.

**Decision to settle before implementation.** Bundled ORT (`ort-bundled`, downloads a
prebuilt runtime, needs glibc ≥ 2.38) vs. system-linked (`ort-dynamic`) — xberg's own
`publish.yaml` comment notes it uses bundled in dev, system-linked in release for exactly
this reason. hacienda should make the same call explicitly rather than inheriting whichever
one a feature happens to default to.

**Exit criteria.** Each enabled feature's `cargo check` passes with the chosen ORT strategy
documented in the same Cargo.toml comment style the existing `xberg` entry already uses.
Extraction-time features (`transcription`, `layout-*`) pass the same control-corpus PII test
as X1. Retrieval-time features (`embeddings`, `reranker`, ...) are exercised only behind E4's
existing guard — no new test class needed if E4's is written generally enough; verify that it
is, or extend it, as part of this spec's exit criteria.

**Sequencing.** Independent of P6/X1/X3. Gate on the ORT-strategy decision, then proceed
per-row in whatever order matches product priority — nothing here blocks anything else.

---

## 8. X3 — LLM-backed enrichment features

**Problem.** Five xberg capabilities work only by calling an LLM through `liter-llm`:
`translation`, `summarization-llm` (abstractive, vs. X1's extractive TextRank), `captioning`
(image → text description), `classification` (document/text categorisation), and
`structured` (schema-guided structured extraction). Each sends document content to a
third-party API — the exact boundary P6 exists to guard.

**Scope.** Once P6 ships, each of these five is: enable the Cargo feature, route its call
through `GuardedLlm` rather than a raw `liter_llm`/`xberg::llm` call, and decide **per
feature** whether the LLM call happens before or after redaction:

- **Redact-first (the safe default):** the LLM never sees raw PII — correct for data
  minimisation (GDPR Art. 5) regardless of what the feature is for, and the only option that
  needs no new review. Costs fidelity: `captioning` describing a redacted `[PERSON]` token
  instead of a name, `translation` of a document with `[EMAIL]` tokens in place of addresses.
- **Raw-input, redact-output:** better fidelity, but means an unredacted request left the
  process — a materially different risk (I5) than a redacted one, and needs its own sign-off
  per feature, not a blanket policy.

This decision is not free to defer per capability — `captioning` in particular takes an
*image*, not text, and the pipeline has no image-redaction step today (visible PII in a photo
— a face, a name badge, a document photographed in the background — is not covered by any
existing control). **`captioning` is therefore not exit-clean under the redact-first default**
until an image-redaction story exists or is explicitly scoped out; the other four operate on
text, where the existing PII pipeline already applies.

**Non-objectives.** Building the image-redaction capability itself (a real gap, but a
separate scope — visual PII detection is a different problem from text PII detection, likely
needing its own model). Multi-turn LLM workflows, agentic tool use — these are single-call
enrichment features, not conversational surfaces.

**Exit criteria.** Every one of the five, where enabled, states explicitly in its own PR
which policy (redact-first vs. raw-input-redact-output) it uses and why, mirroring the
program's existing D3 (diff-mode-degradation) and V2 (adapter-fallback) precedent of
"degradation must be announced, never silent." `captioning` either ships with an explicit
image-redaction answer or stays unshipped, not shipped with a silent gap.

**Sequencing.** Blocked on P6. Independent of X1/X2/X4.

---

## 9. X4 — Presets and diff engine: reuse vs. reinvent

**Problem.** hacienda already reinvented two things xberg ships as reusable modules:

- **Presets.** xberg's `presets` feature ships a format, registry, loader, and *resolver*
  (`xberg::presets::resolve`, merge-mode logic over a base preset plus overrides).
  hacienda's `/v1/presets` is a separate Postgres-backed CRUD store
  (`hacienda_core::store::postgres::presets::PresetStore`) with none of that — confirmed by
  reading both `crates/xberg/src/presets/mod.rs` and `hacienda-api/src/handlers/presets.rs`
  directly; they share no code.
- **Diff.** xberg's `diff` feature (`crates/xberg/src/diff/`, built on the pure-Rust
  `similar` crate) structurally diffs two `ExtractedDocument`s — tables, embedded content,
  hunks. hacienda's `/v1/documents/{id}/diff` (E2) instead diffs on pseudonymised tokens,
  by design (D3 in the platform-parity program: diffing raw text would violate I1). That
  design choice is right; what's untested is whether xberg's *diff engine* could be the
  mechanism underneath it — diffing two already-pseudonymised `ExtractedDocument`s structurally,
  instead of hacienda's own from-scratch text diff — which would get table-aware and
  embedded-content-aware diffing for free.

**Scope.** Two independent per-feature decisions, not a single verdict:

1. **Presets:** does adopting xberg's resolver buy anything hacienda's simpler CRUD store
   doesn't already cover? The platform-parity program's own D5 called named/versioned
   presets "pure commodity, no bid stops there" — this spec should confirm that verdict
   still holds against the resolver specifically (merge-mode composition is a real capability
   gap, not just naming) before deciding to leave it reinvented.
2. **Diff:** can xberg's `diff` module run on two pseudonymised `ExtractedDocument`s as E2's
   underlying engine, replacing hacienda's own text diff with something table/embedded-aware,
   without reopening I1 (the module never sees unredacted content if fed post-guard input)?

**Non-objectives.** Rewriting either system speculatively before the decision is made.

**Exit criteria.** A written decision per feature (adopt / confirm-reinvent, with the reason
either way) — this spec's deliverable is the decision, not necessarily new code. If "adopt"
for diff: E2's diff output is unchanged for a caller (same pseudonym-token-diff contract),
only the engine underneath changes, verified by E2's existing tests continuing to pass.

**Sequencing.** Independent of everything else in this program. Lowest priority — a
correctness/leverage question, not a missing capability.

---

## 10. Delivery order

| Wave | Content | Why |
| --- | --- | --- |
| **-1** | **P7 (a), the hotfix** | Urgent — live in shipped code (§0). Strip/null the unredacted structured fields now; (b), the real recursive-redaction fix, follows but should not gate stopping the active leak |
| **0** | **P7 (b)** and **P6**, in parallel | P7(b) unblocks `keywords`/`summarization`/`transcription`/`layout-*`/4-of-5 X3 features; P6 unblocks X3's LLM calls and closes I5 for the one call site that already exists (`rag_stream`). Independent of each other |
| **1** | **X1** rows not gated on P7 (`qr-codes`/`tree-sitter`/`static-embeddings`/`candle-ocr`) | Zero new infrastructure, same shape as the already-shipped extraction-format win — highest leverage for the effort; can start immediately, does not wait on wave 0 |
| **1b** | **X1**'s `keywords`/`summarization` | After P7(b) |
| **2** | **X2** | Gated on the ORT-strategy decision (§7); retrieval-time rows independent of P7, extraction-time rows (`transcription`/`layout-*`) gated on P7(b) same as X1 |
| **3** | **X3** | After P6; 4 of 5 features additionally after P7(b); `captioning` specifically waits on an image-redaction answer regardless of both |
| **—** | **X4** | No dependency on the others; schedule whenever, lowest priority |

---

## 11. What this program does not cover

| Exclusion | Reason |
| --- | --- |
| Image/visual PII redaction | A real gap surfaced by X3's `captioning` row, but a distinct capability (visual, not textual, detection) — needs its own spec once scoped |
| The 8 first-party framework integrations (LangChain, LlamaIndex, CrewAI, txtai, SurrealDB, Spring AI, n8n) | Already the subject of spec **E0** in the existing platform-parity program |
| xberg's plugin system (custom extractors/OCR/embedding backends) | Not feature-gated in xberg (always compiled) and not an API/CLI/MCP-facing capability — an extensibility hook for third parties, lower priority than the product-facing gaps above |
| Model selection / fine-tuning for any ONNX or LLM-backed feature | Product configuration, not a parity gap |
| GPU acceleration (`candle-cuda`/`candle-metal`/`candle-accelerate`/`candle-mkl`) | Deployment-target decision, orthogonal to whether a capability exists at all |
