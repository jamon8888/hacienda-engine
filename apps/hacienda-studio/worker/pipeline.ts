import type {
  FileInput,
  ProcessedFile,
  ProgressUpdate,
  AppConfig,
  Entity,
} from "../lib/types";

import {
  XbergEngine,
  WasmExtractInput,
  WasmExtractionConfig,
  WasmOutputFormat,
  WasmChunkingConfig,
  WasmOcrConfig,
  WasmNerConfig,
  WasmNerBackendKind,
  WasmImageExtractionConfig,
  WasmSecurityLimits,
} from "@xberg-io/xberg-wasm";
import initWasm from "@xberg-io/xberg-wasm";
// Let the bundler resolve and emit the binary. The previous absolute
// "/node_modules/@xberg-io/…/xberg_wasm_bg.wasm" only ever existed on a dev
// server: in production that path is answered by the SPA fallback with
// index.html, and WebAssembly rejects it as "expected magic word 00 61 73 6d,
// found 3c 21 64 6f".
import xbergWasmUrl from "@xberg-io/xberg-wasm/pkg/web/xberg_wasm_bg.wasm?url";
// Same class of bug as xbergWasmUrl above, for tesseract-wasm's OCR engine: its own
// `createOCREngine()` locates its .wasm via `resolve(path, import.meta.url)` — a
// *runtime* string concatenation, not a statically-analyzable `new URL('literal.wasm',
// import.meta.url)` — so Vite's production build never discovers the file to copy it
// into `dist/`. In dev, `node_modules` is served with real relative paths so it works
// by accident; in production the request 404s and Vercel's SPA fallback answers with
// index.html ("expected magic word ... found 54 68 65 20", i.e. the start of "The page
// could not be found"), which silently disables OCR everywhere it's deployed.
//
// Neither a bare `tesseract-wasm/dist/...?url` specifier (blocked — the package's own
// `exports` map only lists "." and "./node") nor a relative `../node_modules/...?url`
// import (resolves locally under pnpm, but Vercel's `npm run build:vercel` lays out
// node_modules differently and Rollup's worker-bundling context fails to resolve it
// there — confirmed by a real deploy) survives every environment this app builds in.
// Copied into `public/tesseract-wasm/` instead, matching `public/models/`'s existing
// convention for exactly this problem: files Vite serves/copies verbatim at a stable,
// bundler-independent root path in dev, `vite build`, and any package manager alike.
// Re-copy from `node_modules/tesseract-wasm/dist/*.wasm` if the tesseract-wasm version
// in package.json ever changes. Both SIMD and non-SIMD builds are present so
// `createOcrBackend` below can pick the right one via `supportsFastBuild()`, mirroring
// the check tesseract-wasm's own (broken, in production) default path does internally.
const tesseractCoreWasmUrl = "/tesseract-wasm/tesseract-core.wasm";
const tesseractCoreFallbackWasmUrl = "/tesseract-wasm/tesseract-core-fallback.wasm";
import {
  extractEntities,
  isBridgeEntityArray,
  type BridgeEntity,
} from "../lib/ner-bridge";
import { createNerBackend, loadNerModel, loadTessdata } from "../lib/asset-loader";
// `toLiteParseOcrEngine` is unused while the LiteParse-OCR stopgap above forces
// `ocrEngine: null` unconditionally — restore the import alongside that code once
// upstream fixes the ocr_merge.rs Tokio panic.
import { extractPdfWithLiteParse, checkPdfNeedsOcr } from "../lib/pdf-liteparse";
import {
  initPiiEngine,
  redactPii,
  scanForPii,
  redactPiiWithModelEntities,
  scanForPiiWithModelEntities,
  recordPiiAudit,
  type PiiEntity,
  type PiiCategoryWire,
  type PiiPipelineResult,
} from "../lib/pii-engine";
import { deriveKeyHex, mintToken } from "../lib/pseudonymize";
import { hashSpanForProcessing, looksLikePseudonymToken } from "../lib/redaction-modes";
import { computeContentHash } from "../lib/content-hash";
import { VerticalDictionary } from "../lib/verticals/dictionary";
import {
  loadVerticalTaxonomy,
  VerticalEntityMetadata,
} from "../lib/verticals/index";
import { BatchEntityRegistry, type RegistryEntity } from "../lib/registry";
import { KGExporter } from "../lib/kg-export";
import { TranscriptionRequestBridge } from "./transcribe-bridge";
import type { TranscriptionResult } from "../lib/transcription/types";
import {
  relativeEntityLink,
  renderAnnotatedMarkdown,
} from "../lib/annotate";
import {
  assembleZip,
  buildEntityFile,
  buildGlossaryIndex,
  type ZipBatch,
} from "../lib/zip-export";
import {
  computeRelatedDocuments,
  topRelatedDocuments,
  buildRelatedDocumentsSection,
} from "../lib/related-documents";

// Track I4: re-exported unchanged so nothing importing these from "./pipeline" (the
// vitest suite included) needs to know they now live in lib/annotate.ts — see that
// file's header for why the split exists (App.tsx needs them without this module's
// top-level `self.onmessage =`).
export { relativeEntityLink, renderAnnotatedMarkdown };
// Track K/Phase 2: same re-export pattern, now for lib/zip-export.ts — see that
// file's header for why the split exists (the worker needs a "build-zip" round trip
// that runs independently of processFiles(), not just once at the end of it).
export { buildEntityFile, buildGlossaryIndex };

// Browser/wasm32-specific extraction tuning. xberg's own OCR memory throttle
// (`adapt_batch_size_to_memory`) never activates in the browser build (its
// `get_available_memory()` only implements the syscall for native Linux/macOS), so
// these are the mitigations xberg's docs actually expose for wasm — see
// docs-site/src/content/docs/guides/ocr.mdx and reference/api-wasm.md in the xberg repo.

// xberg's own OCR troubleshooting guide recommends 150 DPI as the floor for "faster
// throughput" / "significantly less memory" on large PDFs (300 is xberg's balanced
// default, 600 is accuracy-first). `maxDpi` is capped at the same value so
// `autoAdjustDpi` (on by default) can never escalate back up to a heavier DPI for a
// low-quality page — that escalation is exactly the unbounded-memory path we're
// avoiding. `minDpi` is left at its default floor: autoAdjustDpi lowering further when
// a page doesn't need 150 only saves more memory, never costs anything.
const OCR_TARGET_DPI = 150;

// A native server can reasonably wait out the default 600s extraction timeout; a
// browser tab blocking a worker for 10 minutes on one pathological upload is a bad UX
// on its own, independent of memory. 90s is generous for OCR on a normal-sized upload
// (page-count-limited by `checkPdfPageSafety` in lib/asset-loader.ts) while still
// failing a stuck extraction back to the user instead of hanging indefinitely.
const EXTRACTION_TIMEOUT_SECS = 90n;

// Lowered from xberg's native-oriented defaults (100MB / 500MB / 50MB) to match this
// app's own 50MB upload cap (`validateFile` in lib/asset-loader.ts) — a single upload
// can never legitimately need more extracted-text growth, archive expansion, or
// embedded-file size than the cap it was admitted under, so these bound worst-case
// memory for a zip-bomb-style or otherwise adversarial file without affecting any
// normal document.
const SECURITY_LIMITS = {
  maxContentSize: 30 * 1024 * 1024,
  maxArchiveSize: 100 * 1024 * 1024,
};
const MAX_EMBEDDED_FILE_BYTES = 20n * 1024n * 1024n;

let wasmReady: Promise<void> | null = null;

// Track K/Phase 2: the most recently completed batch's state, retained so the
// on-demand "build-zip" message (self.onmessage below) can call assembleZip()
// without processFiles() needing to build the zip eagerly. Concurrent batches
// aren't supported (see the plan's risk notes) — a second "process" message
// mid-batch would overwrite this before the first batch's zip request lands.
let lastBatch: ZipBatch | null = null;

// Worker-side half of the whisper transcription bridge — see `worker/transcribe-bridge.ts`'s
// header for why transcription has to be requested from the main thread rather than run
// here. Module-scope (not per-batch): `self.onmessage`'s "transcribe-response" case below
// needs the same instance `processFile` called `.request()` on, and a single worker instance
// legitimately outlives one "process" batch (the user can upload again without a page
// reload), so requestIds must stay unique across batches too — `TranscriptionRequestBridge`
// only guarantees that for calls against the same instance.
const transcriptionBridge = new TranscriptionRequestBridge((message, transfer) =>
  // Not `self.postMessage(message, transfer ?? [])`: this file's tsconfig has no
  // "webworker" lib (App.tsx, sharing the same tsconfig, needs "DOM" instead — the two
  // are mutually exclusive in one `lib` array), so TypeScript types `self` as `Window`
  // here, not `DedicatedWorkerGlobalScope`. `Window.postMessage`'s array-transfer overload
  // requires a `targetOrigin` string first, which makes no sense for a worker; its
  // options-object overload (`{ transfer }`) has no such requirement and is also exactly
  // what `DedicatedWorkerGlobalScope.postMessage` accepts at runtime — valid under both
  // the (slightly wrong) compile-time type and the real one.
  self.postMessage(message, { transfer }),
);

