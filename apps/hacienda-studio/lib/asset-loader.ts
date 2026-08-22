import { openDB } from "idb";
import type { NerModel } from "@xberg-io/xberg-wasm";

const DB_NAME = "xberg-studio-assets";
const DB_VERSION = 1;
const MODEL_STORE = "models";
const TESSDATA_STORE = "tessdata";

/**
 * GLiNER2 weights for the xberg-wasm `NerModel`. These are public model files —
 * no document content is transmitted, only downloaded. huggingface.co serves
 * them with permissive CORS headers, so they are fetched directly rather than
 * through a dev proxy: a proxy that only exists in dev leaves production
 * resolving the same URL against the SPA fallback.
 *
 * Override the base to self-host the weights, e.g. for firms that require all
 * traffic to stay within the EU.
 */
const RAW_MODEL_BASE =
  import.meta.env.VITE_MODEL_BASE_URL ??
  "https://huggingface.co/jamon8888/gliner2-guardrails-pii-f16/resolve/main";
const MODEL_BASE = import.meta.env.DEV && RAW_MODEL_BASE.startsWith('https://huggingface.co')
  ? RAW_MODEL_BASE.replace('https://huggingface.co', '/hf-model')
  : RAW_MODEL_BASE;
const MODEL_URL = `${MODEL_BASE}/model.safetensors`;
const TOKENIZER_URL = `${MODEL_BASE}/tokenizer.json`;
// The encoder config (mdeberta-v3-base), not the top-level extractor config —
// NerModel.load reads hidden_size, layer counts and vocab size from this.
const ENCODER_CONFIG_URL = `${MODEL_BASE}/encoder_config/config.json`;

/**
 * `tessdata_fast` is the same source tesseract.js and most browser OCR integrations use:
 * small (~4MB/language), permissively licensed (Apache-2.0), and served by
 * raw.githubusercontent.com with a wide-open `Access-Control-Allow-Origin: *`.
 */
const TESSDATA_BASE =
  import.meta.env.VITE_TESSDATA_BASE_URL ??
  "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/main";

export interface DownloadProgress {
  receivedBytes: number;
  /** `null` when the server doesn't send `content-length` (e.g. chunked transfer-encoding). */
  totalBytes: number | null;
}

interface FetchAssetOptions {
  onProgress?: (progress: DownloadProgress) => void;
  stallTimeoutMs?: number;
  /**
   * A single HTTP/2 (or HTTP/3) request to huggingface.co's CDN ramps up through TCP/QUIC
   * slow start like any connection — for a ~600MB object that ramp dominates the transfer and
   * a lone stream often never reaches the link's real throughput. Splitting the file across a
   * handful of concurrent byte-range requests lets each ramp up independently, which
   * measurably increases aggregate throughput on higher-latency links. Only takes effect if
   * the server actually answers a probe range request with 206; otherwise this is a no-op and
   * falls back to the plain streamed download below with no behavior change. Opt-in because
   * the probe is an extra round-trip not worth paying for small assets like the tokenizer.
   */
  parallel?: boolean;
}

async function assertAssetResponse(
  response: Response,
  url: string,
): Promise<void> {
  if (!response.ok) {
    throw new Error(`Failed to download ${url}: HTTP ${response.status}`);
  }
  if ((response.headers.get("content-type") ?? "").includes("text/html")) {
    throw new Error(
      `Expected an asset at ${url} but the server returned HTML — the URL does not resolve.`,
    );
  }
}

function assertNotHtmlBody(bytes: Uint8Array, url: string): void {
  if (bytes[0] === 0x3c) {
    throw new Error(
      `Expected an asset at ${url} but the response body begins with '<' — the URL does not resolve.`,
    );
  }
}

/**
 * A URL that does not resolve is answered by the SPA fallback with index.html
 * and HTTP 200, so `response.ok` alone does not prove an asset was returned —
 * the HTML reaches the consumer disguised as model weights and fails much
 * later, somewhere unrelated. Reject non-asset responses at the boundary.
 *
 * Reads the body as a stream (rather than `response.arrayBuffer()`) so large assets can
 * report incremental progress via `onChunk`, and so a connection that stalls partway through
 * (no bytes for `stallTimeoutMs`) aborts with a real error instead of leaving the fetch
 * pending forever. A plain `await fetch()` has no such timeout: nothing rejects, nothing
 * resolves, and the onboarding screen sits at a frozen percentage indefinitely with no way to
 * tell "still downloading" from "actually stuck". Shared by the single-stream and
 * range-parallel download paths so both get the same stall/abort/HTML-fallback guards.
 *
 * `onHeaders` runs once the response is validated but before the body is read; returning
 * `false` from it skips reading the body entirely (used by the range probe below, which only
 * needs headers and must not accidentally download an entire unranged response).
 */
