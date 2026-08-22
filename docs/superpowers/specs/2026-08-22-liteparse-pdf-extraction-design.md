# LiteParse PDF Extraction — Design Specification

**Date**: 2026-08-22
**Status**: Proposed — ready for implementation planning
**Author**: investigation + spec by Claude, from a live debugging session with the project owner

---

## 0. Summary

Replace `@xberg-io/xberg-wasm`'s PDF-extraction path with `@llamaindex/liteparse-wasm` (PDFium-backed, Apache-2.0, actively maintained) for PDF inputs only. Every other input format (DOCX, XLSX, PPTX, images, audio/video) keeps using `@xberg-io/xberg-wasm` unchanged. Everything downstream of extraction — NER, PII detection, redaction, entity glossary, audit trail, zip export — is unaffected, because that stage already consumes a plain `markdown: string`, not an xberg-specific type.

This is scoped narrowly on purpose: it fixes the PDF-extraction robustness and memory problems diagnosed this session without touching the parts of the pipeline that already work.

---

## 1. Problem statement

Three independent, previously-diagnosed issues motivate this spec:

1. **PDF extraction crashes/freezes the tab.** `@xberg-io/xberg-wasm` extracts PDFs via xberg's `pdf_oxide` backend (pure Rust, no PDFium — xberg dropped PDFium entirely in commit `124ba41808`, May 2026; see upstream issue [xberg-io/xberg#1448](https://github.com/xberg-io/xberg/issues/1448)). `pdf_oxide` has open, unfixed defects that surface as crashes rather than degraded output, notably a rasterizer panic (upstream #1408) — and PDFium's return as an opt-in backend in xberg 1.1.0 is explicitly **not** available for WASM, so the browser build is stuck on `pdf_oxide` regardless. `worker/pipeline.ts`'s own comments confirm this was hit live: *"traps the whole wasm module (`RuntimeError: unreachable executed`, confirmed live on a 16MB/many-page scanned PDF)"* ([`worker/pipeline.ts:598`](../../../apps/hacienda-studio/worker/pipeline.ts)).
2. **No bounded-memory path for large PDFs.** `xberg-wasm`'s `extract()` has no page-batching mode — the app's only mitigation is `checkPdfPageSafety` in `lib/asset-loader.ts`, which disables OCR and page rasterization entirely past a fixed page-count threshold rather than degrading gracefully.
3. **The WASM module itself is heavy.** `@xberg-io/xberg-wasm@1.0.12`'s binary is **48MB**; on an 8GB machine, compiling/instantiating that module is a meaningful fixed memory cost before a single document is processed, on top of the ~600MB NER model and worker-pool multiplication already addressed in the device-tier fix (`lib/device-tier.ts`).

None of this is fixable by patching our own code against `pdf_oxide` — it's upstream, unpatched, and (per xberg's own maintainers) staying that way for WASM.

---

## 2. Evaluation: why LiteParse