// Track B1/B2: `createNerBackend()` targets xberg-wasm's neural `NerModel` — multilingual,
// PII-specific, and already the model the onboarding screen downloads. `null` means the
// model failed to load (or was never cached), in which case `selectNerBridge` falls back to
// `extractEntities` (compromise.js, English-only). IndexedDB is worker-accessible, so this
// load hits the cache `App.tsx`'s preloadAssets already populated — no re-download here.
type NerRuntime = Awaited<ReturnType<typeof createNerBackend>>;
let nerRuntime: NerRuntime | null = null;

async function initNerBackend(): Promise<void> {
  try {
    const load = await loadNerModel();
    if (!load.ok) {
      // `selectNerBridge` falls back to compromise.js when `nerRuntime` stays null. The
      // main thread already surfaced the reason to the user in `App.tsx`; repeating it
      // from the worker would only duplicate the banner.
      console.warn("[Worker] Neural NER unavailable:", load.reason, load.message);
      nerRuntime = null;
      return;
    }
    const { model, tokenizer, encoderConfig } = load.assets;
    nerRuntime = await createNerBackend(model, tokenizer, encoderConfig);
    console.log("[Worker] Neural NER backend loaded");

    // Tier 1.1: No longer loading model into hacienda-wasm for duplicate inference.
    // PII detection now reuses the entity glossary NER results via
    // redactPiiWithModelEntities/scanForPiiWithModelEntities.
    // The hacienda-wasm regex-only fallback is used when neural NER is unavailable.
  } catch (e) {
    console.warn(
      "[Worker] Neural NER backend unavailable, using regex/compromise fallback:",
      e,
    );
    nerRuntime = null;
  }
}

/**
 * Takes the runtime explicitly rather than reading `nerRuntime` itself, so the fallback
 * decision is a pure function of its input and testable without touching worker/module
 * state or a real WASM model.
 */
export function selectNerBridge(
  runtime: NerRuntime | null,
): (text: string, categories: string[]) => Promise<BridgeEntity[]> {
  if (!runtime) return extractEntities;
  return async (text, categories) => {
    try {
      const raw: unknown = await runtime.detect(text, { categories });
      // `runtime.detect()` crosses a WASM boundary — its result isn't something this
      // code controls the shape of. Validate rather than `as BridgeEntity[]`-assert it.
      if (!isBridgeEntityArray(raw)) {
        throw new Error(
          "Neural NER backend returned a malformed result (not a BridgeEntity[])",
        );
      }
      return raw;
    } catch (err: unknown) {
      // Candle F16/F32 dtype mismatch — GLiNER2 weights are F16 but candle creates
      // F32 activations with no auto-cast. Falls back to regex/compromise for this
      // document rather than crashing the batch.
      const msg = err instanceof Error ? err.message : String(err);
      if (msg.includes("dtype mismatch")) {
        console.warn(
          "[Worker] Neural NER dtype mismatch (F16/F32), falling back to regex:",
          msg,
        );
        return extractEntities(text, categories);
      }
      throw err;
    }
  };
}

/**
 * xberg-wasm has no in-binary OCR backend (see `xberg-wasm/src/bridge/ocr.rs`'s own
 * header: "There is no in-binary fallback: when no backend is injected, OCR is
 * unavailable") — every `engine.extract()` call needs an `ocr` bridge injected into
 * `XbergEngine`'s constructor, the same injection point `ner` already uses. Without
 * one, `extractConfig.ocr.backend = "tesseract-wasm"` below is a no-op string nobody
 * reads: scanned pages and embedded images silently got zero OCR text. `null` means
 * the backend failed to load (network/wasm error); `selectOcrBridge` returns
 * `undefined` in that case, matching "no backend injected" exactly — there's no
 * regex-style fallback for OCR the way there is for NER.
 */
type OcrRuntime = Awaited<ReturnType<typeof createOcrBackend>>;
let ocrRuntime: OcrRuntime | null = null;

async function createOcrBackend() {
  const { createOCREngine, supportsFastBuild } = await import("tesseract-wasm");
  // Fetch the binary ourselves via the bundler-resolved URLs above and hand it in
  // directly — this is exactly the `wasmBinary` override tesseract-wasm's own API
  // documents "to customize how the binary URL is determined and fetched", used here
  // to bypass its default resolution, which is broken in production (see the import
  // comment above).
  const wasmUrl = supportsFastBuild() ? tesseractCoreWasmUrl : tesseractCoreFallbackWasmUrl;
  const wasmResponse = await fetch(wasmUrl);
  // `fetch` resolves (doesn't reject) on a 404 — without this check, a broken/missing
  // asset path hands an HTML error page to `createOCREngine({ wasmBinary })` and fails
  // with the opaque "expected magic word" error instead of a clear HTTP status + URL.
  if (!wasmResponse.ok) {
    throw new Error(`Failed to fetch OCR wasm binary: HTTP ${wasmResponse.status} ${wasmUrl}`);
  }
  const wasmBinary = await wasmResponse.arrayBuffer();
  const engine = await createOCREngine({ wasmBinary });
  const tessdata = await loadTessdata("eng");
  engine.loadModel(tessdata);
  return engine;
}

async function initOcrBackend(): Promise<void> {
  try {
    ocrRuntime = await createOcrBackend();
    console.log("[Worker] Tesseract OCR backend loaded");
  } catch (e) {
    console.warn("[Worker] OCR backend unavailable:", e);
    ocrRuntime = null;
  }
}

/**
 * Shape `xberg-wasm`'s injected-OCR contract expects back, per
 * `xberg-wasm/src/bridge/ocr.rs`'s `OcrResult`/`OcrLineResult` — confirmed from that
 * source, not guessed, including that `lines` is optional on the wire (a missing/
 * malformed array degrades to empty rather than an error).
 */
interface OcrBridgeResult {
  text: string;
  lines: Array<{
    text: string;
    confidence: number;
    bbox?: { x: number; y: number; width: number; height: number };
  }>;
}

/**
 * Takes the runtime explicitly, same rationale as `selectNerBridge`: a pure function
 * of its input, testable without touching worker/module state or a real wasm engine.
 */
export function selectOcrBridge(
  runtime: OcrRuntime | null,
): ((imageBytes: Uint8Array, opts: { language: string }) => Promise<OcrBridgeResult>) | undefined {
  if (!runtime) return undefined;
  return async (imageBytes) => {
    // The Rust side hands us encoded (PNG/JPEG) image bytes, not raw pixels —
    // tesseract-wasm's `loadImage` wants an `ImageBitmap`/`ImageData`, so decode
    // first. `createImageBitmap` is available in a dedicated worker's global scope.
    // `.buffer` alone would be wrong if `imageBytes` is a view into a larger
    // buffer (wrong byteOffset/length); slice to the view's exact bytes.
    const bytes = imageBytes.buffer.slice(
      imageBytes.byteOffset,
      imageBytes.byteOffset + imageBytes.byteLength,
    ) as ArrayBuffer;
    const bitmap = await createImageBitmap(new Blob([bytes]));
    try {
      runtime.loadImage(bitmap);
      const lines = runtime.getTextBoxes("line");
      return {
        text: lines.map((l) => l.text).join("\n"),
        lines: lines.map((l) => ({
          text: l.text,
          confidence: l.confidence,
          bbox: {
            x: l.rect.left,
            y: l.rect.top,
            width: l.rect.right - l.rect.left,
            height: l.rect.bottom - l.rect.top,
          },
        })),
      };
    } finally {
      // Keeps the loaded model, drops just this page's image data — matches
      // `OCREngine.clearImage()`'s own documented intent (no way to shrink wasm
      // memory otherwise until the whole engine is destroyed).
      runtime.clearImage();
      bitmap.close();
    }
  };
}

function postProgress(update: ProgressUpdate): void {
  self.postMessage({ type: "progress", ...update });
}

function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .substring(0, 64);
}

