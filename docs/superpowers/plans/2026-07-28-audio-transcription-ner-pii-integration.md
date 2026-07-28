# Audio/Video Transcription + NER/PII Integration Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add audio/video transcription with full NER/PII detection and vertical classification to hacienda-studio, seamlessly integrated with the existing document processing pipeline.

**Architecture:** Extend the existing Web Worker pipeline to handle audio/video files via @remotion/whisper-web, then feed transcripts through the existing NER + vertical classification + entity registry + KG export pipeline.

**Tech Stack:** Svelte 5, TypeScript, Vite, @remotion/whisper-web, @xberg-io/xberg-wasm, Compromise.js (NER bridge), JSZip.

## Global Constraints

- hacienda-studio runs in browser via WebAssembly (no server)
- Models cached in IndexedDB after first download
- COOP/COEP headers required for SharedArrayBuffer/WASM
- All processing in Web Worker (no main thread blocking)
- xberg-wasm version: rc.42 (current)
- Whisper models: tiny.en (75MB), tiny (75MB), base.en (142MB), base (142MB), small.en (466MB), small (466MB)
- EU languages priority: de, fr, es, it, pt, nl, pl, sv, da, fi, cs, hu, el, ro, bg, hr, sk, sl, et, lv, lt, mt, ga

---

## Phase 1: Core Infrastructure (Week 1)

### Task 1: Install @remotion/whisper-web Dependency

- [ ] **Step 1: Install package**

```bash
cd apps/hacienda-studio
npm install @remotion/whisper-web
```

- [ ] **Step 2: Update vite.config.ts**

```typescript
// Add to existing vite.config.ts
export default defineConfig({
  optimizeDeps: {
    exclude: ["@remotion/whisper-web"],
  },
  // ...
});
```

- [ ] **Step 3: Verify installation**

```bash
ls node_modules/@remotion/whisper-web/dist/
```

- [ ] **Step 4: Commit**

```bash
git add apps/hacienda-studio/package.json apps/hacienda-studio/package-lock.json apps/hacienda-studio/vite.config.ts
git commit -m "feat: add @remotion/whisper-web for browser transcription"
```

---

### Task 2: Create Transcription Bridge

**Files:**

- Create: `apps/hacienda-studio/lib/transcription/whisper-bridge.ts`
- Create: `apps/hacienda-studio/lib/transcription/types.ts`
- Create: `apps/hacienda-studio/lib/transcription/index.ts`

**Interfaces:**

- Consumes: None (first task)
- Produces: `WhisperBridge`, types

#### Task 2.1: Types

**Files:**

- Create: `apps/hacienda-studio/lib/transcription/types.ts`

- [ ] **Step 1: Create types.ts with all transcription types**

```typescript
// apps/hacienda-studio/lib/transcription/types.ts
export interface TranscriptionConfig {
  modelSize: "tiny.en" | "tiny" | "base.en" | "base" | "small.en" | "small";
  language?: string; // ISO 639-1 (de, fr, es, it, pt, nl, pl, etc.)
  task: "transcribe" | "translate";
  threads?: number;
}

export interface TranscriptionSegment {
  start: number;
  end: number;
  text: string;
  confidence?: number;
}

export interface TranscriptionResult {
  text: string;
  segments: TranscriptionSegment[];
  language: string;
  duration: number;
  metadata: AudioMetadata;
}

export interface AudioMetadata {
  durationMs: number;
  sampleRateHz: number;
  channels: number;
  codec: string;
}
```

- [ ] **Step 2: Run test to verify it compiles**

```bash
cd apps/hacienda-studio && npx tsc --noEmit
```

- [ ] **Step 3: Commit**

```bash
git add apps/hacienda-studio/lib/transcription/types.ts
git commit -m "feat(transcription): add core types"
```

#### Task 2.2: Whisper Bridge

**Files:**

- Create: `apps/hacienda-studio/lib/transcription/whisper-bridge.ts`

- [ ] **Step 1: Implement WhisperBridge**

