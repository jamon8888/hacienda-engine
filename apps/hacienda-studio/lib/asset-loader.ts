import { openDB } from "idb";

const DB_NAME = "xberg-studio-assets";
const DB_VERSION = 1;
const MODEL_STORE = "models";
const TESSDATA_STORE = "tessdata";

// GLiNER2 model from xberg-wasm (ner-candle-wasm feature)
// Using the new NerModel class from @xberg-io/xberg-wasm
// In dev, use Vite proxy to bypass HuggingFace CORS restrictions
const HF_BASE =
  "/api/huggingface/fastino/GLiNER2-Guardrails-PII-Multi/resolve/main";
const MODEL_URL = `${HF_BASE}/model.safetensors`;
const TOKENIZER_URL = `${HF_BASE}/tokenizer.json`;
const CONFIG_URL = `${HF_BASE}/config.json`;

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

export async function loadNerModel(): Promise<{
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

  // Fetch from HuggingFace CDN
  const [modelRes, tokenizerRes, configRes] = await Promise.all([
    fetch(MODEL_URL),
    fetch(TOKENIZER_URL),
    fetch(CONFIG_URL),
  ]);

  if (!modelRes.ok || !tokenizerRes.ok || !configRes.ok) {
    throw new Error("Failed to download NER model from HuggingFace");
  }

  const modelData = new Uint8Array(await modelRes.arrayBuffer());
  const tokenizerData = new Uint8Array(await tokenizerRes.arrayBuffer());
  const configData = new Uint8Array(await configRes.arrayBuffer());

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
): Promise<any> {
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
];