/**
 * Task 3 (spec §8 step 3): `slugify` above derives a filename from an entity's real
 * surface form — exactly what a pseudonym-keyed entity must never expose. This is the
 * pseudonymize-mode equivalent: a short, deterministic, filesystem-safe slug derived
 * from the *token* instead, so `entities/person-<slug>.md` carries no trace of the real
 * name. FNV-1a, same construction as `lib/redaction-modes.ts`'s Hash-mode `fingerprint`
 * — not a security boundary (the token itself is already the non-identifying value;
 * this only needs to avoid collisions across one batch's entity count, not resist
 * attack), so a fast non-cryptographic hash is the right tool, not `crypto.subtle`.
 */
/**
 * SHA-256 via `computeContentHash`, truncated to 12 hex chars (48 bits) — the same
 * construction `assignDocId` and `lib/registry.ts`'s `identityFor` already use for
 * document and entity ids, not the FNV-1a 32-bit hash this function used before. A
 * 32-bit space starts having non-trivial collision odds (birthday bound) around
 * 65,536 distinct values, which is not obviously unreachable for pseudonym tokens —
 * SIV ciphertext looks uniformly random to a hash, and two different real people
 * colliding to the same slug would silently overwrite one's entity file with the
 * other's, corrupting both dossiers with no error raised. 48 bits pushes that bound
 * out of any realistic batch's reach, matching the other two id schemes in this
 * codebase rather than being the one weaker outlier among them.
 */
async function tokenSlug(token: string): Promise<string> {
  const digest = await computeContentHash(new TextEncoder().encode(token).buffer);
  return digest.slice(0, 12);
}

/**
 * Task 4 (spec §8 step 4): content-derived document id, replacing assignment-ordinal
 * `doc-001`/`doc-002` — those renumbered on any re-export whose upload order changed,
 * breaking `document_entities` in `entities-registry.json` and any external reference
 * into it. `usedIds` is this batch's already-assigned ids, checked *before* calling
 * `processFile` for the file about to receive one (not `docPaths`, which is only
 * populated once `processFile` returns, too late for the file currently in flight).
 *
 * Genuinely duplicate uploads (byte-identical files) hash to the same base id; a
 * disambiguating numeric suffix is appended rather than silently colliding, because
 * `docId` doubles as a `Map` key for `docPaths` — an undetected collision there would
 * silently overwrite one file's backlinks with the other's, corrupting both.
 */
export async function assignDocId(
  bytes: ArrayBuffer,
  usedIds: Set<string>,
): Promise<string> {
  const hash = await computeContentHash(bytes);
  const base = `doc-${hash.slice(0, 12)}`;
  let docId = base;
  let suffix = 2;
  while (usedIds.has(docId)) {
    docId = `${base}-${suffix}`;
    suffix++;
  }
  usedIds.add(docId);
  return docId;
}

/** The one shape the NER-result loop below actually reads from an entity. */
interface RawNerEntity {
  category: string;
  text: string;
  start: number;
  end: number;
}

/**
 * `nerEngine.ner()` is an external WASM bridge call — its result isn't
 * something this code controls the shape of. Without this guard, a malformed
 * or incompatible result would throw a raw, uncaught `TypeError` reading
 * `.category`/`.text`/`.start`/`.end` off an unexpected value, failing the
 * whole file's processing instead of the "continue without NER" degradation
 * the surrounding try/catch already intends for a NER failure.
 */
function isRawNerEntity(value: unknown): value is RawNerEntity {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.category === "string" &&
    typeof v.text === "string" &&
    typeof v.start === "number" &&
    typeof v.end === "number"
  );
}

/**
 * Map NER category (from xberg/entity glossary) to the wire format
 * `hacienda-core`'s `PiiCategory` enum actually deserializes — mirrors
 * `hacienda-core/src/pii/ner.rs`'s `to_pii_category` function exactly, variant for
 * variant. `PiiCategory` derives `#[serde(rename_all = "snake_case")]`: every unit
 * variant is a bare lowercase-snake_case string (`"phone_number"`, `"full_name"`, ...),
 * but the one tuple variant, `Custom(String)`, is externally tagged as
 * `{ custom: "<label>" }` — a bare string there does not deserialize (confirmed via a
 * live upload after the Aug 2026 hacienda-wasm rebuild: `process_with_model_entities`
 * had never actually run against real model entities before that rebuild, since the
 * stale committed `pkg/` output didn't export it yet — this category-shape mismatch
 * was dormant the whole time, masked by that staleness, and only surfaced once the
 * function started being called for real).
 */
function nerCategoryToPiiCategory(nerCategory: string): PiiCategoryWire {
  const lower = nerCategory.toLowerCase();
  switch (lower) {
    case "person":
      return "person";
    case "organization":
      return "organization";
    case "location":
      return "address";
    case "email":
      return "email";
    case "phone":
      return "phone_number";
    case "url":
      return "url";
    case "date":
      return { custom: "Date" };
    case "time":
      return { custom: "Time" };
    case "money":
      return { custom: "Money" };
    case "percent":
      return { custom: "Percent" };
    case "ssn":
    case "social_security_number":
      return "ssn";
    case "credit_card":
    case "creditcard":
      return "credit_card";
    case "iban":
      return "iban";
    case "passport":
    case "passport_number":
      return "passport_number";
    case "address":
      return "address";
    case "full_name":
    case "fullname":
    case "name":
      return "full_name";
    default:
      // Unknown categories pass through as custom, matching `to_pii_category`'s
      // `_ => PiiCategory::Custom(label.clone())` fallback — the *original* NER
      // category text (not lowercased), since that's what Rust's `label.clone()` uses.
      return { custom: nerCategory };
  }
}

const MA_TERMS =
  /\b(m&a|merger|acquisition|acquirer|acquired|acquires|target|spa|share purchase|earnout|indemnification|representation and warranty|material adverse change|break fee|closing condition|deal value|purchase price)\b/;
const FS_TERMS =
  /\b(private equity|venture capital|limited partner|general partner|carried interest|management fee|nav|irr|dpi|tvpi|portfolio company|fund size|capital commitment)\b/;

/**
 * Assign a vertical to entities the taxonomy does not recognise, based on the
 * vocabulary of the surrounding document.
 */
function classifyDocumentVertical(markdown: string): string {
  const text = markdown.toLowerCase();
  if (MA_TERMS.test(text)) return "m&a";
  if (FS_TERMS.test(text)) return "financial_services";
  return "shared";
}

function deduplicateEntities(entities: Entity[]): Entity[] {
  const map = new Map<string, Entity>();
  for (const e of entities) {
    const key = `${e.type}:${e.slug}`;
    if (map.has(key)) {
      const existing = map.get(key)!;
      existing.count += e.count;
      existing.spans.push(...e.spans);
    } else {
      map.set(key, e);
    }
  }

  // Filter overlapping spans within each entity to prevent nested links. Compares each
  // candidate against the last *retained* span, not the previous span in sorted order —
  // for [0,3], [2,10], [4,5], comparing against arr[i-1] would drop [4,5] against the
  // already-dropped [2,10] even though [4,5] doesn't overlap the retained [0,3].
  return Array.from(map.values())
    .map((e) => {
      const sorted = [...e.spans].sort((a, b) => a.start - b.start);
      const retained: typeof sorted = [];
      for (const span of sorted) {
        const previous = retained[retained.length - 1];
        if (!previous || span.start >= previous.end) retained.push(span);
      }
      return { ...e, spans: retained };
    })
    .sort((a, b) => b.count - a.count);
}

/**
 * Track A2: an entity whose span the markdown body is about to redact must not
 * still be named in the frontmatter, the "## Entities" glossary,
 * entities-registry.json, or the KG export — exporting a redacted document beside
 * a knowledge graph naming every entity would defeat the point of redacting.
 * Only filters when output is actually being rewritten; scan-only mode doesn't
 * touch the markdown, so there's nothing to defeat.
 *
 * Task 3 (spec §8 step 3): dropping the whole entity is still correct for mask, hash,
 * and remove — none of those leave anything reversible behind, so the entity's real
 * name has nowhere safe to live in the export. Pseudonymize with a real key is
 * different: `[PERSON:session:a41f]` is a stable, non-identifying, reversible-only-
 * with-the-key handle. Dropping the entity there throws away exactly the
 * cross-document structure pseudonymization exists to preserve — see the spec's §3.2.
 * An entity whose *every* overlapping finding carries a real pseudonym token
 * (`looksLikePseudonymToken`) survives, rekeyed on that token: `name` and `slug`
 * become the token and `tokenSlug(token)` respectively, so nothing downstream
 * (registry, entity files, glossary, frontmatter, KG export) ever sees the real
 * surface form again. An entity with even one overlapping finding that is *not* a
 * real token — mask, hash, remove, or pseudonymize silently degraded to a mask
 * template because no passphrase was given (`AppConfig.redactionMode`'s doc comment)
 * — is dropped exactly as before: under-including is still the safe direction when
 * there is no token to key it on.
 *
 * Deliberately keyed on each finding's `redact_template` shape rather than a
 * `redactionMode` parameter: mask templates (`[EMAIL]`), hash fingerprints
 * (`#email:1a2b…`), and empty remove templates all fail
 * `looksLikePseudonymToken`'s stricter three-part pattern on their own, so this
 * cannot be told the batch is pseudonymized while the findings it actually receives
 * say otherwise — the caller has one fewer parameter to get out of sync.
 */
