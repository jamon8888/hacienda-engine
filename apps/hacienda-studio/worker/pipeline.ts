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
} from "@xberg-io/xberg-wasm";
import initWasm from "@xberg-io/xberg-wasm";
// Let the bundler resolve and emit the binary. The previous absolute
// "/node_modules/@xberg-io/…/xberg_wasm_bg.wasm" only ever existed on a dev
// server: in production that path is answered by the SPA fallback with
// index.html, and WebAssembly rejects it as "expected magic word 00 61 73 6d,
// found 3c 21 64 6f".
import xbergWasmUrl from "@xberg-io/xberg-wasm/pkg/web/xberg_wasm_bg.wasm?url";
import { extractEntities, type BridgeEntity } from "../lib/ner-bridge";
import { createNerBackend, loadNerModel } from "../lib/asset-loader";
import {
  initPiiEngine,
  redactPii,
  scanForPii,
  type PiiEntity,
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
  entityFileName,
  relativeEntityLink,
  relativeDocLink,
  renderAnnotatedMarkdown,
} from "../lib/annotate";

// Track I4: re-exported unchanged so nothing importing these from "./pipeline" (the
// vitest suite included) needs to know they now live in lib/annotate.ts — see that
// file's header for why the split exists (App.tsx needs them without this module's
// top-level `self.onmessage =`).
export { relativeEntityLink, renderAnnotatedMarkdown };

let wasmReady: Promise<void> | null = null;

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
  return async (text, categories) =>
    (await runtime.detect(text, { categories })) as BridgeEntity[];
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
  return Array.from(map.values()).sort((a, b) => b.count - a.count);
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

/**
 * Track G3: without this, a Claude Desktop session that opens the zip sees a
 * pile of markdown files, a JSON registry and a `kg-export/` folder with no
 * explanation — nothing tells it the registry and KG files exist to answer
 * cross-document questions the prose alone can't (which entities appear in
 * multiple files, how they relate), so a session would only ever read the
 * prose. `fileCount`/`entityCount` are computed by the caller, which already
 * has `results`/`registry` in scope; this function only formats.
 */