async function fetchStreamed(
  url: string,
  init: RequestInit,
  stallTimeoutMs: number,
  onChunk: (chunk: Uint8Array) => void,
  onHeaders?: (response: Response) => boolean | void,
): Promise<Response> {
  const controller = new AbortController();
  let stallTimer: ReturnType<typeof setTimeout> | undefined = undefined;
  const armStallTimer = () => {
    if (stallTimer !== undefined) clearTimeout(stallTimer);
    stallTimer = setTimeout(() => controller.abort(), stallTimeoutMs);
  };

  let response: Response;
  armStallTimer();
  try {
    response = await fetch(url, { ...init, signal: controller.signal });
  } catch (e) {
    if (controller.signal.aborted) {
      throw new Error(
        `Download of ${url} stalled: no response within ${stallTimeoutMs / 1000}s`,
      );
    }
    throw e;
  } finally {
    clearTimeout(stallTimer);
  }

  await assertAssetResponse(response, url);
  if (onHeaders?.(response) === false) {
    await response.body?.cancel();
    return response;
  }

  const reader = response.body?.getReader();
  if (!reader) {
    // No streaming body available (older environment) — fall back to a single whole-buffer
    // read, same as before streaming/progress support existed.
    onChunk(new Uint8Array(await response.arrayBuffer()));
    return response;
  }
  try {
    for (;;) {
      armStallTimer();
      const { done, value } = await reader.read();
      if (done) break;
      onChunk(value);
    }
  } catch (e) {
    if (controller.signal.aborted) {
      throw new Error(
        `Download of ${url} stalled: no data received for ${stallTimeoutMs / 1000}s`,
      );
    }
    throw e;
  } finally {
    clearTimeout(stallTimer);
  }
  return response;
}

const RANGED_DOWNLOAD_CONCURRENCY = 6;
// Below this, slow-start ramp-up isn't the bottleneck — a plain stream finishes before
// splitting it up would pay for itself, and it isn't worth the extra probe round-trip.
const RANGED_DOWNLOAD_MIN_BYTES = 32 * 1024 * 1024;
const RANGE_CHUNK_MAX_RETRIES = 2;

/**
 * Asks `url` for exactly one byte to find out whether it honors `Range` requests. Returns the
 * file's total size (parsed from `Content-Range`'s `.../total`) when ranging works, `null`
 * otherwise so the caller falls back to a plain sequential download. Never reads past the
 * probe response's headers — a server that ignores `Range` and echoes the whole body would
 * otherwise turn this "is ranging supported" check into a full, wasted download.
 */
async function probeRangeSupport(
  url: string,
  stallTimeoutMs: number,
): Promise<number | null> {
  let totalBytes: number | null = null;
  let supportsRanges = false;
  try {
    await fetchStreamed(
      url,
      { headers: { Range: "bytes=0-0" } },
      stallTimeoutMs,
      () => {},
      (response) => {
        supportsRanges = response.status === 206;
        const match = /\/(\d+)$/.exec(
          response.headers.get("content-range") ?? "",
        );
        totalBytes = match ? Number(match[1]) : null;
        return false;
      },
    );
  } catch {
    return null;
  }
  return supportsRanges && totalBytes !== null ? totalBytes : null;
}

/**
 * Downloads one byte range, retrying the whole range from scratch (up to
 * `RANGE_CHUNK_MAX_RETRIES` times) if the connection stalls or drops. A retry can double-count
 * the bytes it had already received on the failed attempt in `onProgress`'s running total —
 * cosmetic only, since `onChunk` writes into the shared output buffer by absolute offset and
 * the final assembled bytes are unaffected either way.
 */
async function fetchRangeWithRetry(
  url: string,
  start: number,
  end: number,
  stallTimeoutMs: number,
  onChunk: (chunk: Uint8Array, offset: number) => void,
): Promise<void> {
  for (let attempt = 0; ; attempt++) {
    let offset = start;
    try {
      await fetchStreamed(
        url,
        { headers: { Range: `bytes=${start}-${end}` } },
        stallTimeoutMs,
        (chunk) => {
          onChunk(chunk, offset);
          offset += chunk.length;
        },
      );
      return;
    } catch (e) {
      if (attempt >= RANGE_CHUNK_MAX_RETRIES) throw e;
    }
  }
}