```typescript
// apps/hacienda-studio/lib/transcription/whisper-bridge.ts
import {
  transcribe,
  canUseWhisperWeb,
  resampleTo16Khz,
  downloadWhisperModel,
} from "@remotion/whisper-web";
import type {
  TranscriptionConfig,
  TranscriptionResult,
  TranscriptionSegment,
} from "./types";

export class WhisperBridge {
  private loaded = false;
  private modelSize: string = "tiny.en";

  async load(
    modelSize:
      | "tiny.en"
      | "tiny"
      | "base.en"
      | "base"
      | "small.en"
      | "small" = "tiny.en",
  ) {
    if (this.loaded && this.modelSize === modelSize) return;

    const { supported, detailedReason } = await canUseWhisperWeb(modelSize);
    if (!supported) {
      throw new Error(`Whisper Web is not supported: ${detailedReason}`);
    }

    console.log(`[WhisperBridge] Downloading model: ${modelSize}`);
    await downloadWhisperModel({
      model: modelSize,
      onProgress: ({ progress }) => {
        console.log(
          `[WhisperBridge] Downloading model (${Math.round(progress * 100)}%)...`,
        );
      },
    });

    this.loaded = true;
    this.modelSize = modelSize;
    console.log(`[WhisperBridge] Model ready: ${modelSize}`);
  }

  async transcribe(
    audioBytes: Uint8Array,
    mimeType: string,
    config: TranscriptionConfig,
  ): Promise<TranscriptionResult> {
    if (!this.loaded) {
      await this.load(config.modelSize || "tiny.en");
    }

    console.log("[WhisperBridge] Resampling audio...");
    const file = new File([audioBytes], "audio", { type: mimeType });
    const channelWaveform = await resampleTo16Khz({
      file,
      onProgress: (p) =>
        console.log(
          `[WhisperBridge] Resampling audio (${Math.round(p * 100)}%)...`,
        ),
    });

    console.log("[WhisperBridge] Transcribing...");
    const { transcription } = await transcribe({
      channelWaveform,
      model: this.modelSize,
      onProgress: (p) =>
        console.log(
          `[WhisperBridge] Transcribing (${Math.round(p * 100)}%)...`,
        ),
    });

    const segments: TranscriptionSegment[] = transcription.map((t) => ({
      start: t.start,
      end: t.end,
      text: t.text,
      confidence: t.noSpeechProb ? 1 - t.noSpeechProb : undefined,
    }));

    const fullText = segments.map((s) => s.text).join(" ");

    return {
      text: fullText,
      segments,
      language: config.language || "en",
      duration: segments.length > 0 ? segments[segments.length - 1].end : 0,
      metadata: {
        durationMs: segments.length > 0 ? segments[segments.length - 1].end : 0,
        sampleRateHz: 16000,
        channels: 1,
        codec: "whisper-wasm",
      },
    };
  }
}
```

- [ ] **Step 2: Run test to verify it compiles**

```bash
cd apps/hacienda-studio && npx tsc --noEmit
```

- [ ] **Step 3: Commit**

```bash
git add apps/hacienda-studio/lib/transcription/whisper-bridge.ts
git commit -m "feat(transcription): add whisper.web bridge"
```

#### Task 2.3: Index & Package Config

**Files:**

- Create: `apps/hacienda-studio/lib/transcription/index.ts`

- [ ] **Step 1: Create index.ts**

```typescript
// apps/hacienda-studio/lib/transcription/index.ts
export { WhisperBridge } from "./whisper-bridge";
export type {
  TranscriptionConfig,
  TranscriptionResult,
  TranscriptionSegment,
  AudioMetadata,
} from "./types";
```

- [ ] **Step 2: Run test to verify it compiles**

```bash
cd apps/hacienda-studio && npx tsc --noEmit
```

- [ ] **Step 3: Commit**

```bash
git add apps/hacienda-studio/lib/transcription/index.ts
git commit -m "feat(transcription): add index exports"
```

---

## Phase 2: Pipeline Integration (Week 2-3)

### Task 3: Update Types for Transcription

**File:** `apps/hacienda-studio/lib/types.ts`

- [ ] **Step 1: Add transcription types to types.ts**

```typescript
// Add to existing types.ts
export interface TranscriptionConfig {
  modelSize: "tiny.en" | "tiny" | "base.en" | "base" | "small.en" | "small";
  language?: string;
  task?: "transcribe" | "translate";
  threads?: number;
}

export interface TranscriptionSegment {
  start: number;
  end: number;
  text: string;
  confidence?: number;
}

export interface TranscriptionResult {
  text: string;
  segments: TranscriptionSegment[];
  language: string;
  duration: number;
  metadata: AudioMetadata;
}

export interface AudioMetadata {
  durationMs: number;
  sampleRateHz: number;
  channels: number;
  codec: string;
  bitrate?: number;
}

export interface FileInput {
  name: string;
  bytes: ArrayBuffer;
  type: string;
  isAudio?: boolean;
  isVideo?: boolean;
}

export interface AppConfig {
  // ... existing
  // NEW: Transcription config
  enableTranscription: boolean;
  transcriptionModel:
    "tiny.en" | "tiny" | "base.en" | "base" | "small.en" | "small";
  transcriptionLanguage:
    | "auto"
    | "de"
    | "fr"
    | "es"
    | "it"
    | "pt"
    | "nl"
    | "pl"
    | "sv"
    | "da"
    | "fi"
    | "cs"
    | "hu"
    | "el"
    | "ro"
    | "bg"
    | "hr"
    | "sk"
    | "sl"
    | "et"
    | "lv"
    | "lt"
    | "mt"
    | "ga";
  translateToEnglish: boolean;
  // PII
  enablePiiDetection: boolean;
  redactPiiInOutput: boolean;
  // Vertical
  enabledVerticals: ("m&a" | "financial_services" | "shared")[];
}
```