export async function filterExportableEntities(
  entities: Entity[],
  piiFindings: PiiEntity[],
  redactPiiInOutput: boolean,
): Promise<Entity[]> {
  if (!redactPiiInOutput) return entities;
  const overlappingFindings = (span: { start: number; end: number }) =>
    piiFindings.filter((p) => span.start < p.end && p.start < span.end);

  const result: Entity[] = [];
  for (const e of entities) {
    const overlaps = e.spans.flatMap(overlappingFindings);
    if (overlaps.length === 0) {
      result.push(e);
      continue;
    }
    if (!overlaps.every((f) => looksLikePseudonymToken(f.redact_template))) {
      continue; // dropped: at least one overlapping finding has no reversible token
    }
    // AES-SIV is deterministic (`lib/pseudonymize.ts`'s `mintToken`), so every mention
    // of the same real name in this document already minted the identical token —
    // any overlapping finding's template is as good as another to key on.
    const token = overlaps[0].redact_template;
    result.push({ ...e, name: token, slug: await tokenSlug(token) });
  }
  return result;
}

/**
 * Always-quote YAML scalar: correctness over minimalism. Entity names, filenames, and
 * roles are arbitrary text from an uploaded document — a name containing `:`, starting
 * with `-`, or looking like a YAML flow character would silently corrupt an unquoted
 * plain scalar. A double-quoted scalar with `\`/`"`/newline escaped is unambiguous for
 * every input, so nothing here needs to reason about which values are "safe enough" to
 * leave bare.
 */
function yamlString(value: string): string {
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n")}"`;
}

/**
 * Task 5.3 (spec §8 step 5, §5.1.6): replaces the single-line `entities: [{...}]` JSON
 * blob with a real multi-line YAML list — the JSON form put every entity's full metadata
 * on one line, which a filesystem-MCP reader's `grep` returns as one unparseable
 * fragment rather than a readable record (spec §4's whole argument for why this bundle's
 * shape matters to an agent reading it, not just a human).
 *
 * `doc_type` is `classifyDocumentVertical`'s own coarse, keyword-matched classification
 * (`m&a` / `financial_services` / `shared`), surfaced at the document level for the
 * first time — previously computed but only ever used as a per-entity vertical
 * *fallback*, never written to frontmatter itself. Deliberately not a richer
 * "contract vs invoice vs board-minutes" classification: no classifier head exists on
 * the reachable GLiNER2 path (spec §2.2), and Tier 1 semantic labels are out of this
 * plan's scope — inventing a more specific-sounding field from the same regex signal
 * would overclaim what was actually derived.
 *
 * `entity_ids` reconstructs each `ent-<slug>` from `Entity.slug` rather than carrying a
 * separate `id` field on the worker-local `Entity` type: post-Task-4, `slug` already *is*
 * the id's hash suffix (`registry.ts`'s `identityFor`: `id = "ent-" + slug`), so
 * widening `Entity` for a value already fully recoverable from a field it already has
 * would be duplication, not a new capability.
 */
export function buildFrontmatter(
  input: FileInput,
  entities: Entity[],
  piiEntitiesFound: number,
  docType: string,
): string {
  const type = input.type.split("/")[1] || "unknown";
  const entityIds = entities.map((e) => `"ent-${e.slug}"`).join(", ");

  const entityLines = entities.map((e) => {
    const lines = [
      `  - name: ${yamlString(e.name)}`,
      `    type: ${e.type.charAt(0).toUpperCase() + e.type.slice(1)}`,
      `    slug: ${yamlString(e.slug)}`,
    ];
    if (e.vertical) lines.push(`    vertical: ${yamlString(e.vertical)}`);
    if (e.sector) lines.push(`    sector: ${yamlString(e.sector)}`);
    if (e.roles && e.roles.length) {
      lines.push(`    roles: [${e.roles.map(yamlString).join(", ")}]`);
    }
    return lines.join("\n");
  });
  const entitiesBlock =
    entities.length === 0 ? "entities: []" : `entities:\n${entityLines.join("\n")}`;

  return `---
source: ${yamlString(input.name)}
type: ${type}
processed: ${new Date().toISOString()}
pii_entities_found: ${piiEntitiesFound}
doc_type: ${yamlString(docType)}
entity_ids: [${entityIds}]
${entitiesBlock}
---`;
}

function buildGlossary(entities: Entity[], docPath: string): string {
  if (entities.length === 0) return "";
  let md = "\n## Entities\n\n";
  for (const e of entities) {
    const verticalInfo =
      e.vertical && e.vertical !== "shared" ? ` [${e.vertical}]` : "";
    md += `- [${e.name}](${relativeEntityLink(docPath, e)}) \`${e.type.charAt(0).toUpperCase() + e.type.slice(1)}${verticalInfo}\` — mentioned ${e.count} time${e.count > 1 ? "s" : ""}\n`;
  }
  return md;
}

async function initEngine(): Promise<void> {
  await Promise.all([
    initWasm({ module_or_path: fetch(xbergWasmUrl) }),
    initPiiEngine(),
  ]);
  // NER model loading is deliberately not awaited here. On a fresh/uncached browser it is a
  // ~614 MB network fetch (minutes, not seconds) — awaiting it inside the same Promise.all
  // that gates the worker's "ready" handshake left the file input disabled for that whole
  // fetch on every uncached session, independent of whether onboarding had already been
  // dismissed (the worker's IndexedDB cache is separate from the main thread's
  // "xberg-studio-visited" flag). `initNerBackend` already catches its own errors and
  // `selectNerBridge` reads module-level `nerRuntime` fresh per file, so a file processed
  // while this is still in flight correctly falls back to the regex/compromise bridge and
  // later files upgrade to neural NER transparently once it resolves.
  void initNerBackend();
  // Unlike NER, this IS awaited: it's small (~4MB tessdata + a small wasm binary, seconds
  // not minutes) and, unlike NER, there is no regex-style fallback for OCR — a file whose
  // first page needs OCR before this resolves silently gets zero text for that page with
  // no error and no way to "upgrade" later (see `selectOcrBridge`'s header). Awaiting it
  // here means the worker's "ready" handshake — which gates the file input — only fires
  // once OCR is actually usable, closing that race instead of leaving it to chance.
  await initOcrBackend();
}

/** Shared by both extraction branches (xberg and LiteParse) in `processFile` below. */
function logExtractionError(fileName: string, extractError: unknown): void {
  console.error(`[Worker] EXTRACTION FAILED for ${fileName}:`, extractError);
  console.error("[Worker] Extraction error type:", typeof extractError);
  console.error(
    "[Worker] Extraction error constructor:",
    (extractError as { constructor?: { name?: string } })?.constructor?.name,
  );
  if (extractError instanceof Error) {
    console.error("[Worker] Extraction error name:", extractError.name);
    console.error("[Worker] Extraction error message:", extractError.message);
    console.error("[Worker] Extraction error stack:", extractError.stack);
  } else {
    console.error("[Worker] Raw extraction error:", JSON.stringify(extractError));
  }
}