`run-llama/liteparse` ([github.com/run-llama/liteparse](https://github.com/run-llama/liteparse)), published as `@llamaindex/liteparse-wasm` on npm:

| | `@xberg-io/xberg-wasm@1.0.12` (PDF path) | `@llamaindex/liteparse-wasm@2.13.1` |
|---|---|---|
| PDF backend | `pdf_oxide` (pure Rust, young, open crash-class bugs) | **PDFium** (Chrome's engine, battle-tested) |
| WASM binary size | 48MB | **5.5MB** (9x smaller) |
| Large-doc memory model | Whole-document parse; no batching | `openBatchSession()` — bounded-memory, page-batched (25/batch default), designed for documents that would exhaust the wasm heap if parsed whole |
| Pre-OCR complexity check | None (rasterizes blindly, or an app-side page-count cutoff) | `isComplex()` — cheap (0.1–6.5ms observed), per-page `needsOcr` verdict with reasons (`scanned`, `sparse-text`, `garbled`, ...) |
| OCR integration | In-binary bridge injection (`bridge/ocr.rs`) | Same bridge-injection pattern (`ocrEngine.recognize()`) — directly reusable with our existing `tesseract-wasm` engine |
| License | MIT (xberg) | Apache-2.0 |
| Maintenance | Active | Active (commits day-of-writing; 12k★) |

**Validation done this session** (not just reading docs): ran the `lit` CLI against 5 real fixtures, including xberg's own OCR test fixtures (`mixed_native_scanned.pdf`, `scanned_hello.pdf`) and our `pii-test.pdf`. Zero crashes, zero exceptions; OCR correctly merged with native text; PII-dense text extracted intact; `is-complex` correctly flagged which pages needed OCR. Checked LiteParse's own issue tracker for large-document problems: one relevant closed issue (#290, a 644-page/~100MB doc deadlocking against an **external HTTP OCR server** — not applicable, we use the in-process bridge — fixed); no evidence of a hard size ceiling.

**What LiteParse does *not* solve, and this spec does not attempt**: the candle GLiNER2 NER F16/F32 dtype bug (tracked and patched separately, see the `vendor/candle-transformers-0.11.0-wasm-fix` patch and the not-yet-filed upstream issue against `xberg-io/xberg`). NER/PII behavior is out of scope here — this spec only touches how PDF bytes become a `markdown` string.

---

## 3. Goals

- PDF documents route through LiteParse for extraction (text + OCR merge + markdown rendering).
- Every non-PDF format keeps routing through `@xberg-io/xberg-wasm`, byte-for-byte the same as today.
- The NER/PII/redaction/audit/chunking pipeline downstream of extraction is **unmodified** — it already operates on `markdown: string` regardless of source.
- Large PDFs degrade gracefully (bounded memory via batching) instead of crashing.
- OCR reuses the existing `tesseract-wasm` engine already loaded for the xberg OCR bridge — no second OCR engine, no second tessdata download.
- Existing device-tier / worker-pool memory math (`lib/device-tier.ts`, `lib/worker-pool.ts`) is not invalidated by this change; if it can be relaxed later because the new binary is lighter, that's a separate follow-up, not part of this spec.

## 4. Non-goals

- Replacing `@xberg-io/xberg-wasm` for non-PDF formats.
- Changing NER, PII detection, redaction, audit trail, entity glossary, or zip export logic.
- Fixing the candle F16/F32 NER bug (tracked separately).
- Raising the 50MB upload cap (`validateFile` in `lib/asset-loader.ts`) — LiteParse's batching makes larger PDFs *safer* to support, but actually raising the cap is a product decision for a later spec, not a consequence of this one.
- A LlamaParse (cloud) integration — LiteParse is the local/offline OSS tool; the cloud product is out of scope entirely.

---

## 5. Architecture

### 5.1 Routing

`worker/pipeline.ts`'s `processFile` currently has one extraction branch (besides audio/video transcription) that always goes through `XbergEngine.extract()` (`worker/pipeline.ts:562-645`). This becomes a MIME-type gate:

```
isAudio/isVideo  → transcription bridge (unchanged)
input.type === "application/pdf"  → LiteParse path (new)
everything else  → XbergEngine.extract() (unchanged)
```

Both paths converge on the same `markdown: string` local variable that already feeds NER (`worker/pipeline.ts:648` onward), so nothing past that point changes.

### 5.2 New module: `lib/pdf-liteparse.ts`

A new file, mirroring the existing module boundaries (`lib/asset-loader.ts` for fetching/caching, `lib/wasm-cache.ts` for Tier-2.2 caching, `lib/ner-bridge.ts` for the fallback-selection pattern):

```ts
// lib/pdf-liteparse.ts
export interface LiteParseRuntime { parser: LiteParse /* from @llamaindex/liteparse-wasm */ }

export async function initLiteParse(): Promise<void>
export function selectPdfExtractor(
  runtime: LiteParseRuntime | null,
  ocrRuntime: OcrRuntime | null,   // reuse worker/pipeline.ts's existing tesseract-wasm handle
): (bytes: Uint8Array, opts: { disableOcr: boolean }) => Promise<{ markdown: string; pageCount: number }>
```

Responsibilities:
- Lazily init the LiteParse WASM module once per worker (mirrors `initPiiEngine`'s idempotent-promise pattern in `lib/pii-engine.ts:53-66`).
- Wrap our existing `tesseract-wasm` engine (already created in `worker/pipeline.ts`'s `createOcrBackend()`) behind LiteParse's `ocrEngine.recognize(imageData, width, height, language)` shape. This is a thin adapter — the underlying engine and tessdata are already loaded and cached; no new download, no second engine instance.
- Decide `parse()` vs `openBatchSession()` based on a page-count/byte-size threshold (see 5.4).
- Return `{ markdown, pageCount }` — `markdown` slots directly into `processFile`'s existing `markdown` variable.

### 5.3 OCR bridge adapter

```ts
function toLiteParseOcrEngine(tesseract: OcrRuntime) {
  return {
    async recognize(imageData: Uint8Array, width: number, height: number, language: string) {
      const result = await runTesseractOnImage(tesseract, imageData, { language }); // existing logic, factored out of selectOcrBridge
      return result.lines.map(l => ({
        text: l.text,
        bbox: [l.bbox?.x ?? 0, l.bbox?.y ?? 0, (l.bbox?.x ?? 0) + (l.bbox?.width ?? 0), (l.bbox?.y ?? 0) + (l.bbox?.height ?? 0)],
        confidence: l.confidence,
      }));
    },
  };
}
```

`selectOcrBridge`'s existing image-decode logic (`worker/pipeline.ts:255-260`, `createImageBitmap` from encoded bytes) is reused as-is; only the shape of the returned array changes (LiteParse wants `[x1,y1,x2,y2]` tuples, xberg's bridge wants `{x,y,width,height}` objects) — a pure mapping function, unit-testable the same way `selectNerBridge`/`selectOcrBridge` already are.

If `ocrRuntime` is `null` (OCR backend failed to load — the race this session already fixed by awaiting `initOcrBackend()`), pass `ocrEnabled: false` to LiteParse's config instead of an `ocrEngine` — LiteParse degrades to text-only for scanned pages, and the existing OCR-unavailable toast warning (`worker/pipeline.ts`, added this session) still fires.

### 5.4 Large-document handling

LiteParse's `openBatchSession()` yields pages in bounded batches (default 25) instead of parsing the whole document into memory at once. Threshold to decide `parse()` (single call) vs `openBatchSession()` (streamed):

- Reuse the existing page-count probe already computed by `checkPdfPageSafety` in `lib/asset-loader.ts` (it already opens the PDF to count pages before deciding `disableOcr`) — do not add a second page-count pass.
- Proposed threshold: **> 25 pages → batch session**, matching LiteParse's own default batch size, so a single-page-count check both decides routing and sizes the batch call. This number is a starting point, not load-bearing — validate empirically per 8.2.
- Batch results are concatenated into one `markdown` string before returning — cross-batch passes (header/footer dedup) are lost per-batch per LiteParse's own docs; acceptable, since xberg's current whole-document parse has the same limitation implicitly (it just doesn't survive large documents at all).

### 5.5 Caching

Mirror `lib/wasm-cache.ts`'s Tier 2.2 pattern (`initializeWasmWithCache`) for the new `liteparse_wasm_bg.wasm` binary — same Cache-API-then-IndexedDB-then-network fallback already used for `hacienda_wasm_bg.wasm`. `pii-engine.ts:53-66` is the template to copy, not reinvent.

### 5.6 Complexity pre-check surfacing

`isComplex()` is cheap enough (sub-10ms) to call unconditionally before extraction and log/telemetry it, even before deciding OCR routing — useful diagnostic signal for "why did this file take so long" that the app currently has no equivalent of. Not required for correctness; recommended as a low-cost addition.

---

## 6. Files touched

| File | Change |
|---|---|
| `apps/hacienda-studio/package.json` | add `@llamaindex/liteparse-wasm` dependency |
| `apps/hacienda-studio/lib/pdf-liteparse.ts` | **new** — init, caching, batch-vs-whole routing, OCR adapter |
| `apps/hacienda-studio/lib/pdf-liteparse.test.ts` | **new** — unit tests for the OCR-shape adapter and batch-threshold decision (pure functions, no live WASM) |
| `apps/hacienda-studio/worker/pipeline.ts` | MIME-type gate in `processFile`'s extraction branch; call `selectPdfExtractor` for `application/pdf` |
| `apps/hacienda-studio/lib/wasm-cache.ts` | extend/reuse for the LiteParse binary (or a same-shaped sibling function) |
| `apps/hacienda-studio/lib/asset-loader.ts` | expose `checkPdfPageSafety`'s page count to the caller instead of only its disable-OCR verdict, so `pdf-liteparse.ts` can reuse it for the batch threshold |
| `apps/hacienda-studio/tests/e2e/*` | one new e2e case: upload a PDF, confirm markdown output and entity glossary match today's xberg-produced baseline on `note.pdf`/`pii-test.pdf` (regression, not just "it doesn't crash") |

No changes to: `hacienda-core`, `crates/hacienda-wasm`, the candle-transformers patch, `lib/device-tier.ts`, `lib/pii-engine.ts`, `lib/ner-bridge.ts`, any redaction/audit/export code.

---

## 7. Testing strategy

1. **Unit**: `selectPdfExtractor`-style pure functions (OCR-shape adapter, batch-threshold decision) tested the same way `selectNerBridge`/`selectOcrBridge` already are in `worker/pipeline.test.ts` — inputs in, outputs out, no live WASM required.
2. **Regression**: run both the current xberg path and the new LiteParse path against `apps/hacienda-studio/tests/e2e/fixtures/{note,pii-test}.pdf`, diff the resulting entity glossary and PII findings. Some drift in markdown formatting is expected and fine (different heading/table heuristics); drift in **which PII entities are found** is not — that's the regression bar.
3. **Crash corpus**: re-run this session's spike fixtures (`mixed_native_scanned.pdf`, `scanned_hello.pdf`) plus, if reproducible, the "16MB/many-page scanned PDF" that trapped xberg-wasm (`worker/pipeline.ts:598`'s comment) — confirm it now completes instead of crashing.
4. **Memory**: in a real browser tab on an 8GB-class machine (or Chrome DevTools' memory throttling), confirm worker-pool peak memory with the LiteParse path is lower than the current xberg-wasm path for an equivalent scanned PDF — this is the empirical validation flagged as outstanding in the spike (§8.2 below).
5. **Large-doc**: synthesize or source a >50MB scanned PDF and confirm the batch-session path completes without exhausting the wasm heap (not validated in the CLI spike — flagged as an open risk).

---

## 8. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Markdown/heading/table reconstruction differs from xberg's, subtly shifting NER/PII entity boundaries on some documents | Regression test (§7.2) against known-good fixtures before rollout; both are heuristic renderers, so some drift is expected — the bar is PII-finding parity, not byte-identical markdown |
| Upstream LiteParse issue #315 (batch-mode markdown quality can regress vs whole-document parse for `target_pages`) | Batch mode only engages above the page-count threshold (§5.4); most real-world documents in this app's use case (contracts, forms, short reports) stay under it and get whole-document parse, same quality as CLI-tested today |
| Upstream LiteParse issue #425 (open, image identification) | Doesn't affect text/PII extraction quality; monitor, not blocking |
| Two WASM binaries now loaded per worker instead of one (xberg-wasm for non-PDF + liteparse-wasm for PDF) | Net memory is still lower than today for any PDF-processing worker: 48MB (xberg) replaced by 5.5MB (liteparse) when a PDF is the input; a worker that only ever sees non-PDF files never loads liteparse-wasm at all (lazy init, §5.2) |
| Large-PDF memory behavior in a real 8GB browser tab is unvalidated (CLI spike only) | Explicit test task in §7.4 before this ships as default, not after |
| LiteParse's OCR bridge shape differs from xberg's (`[x1,y1,x2,y2]` vs `{x,y,width,height}`, `recognize()` vs xberg's own signature) | Isolated in one adapter function (§5.3), unit tested, no change to the underlying tesseract-wasm engine or tessdata |

---

## 9. Rollout

Recommend a config-gated rollout, not an immediate default flip:

1. Land behind a `config.pdfEngine: "xberg" | "liteparse"` flag (default `"xberg"`, i.e. no behavior change) so both paths exist side by side.
2. Run the regression + crash-corpus + memory tests (§7) with the flag flipped locally.
3. Flip the default to `"liteparse"` for PDF inputs once §7.4/§7.5 (real-browser memory, large-doc) are empirically confirmed — these are the two things the CLI spike couldn't validate.
4. Remove the xberg PDF path and the flag once `"liteparse"` has run clean for a while — but keep `@xberg-io/xberg-wasm` itself, since it's still load-bearing for every non-PDF format.

## 10. Open questions

- Exact batch-threshold page count (§5.4 proposes 25, matching LiteParse's own default — not yet empirically tuned against this app's real document mix).
- Whether `extractStructureTree`/`extractBlocks` (LiteParse's structured layout output) are worth consuming for a future redaction-overlay feature — out of scope here, flagged for later.
- Whether the device-tier worker-pool sizing (`lib/device-tier.ts`) should eventually be relaxed given the lighter binary — deliberately deferred (§4) to avoid conflating this change with the memory fix already shipped this session.