/**
 * Downloads `url` as `RANGED_DOWNLOAD_CONCURRENCY` concurrent byte-range requests instead of
 * one stream — see `FetchAssetOptions.parallel` for why. Returns `null` (never throws for
 * "unsupported") when the server doesn't answer ranges or the file is too small for this to be
 * worth it, so the caller can fall back to `fetchAsset`'s plain sequential path.
 */
async function fetchAssetInRanges(
  url: string,
  onProgress: ((progress: DownloadProgress) => void) | undefined,
  stallTimeoutMs: number,
): Promise<Uint8Array | null> {
  const totalBytes = await probeRangeSupport(url, stallTimeoutMs);
  if (totalBytes === null || totalBytes < RANGED_DOWNLOAD_MIN_BYTES)
    return null;

  const bytes = new Uint8Array(totalBytes);
  let receivedBytes = 0;
  const chunkSize = Math.ceil(totalBytes / RANGED_DOWNLOAD_CONCURRENCY);
  const ranges = Array.from(
    { length: RANGED_DOWNLOAD_CONCURRENCY },
    (_, i) => ({
      start: i * chunkSize,
      end: Math.min((i + 1) * chunkSize, totalBytes) - 1,
    }),
  ).filter(({ start }) => start < totalBytes);

  await Promise.all(
    ranges.map(({ start, end }) =>
      fetchRangeWithRetry(url, start, end, stallTimeoutMs, (chunk, offset) => {
        bytes.set(chunk, offset);
        receivedBytes += chunk.length;
        onProgress?.({ receivedBytes, totalBytes });
      }),
    ),
  );

  assertNotHtmlBody(bytes, url);
  return bytes;
}

export async function fetchAsset(
  url: string,
  options: FetchAssetOptions = {},
): Promise<Uint8Array> {
  const { onProgress, stallTimeoutMs = 30_000, parallel = false } = options;

  if (parallel) {
    const ranged = await fetchAssetInRanges(url, onProgress, stallTimeoutMs);
    if (ranged) return ranged;
  }

  const doFetch = async (u: string) => {
    const chunks: Uint8Array[] = [];
    let receivedBytes = 0;
    let totalBytes: number | null = null;
    await fetchStreamed(
      u,
      {},
      stallTimeoutMs,
      (chunk) => {
        chunks.push(chunk);
        receivedBytes += chunk.length;
        onProgress?.({ receivedBytes, totalBytes });
      },
      (response) => {
        totalBytes = Number(response.headers.get("content-length")) || null;
      },
    );
    const bytes = new Uint8Array(receivedBytes);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.length;
    }
    assertNotHtmlBody(bytes, u);
    return bytes;
  };

  try {
    return await doFetch(url);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    // Hugging Face XET returns 405 for Range probes or  HTML fallback for
    // bad URLs — retry autonomously with ?download=true which forces a
    // direct download and bypasses the XET bridge.
    if (msg.includes("405") && !url.includes("?download=true")) {
      console.warn(`[asset-loader] 405 for ${url}, retrying with ?download=true`);
      return doFetch(`${url}?download=true`);
    }
    // If the body was HTML (SPA fallback), the URL never resolved — also
    // retry with download param once before giving up.
    if (msg.includes("does not resolve") && !url.includes("?download=true")) {
      console.warn(`[asset-loader] HTML fallback for ${url}, retrying with ?download=true`);
      return doFetch(`${url}?download=true`);
    }
    throw e;
  }
}

async function getDB() {
  return openDB(DB_NAME, DB_VERSION, {
    upgrade(db) {
      if (!db.objectStoreNames.contains(MODEL_STORE))
        db.createObjectStore(MODEL_STORE);
      if (!db.objectStoreNames.contains(TESSDATA_STORE))
        db.createObjectStore(TESSDATA_STORE);
    },
  });
}

export async function isModelCached(): Promise<boolean> {
  const db = await getDB();
  const model = await db.get(MODEL_STORE, "gliner2-guardrails-pii-model");
  const tokenizer = await db.get(
    MODEL_STORE,
    "gliner2-guardrails-pii-tokenizer",
  );
  const config = await db.get(MODEL_STORE, "gliner2-guardrails-pii-config");
  return !!(model && tokenizer && config);
}