- [ ] **Step 1-2: Modify types.ts, run build to verify**

### Task 3.1: Update ProcessFile for Audio/Video

**File:** `apps/hacienda-studio/worker/pipeline.ts`

- [ ] **Step 1: Add imports**

```typescript
import { WhisperBridge } from "../lib/transcription/whisper-bridge";
import type {
  TranscriptionConfig,
  TranscriptionResult,
  TranscriptionSegment,
} from "../lib/transcription/types";
```

- [ ] **Step 2: Update processFile signature**

```typescript
async function processFile(
  input: FileInput,
  config: AppConfig,
  verticalDict: VerticalDictionary,
  registry: BatchEntityRegistry,
  docId: string,
  whisperBridge: WhisperBridge,  // NEW
): Promise<ProcessedFile> {
```

- [ ] **Step 3: Add transcription logic in processFile**

```typescript
// In processFile, after markdown extraction:
let markdown = "";
let transcriptionResult: TranscriptionResult | null = null;

const isAudio = input.type.startsWith("audio/");
const isVideo = input.type.startsWith("video/");

if ((isAudio || isVideo) && config.enableTranscription) {
  // 1. Transcribe
  postProgress({ file: input.name, stage: "extract", percent: 10 });

  const transcription = await whisperBridge.transcribe(
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
  transcriptionResult = transcription;
  markdown = transcription.text;
} else {
  // Regular document extraction
  const result = await engine.extract(extractInput, extractConfig);
  markdown = result.results[0]?.content || "";
}

// ... rest of existing NER + vertical logic

// Build frontmatter with transcription metadata
const frontmatter = buildFrontmatter(input, deduped, transcriptionResult);
```

### Task 3.2: Update processFiles for Transcription

**File:** `apps/hacienda-studio/worker/pipeline.ts`

- [ ] **Step 1: Update processFiles to initialize transcription bridge**

```typescript
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
        whisperBridge, // Pass bridge
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
```

---

## Phase 3: NER/PII Integration (Week 3)

### Task 4: PII Detection & Redaction

**File:** `apps/hacienda-studio/lib/pii-detector.ts` (NEW)

```typescript
// apps/hacienda-studio/lib/pii-detector.ts
export interface PIIEntity {
  type: string;
  value: string;
  start: number;
  end: number;
  confidence: number;
}

export const PII_PATTERNS: Record<string, RegExp> = {
  email: /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b/g,
  phone: /(\+?\d{1,3}[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}/g,
  ssn: /\b\d{3}-\d{2}-\d{4}\b/g,
  creditCard: /\b(?:\d{4}[-\s]?){3}\d{4}\b/g,
  iban: /\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b/g,
  bic: /\b[A-Z]{6}[A-Z0-9]{2}(?:[A-Z0-9]{3})?\b/g,
  passport: /\b[A-Z]{1,2}\d{6,9}\b/g,
  driversLicense: /\b[A-Z]\d{7,13}\b/g,
  euVat:
    /\b(?:AT|BE|BG|CY|CZ|DE|DK|EE|EL|ES|FI|FR|HR|HU|IE|IT|LT|LU|LV|MT|NL|PL|PT|RO|SE|SI|SK)\d{2,12}\b/gi,
};

export function detectPII(text: string): PIIEntity[] {
  const entities: PIIEntity[] = [];
  for (const [type, pattern] of Object.entries(PII_PATTERNS)) {
    let match;
    while ((match = pattern.exec(text)) !== null) {
      entities.push({
        type,
        value: match[0],
        start: match.index,
        end: match.index + match[0].length,
        confidence: 0.95,
      });
    }
  }
  return entities;
}

export function redactPII(text: string, entities: PIIEntity[]): string {
  let result = text;
  const sorted = [...entities].sort((a, b) => b.start - a.start);
  for (const entity of sorted) {
    const replacement = `[${entity.type.toUpperCase()}]`;
    result =
      result.slice(0, entity.start) + replacement + result.slice(entity.end);
  }
  return result;
}
```

