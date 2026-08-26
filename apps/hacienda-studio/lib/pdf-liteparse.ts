/**
 * PDF extraction via `@llamaindex/liteparse-wasm` (PDFium-backed) — the alternative to
 * `@xberg-io/xberg-wasm`'s `pdf_oxide` backend selected when `AppConfig.pdfEngine ===
 * "liteparse"`. See `docs/superpowers/specs/2026-08-22-liteparse-pdf-extraction-design.md`
 * for why: `pdf_oxide` has open crash-class bugs (confirmed live: "traps the whole wasm
 * module" on a scanned PDF, see `worker/pipeline.ts`) and no bounded-memory path for large
 * documents, both of which PDFium + `openBatchSession()` fix. Only PDFs route here — every
 * other format keeps going through `@xberg-io/xberg-wasm` unchanged, since LiteParse's wasm
 * build has no DOCX/XLSX/PPTX support (that conversion needs a native LibreOffice
 * subprocess, unavailable in-browser).
 */
import type { OCREngine } from "tesseract-wasm";
import type { LiteParse as LiteParseParser, OcrEngine, OcrResult } from "@llamaindex/liteparse-wasm";

/**
 * Above this page count, `extractPdfWithLiteParse` uses `openBatchSession()` instead of
 * `parse()` — matches `openBatchSession`'s own default batch size (25), so a single
 * page-count check both decides routing and sizes the batch call. Not empirically tuned
 * yet against this app's real document mix (see the design spec's open questions); a
 * starting point, not load-bearing.
 */
const BATCH_THRESHOLD_PAGES = 25;
const BATCH_SIZE = 25;

let wasmReady: Promise<void> | null = null;

/** Idempotent; safe to call from every worker that needs the engine. */
export function initLiteParse(): Promise<void> {
  if (!wasmReady) {
    wasmReady = (async () => {
      const [{ default: init }, { default: url }] = await Promise.all([
        import("@llamaindex/liteparse-wasm"),
        import("@llamaindex/liteparse-wasm/liteparse_wasm_bg.wasm?url"),
      ]);
      // Tier 2.2-style caching (Cache API/IndexedDB on repeat visits) mirrors
      // `lib/pii-engine.ts`'s `initPiiEngine` — see `initializeWasmWithCache`'s own header
      // for the fallback-to-plain-fetch behavior on a cache miss.
      const { initializeWasmWithCache } = await import("./wasm-cache");
      await init({ module_or_path: initializeWasmWithCache(url) });
    })();
  }
  return wasmReady;
}

/**
 * Adapts this app's existing `tesseract-wasm` engine (already loaded and cached for
 * xberg-wasm's own OCR bridge, see `worker/pipeline.ts`'s `createOcrBackend`/
 * `selectOcrBridge`) to the shape LiteParse's `LiteParseInit.ocrEngine` expects. No second
 * OCR engine, no second tessdata download — same engine instance, same loaded model, just
 * a different call signature and result shape:
 * xberg's bridge wants `{bbox: {x,y,width,height}}` objects, LiteParse wants
 * `bbox: [x1,y1,x2,y2]` tuples.
 */
export function toLiteParseOcrEngine(runtime: OCREngine): OcrEngine {
  return {
    async recognize(imageData: Uint8Array, _width: number, _height: number): Promise<OcrResult[]> {
      // LiteParse hands us PNG-encoded page-region bytes (per its own doc: "imageData
      // PNG-encoded image bytes") — same encoded-not-raw shape xberg's bridge uses, so the
      // same decode approach applies (`worker/pipeline.ts`'s `selectOcrBridge`).
      const bytes = imageData.buffer.slice(
        imageData.byteOffset,
        imageData.byteOffset + imageData.byteLength,
      ) as ArrayBuffer;
      const bitmap = await createImageBitmap(new Blob([bytes]));
      try {
        runtime.loadImage(bitmap);
        const lines = runtime.getTextBoxes("line");
        return lines.map((l) => ({
          text: l.text,
          confidence: l.confidence,
          bbox: [l.rect.left, l.rect.top, l.rect.right, l.rect.bottom] as [number, number, number, number],
        }));
      } finally {
        runtime.clearImage();
        bitmap.close();
      }
    },
  };
}

export interface LiteParseExtractResult {
  markdown: string;
  pageCount: number;
}

/**
 * Extracts PDF `bytes` to markdown. `pageCount`, when known (threaded through from
 * `lib/asset-loader.ts`'s `checkPdfPageSafety` via `FileInput.pdfPageCount` — see that
 * field's header), decides `parse()` vs `openBatchSession()` without a second page-count
 * pass; when absent (probe failed and `checkPdfPageSafety` failed open), this always uses
 * `parse()` — the same behavior as today's xberg path in that situation, since there's no
 * cheaper way to learn the count first.
 */