async function processFile(
  input: FileInput,
  config: AppConfig,
  verticalDict: VerticalDictionary,
  registry: BatchEntityRegistry,
  docId: string,
  /** Derived once per batch in `processFiles`; `null` unless `redactionMode` is
   * `"pseudonymize"` and a passphrase was given. */
  pseudonymKeyHex: string | null,
): Promise<ProcessedFile> {
  console.log(`[Worker] processFile START: ${input.name} (${input.type}, ${input.bytes.byteLength} bytes)`);
  postProgress({ file: input.name, stage: "extract", percent: 10 });

  // Track I2: every document lives under `documents/` in the exported vault,
  // at the same relative path it was uploaded from — this is the coordinate
  // both `renderAnnotatedMarkdown`'s body links and `buildGlossary`'s local
  // summary need to compute a correct `../entities/*.md` relative path.
  const docPath = "documents/" + input.name.replace(/\.[^.]+$/, ".md");

  const isAudio = input.type.startsWith("audio/");
  const isVideo = input.type.startsWith("video/");

  let markdown = "";
  let transcriptionResult: TranscriptionResult | null = null;

  if ((isAudio || isVideo) && config.enableTranscription) {
    console.log(`[Worker] Requesting transcription of ${input.name} from the main thread...`);
    transcriptionResult = await transcriptionBridge.request(
      input.name,
      new Uint8Array(input.bytes),
      input.type,
      {
        modelSize: config.transcriptionModel,
        language:
          config.transcriptionLanguage === "auto"
            ? undefined
            : config.transcriptionLanguage,
        task: config.translateToEnglish ? "translate" : "transcribe",
      },
    );
    markdown = transcriptionResult.text;
    console.log(
      `[Worker] Transcription complete: ${markdown.substring(0, 100)}...`,
    );
  } else if (input.type === "application/pdf" && config.pdfEngine === "liteparse") {
    // See docs/superpowers/specs/2026-08-22-liteparse-pdf-extraction-design.md — PDFium-
    // backed, bounded-memory-batched alternative to xberg-wasm's `pdf_oxide` path below,
    // used only when `config.pdfEngine` opts in. Every non-PDF format, and PDFs when this
    // flag is off, keep going through the xberg branch unchanged.
    console.log(`[Worker] Extracting PDF via LiteParse for ${input.name}...`);
    try {
      // OCR is unconditionally withheld from LiteParse right now, not just when
      // `ocrRuntime` failed to load: wiring a real OCR engine into LiteParse's
      // `ocrEngine` option makes it exercise `ocr_merge.rs`, and the compiled
      // @llamaindex/liteparse-wasm@2.13.1 binary panics there ("there is no reactor
      // running, must be called from the context of a Tokio 1.x runtime") — an
      // upstream bug in that crate's WASM target, not something fixable from this
      // repo. Passing `ocrEngine: null` below keeps `needsOcr` false in
      // `extractPdfWithLiteParse` (lib/pdf-liteparse.ts), which skips that code path
      // entirely — the same "no crash" behavior LiteParse gets today whenever
      // `ocrRuntime` itself fails to load, just applied unconditionally until
      // upstream fixes the crate. Scanned/image PDFs export with no text; other OCR
      // uses (images, and PDFs on the `xberg` engine below) are unaffected — they go
      // through a different WASM module (`selectOcrBridge`) that doesn't touch this.
      //
      // The console.warn always fires (diagnostic signal, cheap); the user-facing toast
      // only fires when `checkPdfNeedsOcr` finds a page that actually needs it — most
      // PDFs are fully digital text and would otherwise see a spurious "no text
      // extracted" warning about content that extracted just fine.
      console.warn(`[Worker] OCR withheld from LiteParse for ${input.name} (upstream Tokio panic in ocr_merge.rs)`);
      if (!input.disableOcr && (await checkPdfNeedsOcr(new Uint8Array(input.bytes)))) {
        self.postMessage({
          type: "warning",
          file: input.name,
          message: "OCR est temporairement désactivée pour l'extraction PDF (bug amont) — les pages scannées ou images de ce fichier seront exportées sans texte extrait.",
        });
      }

      const extractStart = performance.now();
      const { markdown: liteparseMarkdown } = await extractPdfWithLiteParse(
        new Uint8Array(input.bytes),
        {
          ocrEngine: null,
          disableOcr: !!input.disableOcr,
          pageCount: input.pdfPageCount,
          dpi: OCR_TARGET_DPI,
        },
      );
      const extractMs = performance.now() - extractStart;
      console.log(`[Worker] LiteParse extract completed in ${extractMs.toFixed(0)}ms for ${input.name}`);
      postProgress({ file: input.name, stage: "extract", percent: 50 });

      if (!liteparseMarkdown) {
        console.error(`[Worker] No content extracted from ${input.name} via LiteParse`);
        throw new Error("No content extracted");
      }

      markdown = liteparseMarkdown;
      console.log(`[Worker] Extracted ${markdown.length} chars from ${input.name} via LiteParse`);
    } catch (extractError) {
      logExtractionError(input.name, extractError);
      throw extractError;
    }
  } else {
    console.log(`[Worker] Extracting content from ${input.name}...`);
    try {
      const extractInput = WasmExtractInput.fromBytes(
        new Uint8Array(input.bytes),
        input.type,
        input.name,
      );

      const extractConfig = WasmExtractionConfig.default();
      extractConfig.outputFormat = WasmOutputFormat.Markdown;
      extractConfig.chunking = WasmChunkingConfig.default();
      extractConfig.chunking.maxCharacters = config.chunkSize;
      extractConfig.ocr = WasmOcrConfig.default();
      extractConfig.ocr.backend = "tesseract-wasm";
      extractConfig.ocr.language = ["eng"];

      extractConfig.images = WasmImageExtractionConfig.default();
      extractConfig.images.targetDpi = OCR_TARGET_DPI;
      extractConfig.images.maxDpi = OCR_TARGET_DPI;

      extractConfig.extractionTimeoutSecs = EXTRACTION_TIMEOUT_SECS;
      extractConfig.maxEmbeddedFileBytes = MAX_EMBEDDED_FILE_BYTES;
      extractConfig.securityLimits = WasmSecurityLimits.default();
      extractConfig.securityLimits.maxContentSize = SECURITY_LIMITS.maxContentSize;
      extractConfig.securityLimits.maxArchiveSize = SECURITY_LIMITS.maxArchiveSize;

      // Set by `lib/asset-loader.ts`'s `checkPdfPageSafety` for documents whose page
      // count makes per-page OCR raster+text accumulation a memory risk — xberg-wasm's
      // own OCR batch-size throttle never actually shrinks anything in the browser
      // build (`adapt_batch_size_to_memory`'s `get_available_memory()` always returns 0
      // on wasm32), so nothing else caps this. Native text extraction is unaffected.
      //
      // `extractConfig.disableOcr` only turns off OCR *text recognition* — it does not
      // stop `extractConfig.images` from rasterizing every page at `OCR_TARGET_DPI`
      // regardless (`extractImages`/`includePageRasters` are independent flags on
      // `WasmImageExtractionConfig`, defaulted on above). For a long, image-heavy PDF
      // that rasterization is the same memory spike the page-count check exists to
      // avoid, and left unthrottled it traps the whole wasm module (`RuntimeError:
      // unreachable executed`, confirmed live on a 16MB/many-page scanned PDF) instead
      // of failing gracefully — so gate it behind the same safety flag.
      if (input.disableOcr) {
        extractConfig.disableOcr = true;
        extractConfig.images.extractImages = false;
        extractConfig.images.includePageRasters = false;
        extractConfig.images.runOcrOnImages = false;
        console.log(`[Worker] OCR and page rasterization disabled for ${input.name} (page-count safety limit)`);
      } else if (!ocrRuntime) {
        // `selectOcrBridge(null)` below returns `undefined`, i.e. "no backend injected" —
        // xberg-wasm has no regex-style fallback for that (unlike NER), so scanned pages
        // and embedded images silently produce zero text unless we say something here.
        console.warn(`[Worker] OCR backend unavailable for ${input.name} — scanned/image content will export with no text.`);
        self.postMessage({
          type: "warning",
          file: input.name,
          message: "Échec du chargement du moteur OCR — les pages scannées ou images de ce fichier seront exportées sans texte extrait.",
        });
      }

      const nerConfig = WasmNerConfig.default();
      nerConfig.backend = WasmNerBackendKind.Onnx;
      nerConfig.categories = config.nerCategories;
      // Additive, not a replacement — matches hacienda-core's "extend, don't replace"
      // vertical behaviour (`ner.rs`'s `categories_with_vertical`). Empty by default,
      // since `nerCategories`'s closed vocabulary already covers the opt-out cases and
      // the engine rejects the whole NER result for a category name outside it — see
      // `ConfigPanel.tsx`'s `ALL_CATEGORIES` comment. `customLabels` is the sanctioned
      // open-vocabulary path for anything beyond that fixed set.
      nerConfig.customLabels = config.nerCustomLabels;
      extractConfig.ner = nerConfig;

      const engine = new XbergEngine(
        { bridgeTimeoutMs: 30000 },
        { ner: { ner: selectNerBridge(nerRuntime) }, ocr: { ocr: selectOcrBridge(ocrRuntime) } },
      );

      console.log(`[Worker] Calling engine.extract for ${input.name}...`);
      const extractStart = performance.now();
      const result = await engine.extract(extractInput, extractConfig);
      const extractMs = performance.now() - extractStart;
      console.log(`[Worker] engine.extract completed in ${extractMs.toFixed(0)}ms for ${input.name}`);
      postProgress({ file: input.name, stage: "extract", percent: 50 });

      if (!result.results[0]?.content) {
        console.error(`[Worker] No content extracted from ${input.name}. Result:`, result);
        throw new Error("No content extracted");
      }

      markdown = result.results[0].content;
      console.log(`[Worker] Extracted ${markdown.length} chars from ${input.name}`);
    } catch (extractError) {
      logExtractionError(input.name, extractError);
      throw extractError;
    }
  }

  postProgress({ file: input.name, stage: "ner", percent: 60 });

  // Run NER on the markdown (works for both transcription and extraction)
  console.log(`[Worker] Running NER on ${input.name}...`);
  let nerResults: unknown[] = [];
  try {
    const nerEngine = new XbergEngine(
      { bridgeTimeoutMs: 30000 },
      { ner: { ner: selectNerBridge(nerRuntime) } },
    );

    // `NerModel.detect`/`XbergEngine.ner`'s `opts.categories` is a single array where
    // unknown names become zero-shot custom labels — unlike `WasmNerConfig` (the
    // `engine.extract()` path above), there's no separate `customLabels` field here, so
    // `nerCustomLabels` must be merged in rather than dropped for this pass to detect
    // Comprehensive PII labels too.
    nerResults = await nerEngine.ner(markdown, {
      categories: [...config.nerCategories, ...config.nerCustomLabels],
    });
    console.log(
      "[Worker] Engine NER results:",
      JSON.stringify(nerResults, null, 2),
    );
  } catch (nerError) {
    console.error(`[Worker] NER FAILED for ${input.name}, continuing without NER:`, nerError);
    if (nerError instanceof Error) {
      console.error("[Worker] NER error:", nerError.name, nerError.message);
    }
    // Don't throw — continue processing with empty NER results so the file
    // still gets PII detection, redaction, and zip export. But say so: without this,
    // a document with zero entities because NER failed is indistinguishable from one
    // that genuinely has none.
    self.postMessage({
      type: "warning",
      file: input.name,
      message: "Échec de la reconnaissance d'entités (NER) pour ce fichier — il sera exporté sans glossaire d'entités.",
    });
  }

  const xbergEntities = nerResults || [];
  console.log(
    "[Worker] Raw entities from extraction:",
    JSON.stringify(xbergEntities, null, 2),
  );

  const entities: Entity[] = [];
  for (const e of xbergEntities) {
    if (!isRawNerEntity(e)) {
      console.warn(`[Worker] Skipping malformed NER entity for ${input.name}:`, e);
      continue;
    }
    // `e.category` is a runtime string from an external WASM bridge, not
    // necessarily a valid `NerCategory` — compare as plain strings rather than
    // asserting it into that narrower type.
    if (!(config.nerCategories as string[]).includes(e.category.toLowerCase())) continue;
    entities.push({
      name: e.text,
      type: e.category.toLowerCase(),
      slug: slugify(e.text),
      count: 1,
      spans: [{ start: e.start, end: e.end }],
    });
  }

  postProgress({ file: input.name, stage: "ner", percent: 80 });

  // Track A1/A2, redirected to the Rust/wasm engine per Track L6: `enablePiiDetection`
  // and `redactPiiInOutput` used to be dead config (nothing read them — see
  // `lib/ConfigPanel.tsx`'s "PII & Compliance" section). `scanForPii`/`redactPii`
  // run the same regex engine `cargo test`'s PII suite asserts against, compiled to
  // wasm32 (`crates/hacienda-wasm`), not a second TypeScript implementation.
  //
  // Runs on the original `markdown`, before entities are enriched/registered/linked,
  // so both the export filter below and `renderAnnotatedMarkdown`'s overlap check
  // (Track F4/L7) work off the same coordinates.
  //
  // Tier 1.1 optimization: reuse NER results from entity glossary pass instead of
  // running inference a second time. Convert xberg BridgeEntity[] to PII model entities.
  let piiEntitiesFound = 0;
  let piiFindings: PiiEntity[] = [];
  // Set alongside `piiResult` below, only when `config.redactPiiInOutput` actually ran a
  // redaction — `recordPiiAudit` is called with this once the mode-dispatch switch below
  // has decided which mode was *actually* applied (not merely configured; pseudonymize
  // falls back to mask when no key was derived). `null` means "nothing to record" (PII
  // detection is off, this call was scan-only, or detection failed).
  let piiResult: PiiPipelineResult | null = null;
  if (config.enablePiiDetection) {
    console.log(`[Worker] Running PII detection on ${input.name}...`);
    postProgress({ file: input.name, stage: "pii", percent: 82 });
    try {
      const piiStart = performance.now();

      // Convert entity glossary NER results (xbergEntities) to PII model entity format
      // This eliminates the duplicate GLiNER2 inference pass (Tier 1.1)
      const modelEntities = xbergEntities
        .filter((e): e is RawNerEntity => isRawNerEntity(e))
        .filter((e) => (config.nerCategories as string[]).includes(e.category.toLowerCase()))
        .map((e) => ({
          category: nerCategoryToPiiCategory(e.category),
          text: e.text,
          start: e.start,
          end: e.end,
          confidence: 1.0, // xberg entities don't always have confidence, default to 1.0
        }));

      const result = config.redactPiiInOutput
        ? await redactPiiWithModelEntities(markdown, modelEntities)
        : await scanForPiiWithModelEntities(markdown, modelEntities);
      if (config.redactPiiInOutput) piiResult = result;

      const piiMs = performance.now() - piiStart;
      console.log(`[Worker] PII detection completed in ${piiMs.toFixed(0)}ms for ${input.name}`);
      piiFindings = result.entities;
      piiEntitiesFound = piiFindings.length;
      console.log(`[Worker] Found ${piiEntitiesFound} PII entities in ${input.name}`);
    } catch (piiError) {
      console.error(`[Worker] PII DETECTION FAILED for ${input.name}:`, piiError);
      console.error("[Worker] PII error type:", typeof piiError);
      console.error("[Worker] PII error constructor:", piiError?.constructor?.name);
      if (piiError instanceof Error) {
        console.error("[Worker] PII error name:", piiError.name);
        console.error("[Worker] PII error message:", piiError.message);
        console.error("[Worker] PII error stack:", piiError.stack);
      } else {
        console.error("[Worker] Raw PII error:", JSON.stringify(piiError));
      }
      throw piiError;
    }

    // Track F1/F2 (extended to all 4 modes): each `redactionMode` replaces the finding's
    // `redact_template` — the string `renderAnnotatedMarkdown` splices into the body — with
    // something other than the wasm engine's default mask-shaped output. Mask needs no
    // branch here: `process()`/`process_with_model_entities` already produced a mask-shaped
    // `redact_template`, so there is nothing to overwrite.
    //
    // `f.text` is *not* what gets hashed/pseudonymized: `MergedEntity.text` (hacienda-core's
    // `pii/merge.rs`) is documented "Empty for regex detections, which carry offsets only"
    // — regex is the only detector active in Studio's default config, so `f.text` is empty
    // for essentially every real finding. `markdown.slice(f.start, f.end)` recovers the
    // actual matched text from the same offsets `renderAnnotatedMarkdown` already treats as
    // JS string indices (Track F4) — consistent with the rest of this pipeline's offset
    // handling, not a new assumption introduced here.
    if (config.redactPiiInOutput) {
      // What actually happened to each span, as opposed to what `config.redactionMode`
      // merely asked for — the two diverge exactly when pseudonymize falls back to mask
      // below. This is what gets recorded into the audit chain, not the requested mode:
      // an audit trail that reports what was configured rather than what was applied
      // isn't trustworthy for the one mode where the two can disagree.
      let appliedMode: "mask" | "hash" | "pseudonymize" | "remove" = "mask";
      switch (config.redactionMode) {
        case "pseudonymize":
          // `pseudonymKeyHex` is `null` whenever no passphrase was given — falls back to
          // the mask-shaped template already on each finding, same as before this mode
          // existed. Every finding shares one key: minting is per-entity, but the key
          // derivation happened once for the whole batch in `processFiles`.
          if (pseudonymKeyHex) {
            piiFindings = await Promise.all(
              piiFindings.map(async (f) => ({
                ...f,
                redact_template: await mintToken(
                  f.category,
                  markdown.slice(f.start, f.end),
                  config.pseudonymKeyId,
                  pseudonymKeyHex,
                ),
              })),
            );
            appliedMode = "pseudonymize";
          }
          break;
        case "hash":
          // Keyless hashing is not a redaction (see `hashSpanForProcessing`), so with no
          // derived key this falls through to the mask-shaped template the engine already
          // produced — the same fallback pseudonymize takes, and warned about once per
          // batch in `processFiles`. `appliedMode` stays "mask" in that case, matching what
          // actually happened, not what was configured — the whole point of this switch.
          if (pseudonymKeyHex) {
            piiFindings = await Promise.all(
              piiFindings.map(async (f) => ({
                ...f,
                redact_template: await hashSpanForProcessing(
                  f.category,
                  markdown.slice(f.start, f.end),
                  pseudonymKeyHex,
                ),
              })),
            );
            appliedMode = "hash";
          }
          break;
        case "remove":
          piiFindings = piiFindings.map((f) => ({ ...f, redact_template: "" }));
          appliedMode = "remove";
          break;
        case "mask":
        default:
          break;
      }

      // Records the batch of redactions `redactPiiWithModelEntities` produced above —
      // deferred until here (rather than happening inside that call, as it used to)
      // because only now is `appliedMode` known. See `recordPiiAudit`'s doc for why the
      // wasm call's own `audit_log` cannot be trusted for anything but mask.
      if (piiResult) {
        await recordPiiAudit(piiResult, appliedMode);
      }
    }
  }

  const exportableEntities = await filterExportableEntities(
    entities,
    piiFindings,
    config.redactPiiInOutput,
  );

  // Classify the document once — the fallback below depends only on the
  // document text, so it does not need recomputing for every entity.
  const documentVertical = classifyDocumentVertical(markdown);

  // Enrich entities with vertical metadata and register them
  const enrichedEntities: Entity[] = [];
  for (const entity of exportableEntities) {
    // Determine vertical based on entity type, falling back to document context
    const verticalMeta = verticalDict.lookup(entity.name.toLowerCase()) ?? {
      canonical: `${documentVertical}_entity`,
      vertical: documentVertical,
      roles: [],
    };

    const enrichedEntity: Entity = {
      ...entity,
      vertical: verticalMeta.vertical,
      sector: verticalMeta.sector,
      roles: verticalMeta.roles || [],
    };

    // Register entity in batch registry
    const registered = await registry.addEntity(
      {
        name: enrichedEntity.name,
        type: enrichedEntity.type,
        slug: enrichedEntity.slug,
        count: enrichedEntity.count,
        spans: enrichedEntity.spans,
      },
      {
        vertical: enrichedEntity.vertical || "shared",
        sector: enrichedEntity.sector,
        roles: enrichedEntity.roles,
      },
      docId,
    );
    // Task 4 (spec §8 step 4): the registry derives a stable, content-hashed slug —
    // shared by every spelling variant of the same entity — instead of trusting the
    // NER-time `slugify(e.text)` this entity was created with above. Copying it back
    // here keeps this document's own links (`renderAnnotatedMarkdown`/`buildGlossary`
    // below, both reading `deduped`'s `.slug` via `relativeEntityLink`) pointing at the
    // exact filename `entities/*.md` actually uses; without this, a link computed from
    // the pre-hash slug would point at a file the registry never writes.
    enrichedEntity.slug = registered.slug;
    enrichedEntities.push(enrichedEntity);
  }

  const deduped = deduplicateEntities(enrichedEntities);

  const linkedMarkdown = renderAnnotatedMarkdown(
    markdown,
    deduped,
    config.redactPiiInOutput ? piiFindings : [],
    docPath,
  );

  const frontmatter = buildFrontmatter(input, deduped, piiEntitiesFound, documentVertical);
  const glossary = buildGlossary(deduped, docPath);

  const finalMarkdown = frontmatter + "\n" + linkedMarkdown + glossary;

  postProgress({ file: input.name, stage: "complete", percent: 100 });

  console.log(`[Worker] processFile DONE: ${input.name} → ${input.name.replace(/\.[^.]+$/, ".md")}, ${deduped.length} entities, ${piiEntitiesFound} PII`);

  return {
    name: input.name.replace(/\.[^.]+$/, ".md"),
    markdown: finalMarkdown,
    rawMarkdown: markdown,
    entities: deduped,
    piiFindings,
    frontmatter: {
      source: input.name,
      type: input.type.split("/")[1] || "unknown",
      processed: new Date().toISOString(),
      piiEntitiesFound,
      entities: deduped.map((e) => ({
        name: e.name,
        type: e.type.charAt(0).toUpperCase() + e.type.slice(1),
        slug: e.slug,
        vertical: e.vertical,
        sector: e.sector,
        roles: e.roles,
      })),
    },
  };
}

