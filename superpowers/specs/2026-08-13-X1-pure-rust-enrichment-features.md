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
capabilities.

## 6. Implementation notes (this pass)

**`tree-sitter` deferred — genuine upstream compile break, not a scope decision.** This
pass shipped five of the six capabilities; `tree-sitter` stayed off. This xberg commit's
`impl From<&TreeSitterProcessConfig> for tree_sitter_language_pack::ProcessConfig`
(`src/core/config/tree_sitter.rs`) builds that struct with a field literal that omits
`max_source_bytes` and `parse_timeout_ms` — fields the `tree-sitter-language-pack`
version Cargo currently resolves to (1.15.0, under this xberg commit's own unpinned
`^`-range requirement) requires. `cargo check -p hacienda-core --features tree-sitter`
fails inside the `xberg` crate itself, not in hacienda's code — reproduced locally after
separately working around `tree-sitter-language-pack`'s own build-time issue (its build
script fetches a parser-sources tarball from a GitHub release at build time; that needs
`TSLP_OFFLINE=1` or real network access to a non-intercepted `github.com` to succeed —
unrelated to the compile error, just what was needed to isolate it). See the workspace
`Cargo.toml`'s `xberg` entry for the full note. `HaciendaFacade::redact_structured_fields`
covers `code_intelligence` on the list of fields *not yet* covered (alongside the other
Cargo-feature-gated fields already there) rather than adding dead code for a field that
cannot currently exist in this build.

**`candle-trocr` regression risk, found and fixed during implementation.** Enabling
`candle-ocr`/`candle-trocr` compiles xberg's `ocr-pipeline` into `extractors::image`,
which changes its default behaviour even for callers who never touch OCR: previously an
image input always fell back to metadata-only extraction; with `ocr-pipeline` compiled,
`ExtractionConfig::effective_disable_ocr()` (default `false`) no longer skips the OCR
branch, and a config with no `[extraction.ocr]` section falls through to
`OcrConfig::default()`, whose backend (`"tesseract"`) is not registered in this build
(`ocr`/`ocr-wasm` stay off). Left alone, that turns every image extraction into a hard
`Plugin` error for every existing caller. `HaciendaFacade::safe_extraction_config`
(`hacienda-core/src/facade.rs`) closes this by forcing `disable_ocr = true` unless the
caller's config already set `extraction.ocr` explicitly, so OCR stays a no-op by default
— exactly the pre-`candle-trocr` behaviour — until a caller opts in with
`[extraction.ocr] backend = "candle-trocr"`.

**§3's `static-embeddings` guard assumption does not hold.** This section says
confirming/extending "E4's existing vector-store guard" is enough — but no such guard
exists in the current implementation. `hacienda-api/src/handlers/rag.rs::upsert_document`
stores caller-supplied chunk content verbatim, with no redaction call anywhere in the
file (its own doc comment already states the caller is responsible for pre-redacting or
re-embedding after upsert — a documented expectation, not an enforced one), and
`migrate_embeddings_work` reads stored chunk content straight from `RagStore` and feeds it
to `xberg::embed_texts_async` with no redaction step in between. This is a pre-existing
gap in E4 (`2026-08-01-hacienda-platform-parity-program.md` §6 names "chaque chunk est
rédigé avant vectorisation" as E4's single most important guard), not something this pass
introduces — X1 only surfaces it because `static-embeddings` would be a second embedder
needing the same guard `P6`'s `GuardedLlm` (`crates/hacienda-rag/src/stream.rs:133-186`)
already provides for the query-time LLM-answer path.

Given that, `static-embeddings` is enabled on the workspace `xberg` dependency in this
pass (so it compiles) but deliberately **not** threaded through `hacienda-rag`/
`hacienda-api`'s own Cargo features or config surface: nothing today constructs the
`EmbeddingModelType::Preset { name: "lightweight" }` config needed to reach the static
backend through `xberg::embed_texts`/`embed_texts_async`, so this stays compile-only and
does not newly expose an unguarded ingestion path. Building an ingestion-time guard
(`GuardedLlm`-shaped, wrapping the embed call sites in `upsert_document` and
`migrate_embeddings_work`) is real, separate work, tracked as its own follow-up rather
than folded into this pass.

**`keywords`/`summarization`/`qr-codes`/`candle-trocr` are reachable today.** Unlike
`static-embeddings`, these already have a config surface — `HaciendaConfig.extraction`
*is* xberg's own `ExtractionConfig`, round-tripped through `[extraction]` in
`hacienda.{toml,yaml,json}` — so enabling the Cargo feature plus extending
`HaciendaFacade::redact_structured_fields` to cover the fields they populate
(`extracted_keywords`, `summary`, `images[].qr_codes`/`.caption`/`.description`/
`.ocr_result`) is a complete, callable capability, not a compile-only stub. See
`redact_structured_fields`'s own doc comment for the full field list and reasoning.
`tree-sitter`'s `code_intelligence` would follow the same shape once the upstream compile
break above clears — deliberately left off the covered-field list for now rather than
adding dead code for a field that cannot currently exist in this build.

**Test scope: core-level, not a new REST/CLI/MCP-level control-corpus test per
transport.** §4's exit criteria describe testing `documents_process`'s (hacienda-mcp) and
`POST /v1/documents`'s (hacienda-api) output directly, mirroring P7's own framing of "REST,
CLI, MCP" as three separate transports needing separate coverage. Checked directly against
current code before adding a new test file: `hacienda-api/src/dto.rs`'s `DocumentResult` —
the actual `POST /v1/documents` response shape — exposes only `content`, `entities`,
`document_id`, `version_sequence`; none of `tables`/`extracted_keywords`/`summary`/`images`/
etc. are serialised into that response at all (confirmed by grep: zero references to
`ExtractedDocument`'s structured fields anywhere under `hacienda-api/src/`). There is
nothing at that REST layer for a corpus sweep to catch, independent of this pass's changes.
`hacienda-cli`'s `--format json` output (`commands.rs`) and `hacienda-mcp`'s
`documents_process` tool result both serialise the `ExtractedDocument` returned by
`HaciendaFacade::process_batch_with_auth` directly, with no additional field-stripping —
confirmed by reading both call sites — so both automatically carry
`redact_structured_fields`'s redaction with no CLI/MCP-side code change needed; the
correctness proof lives entirely in the core-level test
(`redact_structured_fields_covers_every_known_field`, extended this pass to cover
`extracted_keywords`/`summary`/`images[].qr_codes`/`.caption`/`.description`/`.ocr_result`),
not in a duplicate per-transport HTTP/MCP round trip. A `candle-trocr` end-to-end test
using a real scanned image would additionally need the TrOCR model weights downloaded from
Hugging Face at test time — infeasible offline and undesirably flaky/slow even where network
access exists — so that specific exit-criterion test (as literally scoped: OCR a real image
through the real model) is not attempted; `redact_structured_fields`'s coverage of
`images[].ocr_result` as a recursively-redacted nested `ExtractedDocument` is exercised
today via a synthetic `ocr_result` in the same core-level test, proving the redaction wiring
without requiring the model.
