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
import { extractEntities } from "../lib/ner-bridge";
import { initPiiEngine, redactPii, scanForPii } from "../lib/pii-engine";
import { VerticalDictionary } from "../lib/verticals/dictionary";
import {
  loadVerticalTaxonomy,
  VerticalEntityMetadata,
} from "../lib/verticals/index";
import { BatchEntityRegistry } from "../lib/registry";
import { KGExporter } from "../lib/kg-export";
import { WhisperBridge } from "../lib/transcription/whisper-bridge";
import type { TranscriptionResult } from "../lib/transcription/types";

let wasmReady: Promise<void> | null = null;

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

function linkEntities(markdown: string, entities: Entity[]): string {
  let result = markdown;
  const allSpans = entities.flatMap((e) =>
    e.spans.map((s) => ({ ...s, entity: e })),
  );
  allSpans.sort((a, b) => b.start - a.start);

  for (const span of allSpans) {
    const before = result.slice(0, span.start);
    const matched = result.slice(span.start, span.end);
    const after = result.slice(span.end);
    const link = `[${matched}](entity:${span.entity.type}/${span.entity.slug})`;
    result = before + link + after;
  }
  return result;
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

function buildGlossary(entities: Entity[]): string {
  if (entities.length === 0) return "";
  let md = "\n## Entities\n\n";
  for (const e of entities) {
    const verticalInfo =
      e.vertical && e.vertical !== "shared" ? ` [${e.vertical}]` : "";
    md += `- **${e.name}** \`${e.type.charAt(0).toUpperCase() + e.type.slice(1)}${verticalInfo}\` — mentioned ${e.count} time${e.count > 1 ? "s" : ""}\n`;
  }
  return md;
}

async function initEngine(): Promise<void> {
  await Promise.all([
    initWasm({ module_or_path: fetch(xbergWasmUrl) }),
    initPiiEngine(),
  ]);
}

async function processFile(
  input: FileInput,
  config: AppConfig,
  verticalDict: VerticalDictionary,
  registry: BatchEntityRegistry,
  docId: string,
  whisperBridge: WhisperBridge,
): Promise<ProcessedFile> {
  postProgress({ file: input.name, stage: "extract", percent: 10 });

  const isAudio = input.type.startsWith("audio/");
  const isVideo = input.type.startsWith("video/");

  let markdown = "";
  let transcriptionResult: TranscriptionResult | null = null;

  if ((isAudio || isVideo) && config.enableTranscription) {
    console.log(`[Worker] Transcribing ${input.name}...`);
    transcriptionResult = await whisperBridge.transcribeAudio(
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
      { ner: { ner: extractEntities } },
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
    { ner: { ner: extractEntities } },
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

  // Classify the document once — the fallback below depends only on the
  // document text, so it does not need recomputing for every entity.
  const documentVertical = classifyDocumentVertical(markdown);

  // Enrich entities with vertical metadata and register them
  const enrichedEntities: Entity[] = [];
  for (const entity of entities) {
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

  postProgress({ file: input.name, stage: "ner", percent: 80 });

  const deduped = deduplicateEntities(enrichedEntities);
  let linkedMarkdown = linkEntities(markdown, deduped);

  // Track A1/A2, redirected to the Rust/wasm engine per Track L6: `enablePiiDetection`
  // and `redactPiiInOutput` used to be dead config (nothing read them — see
  // `lib/ConfigPanel.svelte`'s "PII & Compliance" section). `scanForPii`/`redactPii`
  // run the same regex engine `cargo test`'s PII suite asserts against, compiled to
  // wasm32 (`crates/hacienda-wasm`), not a second TypeScript implementation.
  //
  // Known rough edge, not fixed here: this redacts `linkedMarkdown`, i.e. *after*
  // `linkEntities` has already turned NER spans into `[text](entity:type/slug)` links.
  // A PII span that overlaps a link's visible text rewrites inside the link syntax too
  // (e.g. an IBAN digit run inside both a phone-entity link's text and its `entity:`
  // target), corrupting the link rather than just masking prose. Redacting before
  // linking instead would need entity offsets recomputed against the redacted text —
  // exactly Track F4's "offset problem", which Track L7 is scoped to investigate
  // (`WasmRedactionConfig.preserveOffsets` may already solve it on the Rust side). Do
  // not hand-roll an offset fix here; wait for L7.
  let piiEntitiesFound = 0;
  if (config.enablePiiDetection) {
    postProgress({ file: input.name, stage: "pii", percent: 85 });
    const piiResult = config.redactPiiInOutput
      ? await redactPii(linkedMarkdown)
      : await scanForPii(linkedMarkdown);
    piiEntitiesFound = piiResult.entities.length;
    if (config.redactPiiInOutput) {
      linkedMarkdown = piiResult.redacted_text;
    }
  }

  const frontmatter = buildFrontmatter(input, deduped, piiEntitiesFound);
  const glossary = buildGlossary(deduped);

  const finalMarkdown = frontmatter + "\n" + linkedMarkdown + glossary;

  postProgress({ file: input.name, stage: "complete", percent: 100 });

  return {
    name: input.name.replace(/\.[^.]+$/, ".md"),
    markdown: finalMarkdown,
    entities: deduped,
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
  const taxonomies = await Promise.all(
    ["m&a", "financial_services", "shared"].map((v) => loadVerticalTaxonomy(v)),
  );
  const verticalDict = new VerticalDictionary(taxonomies);
  const registry = new BatchEntityRegistry();

  // Initialize transcription bridge
  const whisperBridge = new WhisperBridge();
  if (config.enableTranscription) {
    await whisperBridge.load(config.transcriptionModel);
  }

  let docCounter = 0;
  const results: ProcessedFile[] = [];

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
        whisperBridge,
      );
      // Infer relationships for this document
      registry.inferRelationships(docId);
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
    zip.file(r.name, r.markdown);
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