async function processFiles(
  files: FileInput[],
  config: AppConfig,
): Promise<void> {
  console.log("[Worker] processFiles STARTED for", files.length, "files");

  // Drop the previous batch before this one starts. `lastBatch` is only reassigned
  // once this whole run completes (see the bottom of this function) — if it were left
  // set here and this run then failed or threw partway through, a later "build-zip"
  // request would still export the *previous* run's documents/manifest/registry as if
  // they belonged to the batch the user just saw fail.
  lastBatch = null;

  // Initialize vertical dictionary and registry
  //
  // Track D1 found `config.enabledVerticals` was never read here — every
  // taxonomy loaded regardless of what the "Vertical NER" checkboxes in
  // ConfigPanel.tsx said, another dead toggle in the same family A1-A4
  // fixed. An empty selection is not an error case: it means no taxonomy
  // vocabulary is consulted, so every entity falls through to
  // classifyDocumentVertical's document-level fallback below.
  let verticalDict: VerticalDictionary;
  try {
    const taxonomies = await Promise.all(
      config.enabledVerticals.map((v) => loadVerticalTaxonomy(v)),
    );
    verticalDict = new VerticalDictionary(taxonomies);
  } catch (taxonomyErr) {
    console.error("[Worker] Taxonomy loading failed, continuing without verticals:", taxonomyErr);
    verticalDict = new VerticalDictionary([]);
    // Every entity in this batch will fall through to classifyDocumentVertical's
    // document-level fallback instead of the taxonomy's actual vertical metadata — say
    // so, rather than exporting a knowledge graph that's silently missing that metadata.
    self.postMessage({
      type: "warning",
      message: "Échec du chargement de la taxonomie verticale — les entités exportées utiliseront un vertical plus large, au niveau du document, plutôt que des correspondances de taxonomie.",
    });
  }
  const registry = new BatchEntityRegistry();

  // Transcription (Track D3, superseded by the main-thread migration `transcribe-bridge.ts`'s
  // header describes): no per-batch setup happens here anymore. This module holds no
  // `WhisperBridge` instance at all — `processFile`'s transcription branch calls
  // `transcriptionBridge.request()`, which asks `App.tsx`'s own long-lived `WhisperBridge`
  // (main thread only — see that class's header for why) to do the work and awaits the
  // reply. That instance's `load()` is idempotent per model size and outlives this function
  // (it is a ref in `App.tsx`, not constructed per batch), so "don't redownload/reinitialize
  // the model per file" still holds — main-thread instance reuse provides it now, instead of
  // the upfront preload this comment used to describe. That preload also used to run
  // whenever `enableTranscription` was on even if this batch had zero audio/video files;
  // requesting lazily, only when a file actually needs it, fixes that for free. Each file's
  // own request surfaces its own failure through the normal per-file catch in the loop below
  // (Track D3's isolation guarantee, unchanged), whether that failure is a real transcription
  // error or `transcriptionBridge.request()`'s own timeout guarding against a lost reply.

  // Track F1/F2: derived once for the whole batch — PBKDF2 is deliberately expensive
  // (600,000 iterations), so this must not run per file or per finding. `null` leaves
  // every `processFile` call in the existing mask-mode behavior unchanged.
  let pseudonymKeyHex: string | null = null;
  if (
    config.enablePiiDetection &&
    config.redactPiiInOutput &&
    // Hash needs the key too, not just pseudonymize: `hashSpanForProcessing` is an HMAC,
    // because an unsalted digest of a low-entropy value (a name, a 9-digit SSN) is
    // recovered by enumeration and so is not a redaction at all. See that function's doc
    // for why a fixed or per-document salt cannot substitute for a secret key here.
    (config.redactionMode === "pseudonymize" || config.redactionMode === "hash") &&
    config.pseudonymPassphrase
  ) {
    try {
      pseudonymKeyHex = await deriveKeyHex(config.pseudonymPassphrase, config.pseudonymKeyId);
    } catch (keyErr) {
      console.error("[Worker] Key derivation failed, falling back to mask mode:", keyErr);
      // `pseudonymKeyHex` stays null, so every `processFile` call below silently uses
      // the mask-mode branch even though the user asked for reversible pseudonymize
      // tokens — tell them, rather than letting the export look like it honored their
      // choice. (`_manifest.json`/zip-export.ts's manifest does not record
      // `redactionMode` at all, so there is no stale "pseudonymize" label to correct.)
      self.postMessage({
        type: "warning",
        message: "Échec de la dérivation de la clé de pseudonymisation — ce lot utilisera le mode masquage plutôt que des jetons réversibles.",
      });
    }
  }

  // Hash mode with no passphrase would otherwise fall through to the mask-shaped template
  // the engine already produced, silently — the user asked for hashed output and got
  // masked output with nothing said. Same contract as the pseudonymize fallback above.
  if (
    config.enablePiiDetection &&
    config.redactPiiInOutput &&
    config.redactionMode === "hash" &&
    !pseudonymKeyHex
  ) {
    self.postMessage({
      type: "warning",
      message:
        "Le mode hachage nécessite une phrase secrète pour dériver son empreinte — sans elle, les hachages seraient réversibles par force brute. Ce lot utilisera le mode masquage à la place.",
    });
  }

  const results: ProcessedFile[] = [];
  // Track I2's backlinks: `RegistryEntity.source_documents` only holds
  // docIds, not the zip-relative path a link needs to point at. Populated
  // alongside `results` below, from the same `processed.name` the zip
  // entries themselves use, so the two can never disagree.
  const docPaths = new Map<string, string>();
  // Task 4: `assignDocId` checked against here, not `docPaths` — `docPaths` is only
  // populated once `processFile` returns, too late to guard the file currently in flight.
  const usedDocIds = new Set<string>();

  for (let i = 0; i < files.length; i++) {
    const file = files[i];
    try {
      console.log(
        `[Worker] === FILE ${i + 1}/${files.length} ===`,
        file.name,
        file.type,
        file.bytes.byteLength,
        "bytes",
      );
      const docId = await assignDocId(file.bytes, usedDocIds);
      const startTime = performance.now();
      const processed = await processFile(
        file,
        config,
        verticalDict,
        registry,
        docId,
        pseudonymKeyHex,
      );
      const elapsed = performance.now() - startTime;
      console.log(
        `[Worker] ✓ FILE ${i + 1}/${files.length} COMPLETE:`,
        file.name,
        `(${elapsed.toFixed(0)}ms)`,
        `entities:${processed.entities.length}`,
        `pii:${processed.piiFindings.length}`,
      );
      // Infer relationships for this document. `rawMarkdown` — not `markdown` — is what the
      // registry's stored spans are offsets into (`processFile` returns `rawMarkdown: markdown`,
      // the text NER ran on, before link/redaction splicing shifts every position). Passing the
      // spliced `markdown` would misclassify proximity silently.
      registry.inferRelationships(docId, processed.rawMarkdown);
      docPaths.set(docId, "documents/" + processed.name);
      results.push(processed);
      self.postMessage({ type: "file-complete", ...processed });
    } catch (error) {
      console.error(`[Worker] ✗ FILE ${i + 1}/${files.length} FAILED:`, file.name, error);
      console.error("[Worker] Error type:", typeof error);
      console.error("[Worker] Error constructor:", error?.constructor?.name);
      if (error instanceof Error) {
        console.error("[Worker] Error name:", error.name);
        console.error("[Worker] Error message:", error.message);
        console.error("[Worker] Error stack:", error.stack);
      } else {
        console.error("[Worker] Raw error value:", JSON.stringify(error));
      }
      // Try to extract a meaningful message from any error type
      let errorMessage = "Unknown error";
      if (error instanceof Error) {
        errorMessage = error.message;
      } else if (typeof error === "string") {
        errorMessage = error;
      } else if (error && typeof error === "object") {
        errorMessage = JSON.stringify(error);
      }
      self.postMessage({
        type: "error",
        file: file.name,
        message: errorMessage,
      });
    }
  }
  console.log("[Worker] All files processed");

  // Task 5.2 (spec §8 step 5): document-to-document relatedness needs every file's
  // entities registered, which is only true once the whole loop above has finished —
  // this cannot run per-document inside it. `docPaths.keys()`, not a separately-tracked
  // id list: a failed file's docId (if `assignDocId` ran before it failed) never reaches
  // `docPaths.set`, correctly excluding it from relatedness the same way it's already
  // excluded from the registry. Spliced onto `result.markdown` *after* `buildGlossary`'s
  // "## Entities" section is already baked in — see `buildRelatedDocumentsSection`'s doc
  // comment for why that ordering, not this one, is what keeps `export-resolve.ts`'s
  // redaction-edit re-export working with zero changes there.
  const allDocIds = Array.from(docPaths.keys());
  const related = computeRelatedDocuments(registry.getEntities(), allDocIds);
  const docIdByPath = new Map(
    Array.from(docPaths.entries()).map(([docId, path]) => [path, docId]),
  );
  for (const result of results) {
    const ownPath = "documents/" + result.name;
    const docId = docIdByPath.get(ownPath);
    if (!docId) continue;
    const topRelated = topRelatedDocuments(related.get(docId) ?? []);
    if (topRelated.length > 0) {
      result.markdown += buildRelatedDocumentsSection(topRelated, ownPath, docPaths);
    }
  }

  // Track K/Phase 2: the zip is no longer built here — retained so a later on-demand
  // "build-zip" message (see self.onmessage below) can call assembleZip() without
  // re-deriving the registry/docPaths/config that only this function's scope has.
  // batch-complete is now a pure queue->browser signal, no zip field.
  lastBatch = { results, registry, docPaths, config };
  self.postMessage({ type: "batch-complete" });
}

