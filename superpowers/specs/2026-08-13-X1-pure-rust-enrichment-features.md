# X1 — Pure-Rust enrichment features

**Date:** 2026-08-13
**Status:** Proposed
**Program:** `2026-08-13-hacienda-xberg-capability-parity-program.md` §5
**Depends on:** **`2026-08-13-P7-structured-field-redaction-gap.md` for `keywords` specifically**
— `extracted_keywords` is exactly the field shape P7 exists to stop shipping unguarded (see
P7 §5). `summarization`/`qr-codes`/`tree-sitter`/`static-embeddings`/`candle-ocr` do not add a
new structured field in the same way and are not blocked on P7.
**Blocks:** nothing

---

## 1. Problem

Six xberg capabilities compile and run with **no ONNX Runtime, no native system library, and
in most cases no model download** — confirmed by reading `crates/xberg/Cargo.toml`'s feature
graph directly, the same way the already-shipped extraction-format win was verified. Every
one of them is currently off. This is the same category of gap as "hacienda couldn't extract
a PDF" was before that fix: capability sitting in the dependency graph, unreachable only
because a Cargo feature was never flipped.

| Feature | What it does | New dependencies | Model download? |
| --- | --- | --- | --- |
| `keywords` (`keywords-yake` + `keywords-rake`) | Keyword/keyphrase extraction | `stopwords` (already-vendored pure Rust) | No |
| `summarization` | Extractive TextRank summarisation | `stopwords` | No |
| `qr-codes` | QR code decoding | `rqrr`, `image` (already pulled in transitively by `pdf`) | No |
| `tree-sitter` | Syntax-aware code parsing | `tree-sitter-language-pack` | No |
| `static-embeddings` | Dense embeddings via model2vec, no ORT | `model2vec-rs`, `tokenizers`, `ndarray`, `hf-hub`, `reqwest` | Yes, once, from Hugging Face |
| `candle-ocr` (+ `candle-trocr`/`candle-paddleocr-vl`/`candle-glm-ocr`/`candle-deepseek-ocr`) | OCR via candle transformer inference, no Tesseract, no ORT | `xberg-candle-ocr`, `hf-hub`, `reqwest` | Yes, per backend |

`candle-ocr` is worth calling out specifically: hacienda **already vendors candle**, for its
own GLiNER2 NER backend (`hacienda-core`'s `ner-candle` feature). Adding `candle-ocr` costs
the deployment no new inference runtime — only new model weights, downloaded the same way
the NER model already is.

## 2. Objectives / Non-objectives

**Objectives**

- Enable all six features on the workspace `xberg` dependency.
- For every feature producing new document text (`keywords`, `summarization`, `candle-ocr`'s
  transcribed text) or new structured-but-scannable data (`qr-codes`' decoded payload,
  `tree-sitter`'s extracted symbols if surfaced): confirm it reaches `HaciendaFacade::process`'s
  existing PII stage automatically, the same way `pdf` needed no `hacienda-core` change.
- For `static-embeddings`: confirm it is only ever called on already-redacted/pseudonymised
  text (E4's existing vector-store guard), since embedding raw PII is a similarity-search
  leak, not a storage-format question — no new guard needed here, just verification that the
  existing one applies.

**Non-objectives**

| Deferred | Reason |
| --- | --- |
| ONNX-backed alternatives (`embeddings`, `layout-detection`, `ocr`/Tesseract, `transcription`) | → X2, real infra provisioning decisions |
| Product-facing configuration (backend choice, per-request override, model selection) | Capability enablement only; configuration surface is separate follow-up work |
| GPU acceleration passthroughs for `candle-*` | Deployment-target decision, not a parity gap |

## 3. Per-feature notes

**`keywords`/`summarization`.** Both are pure text-in, text/list-out transforms over already
extracted content — the natural integration point is inside `xberg::extract_batch`'s pipeline
(if xberg wires them as post-processors keyed by `ExtractionConfig` flags) or as a
`HaciendaFacade`-level post-step applied to `ExtractedDocument.content` after extraction,
before the PII stage runs. Either shape works; the requirement is only that PII detection
sees the keyword/summary output too, not just the original content, since a summary or a
"keyword" can itself be a PII value (a person's name is a plausible YAKE/RAKE keyword).

**`qr-codes`.** Decodes to a string payload, not prose. Still routed through the PII stage:
a QR code can encode a phone number, an email, or a vCard. Treat the decoded payload as
`ExtractedDocument` content for PII-scanning purposes even though it isn't natural-language
text.

**`tree-sitter`.** Lower priority than the others — code files carry PII less often than
prose formats, though hardcoded credentials, internal hostnames, or committed personal data
in comments are a real (if narrower) case. Scan its output the same as any other extracted
text; do not special-case it as PII-exempt just because it's code.

**`static-embeddings`.** Not extraction-time — this is retrieval infrastructure (E4). The
integration point is `hacienda-rag`'s embedding step, and the requirement is identical to
what E4 already states for any embedder: only ever called on redacted/pseudonymised chunk
content, never on raw text pulled straight from extraction. This spec's job is to confirm
E4's guard is written generally enough to cover a *second* embedder (today it presumably
only has ONNX `embeddings` in mind, or none at all, wired) — extend it if not.

**`candle-ocr`.** Integration point is `xberg::extract_batch` for image/scanned-PDF inputs —
same as any other extractor. `candle-ocr` has four backend sub-features
(`candle-trocr`, `candle-paddleocr-vl`, `candle-glm-ocr`, `candle-deepseek-ocr`); start with
one (TrOCR is the smallest/best-understood) rather than enabling all four in the first pass —
each adds its own model download and inference cost, and the product decision of which
backend(s) to ship is separate from proving the integration works at all.

## 4. Exit criteria

- `cargo check -p hacienda-core -p hacienda-api -p hacienda-cli -p hacienda-mcp` passes clean
  with all six features enabled (`candle-trocr` only, not the full `candle-vlm-ocr` umbrella,
  for the first pass per §3).
- A control-corpus test: a PII value present in test input to `keywords`/`summarization`
  never survives unredacted in the corresponding tool call's / route's output. A PII value
  visible in a scanned test image, run through `candle-ocr`, is redacted in
  `documents_process`'s (hacienda-mcp) and `POST /v1/documents`'s (hacienda-api) output —
  the same class of test `hacienda-api/src/handlers/audit.rs` already runs, extended to a
  new input modality (an image, not a text file).
- `qr-codes`: a QR-encoded PII value is redacted in the tool/route output, same test class.
- E4's vector-store guard, confirmed (or extended) to cover `static-embeddings` as a second
  embedder alongside whatever it already guards.

## 5. Sequencing

No dependency on P6 (no LLM call here at all) or X2. This is the highest-leverage remaining
slice in the program: same cost profile as the already-shipped `pdf`/`office`/`excel`/...
change (a Cargo feature flip plus a PII-stage verification test), applied to six more
capabilities. See the accompanying implementation plan,
`2026-08-13-X1-pure-rust-enrichment-features-implementation.md`, for the concrete task
breakdown.