/**
 * Cheap (`isComplex()`, sub-10ms) probe for whether any page in this PDF needs OCR —
 * exported standalone so callers can decide whether an "OCR unavailable/withheld"
 * warning is actually relevant to *this* document, instead of firing it for every PDF
 * regardless of content. `extractPdfWithLiteParse` no longer runs this probe itself
 * while OCR is unconditionally withheld from it (see `worker/pipeline.ts`'s
 * `ocrEngine: null` stopgap for the ocr_merge.rs Tokio panic) — `needsOcr` there
 * short-circuits to `false` before ever reaching the probe.
 */
export async function checkPdfNeedsOcr(bytes: Uint8Array): Promise<boolean> {
  await initLiteParse();
  const { LiteParse } = await import("@llamaindex/liteparse-wasm");
  const probe = new LiteParse({ ocrEnabled: false, quiet: true });
  try {
    const pages = await probe.isComplex(bytes);
    return pages.some((p) => p.needsOcr);
  } finally {
    probe.free();
  }
}

export async function extractPdfWithLiteParse(
  bytes: Uint8Array,
  opts: { ocrEngine: OcrEngine | null; disableOcr: boolean; pageCount?: number; dpi?: number },
): Promise<LiteParseExtractResult> {
  await initLiteParse();
  const { LiteParse } = await import("@llamaindex/liteparse-wasm");

  // LiteParse already OCRs selectively per page internally ("only on embedded images or
  // pages where native text extraction didn't find text" — LiteParse's own OCR guide), so
  // this pre-check isn't needed to avoid redundant per-page OCR. It's needed to skip wiring
  // the OCR engine into the parser AT ALL for the common case of a fully-digital PDF — no
  // `ocrEngine` construction, no tesseract-wasm touched, matching the "cheap path — skip
  // OCR entirely" pattern LiteParse's own complexity guide recommends. `isComplex()` costs
  // sub-10ms (measured against real fixtures during evaluation), so this is strictly a win
  // whenever it can skip OCR. Skipped entirely when OCR is already forced off by
  // `disableOcr` (the page-count memory-safety gate) or no engine is available — nothing to
  // decide in either case.
  let needsOcr = !opts.disableOcr && opts.ocrEngine !== null;
  if (needsOcr) {
    const probe = new LiteParse({ ocrEnabled: false, quiet: true });
    try {
      const pages = await probe.isComplex(bytes);
      needsOcr = pages.some((p) => p.needsOcr);
    } finally {
      probe.free();
    }
  }

  // `LiteParse` (like `ParseSession`) is a wasm-bindgen object, not garbage-collected —
  // `free()` it once we're done, same as the batch-session path below already does. Left
  // unfreed, every processed PDF leaks its wasm-side state for the life of the worker,
  // which is exactly the kind of unbounded-growth memory problem this integration exists
  // to avoid (see the design spec's problem statement).
  const parser: LiteParseParser = new LiteParse({
    outputFormat: "markdown",
    ocrEnabled: needsOcr,
    ocrEngine: needsOcr ? (opts.ocrEngine ?? undefined) : undefined,
    // Explicit rather than relying on LiteParse's own default (also 150) — this way
    // `worker/pipeline.ts`'s `OCR_TARGET_DPI` constant is the one place that tunes
    // rasterization cost for both engines, xberg and LiteParse alike.
    dpi: opts.dpi,
    // Default is `true` (LiteParse's browser-usage guide) — one page's OCR failure would
    // throw and discard the whole document's extraction, including every other page's
    // already-successful native text. `false` returns partial results instead: the pages
    // that succeeded keep their text, the page(s) that failed OCR just have none — same
    // "degrade, don't discard" contract xberg's own OCR bridge already has via
    // `selectOcrBridge`'s per-file (not per-page) granularity.
    ocrFailureFatal: false,
    quiet: true,
  });

  try {
    if (opts.pageCount !== undefined && opts.pageCount > BATCH_THRESHOLD_PAGES) {
      const session = await parser.openBatchSession(bytes, BATCH_SIZE);
      try {
        const chunks: string[] = [];
        let batch;
        while ((batch = await session.nextBatch()) !== undefined) {
          chunks.push(batch.result.text);
        }
        return { markdown: chunks.join("\n\n"), pageCount: session.totalPages };
      } finally {
        session.free();
      }
    }

    const result = await parser.parse(bytes);
    return { markdown: result.text, pageCount: result.totalPages };
  } finally {
    parser.free();
  }
}