async function clearStaleModelCache(): Promise<void> {
  try {
    const db = await getDB();
    const tx = db.transaction(MODEL_STORE, "readwrite");
    await Promise.all([
      tx.store.delete("gliner2-guardrails-pii-model"),
      tx.store.delete("gliner2-guardrails-pii-tokenizer"),
      tx.store.delete("gliner2-guardrails-pii-config"),
      tx.done,
    ]);
    console.log("[asset-loader] Cleared stale model cache");
  } catch {}
}

export async function loadNerModel(
  onProgress?: (progress: DownloadProgress) => void,
): Promise<{
  model: Uint8Array;
  tokenizer: Uint8Array;
  encoderConfig: Uint8Array;
}> {
  const db = await getDB();

  // Check cache first
  const [model, tokenizer, config] = await Promise.all([
    db.get(MODEL_STORE, "gliner2-guardrails-pii-model"),
    db.get(MODEL_STORE, "gliner2-guardrails-pii-tokenizer"),
    db.get(MODEL_STORE, "gliner2-guardrails-pii-config"),
  ]);

  if (model && tokenizer && config) {
    return { model, tokenizer, encoderConfig: config };
  }

  // Only the model's own progress is reported — it dwarfs the tokenizer (~16MB) and config
  // (<1KB), so tracking all three separately would add complexity without changing what the
  // user sees in any meaningful way. Self-healing: on 405/HTML failure (XET
  // 405 or SPA fallback) clear any half-written IndexedDB entries and retry
  // once autonomously — no manual "Delete database" needed.
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      const downloadStart = performance.now();
      const [modelData, tokenizerData, configData] = await Promise.all([
        fetchAsset(MODEL_URL, { onProgress, parallel: false }),
        fetchAsset(TOKENIZER_URL),
        fetchAsset(ENCODER_CONFIG_URL),
      ]);
      const downloadMs = performance.now() - downloadStart;
      console.log(`[PERF] Download: ${downloadMs.toFixed(0)}ms (${(modelData.byteLength / 1024 / 1024).toFixed(1)} MB)`);

      // Cache in IndexedDB
      const tx = db.transaction(MODEL_STORE, "readwrite");
      await Promise.all([
        tx.store.put(modelData, "gliner2-guardrails-pii-model"),
        tx.store.put(tokenizerData, "gliner2-guardrails-pii-tokenizer"),
        tx.store.put(configData, "gliner2-guardrails-pii-config"),
        tx.done,
      ]);

      return {
        model: modelData,
        tokenizer: tokenizerData,
        encoderConfig: configData,
      };
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      const isRetriable = msg.includes("405") || msg.includes("does not resolve") || msg.includes("HTML");
      if (attempt === 0 && isRetriable) {
        console.warn("[asset-loader] Model download failed, clearing stale cache and retrying autonomously:", e);
        await clearStaleModelCache();
        continue;
      }
      throw e;
    }
  }
  throw new Error("Model download failed after retry");
}

export async function createNerBackend(
  model: Uint8Array,
  tokenizer: Uint8Array,
  encoderConfig: Uint8Array,
): Promise<NerModel> {
  // Use the new NerModel from xberg-wasm (ner-candle-wasm feature)
  const { NerModel } = await import("@xberg-io/xberg-wasm");
  const loadStart = performance.now();
  const runtime = await NerModel.load({
    weights: model,
    tokenizer,
    encoderConfig,
  });
  const loadMs = performance.now() - loadStart;
  console.log(`[PERF] Model load (WASM): ${loadMs.toFixed(0)}ms`);
  return runtime;
}

export async function loadTessdata(lang: string = "eng"): Promise<Uint8Array> {
  const db = await getDB();
  const cached = await db.get(TESSDATA_STORE, `tessdata_${lang}`);
  if (cached) return cached;

  const bytes = await fetchAsset(`${TESSDATA_BASE}/${lang}.traineddata`);
  await db.put(TESSDATA_STORE, bytes, `tessdata_${lang}`);
  return bytes;
}

export async function preloadXbergWasm(): Promise<void> {
  await import("@xberg-io/xberg-wasm");
}

