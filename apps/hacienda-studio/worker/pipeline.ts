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

function buildFrontmatter(input: FileInput, entities: Entity[]): string {
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
  const wasmUrl = new URL(
    "/node_modules/@xberg-io/xberg-wasm/pkg/web/xberg_wasm_bg.wasm",
    self.location.origin,
  );
  console.log("[Worker] WASM URL:", wasmUrl.href);
  const wasmResp = await fetch(wasmUrl);
  console.log(
    "[Worker] WASM fetch status:",
    wasmResp.status,
    "content-type:",
    wasmResp.headers.get("content-type"),
  );
  await initWasm({ module_or_path: wasmResp });
  console.log("[Worker] WASM engine initialized");
}

const createNerBridge = async (text: string, categories: string[]) => {
  const compromise = await import("compromise");
  const doc = compromise.default(text);
  const entities = [];

  if (categories.includes("person")) {
    doc
      .people()
      .out("array")
      .forEach((text: string) => {
        entities.push({
          label: "person",
          text,
          start: 0,
          end: text.length,
          score: 0.9,
        });
      });
  }
  if (categories.includes("organization")) {
    doc
      .organizations()
      .out("array")
      .forEach((text: string) => {
        entities.push({
          label: "organization",
          text,
          start: 0,
          end: text.length,
          score: 0.9,
        });
      });
  }
  if (categories.includes("location")) {
    doc
      .places()
      .out("array")
      .forEach((text: string) => {
        entities.push({
          label: "location",
          text,
          start: 0,
          end: text.length,
          score: 0.9,
        });
      });
  }
  if (categories.includes("email")) {
    const emailRegex = /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b/g;
    let match;
    while ((match = emailRegex.exec(text)) !== null) {
      entities.push({
        label: "email",
        text: match[0],
        start: match.index,
        end: match.index + match[0].length,
        score: 0.9,
      });
    }
  }
  if (categories.includes("phone_number")) {
    const phoneRegex =
      /(\+?\d{1,3}[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}/g;
    let match;
    while ((match = phoneRegex.exec(text)) !== null) {
      entities.push({
        label: "phone_number",
        text: match[0],
        start: match.index,
        end: match.index + match[0].length,
        score: 0.8,
      });
    }
  }
  if (categories.includes("date")) {
    doc
      .dates()
      .out("array")
      .forEach((text: string) => {
        entities.push({
          label: "date",
          text,
          start: 0,
          end: text.length,
          score: 0.8,
        });
      });
  }
  if (categories.includes("money")) {
    doc
      .money()
      .out("array")
      .forEach((text: string) => {
        entities.push({
          label: "money",
          text,
          start: 0,
          end: text.length,
          score: 0.8,
        });
      });
  }
  if (categories.includes("percent")) {
    doc
      .percentages()
      .out("array")
      .forEach((text: string) => {
        entities.push({
          label: "percent",
          text,
          start: 0,
          end: text.length,
          score: 0.8,
        });
      });
  }
  if (categories.includes("url")) {
    const urlRegex = /https?:\/\/[^\s]+/g;
    let match;
    while ((match = urlRegex.exec(text)) !== null) {
      entities.push({
        label: "url",
        text: match[0],
        start: match.index,
        end: match.index + match[0].length,
        score: 0.9,
      });
    }
  }

  return entities.map((e) => ({
    category: e.label,
    text: e.text,
    start: e.start,
    end: e.end,
    confidence: e.score,
  }));
};

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
      { ner: { ner: createNerBridge } },
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
    { ner: { ner: createNerBridge } },
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

  // Enrich entities with vertical metadata and register them
  const enrichedEntities: Entity[] = [];
  for (const entity of entities) {
    // Determine vertical based on entity type and document context
    let verticalMeta = verticalDict.lookup(entity.name.toLowerCase());

    // If not found in dictionary, classify based on entity type and document context
    if (!verticalMeta) {
      // Check if document contains M&A terms
      const text = markdown.toLowerCase();
      const hasMATerms =
        /\b(m&a|merger|acquisition|acquirer|acquired|acquires|target|spa|share purchase|earnout|indemnification|representation and warranty|material adverse change|break fee|closing condition|deal value|purchase price)\b/.test(
          markdown,
        );
      const hasFSTerms =
        /\b(private equity|venture capital|limited partner|general partner|carried interest|management fee|nav|irr|dpi|tvpi|portfolio company|limited partner|general partner|fund size|capital commitment)\b/.test(
          markdown,
        );

      if (hasMATerms) {
        verticalMeta = {
          canonical: "m&a_entity",
          vertical: "m&a",
          sector: "m&a",
          roles: [],
        };
      } else if (hasFSTerms) {
        verticalMeta = {
          canonical: "fs_entity",
          vertical: "financial_services",
          sector: "financial_services",
          roles: [],
        };
      } else {
        verticalMeta = {
          canonical: "shared_entity",
          vertical: "shared",
          sector: "shared",
          roles: [],
        };
      }
    }

    const enrichedEntity: Entity = {
      ...entity,
      vertical: verticalMeta?.vertical || "shared",
      sector: verticalMeta?.sector,
      roles: verticalMeta?.roles || [],
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
  const linkedMarkdown = linkEntities(markdown, deduped);
  const frontmatter = buildFrontmatter(input, deduped);
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
  const registryJson = registry.toJSON();
  if (config.enableTranscription) {
    registryJson.transcription = {
      model: config.transcriptionModel,
      language: config.transcriptionLanguage,
      enabled: true,
    };
  }
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
