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
import {
  extractEntities,
  isBridgeEntityArray,
  type BridgeEntity,
} from "../lib/ner-bridge";
import { createNerBackend, loadNerModel } from "../lib/asset-loader";
import {
  initPiiEngine,
  redactPii,
  scanForPii,
  redactPiiWithModelEntities,
  scanForPiiWithModelEntities,
  type PiiEntity,
  type PiiCategoryWire,
} from "../lib/pii-engine";
import { deriveKeyHex, mintToken } from "../lib/pseudonymize";
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
    const { model, tokenizer, encoderConfig } = await loadNerModel();
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
 * touch the markdown, so there's nothing to defeat. Drops the whole entity if
 * *any* of its mentions overlaps a PII finding, not just the overlapping span —
 * under-including is the safe direction here.
 */
export function filterExportableEntities(
  entities: Entity[],
  piiFindings: PiiEntity[],
  redactPiiInOutput: boolean,
): Entity[] {
  if (!redactPiiInOutput) return entities;
  const overlapsPiiFinding = (span: { start: number; end: number }) =>
    piiFindings.some((p) => span.start < p.end && p.start < span.end);
  return entities.filter((e) => !e.spans.some(overlapsPiiFinding));
}

function buildFrontmatter(
  input: FileInput,
  entities: Entity[],
  piiEntitiesFound: number,
): string {
  const entityMeta = entities.map((e) => ({
    name: e.name,
    type: e.type.charAt(0).toUpperCase() + e.type.slice(1),
    slug: e.slug,
    ...(e.vertical && { vertical: e.vertical }),
    ...(e.sector && { sector: e.sector }),
    ...(e.roles && e.roles.length && { roles: e.roles }),
  }));

  const type = input.type.split("/")[1] || "unknown";
  return `---
source: ${input.name}
type: ${type}
processed: ${new Date().toISOString()}
pii_entities_found: ${piiEntitiesFound}
entities: ${JSON.stringify(entityMeta)}
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
      }

      const nerConfig = WasmNerConfig.default();
      nerConfig.backend = WasmNerBackendKind.Onnx;
      nerConfig.categories = config.nerCategories;
      extractConfig.ner = nerConfig;

      const engine = new XbergEngine(
        { bridgeTimeoutMs: 30000 },
        { ner: { ner: selectNerBridge(nerRuntime) } },
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
      console.error(`[Worker] EXTRACTION FAILED for ${input.name}:`, extractError);
      console.error("[Worker] Extraction error type:", typeof extractError);
      console.error("[Worker] Extraction error constructor:", extractError?.constructor?.name);
      if (extractError instanceof Error) {
        console.error("[Worker] Extraction error name:", extractError.name);
        console.error("[Worker] Extraction error message:", extractError.message);
        console.error("[Worker] Extraction error stack:", extractError.stack);
      } else {
        console.error("[Worker] Raw extraction error:", JSON.stringify(extractError));
      }
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

    nerResults = await nerEngine.ner(markdown, {
      categories: config.nerCategories,
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
      message: "Entity extraction (NER) failed for this file — it will export with no entity glossary.",
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

      const piiResult = config.redactPiiInOutput
        ? await redactPiiWithModelEntities(markdown, modelEntities)
        : await scanForPiiWithModelEntities(markdown, modelEntities);

      const piiMs = performance.now() - piiStart;
      console.log(`[Worker] PII detection completed in ${piiMs.toFixed(0)}ms for ${input.name}`);
      piiFindings = piiResult.entities;
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

    // Track F1/F2: `redactionMode: "pseudonymize"` replaces each finding's
    // `redact_template` — the string `renderAnnotatedMarkdown` splices into the body — with
    // a reversible token instead of the format-preserving mask. `pseudonymKeyHex` is `null`
    // whenever pseudonymization doesn't apply (mode is "mask", or no passphrase was given),
    // so this is a no-op in the default configuration. Every finding shares one key: minting
    // is per-entity, but the key derivation happened once for the whole batch in
    // `processFiles`, not per file or per finding.
    //
    // `f.text` is *not* what gets pseudonymized: `MergedEntity.text` (hacienda-core's
    // `pii/merge.rs`) is documented "Empty for regex detections, which carry offsets only"
    // — regex is the only detector active in Studio's default config, so `f.text` is empty
    // for essentially every real finding. `markdown.slice(f.start, f.end)` recovers the
    // actual matched text from the same offsets `renderAnnotatedMarkdown` already treats as
    // JS string indices (Track F4) — consistent with the rest of this pipeline's offset
    // handling, not a new assumption introduced here.
    if (config.redactPiiInOutput && pseudonymKeyHex) {
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
    }
  }

  const exportableEntities = filterExportableEntities(
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
    enrichedEntities.push(enrichedEntity);

    // Register entity in batch registry
    registry.addEntity(
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
  }

  const deduped = deduplicateEntities(enrichedEntities);

  const linkedMarkdown = renderAnnotatedMarkdown(
    markdown,
    deduped,
    config.redactPiiInOutput ? piiFindings : [],
    docPath,
  );

  const frontmatter = buildFrontmatter(input, deduped, piiEntitiesFound);
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
      message: "Vertical taxonomy failed to load — exported entities will use a coarser, document-level vertical instead of taxonomy matches.",
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
    config.redactionMode === "pseudonymize" &&
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
        message: "Pseudonymization key derivation failed — this batch will use mask mode instead of reversible tokens.",
      });
    }
  }

  let docCounter = 0;
  const results: ProcessedFile[] = [];
  // Track I2's backlinks: `RegistryEntity.source_documents` only holds
  // docIds, not the zip-relative path a link needs to point at. Populated
  // alongside `results` below, from the same `processed.name` the zip
  // entries themselves use, so the two can never disagree.
  const docPaths = new Map<string, string>();

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
      const docId = `doc-${String(++docCounter).padStart(3, "0")}`;
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
      // Infer relationships for this document
      registry.inferRelationships(docId);
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