- [ ] **Step 1-3: Create file, integrate into pipeline.ts, test**

---

## Phase 4: Frontmatter & Glossary Enhancement (Week 4)

### Task 5: Enhanced Frontmatter

```typescript
// In pipeline.ts - update buildFrontmatter
function buildFrontmatter(
  input: FileInput,
  entities: Entity[],
  transcription?: TranscriptionResult,
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

  let frontmatter = `---
source: ${input.name}
type: ${type}
processed: ${new Date().toISOString()}
entities: ${JSON.stringify(entityMeta)}
`;

  if (transcription) {
    frontmatter += `transcription:
  language: ${transcription.language}
  duration_ms: ${transcription.metadata.durationMs}
  segments: ${transcription.segments.length}
`;
  }

  frontmatter += `---`;
  return frontmatter;
}
```

---

## Phase 5: Config Panel UI (Week 4)

### Task 6: ConfigPanel.svelte Updates

```svelte
<!-- apps/hacienda-studio/src/lib/ConfigPanel.svelte -->
<script lang="ts">
  import type { AppConfig } from '../types';
  let { config }: { config: AppConfig } = $props();

  const transcriptionModels = ['tiny.en', 'tiny', 'base.en', 'base', 'small.en', 'small'] as const;
  const languages = [
    { code: 'auto', label: 'Auto-detect' },
    { code: 'de', label: 'German' },
    { code: 'fr', label: 'French' },
    { code: 'es', label: 'Spanish' },
    { code: 'it', label: 'Italian' },
    { code: 'pt', label: 'Portuguese' },
    { code: 'nl', label: 'Dutch' },
    { code: 'pl', label: 'Polish' },
    { code: 'sv', label: 'Swedish' },
    { code: 'da', label: 'Danish' },
    { code: 'fi', label: 'Finnish' },
    { code: 'cs', label: 'Czech' },
    { code: 'hu', label: 'Hungarian' },
    { code: 'el', label: 'Greek' },
    { code: 'ro', label: 'Romanian' },
    { code: 'bg', label: 'Bulgarian' },
    { code: 'hr', label: 'Croatian' },
    { code: 'sk', label: 'Slovak' },
    { code: 'sl', label: 'Slovenian' },
    { code: 'et', label: 'Estonian' },
    { code: 'lv', label: 'Latvian' },
    { code: 'lt', label: 'Lithuanian' },
    { code: 'mt', label: 'Maltese' },
    { code: 'ga', label: 'Irish' },
  ];

  const verticalOptions = ['m&a', 'financial_services', 'shared'] as const;
</script>

<details open>
  <summary>🎙️ Transcription</summary>
  <div class="config-row">
    <label>
      <input type="checkbox" bind:checked={config.enableTranscription} />
      Enable Audio/Video Transcription
    </label>
  </div>
  {#if config.enableTranscription}
    <div class="config-row">
      <label>Model</label>
      <select bind:value={config.transcriptionModel}>
        <option value="tiny.en">Tiny English (75 MB, fastest)</option>
        <option value="tiny">Tiny Multilingual (75 MB)</option>
        <option value="base.en">Base English (142 MB, balanced)</option>
        <option value="base">Base Multilingual (142 MB)</option>
        <option value="small.en">Small English (466 MB, better quality)</option>
        <option value="small">Small Multilingual (466 MB)</option>
      </select>
    </div>
    <div class="config-row">
      <label>Language</label>
      <select bind:value={config.transcriptionLanguage}>
        <option value="auto">Auto-detect</option>
        {#each languages as lang}
          <option value={lang.code}>{lang.label}</option>
        {/each}
      </select>
    </div>
    <label class="checkbox">
      <input type="checkbox" bind:checked={config.translateToEnglish} />
      Translate to English
    </label>
  {/if}
</details>

<details open>
  <summary>🔒 PII & Compliance</summary>
  <label class="checkbox">
    <input type="checkbox" bind:checked={config.enablePiiDetection} />
    Enable PII Detection
  </label>
  <label class="checkbox">
    <input type="checkbox" bind:checked={config.redactPiiInOutput} />
    Redact PII in Output
  </label>
</details>

<details open>
  <summary>🏷️ Vertical NER</summary>
  <div class="config-row">
    <label>Enabled Verticals</label>
    <div class="checkbox-group">
      {#each ['m&a', 'financial_services', 'shared'] as v}
        <label class="checkbox">
          <input type="checkbox" value={v} bind:group={config.enabledVerticals} />
          {v.toUpperCase()}
        </label>
      {/each}
    </div>
  </div>
</details>
```

