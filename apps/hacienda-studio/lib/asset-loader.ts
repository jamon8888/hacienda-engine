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
const MODEL_BASE =
  import.meta.env.VITE_MODEL_BASE_URL ??
  "https://huggingface.co/jamon8888/gliner2-guardrails-pii-f16/resolve/main";
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
 * `RANGE_CHUNK_MAX_RETRIES` times) if the connection stalls or drops, answers with a status
 * other than 206 (a CDN node dropping `Range`, or a redirect that strips the header), or sends
 * more bytes than the requested range covers. Any of those would otherwise either silently
 * hand back the wrong bytes or throw a raw `RangeError` from `bytes.set` deep inside
 * `fetchAssetInRanges` when a later chunk no longer lines up with its intended offset.
 * `rangeReceivedBytes` (this range's own received-byte count, passed to `onChunk`) resets to 0
 * on each retry so a caller accumulating progress per range never counts a failed attempt's
 * bytes twice.
 */
async function fetchRangeWithRetry(
  url: string,
  start: number,
  end: number,
  stallTimeoutMs: number,
  onChunk: (
    chunk: Uint8Array,
    offset: number,
    rangeReceivedBytes: number,
  ) => void,
): Promise<void> {
  for (let attempt = 0; ; attempt++) {
    let offset = start;
    try {
      await fetchStreamed(
        url,
        { headers: { Range: `bytes=${start}-${end}` } },
        stallTimeoutMs,
        (chunk) => {
          if (offset + chunk.length > end + 1) {
            throw new Error(
              `Range ${start}-${end} of ${url} returned more bytes than requested.`,
            );
          }
          onChunk(chunk, offset, offset + chunk.length - start);
          offset += chunk.length;
        },
        (response) => {
          if (response.status !== 206) {
            throw new Error(
              `Range ${start}-${end} of ${url} was answered with HTTP ${response.status}, not 206.`,
            );
          }
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
 * one stream — see `FetchAssetOptions.parallel` for why. Returns `null` (never throws) both
 * when the server doesn't answer ranges/the file is too small for this to be worth it, and
 * when a range ends up failing every retry (e.g. a CDN node that stops honoring `Range`
 * mid-download) — either way the caller falls back to `fetchAsset`'s plain sequential path
 * instead of a broken parallel download taking down the whole asset fetch.
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
  // Per-range confirmed byte counts rather than one shared running total — a retry restarts
  // its range from scratch, and a shared counter would keep the bytes the failed attempt had
  // already added, letting `receivedBytes` exceed `totalBytes` and the onboarding percentage
  // exceed 100%.
  const receivedPerRange = new Map<number, number>();
  const reportProgress = (): void => {
    if (!onProgress) return;
    let receivedBytes = 0;
    for (const n of receivedPerRange.values()) receivedBytes += n;
    onProgress({ receivedBytes, totalBytes });
  };
  const chunkSize = Math.ceil(totalBytes / RANGED_DOWNLOAD_CONCURRENCY);
  const ranges = Array.from(
    { length: RANGED_DOWNLOAD_CONCURRENCY },
    (_, i) => ({
      start: i * chunkSize,
      end: Math.min((i + 1) * chunkSize, totalBytes) - 1,
    }),
  ).filter(({ start }) => start < totalBytes);

  try {
    await Promise.all(
      ranges.map(({ start, end }) =>
        fetchRangeWithRetry(
          url,
          start,
          end,
          stallTimeoutMs,
          (chunk, offset, rangeReceivedBytes) => {
            bytes.set(chunk, offset);
            receivedPerRange.set(start, rangeReceivedBytes);
            reportProgress();
          },
        ),
      ),
    );
  } catch (e) {
    console.warn(
      `[asset-loader] Parallel range download of ${url} failed, falling back to a sequential download:`,
      e,
    );
    return null;
  }

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

  const chunks: Uint8Array[] = [];
  let receivedBytes = 0;
  let totalBytes: number | null = null;
  await fetchStreamed(
    url,
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
  assertNotHtmlBody(bytes, url);
  return bytes;
}

// Cached rather than opening a fresh connection on every call: an unclosed IndexedDB
// connection is otherwise left dangling per call (every `loadNerModel`/`loadTessdata`/
// `isModelCached` invocation), and a test calling `indexedDB.deleteDatabase(DB_NAME)` after
// exercising this module would block on the native `onblocked` event waiting for a connection
// nothing ever closes. See `closeDB` below.
let dbPromise: ReturnType<typeof openDB> | null = null;

function getDB() {
  if (!dbPromise) {
    dbPromise = openDB(DB_NAME, DB_VERSION, {
      upgrade(db) {
        if (!db.objectStoreNames.contains(MODEL_STORE))
          db.createObjectStore(MODEL_STORE);
        if (!db.objectStoreNames.contains(TESSDATA_STORE))
          db.createObjectStore(TESSDATA_STORE);
      },
    });
  }
  return dbPromise;
}

/**
 * Closes the shared IndexedDB connection opened by `getDB` and clears the cache so the next
 * `getDB()` call reopens fresh. Exists for tests that need `indexedDB.deleteDatabase(DB_NAME)`
 * to actually settle instead of blocking on `onblocked`.
 */
export async function closeDB(): Promise<void> {
  if (!dbPromise) return;
  const db = await dbPromise;
  db.close();
  dbPromise = null;
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
  // user sees in any meaningful way.
  const [modelData, tokenizerData, configData] = await Promise.all([
    fetchAsset(MODEL_URL, { onProgress, parallel: true }),
    fetchAsset(TOKENIZER_URL),
    fetchAsset(ENCODER_CONFIG_URL),
  ]);

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
}

export async function createNerBackend(
  model: Uint8Array,
  tokenizer: Uint8Array,
  encoderConfig: Uint8Array,
): Promise<NerModel> {
  // Use the new NerModel from xberg-wasm (ner-candle-wasm feature)
  const { NerModel } = await import("@xberg-io/xberg-wasm");
  const runtime = await NerModel.load({
    weights: model,
    tokenizer,
    encoderConfig,
  });
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
