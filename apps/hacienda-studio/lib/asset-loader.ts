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

export interface DownloadProgress {
  receivedBytes: number;
  /** `null` when the server doesn't send `content-length` (e.g. chunked transfer-encoding). */
  totalBytes: number | null;
}

/**
 * A URL that does not resolve is answered by the SPA fallback with index.html
 * and HTTP 200, so `response.ok` alone does not prove an asset was returned —
 * the HTML reaches the consumer disguised as model weights and fails much
 * later, somewhere unrelated. Reject non-asset responses at the boundary.
 *
 * Reads the body as a stream (rather than `response.arrayBuffer()`) for two reasons: it lets
 * large assets — the ~614MB model dwarfs the ~16MB tokenizer and <1KB config — report
 * incremental `onProgress`, and it lets a connection that stalls partway through (no bytes for
 * `stallTimeoutMs`) abort with a real error instead of leaving the fetch pending forever. A
 * plain `await fetch()` has no such timeout: nothing rejects, nothing resolves, and the
 * onboarding screen sits at a frozen percentage indefinitely with no way to tell "still
 * downloading" from "actually stuck" — this is exactly what fixes that report.
 */
export async function fetchAsset(
  url: string,
  options: { onProgress?: (progress: DownloadProgress) => void; stallTimeoutMs?: number } = {},
): Promise<Uint8Array> {
  const { onProgress, stallTimeoutMs = 30_000 } = options;
  const controller = new AbortController();
  let stallTimer: ReturnType<typeof setTimeout> | undefined = undefined;
  const armStallTimer = () => {
    if (stallTimer !== undefined) clearTimeout(stallTimer);
    stallTimer = setTimeout(() => controller.abort(), stallTimeoutMs);
  };

  let response: Response;
  armStallTimer();
  try {
    response = await fetch(url, { signal: controller.signal });
  } catch (e) {
    if (controller.signal.aborted) {
      throw new Error(`Download of ${url} stalled: no response within ${stallTimeoutMs / 1000}s`);
    }
    throw e;
  } finally {
    clearTimeout(stallTimer);
  }

  if (!response.ok) {
    throw new Error(`Failed to download ${url}: HTTP ${response.status}`);
  }

  const contentType = response.headers.get("content-type") ?? "";
  if (contentType.includes("text/html")) {
    throw new Error(
      `Expected an asset at ${url} but the server returned HTML — the URL does not resolve.`,
    );
  }

  const totalBytes = Number(response.headers.get("content-length")) || null;
  const reader = response.body?.getReader();
  let bytes: Uint8Array;
  if (!reader) {
    // No streaming body available (older environment) — fall back to a single whole-buffer
    // read, same as before streaming/progress support existed.
    bytes = new Uint8Array(await response.arrayBuffer());
    onProgress?.({ receivedBytes: bytes.length, totalBytes: bytes.length });
  } else {
    const chunks: Uint8Array[] = [];
    let receivedBytes = 0;
    try {
      for (;;) {
        armStallTimer();
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value);
        receivedBytes += value.length;
        onProgress?.({ receivedBytes, totalBytes });
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
    bytes = new Uint8Array(receivedBytes);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.length;
    }
  }

  if (bytes[0] === 0x3c) {
    throw new Error(
      `Expected an asset at ${url} but the response body begins with '<' — the URL does not resolve.`,
    );
  }
  return bytes;
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
    fetchAsset(MODEL_URL, { onProgress }),
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
  return new Uint8Array();
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