---

## Phase 5: Testing & Validation (Week 5)

### Task 7: E2E Tests

**File:** `apps/hacienda-studio/tests/e2e/transcription.spec.ts`

```typescript
import { test, expect } from "@playwright/test";
import * as JSZip from "jszip";

test("transcribes audio and extracts M&A entities", async ({ page }) => {
  await page.addInitScript(() =>
    localStorage.setItem("hacienda-studio-visited", "true"),
  );
  await page.goto("/");
  await page.waitForLoadState("domcontentloaded");
  await page.waitForSelector(".drop-zone", { timeout: 10000 });

  const downloadPromise = page.waitForEvent("download");
  const [fc] = await Promise.all([
    page.waitForEvent("filechooser"),
    page.click(".drop-label"),
  ]);
  await fc.setFiles({
    name: "earnings_call.mp3",
    mimeType: "audio/mpeg",
    buffer: Buffer.from("..."), // test audio fixture
  });

  const download = await downloadPromise;
  const zip = await JSZip.loadAsync(await download.path());

  // Check markdown has vertical entities
  const md = await zip.files["spa.md"]?.async("text");
  expect(md).toContain("vertical:");
  expect(md).toContain("entity:");

  // Check registry
  const registry = JSON.parse(
    await zip.files["entities-registry.json"].async("text"),
  );
  expect(registry.entity_registry.entities.length).toBeGreaterThan(0);
  expect(registry.entity_registry.entities[0]).toHaveProperty("vertical");

  // Check KG exports
  expect(zip.files["kg-export/neo4j.cypher"]).toBeDefined();
  expect(zip.files["kg-export/networkx.json"]).toBeDefined();
  expect(zip.files["kg-export/rdf.ttl"]).toBeDefined();
});
```

---

## Files to Create/Modify Summary

| File                                                       | Action                                   |
| ---------------------------------------------------------- | ---------------------------------------- |
| `apps/hacienda-studio/lib/transcription/`                  | Create new directory                     |
| `apps/hacienda-studio/lib/transcription/types.ts`          | Create types                             |
| `apps/hacienda-studio/lib/transcription/whisper-bridge.ts` | Create bridge                            |
| `apps/hacienda-studio/lib/transcription/index.ts`          | Create exports                           |
| `apps/hacienda-studio/lib/types.ts`                        | Extend types                             |
| `apps/hacienda-studio/lib/pii-detector.ts`                 | Create PII detector                      |
| `apps/hacienda-studio/worker/pipeline.ts`                  | Integrate transcription + NER + PII      |
| `apps/hacienda-studio/lib/ConfigPanel.svelte`              | Add transcription/PII/vertical config UI |
| `apps/hacienda-studio/lib/registry.ts`                     | Add transcription metadata to registry   |
| `apps/hacienda-studio/tests/e2e/transcription.spec.ts`     | E2E tests for transcription              |
| `apps/hacienda-studio/vite.config.ts`                      | Add optimizeDeps exclude for whisper-web |
| `README.md`                                                | Update docs                              |

---

## Key Integration Points

1. **NER Bridge** - Already exists via Compromise.js, extend with audio-specific categories
2. **Vertical Dictionary** - Already exists, extend with audio-specific terms
3. **Entity Registry** - Already exists, add transcription metadata
4. **KG Exports** - Already work, will include transcription entities
5. **Frontmatter** - Already enhanced, add transcription metadata

---

## Testing Checklist

- [ ] Audio file upload → transcript + entities + KG exports
- [ ] Video file upload → audio extraction → transcript + entities
- [ ] PII detection + redaction works
- [ ] Vertical classification works for M&A terms (earnout, SPA, etc.)
- [ ] Financial Services entities detected (LP, GP, carry, NAV)
- [ ] Multi-language (de, fr, es, it, pt, nl, etc.)
- [ ] Zip contains: markdown, registry, neo4j.cypher, networkx.json, rdf.ttl
- [ ] Frontmatter includes transcription metadata
- [ ] Entity linking works in markdown
- [ ] Glossary shows vertical badges

---

## Timeline Summary

| Week | Focus                                                      |
| ---- | ---------------------------------------------------------- |
| 1    | Install @remotion/whisper-web, create transcription bridge |
| 2    | Pipeline integration + NER/PII/Vertical                    |
| 3    | Frontmatter, glossary, KG exports                          |
| 4    | Config UI, E2E tests, documentation                        |

---

This plan integrates transcription seamlessly into the existing pipeline - audio/video becomes just another input type that flows through the exact same NER → vertical classification → entity registry → KG export pipeline.