self.onmessage = async (event: MessageEvent) => {
  const { type, files, config } = event.data;
  console.log("[Worker] Received message:", type, files?.length);

  // `App.tsx`'s reply to a `transcribe-request` this worker sent via `transcriptionBridge`
  // (see that module's header). Handled before the `init`/`process` branches below and
  // returns immediately after: a batch can be mid-flight when this arrives (it is, in fact,
  // the common case — `processFile` is `await`ing exactly this), and it must not wait its
  // turn behind whatever `process`/`init` handling happens to be in progress.
  if (type === "transcribe-response") {
    const { requestId, result, error } = event.data;
    if (error) {
      transcriptionBridge.reject(requestId, error);
    } else {
      transcriptionBridge.resolve(requestId, result);
    }
    return;
  }

  if (type === "init") {
    try {
      wasmReady = initEngine();
      await wasmReady;
    } catch (err) {
      console.error("[Worker] initEngine failed:", err);
      wasmReady = Promise.resolve();
    }
    self.postMessage({ type: "ready" });
    return;
  }

  if (type === "process") {
    console.log("[Worker] Processing files...");
    try {
      if (wasmReady) await wasmReady;
      console.log("[Worker] About to call processFiles");
      await processFiles(files, config);
      console.log("[Worker] processFiles returned");
    } catch (err) {
      console.error("[Worker] processFiles crashed:", err);
      // Without this, the UI's only signal was an empty file browser with no
      // explanation — batch-complete alone tells the main thread the queue is over,
      // not that it ended in failure. Post an error first so the existing error
      // banner (App.tsx's "error" case) can say why.
      self.postMessage({
        type: "error",
        file: "batch",
        message: err instanceof Error ? err.message : "Batch processing failed.",
      });
      // Always fire batch-complete so the UI transitions to the browser
      // screen rather than staying stuck on the queue forever.
      self.postMessage({ type: "batch-complete" });
    }
    return;
  }

  if (type === "build-zip") {
    if (!lastBatch) {
      console.error("[Worker] build-zip requested with no completed batch");
      return;
    }
    console.log("[Worker] Building zip on demand...");
    try {
      const zip = await assembleZip(lastBatch, event.data.overrides);
      console.log("[Worker] Zip built, sending zip-ready");
      self.postMessage({ type: "zip-ready", zip });
    } catch (zipError) {
      console.error("[Worker] Zip build failed:", zipError);
      self.postMessage({
        type: "error",
        file: "zip",
        message: zipError instanceof Error ? zipError.message : String(zipError),
      });
    }
  }
};