function buildBundleReadme(fileCount: number, entityCount: number): string {
  const documents = fileCount === 1 ? "document" : "documents";
  const entities = entityCount === 1 ? "entity" : "entities";
  return `# Hacienda Studio export

This bundle was produced entirely in-browser (Hacienda Studio) — no document
left the device it was processed on. It contains ${fileCount} processed
${documents} and ${entityCount} distinct ${entities} across them.

## What's in here

- **\`documents/\`** — one markdown file per source document, at the same
  relative path it was uploaded from. Each has YAML frontmatter (source name,
  type, processing time, PII count) followed by the extracted content, with
  named entities linked to their file under \`entities/\`, and a local
  \`## Entities\` summary at the bottom.
- **\`entities/\`** — one file per distinct entity across the whole batch:
  type, vertical, roles, aliases, and a backlink to every document that
  mentions it. This is what makes the bundle RAG-ready rather than merely
  readable — open \`entities/organization-acme-sas.md\` and find every
  document naming Acme SAS, without reading every file in \`documents/\`.
- **\`GLOSSARY.md\`** — the index into \`entities/\`, grouped by type. Start
  here for "what entities does this bundle know about".
- **\`_manifest.json\`** — the file list for this batch, with per-file entity
  counts.
- **\`entities-registry.json\`** — every entity across the whole batch, with
  which document(s) it appears in and inferred relationships between
  entities. Use this, not just the prose, to answer questions that span more
  than one document — an entity mentioned in three files only has one row
  here, not three.
- **\`kg-export/\`** — the same registry as a knowledge graph, in three
  formats: \`neo4j.cypher\` (importable into Neo4j), \`networkx.json\`
  (Python's NetworkX), and \`rdf.ttl\` (RDF/Turtle). Prefer these over
  re-deriving relationships from the prose.

## Reading this bundle

For cross-document questions (shared entities, relationships between
documents), start from \`GLOSSARY.md\`, \`entities/\`, \`entities-registry.json\`
or \`kg-export/\`, not by reading every file in \`documents/\`. For a single
document's content, its own \`.md\` file is self-contained — frontmatter,
prose, and local entity summary together.
`;
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

/**
 * Track I2: "one file per entity, with backlinks." `docLinks` are already
 * `documents/...` paths (see `processFiles`'s `docPaths` map) — sorted by
 * the caller so file output is deterministic across runs.
 */
export function buildEntityFile(
  entity: RegistryEntity,
  docLinks: string[],
): string {
  const typeLabel = entity.type.charAt(0).toUpperCase() + entity.type.slice(1);
  const lines = [`# ${entity.display_name}`, "", `- **Type:** ${typeLabel}`];
  if (entity.vertical) lines.push(`- **Vertical:** ${entity.vertical}`);
  if (entity.sector) lines.push(`- **Sector:** ${entity.sector}`);
  if (entity.roles.length) lines.push(`- **Roles:** ${entity.roles.join(", ")}`);
  if (entity.aliases.length)
    lines.push(`- **Aliases:** ${entity.aliases.join(", ")}`);
  lines.push(
    `- **Mentions:** ${entity.mention_count} across ${docLinks.length} document${docLinks.length === 1 ? "" : "s"}`,
  );
  lines.push("", "## Appears in", "");
  for (const docPath of docLinks) {
    lines.push(`- [${docPath.replace(/^documents\//, "")}](${relativeDocLink(docPath)})`);
  }
  return lines.join("\n") + "\n";
}

/**
 * Track I2: "GLOSSARY.md is the entry point" — the global index into
 * `entities/`, grouped by type and sorted for deterministic output.
 */
export function buildGlossaryIndex(entities: RegistryEntity[]): string {
  if (entities.length === 0) {
    return "# Glossary\n\nNo entities were detected in this batch.\n";
  }
  const byType = new Map<string, RegistryEntity[]>();
  for (const e of entities) {
    const list = byType.get(e.type) ?? [];
    list.push(e);
    byType.set(e.type, list);
  }
  let md =
    "# Glossary\n\nEvery entity detected across this batch. Open an entry " +
    "for its full detail and backlinks into the documents that mention it.\n";
  for (const type of Array.from(byType.keys()).sort()) {
    const typeLabel = type.charAt(0).toUpperCase() + type.slice(1);
    md += `\n## ${typeLabel}\n\n`;
    const sorted = byType
      .get(type)!
      .sort((a, b) => a.display_name.localeCompare(b.display_name));
    for (const e of sorted) {
      const verticalInfo =
        e.vertical && e.vertical !== "shared" ? ` — ${e.vertical}` : "";
      const docCount = e.source_documents.length;
      md += `- [${e.display_name}](entities/${entityFileName(e)})${verticalInfo}, mentioned ${e.mention_count} time${e.mention_count > 1 ? "s" : ""} across ${docCount} document${docCount === 1 ? "" : "s"}\n`;
    }
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

    const nerConfig = WasmNerConfig.default();
    nerConfig.backend = WasmNerBackendKind.Onnx;
    nerConfig.categories = config.nerCategories;
    extractConfig.ner = nerConfig;

    const engine = new XbergEngine(
      { bridgeTimeoutMs: 30000 },
      { ner: { ner: selectNerBridge(nerRuntime) } },
    );

    const result = await engine.extract(extractInput, extractConfig);
    postProgress({ file: input.name, stage: "extract", percent: 50 });

    if (!result.results[0]?.content) {
      throw new Error("No content extracted");
    }

    markdown = result.results[0].content;
  }

  postProgress({ file: input.name, stage: "ner", percent: 60 });

  // Run NER on the markdown (works for both transcription and extraction)
  const nerEngine = new XbergEngine(
    { bridgeTimeoutMs: 30000 },
    { ner: { ner: selectNerBridge(nerRuntime) } },
  );

  const nerResults = await nerEngine.ner(markdown, {
    categories: config.nerCategories,
  });
  console.log(
    "[Worker] Engine NER results:",
    JSON.stringify(nerResults, null, 2),
  );

  const xbergEntities = nerResults || [];
  console.log(
    "[Worker] Raw entities from extraction:",
    JSON.stringify(xbergEntities, null, 2),
  );

  const entities: Entity[] = [];
  for (const e of xbergEntities) {
    if (!config.nerCategories.includes(e.category.toLowerCase())) continue;
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
  let piiEntitiesFound = 0;
  let piiFindings: PiiEntity[] = [];
  if (config.enablePiiDetection) {
    postProgress({ file: input.name, stage: "pii", percent: 82 });
    const piiResult = config.redactPiiInOutput
      ? await redactPii(markdown)
      : await scanForPii(markdown);
    piiFindings = piiResult.entities;
    piiEntitiesFound = piiFindings.length;

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

  // Initialize vertical dictionary and registry
  //
  // Track D1 found `config.enabledVerticals` was never read here — every
  // taxonomy loaded regardless of what the "Vertical NER" checkboxes in
  // ConfigPanel.tsx said, another dead toggle in the same family A1-A4
  // fixed. An empty selection is not an error case: it means no taxonomy
  // vocabulary is consulted, so every entity falls through to
  // classifyDocumentVertical's document-level fallback below.
  const taxonomies = await Promise.all(
    config.enabledVerticals.map((v) => loadVerticalTaxonomy(v)),
  );
  const verticalDict = new VerticalDictionary(taxonomies);
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
    pseudonymKeyHex = await deriveKeyHex(config.pseudonymPassphrase, config.pseudonymKeyId);
  }

  let docCounter = 0;
  const results: ProcessedFile[] = [];
  // Track I2's backlinks: `RegistryEntity.source_documents` only holds
  // docIds, not the zip-relative path a link needs to point at. Populated
  // alongside `results` below, from the same `processed.name` the zip
  // entries themselves use, so the two can never disagree.
  const docPaths = new Map<string, string>();

  for (const file of files) {
    try {
      console.log(
        "[Worker] processing:",
        file.name,
        file.type,
        file.bytes.byteLength,
      );
      const docId = `doc-${String(++docCounter).padStart(3, "0")}`;
      const processed = await processFile(
        file,
        config,
        verticalDict,
        registry,
        docId,
        pseudonymKeyHex,
      );
      // Infer relationships for this document
      registry.inferRelationships(docId);
      docPaths.set(docId, "documents/" + processed.name);
      results.push(processed);
      self.postMessage({ type: "file-complete", ...processed });
    } catch (error) {
      console.error("[Worker] error processing", file.name, error);
      self.postMessage({
        type: "error",
        file: file.name,
        message: error instanceof Error ? error.message : "Unknown error",
      });
    }
  }
  console.log("[Worker] All files processed, creating zip...");
  const JSZip = (await import("jszip")).default;
  const zip = new JSZip();
  for (const r of results) {
    zip.file("documents/" + r.name, r.markdown);
  }
  const manifest = {
    files: results.map((r) => ({
      name: r.name,
      entityCount: r.entities.length,
    })),
    generated: new Date().toISOString(),
  };
  zip.file("_manifest.json", JSON.stringify(manifest, null, 2));

  // Add entities registry to zip
  const registryJson = {
    ...registry.toJSON(),
    ...(config.enableTranscription && {
      transcription: {
        model: config.transcriptionModel,
        language: config.transcriptionLanguage,
        enabled: true,
      },
    }),
  };
  zip.file("entities-registry.json", JSON.stringify(registryJson, null, 2));
  zip.file(
    "README.md",
    buildBundleReadme(
      results.length,
      registryJson.entity_registry.entities.length,
    ),
  );

  // Track I2: one file per entity with backlinks, plus the global index.
  const entitiesFolder = zip.folder("entities");
  for (const entity of registry.getEntities()) {
    const docLinks = entity.source_documents
      .map((docId) => docPaths.get(docId))
      .filter((p): p is string => !!p)
      .sort();
    entitiesFolder?.file(
      entityFileName(entity),
      buildEntityFile(entity, docLinks),
    );
  }
  zip.file("GLOSSARY.md", buildGlossaryIndex(registry.getEntities()));

  // Add KG exports
  const kgExporter = new KGExporter(registry);
  const kgFolder = zip.folder("kg-export");
  kgFolder?.file("neo4j.cypher", kgExporter.toCypher());
  kgFolder?.file(
    "networkx.json",
    JSON.stringify(kgExporter.toNetworkX(), null, 2),
  );
  kgFolder?.file("rdf.ttl", kgExporter.toRDF());

  const blob = await zip.generateAsync({ type: "blob" });
  console.log("[Worker] Zip created, sending batch-complete");
  self.postMessage({ type: "batch-complete", zip: blob });
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
    wasmReady = initEngine();
    await wasmReady;
    self.postMessage({ type: "ready" });
    return;
  }

  if (type === "process") {
    console.log("[Worker] Processing files...");
    if (wasmReady) await wasmReady;
    console.log("[Worker] About to call processFiles");
    await processFiles(files, config);
    console.log("[Worker] processFiles returned");
  }
};