export function validateFile(file: File): { valid: boolean; error?: string } {
  if (file.size === 0) return { valid: false, error: "File is empty" };
  if (file.size > 50 * 1024 * 1024)
    return { valid: false, error: "File too large (>50MB)" };
  const type = file.type || "";
  const supported = SUPPORTED_MIME_PREFIXES.some((p) => type.startsWith(p));
  if (!supported)
    return {
      valid: false,
      error: `Unsupported file type: ${type || file.name}`,
    };
  return { valid: true };
}

/**
 * Above this page count, a PDF is rejected outright rather than sent to the worker.
 * Real business documents (contracts, reports, decks) essentially never reach this;
 * scanned books/archives routinely do, and those are exactly the documents whose
 * per-page OCR raster + text results accumulate for the whole file with no cap (see
 * `disableOcr` on `FileInput`).
 */
const HARD_MAX_PDF_PAGES = 400;

/**
 * Above this page count, native text extraction still runs but the OCR fallback for
 * scanned/low-quality pages is skipped for this file — xberg-wasm's own OCR batch-size
 * throttle (`adapt_batch_size_to_memory` in the `xberg` crate) never actually shrinks
 * anything in the browser build, so nothing else caps per-page OCR memory as page count
 * grows. Chosen well below `HARD_MAX_PDF_PAGES` so a document only large enough to be
 * risky, not pathological, still gets processed instead of rejected.
 */
const OCR_SAFE_PAGE_LIMIT = 20;

export interface PdfPageSafety {
  valid: boolean;
  error?: string;
  /** True when this file's OCR fallback should be skipped for memory safety. */
  disableOcr?: boolean;
  /** Set alongside `disableOcr: true`, for surfacing to the user. */
  warning?: string;
  /**
   * Set whenever the pdfium page-count probe below actually ran and succeeded — absent
   * (not 0) when the file isn't a PDF, or when the probe itself failed and this fails
   * open. `lib/pdf-liteparse.ts` reuses this instead of running its own page-count pass,
   * since `openDocumentUrl`+`getPdfPageCount` already paid that cost here.
   */
  pageCount?: number;
}

/**
 * Checks a PDF's page count via the pdfium engine `lib/pdf-thumbnail-utils.ts` already
 * loads for thumbnails (no extra dependency), and decides whether the document is safe
 * to OCR in-browser. Non-PDF files, and PDFs pdfium fails to open (corrupt file, unusual
 * encoding, etc.), pass through unchanged — this is a memory-safety guard on top of
 * `validateFile`, not a replacement for the extractor's own error handling, so a pdfium
 * failure here fails open rather than blocking an upload the real extraction might
 * still handle fine.
 */
export async function checkPdfPageSafety(file: File): Promise<PdfPageSafety> {
  const type = file.type || "";
  if (!type.startsWith("application/pdf") && !file.name.toLowerCase().endsWith(".pdf")) {
    return { valid: true };
  }

  let url: string | null = null;
  try {
    const { getPdfPageCount } = await import("./pdf-thumbnail-utils");
    url = URL.createObjectURL(file);
    const pageCount = await getPdfPageCount(url);

    if (pageCount > HARD_MAX_PDF_PAGES) {
      return {
        valid: false,
        error: `PDF has ${pageCount} pages (limit ${HARD_MAX_PDF_PAGES}) — too large to process safely in-browser.`,
        pageCount,
      };
    }
    if (pageCount > OCR_SAFE_PAGE_LIMIT) {
      return {
        valid: true,
        disableOcr: true,
        warning: `${file.name} has ${pageCount} pages — OCR for scanned pages was skipped to keep memory use safe; native text was still extracted.`,
        pageCount,
      };
    }
    return { valid: true, pageCount };
  } catch (e) {
    console.warn("[asset-loader] PDF page-count check failed, proceeding without it:", e);
    return { valid: true };
  } finally {
    if (url) URL.revokeObjectURL(url);
  }
}

export const SUPPORTED_MIME_PREFIXES = [
  "application/pdf",
  "application/vnd.openxmlformats-officedocument",
  "application/msword",
  "application/vnd.ms-excel",
  "application/vnd.ms-powerpoint",
  "application/vnd.oasis.opendocument",
  "message/rfc822",
  "application/vnd.ms-outlook",
  "application/vnd.ms-pki.stl",
  "text/",
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "image/tiff",
  "image/svg+xml",
  "application/json",
  "application/xml",
  "application/zip",
  "application/x-tar",
  "application/gzip",
  "application/x-7z-compressed",
  "audio/",
  "video/",
];
