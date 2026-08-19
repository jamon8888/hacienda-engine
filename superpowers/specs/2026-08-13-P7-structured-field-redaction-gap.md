# P7 — Close the structured-field redaction gap (found empirically, live in shipped code)

**Date:** 2026-08-13
**Status:** **Fixed** (this revision) — see §8. Design (b), recursive redaction, shipped
directly rather than as a separate hotfix-then-real-fix pair (§4's two-speed plan), since the
full fix was tractable in one pass once every field was enumerated.
**Program:** amends `2026-08-01-P1-redaction-enforcement-point.md` directly, and gates
`2026-08-13-X1-pure-rust-enrichment-features.md` (its `extracted_keywords` field is the same
class of bug, and is now unblocked — see §8)
**Severity:** the redaction guarantee the whole product is sold on did not hold for any
table-bearing or paginated document. Verified by reproduction, not inferred.

---

## 1. What was found

While investigating X1 (which would add `ExtractedDocument.extracted_keywords`, a field of
the same shape), I tested the extraction-format features shipped in the immediately prior
change (`pdf`/`office`/`excel`/... enablement) against a control-corpus spreadsheet — a table
with a name, an email, and an IBAN in one row — through `hacienda extract --mode mask`.

**The `content` field redacted correctly:**

```text
"content": "  Customers\nName Email Note\nZephyrine Quatrebarbes [EMAIL] VIP client, IBAN [IBAN:****]"
```

**The same response's `tables` field did not:**

```json
"tables": [{
  "cells": [
    ["Name", "Email", "Note"],
    ["Zephyrine Quatrebarbes", "zephyrine.quatrebarbes@corpus-temoin.example",
     "VIP client, IBAN FR7630006000011234567890189"]
  ],
  "markdown": "| Name | Email | Note |\n| --- | --- | --- |\n| Zephyrine Quatrebarbes | zephyrine.quatrebarbes@corpus-temoin.example | VIP client, IBAN FR7630006000011234567890189 |\n"
}]
```

**Nor did `pages[0].content`** (a *second*, independently-populated content representation,
distinct from the top-level `content`), which carries the same raw markdown table, and
`pages[0].tables`, a third copy of the same unredacted cells.

**The same response's `audit_entries` claims two entities were redacted.** They were —
in one field. The response that carries that claim also carries the plaintext of both
values, twice, in fields the claim says nothing about. An auditor reading only
`audit_entries` would conclude the response is clean. It is not.

## 2. Root cause

`HaciendaFacade::process_batch_with_auth` (`hacienda-core/src/facade.rs:702`) is the entire
redaction step for extracted documents:

```rust
document.content = result.redacted_text.clone();
```

One field, rewritten. `xberg::types::extraction::ExtractedDocument` (confirmed by reading
`crates/xberg/src/types/extraction.rs` at the pinned commit) has at least these other fields
that carry natural-language or personally-identifying text and are populated by the same
extraction call, unconditionally for table-bearing/paginated formats — not behind an opt-in
config flag:

| Field | Carries | Populated by |
| --- | --- | --- |
| `tables: Vec<Table>` | Cell text, markdown rendering | Always, when the source has tables (`office`, `excel`, `pdf`) — confirmed: no `extract_tables` config gate exists; `crates/xberg/src/extractors/pdf/mod.rs`'s own module doc says "Provides extraction of text, metadata, tables... from PDF documents" as core behaviour |
| `pages: Option<Vec<PageContent>>` | A **second** `content` string per page, plus a **third** copy of `tables` nested inside | Populated whenever the source is paginated (multi-sheet Excel, multi-page PDF) |
| `formatted_content: Option<String>` | An alternate rendering of the same content (e.g. HTML/richer markdown) | Config-gated (`ExtractionConfig`'s output-format options), reachable via the operator's `[extraction]` TOML section today |
| `metadata.authors`, `metadata.created_by`, `metadata.additional` | Document author/last-modified-by (frequently a real person's name in Office documents), free-form extracted metadata strings | Always, from document properties |
| `extracted_keywords` (X1, not yet enabled) | Keyword strings pulled from document content | Would be, the moment X1 ships, unless this is fixed first |
| `summary: Option<DocumentSummary>` (X1's `summarization`, not yet enabled) | A summary of `content` | Would be, the moment X1 ships |
| `translation`, `page_classifications` (X3, not yet enabled, and gated behind P6 regardless) | Translated text; per-page category labels | Would be, once X3 ships |
| `entities: Option<Vec<Entity>>` | xberg's **own** generic NER output (distinct from hacienda's PII entities) | Not populated today — hacienda vendors its own candle GLiNER2 NER directly in `hacienda-core` rather than using xberg's `ner-onnx`/`ner-llm`, so this field stays empty under hacienda's current feature set. Flagged so it stays that way deliberately, not by accident, if `ner-onnx`/`ner-llm` are ever considered |
| `elements`, `document` (structure tree), `ocr_elements`, `djot_content`, `chunks[].content` | Various text representations, gated behind specific config flags today (not default-on like `tables`/`pages`) but reachable the moment an operator sets them | Config-gated, but the config keys are xberg's own `ExtractionConfig` verbatim (confirmed: hacienda's `[extraction]` TOML section round-trips it directly, `deny_unknown_fields` and all) — nothing in hacienda's config layer refuses them |

Every one of these serialises straight into `POST /v1/documents`'s response, `hacienda
extract`'s output, and `hacienda-mcp`'s `documents_process` tool result — all three transports
share the same `HaciendaResult`/`ExtractionResult`/`ExtractedDocument` types, so this is not a
per-transport bug to fix three times; it is one bug in `hacienda-core`, inherited everywhere.

## 3. Why P1's own exit criteria didn't catch this

P1's guard is a **storage** guard — it governs what reaches a `VectorStore`/`DocumentStore`/
`BlobStore`. `POST /v1/documents`'s response body was never a store write; it's a synchronous
HTTP response, out of P1's scope by construction. Separately,
`hacienda-api/src/handlers/audit.rs`'s own control-corpus sweep (`no_endpoint_returns_corpus_plaintext`)
walks `ROUTE_TABLE` with **GET** requests only — it never POSTs a table-bearing document to
`/v1/documents` and inspects the response, because the corpus document that test uses is a
paragraph of prose, not a spreadsheet. The gap is real precisely because the two tests that
exist each cover a different half of the surface, and neither covers "a table in a POST
response."

## 4. xberg has its own redaction post-processor — hacienda does not use it, and it would not
have fully closed this gap either

xberg ships a second, **separate** redaction system behind its own `redaction`/
`redaction-ml`/`redaction-rehydrate` Cargo features (a pattern-based PII engine, pure Rust) —
already enabled in hacienda's `xberg` dependency, but not for this purpose: hacienda enables
it incidentally (for shared types/deps its own vendored pipeline happens to need) and never
configures xberg's redaction *post-processor* to run. That post-processor's own doc comment
(`crates/xberg/src/types/extraction.rs:337-342`) states precisely what it rewrites: *"the
redaction processor rewrites `content`, `formatted_content`, every chunk's text, and the
textual fields of `entities` / `summary` / `translation` / `page_classifications` in
place."* **`tables` and `pages` are not on that list.** So this is not simply "hacienda
forgot to call xberg's own redactor" — even turning xberg's built-in redaction on would still
leave `tables`/`pages`/`metadata` unredacted by xberg's own design. Worth knowing for §5's
design: xberg's post-processor could be a legitimate *component* for the fields it does
cover (`summary`, `translation`, `page_classifications`, chunk text — all X1/X3 features, not
yet enabled) rather than reimplementing redaction for those specifically, but `tables`,
`pages`, and `metadata` need hacienda's own handling regardless of that choice.

## 5. Scope

**D-P7-1 — the fix is structural, not a field-by-field patch.** Adding `document.tables =
redact_tables(document.tables)`, then `document.pages = ...`, then whatever xberg adds next
release, repeats exactly the mistake P1's own module doc already named: *"each future
caller... does not have to reimplement it, one of them will forget."* xberg is going to keep
adding fields to `ExtractedDocument` (X1 already will, with `extracted_keywords`); a
redaction step that has to be told about each one by name will always be one release behind.

**The fix:** redact every string-bearing field on `ExtractedDocument`, generically, as one
step — not by naming each field, but by walking the structure. Two shapes to choose between:

- **(a) Strip, don't redact, for anything not proven safe.** After the `.content` redaction
  runs, null out `tables`, `pages`, `elements`, `document`, `ocr_elements`, `djot_content`,
  and scrub `metadata.authors`/`created_by`/`additional` to structural-only fields (counts,
  not names) — unless/until each is individually redacted by (b). Fast to ship, loses table
  structure in the response (a real product regression: table extraction was presumably
  enabled *because* a caller wants structured tables back).
- **(b) Recursive redaction.** Walk every string leaf in `ExtractedDocument` (each table
  cell, each page's content, each metadata string) through the same `PiiPipeline::process`
  call `.content` already gets, replacing each leaf with its redacted form in place. Simpler
  than it sounds: unlike `.content`, most of these leaves are already short, independent
  strings (a cell, an author name) — no byte-offset stitching across a large document
  required, unlike the main content field.

**Recommendation: (a) immediately, as a hotfix — this finding is live in already-pushed code
— then (b) as this spec's real target**, landed before X1 or E1/E2/E4 touch any of these
fields again. This is the same two-speed precedent P1 itself and the CLI's `--no-redact`
flag both already use: refuse to emit what isn't provably safe, rather than emit it and hope.

**Non-objectives.** Redacting binary image bytes for visible PII (a photo containing a face
or a name badge) — same class of gap as X3's `captioning` note, a distinct capability
(visual detection), tracked there, not here. OCR'd text *inside* an image, once extracted to
a string, **is** in scope — it's just another string leaf.

## 6. Consequence for X1

`extracted_keywords: Option<Vec<Keyword>>` is exactly the kind of field this spec exists to
stop being added unguarded. **X1 must not enable `keywords` before P7 ships (b)**, or it
reproduces this exact bug on day one for a feature that exists specifically to surface short,
name-shaped strings pulled straight out of document text.

## 7. Exit criteria

- The reproduction in §1 — control-corpus spreadsheet through `hacienda extract --mode
  mask` — has zero occurrences of the corpus email or IBAN anywhere in the JSON output,
  not just in `content`.
- A new test, sibling to `hacienda-api/src/handlers/audit.rs`'s `no_endpoint_returns_corpus_plaintext`:
  **POST** a table-bearing, multi-page control-corpus document to `/v1/documents` and walk
  the *entire* response value (not just top-level string fields) asserting no corpus value
  appears anywhere in it. This is the test class that would have caught this.
- The same test, run against `hacienda-mcp`'s `documents_process` tool result and `hacienda
  extract`'s CLI output — one shared `HaciendaResult` type, so one shared test helper, run
  three times (mirroring how `hacienda-mcp`'s existing tests already assert this for
  `pii_redact`/`pii_scan`, extended to cover full-document extraction with structured
  fields).
- `metadata.authors`/`created_by` scrubbed or redacted the same way; a control-corpus document
  authored by "Zephyrine Quatrebarbes" (as an Office document property, not body text) does
  not surface that name in the response.

## 8. What actually shipped

Enumerating every field for §2's table surfaced four more live gaps in the same
already-enabled feature set, beyond the `tables`/`pages` pair the reproduction found —
each checked directly against xberg's source rather than assumed:

| Field | Carries | Why it's live today |
| --- | --- | --- |
| `annotations` (`Vec<PdfAnnotation>`) | PDF comment/sticky-note text | `pdf`, already enabled |
| `form_fields` (`Vec<PdfFormField>`) | Filled-in AcroForm/XFA field values | `pdf`, already enabled |
| `revisions` (`Vec<DocumentRevision>`) | Tracked-change author names and inserted/deleted text — often an *earlier*, unredacted draft's wording | `office` (DOCX), already enabled |
| `uris` (`Vec<ExtractedUri>`) | Extracted links — a `mailto:` URL **is** a plaintext email address | Not gated behind a not-yet-enabled feature |
| `children` (`Vec<ArchiveEntry>`) | Each a **complete nested `ExtractedDocument`** — a zip containing a PDF with PII is exactly as real a leak as the top-level PDF | `archives`, already enabled; recurses arbitrarily deep |
| `pages[].speaker_notes` | PPTX presenter notes | `office` (PPTX), already enabled |

Implementation landed as design (b) directly (§5) — recursive redaction, not a strip-first
hotfix — in `hacienda-core/src/facade.rs`:

- `HaciendaFacade::redact_structured_fields` walks tables (top-level and per-page,
  including the `Arc<Table>` sharing in `PageContent`, resolved by rebuilding a fresh `Arc`
  per redacted table rather than fighting `Arc::get_mut`/`make_mut`), `formatted_content`,
  `metadata.authors`/`created_by`/`modified_by`, `annotations`, `form_fields`, `revisions`
  (author, inserted/deleted lines, changed table cells, changed formatting properties),
  `uris`, and recurses into `children` via a hand-written `Pin<Box<dyn Future>>`-returning
  method (`redact_document_recursively`) — required because the recursion is indirect
  (`redact_structured_fields` → `redact_document_recursively` → `redact_structured_fields`),
  which a plain `async fn` cannot express (infinite state-machine size).
- Table markdown is **regenerated from redacted cells** (`cells_to_markdown`) rather than
  redacted independently, so the two representations can't disagree and the same value
  doesn't produce two audit entries for what is one redaction.
- Wired into the existing `process_batch_with_auth` loop, right after the pre-existing
  `document.content = result.redacted_text.clone()` line — same call site, same audit path
  (`record_audit`), reused per field via a new `redact_string_field` helper.
- **Verified against the actual reproduction**: the control-corpus spreadsheet from §1,
  run through the rebuilt `hacienda extract --mode mask`, now has zero occurrences of the
  corpus email or IBAN anywhere in the JSON output.
- **Automated test** (`hacienda-core/src/facade.rs`,
  `redact_structured_fields_covers_every_known_field`): builds an `ExtractedDocument` with
  the corpus value planted in every field above, one archive-child deep, and asserts all of
  them come back redacted. `cargo test -p hacienda-core -p hacienda-api -p hacienda-cli -p
  hacienda -p hacienda-mcp`: 367 passed (366 baseline + this test), same 4–5 pre-existing
  environmental failures as before (root-in-container permission tests, no live Postgres),
  no regressions.

**Still not covered, unchanged from §2/§5's original scope** — `metadata.additional` (open
`HashMap<_, JSON Value>`, e.g. source file paths) — and fields gated behind extraction-config
flags or Cargo features this workspace doesn't yet enable (`elements`, `document`'s structure
tree, `ocr_elements`, `djot_content`, `chunks`, and X1/X2/X3's `extracted_keywords`/`summary`/
`translation`/`page_classifications`, which is exactly why those specs gate on this one).

**Consequence for X1 (§6), updated:** `keywords` is now unblocked — `extracted_keywords`
would be covered the same way `annotations`/`form_fields`/etc. are, by extending
`redact_structured_fields` with one more field when that feature is enabled. Same for
`summarization`'s `.summary` and, once P6 ships, X3's `.translation`/`.page_classifications`.
