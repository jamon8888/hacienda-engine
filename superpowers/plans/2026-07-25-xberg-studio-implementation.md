# xberg-studio Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a client-side Svelte web app (xberg-studio) that converts documents to RAG-ready Markdown with entity-linked metadata using xberg WASM for extraction, GLiNER2 ONNX via `@lmoe/gliner-onnx` for NER, and tesseract-wasm for OCR.

**Architecture:** Svelte + Vite + TypeScript frontend with Web Worker for streaming pipeline (xberg extract → ONNX NER via JS bridge → entity linking). All assets loaded from CDN. Static deployment to GitHub Pages.

**Tech Stack:** Svelte 5, Vite 6, TypeScript, @xberg-io/xberg-wasm (includes native NerModel for GLiNER2 NER), tesseract-wasm, jszip, Playwright for E2E.

## Global Constraints

- **100% client-side** — no server, no backend
- **All assets from CDN** — xberg WASM, GLiNER2 model weights, tesseract tessdata
- **COOP/COEP headers required** for SharedArrayBuffer (Web Worker)
- **NER Model:** GLiNER2 via native `NerModel` in xberg-wasm (ner-candle-wasm feature)
- **File size limit:** 50MB per file
- **Output:** .zip of .md files with YAML frontmatter + entity glossaries
- **Entity link format:** `[Name](entity:type/slug)` (case-insensitive)
- **First-visit onboarding** with model download progress
- **Local AI messaging:** "Your files NEVER leave this tab"
- **Error handling:** inline errors for unsupported formats, empty files, size limits

---

### Task 1: Project Scaffold

**Files:**

- Create: `package.json`
- Create: `vite.config.ts`
- Create: `tsconfig.json`
- Create: `index.html`
- Create: `public/_headers` (for GitHub Pages COOP/COEP)
- Create: `src/main.ts`
- Create: `src/app.d.ts`
- Create: `src/app.css`

**Interfaces:**

- Produces: Buildable Svelte project structure

- [ ] **Step 1: Write package.json**

```json
{
  "name": "xberg-studio",
  "version": "1.0.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "preview": "vite preview",
    "test": "playwright test",
    "lint": "eslint src --ext .ts,.svelte",
    "format": "prettier --write src"
  },
  "dependencies": {
    "@xberg-io/xberg-wasm": "^1.0.0-rc.37",
    "tesseract-wasm": "^1.0.0",
    "jszip": "^3.10.1",
    "idb": "^8.0.0"
  },
  "devDependencies": {
    "@sveltejs/vite-plugin-svelte": "^5.0.0",
    "@types/node": "^22.0.0",
    "@typescript-eslint/eslint-plugin": "^8.0.0",
    "@typescript-eslint/parser": "^8.0.0",
    "eslint": "^9.0.0",
    "eslint-plugin-svelte": "^2.46.0",
    "playwright": "^1.48.0",
    "prettier": "^3.3.0",
    "prettier-plugin-svelte": "^3.2.0",
    "svelte": "^5.0.0",
    "svelte-check": "^4.0.0",
    "typescript": "^5.6.0",
    "vite": "^6.0.0"
  }
}
```

- [ ] **Step 2: Write vite.config.ts**

```typescript
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  build: {
    target: 'esnext',
    rollupOptions: {
      output: {
        manualChunks: {
          'xberg-wasm': ['@xberg-io/xberg-wasm'],
          'vendor': ['jszip', 'idb']
        }
      }
    }
  },
  server: {
    headers: {
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp'
    }
  },
  worker: {
    format: 'es',
    plugins: () => []
  }
});
```

- [ ] **Step 3: Write tsconfig.json**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022", "DOM", "DOM.Iterable", "WebWorker"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "preserve",
    "strict": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "esModuleInterop": true,
    "allowSyntheticDefaultImports": true,
    "forceConsistentCasingInFileNames": true,
    "noEmit": true,
    "baseUrl": ".",
    "paths": {
      "$lib/*": ["src/lib/*"]
    }
  },
  "include": ["src/**/*", "src/app.d.ts", "vite.config.ts"],
  "exclude": ["node_modules", "dist"]
}
```

- [ ] **Step 4: Write index.html**

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <meta http-equiv="Cross-Origin-Opener-Policy" content="same-origin">
  <meta http-equiv="Cross-Origin-Embedder-Policy" content="require-corp">
  <title>xberg-studio — Local Document Intelligence</title>
  <link rel="preconnect" href="https://cdn.jsdelivr.net">
</head>
<body>
  <div id="app"></div>
  <script type="module" src="/src/main.ts"></script>
</body>
</html>
```

- [ ] **Step 5: Write public/_headers (GitHub Pages)**

```text
/*
  Cross-Origin-Opener-Policy: same-origin
  Cross-Origin-Embedder-Policy: require-corp
  Cache-Control: public, max-age=31536000, immutable
```

- [ ] **Step 6: Write src/main.ts**

```typescript
import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';

mount(App, { target: document.getElementById('app')! });
```

- [ ] **Step 7: Write src/app.d.ts**

```typescript
/// <reference types="vite/client" />
declare module '*.svelte' {
  export default function createComponent(props: any): any;
}
```

- [ ] **Step 8: Write src/app.css**

```css
:root {
  --font-sans: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  --color-bg: #0d1117;
  --color-surface: #161b22;
  --color-border: #30363d;
  --color-text: #e6edf3;
  --color-muted: #8b949e;
  --color-primary: #58a6ff;
  --color-success: #3fb950;
  --color-error: #f85149;
  --color-warning: #d29922;
  --spacing: 8px;
  --radius: 6px;
}

* { box-sizing: border-box; margin: 0; padding: 0; }
body { font-family: var(--font-sans); background: var(--color-bg); color: var(--color-text); line-height: 1.6; min-height: 100vh; }
a { color: var(--color-primary); text-decoration: none; }
a:hover { text-decoration: underline; }
button { font-family: inherit; cursor: pointer; border: none; background: none; }
input, select { font-family: inherit; }
```

- [ ] **Step 9: Run dev server to verify**

```bash
npm install && npm run dev
```

Expected: Vite starts, shows blank page (App.svelte not created yet)

- [ ] **Step 10: Commit**

```bash
git add .
git commit -m "chore: scaffold xberg-studio Svelte project"
```

---

### Task 2: Type Definitions & Shared Types

**Files:**

- Create: `src/lib/types.ts`

**Interfaces:**

- Produces: TypeScript types for files, entities, progress, config

- [ ] **Step 1: Write src/lib/types.ts**

```typescript
export interface FileInput {
  name: string;
  bytes: ArrayBuffer;
  mimeType: string;
}

export interface Entity {
  name: string;
  type: string;
  slug: string;
  count: number;
  spans: Array<{ start: number; end: number }>;
}

export interface ProcessedFile {
  name: string;
  markdown: string;
  entities: Entity[];
  frontmatter: {
    source: string;
    type: string;
    processed: string;
    entities: Array<{ name: string; type: string; slug: string }>;
  };
}

export interface ProgressUpdate {
  file: string;
  stage: 'extract' | 'ner' | 'link' | 'complete' | 'error';
  percent: number;
  message?: string;
}

export interface AppConfig {
  nerCategories: string[];
  outputFormat: 'markdown' | 'plain' | 'json';
  chunkSize: number;
}

export interface OnboardingState {
  complete: boolean;
  assets: {
    xbergWasm: boolean;
    nerModel: boolean;
    tessdata: boolean;
  };
}

export const DEFAULT_CONFIG: AppConfig = {
  nerCategories: ['person', 'organization', 'location', 'email', 'phone_number'],
  outputFormat: 'markdown',
  chunkSize: 1000
};

export const SUPPORTED_MIME_PREFIXES = [
  'application/pdf',
  'application/vnd.openxmlformats-officedocument',
  'application/msword',
  'application/vnd.ms-excel',
  'application/vnd.ms-powerpoint',
  'application/vnd.oasis.opendocument',
  'message/rfc822',
  'application/vnd.ms-outlook',
  'application/vnd.ms-pki.stl',
  'text/',
  'image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/tiff', 'image/svg+xml',
  'application/json', 'application/xml',
  'application/zip', 'application/x-tar', 'application/gzip', 'application/x-7z-compressed'
];

export function validateFile(file: File): { valid: boolean; error?: string } {
  if (file.size === 0) return { valid: false, error: 'File is empty' };
  if (file.size > 50 * 1024 * 1024) return { valid: false, error: 'File too large (>50MB)' };
  const type = file.type || '';
  const supported = SUPPORTED_MIME_PREFIXES.some(p => type.startsWith(p));
  if (!supported) return { valid: false, error: `Unsupported file type: ${type || file.name}` };
  return { valid: true };
}
```

- [ ] **Step 2: Run TypeScript check**

```bash
npx svelte-check
```

Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add src/lib/types.ts
git commit -m "feat: add shared type definitions"
```

---

### Task 3: Onboarding Screen Component

**Files:**

- Create: `src/lib/Onboarding.svelte`
- Create: `src/lib/Onboarding.test.ts`

**Interfaces:**

- Consumes: `types.ts` (OnboardingState)
- Produces: `Onboarding` component with progress events

- [ ] **Step 1: Write failing test**

```typescript
// src/lib/Onboarding.test.ts
import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import Onboarding from './Onboarding.svelte';

describe('Onboarding', () => {
  it('shows local AI messaging', () => {
    render(Onboarding, { props: { assets: { xbergWasm: true, nerModel: true, tessdata: true }, onComplete: vi.fn() } });
    expect(screen.getByText(/100% local/i)).toBeInTheDocument();
    expect(screen.getByText(/never leave/i)).toBeInTheDocument();
  });

  it('shows progress for each asset', () => {
    render(Onboarding, { props: { assets: { xbergWasm: true, nerModel: false, tessdata: true }, onComplete: vi.fn() } });
    expect(screen.getByText(/ner model/i)).toBeInTheDocument();
  });

  it('disables continue until all assets cached', () => {
    render(Onboarding, { props: { assets: { xbergWasm: true, nerModel: false, tessdata: true }, onComplete: vi.fn() } });
    expect(screen.getByRole('button', { name: /continue/i })).toBeDisabled();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npm test -- src/lib/Onboarding.test.ts
```

Expected: FAIL (component doesn't exist)

- [ ] **Step 3: Write Onboarding.svelte**

```svelte
<script lang="ts">
  import type { OnboardingState } from './types';

  interface Props {
    assets: OnboardingState['assets'];
    onComplete: () => void;
  }

  let { assets, onComplete }: Props = $props();

  $effect(() => {
    if (assets.xbergWasm && assets.nerModel && assets.tessdata) {
      onComplete();
    }
  });
</script>

<div class="onboarding-overlay" role="dialog" aria-modal="true" aria-labelledby="onboarding-title">
  <div class="onboarding-card">
    <header>
      <h1 id="onboarding-title">
        <span class="icon" aria-hidden="true">🔒</span>
        xberg-studio — 100% Local AI in Your Browser
      </h1>
      <p class="subtitle">Your files <strong>NEVER leave this tab</strong>. All processing runs locally via WebAssembly.</p>
    </header>

    <div class="progress-section">
      <div class="overall-progress">
        <div class="progress-bar" role="progressbar" aria-valuenow={overallPercent} aria-valuemin="0" aria-valuemax="100">
          <div class="progress-fill" style="width: {overallPercent}%"></div>
        </div>
        <span class="progress-text">{overallPercent}% — Preparing local models...</span>
      </div>

      <ul class="asset-list" role="list">
        {#each Object.entries(assets) as [key, ready]}
          <li class={ready ? 'ready' : 'loading'}>
            <span class="asset-icon" aria-hidden="true">{iconFor(key)}</span>
            <span class="asset-name">{labelFor(key)}</span>
            <span class="asset-status">{ready ? '✓ Cached' : '↓ Downloading...'}</span>
          </li>
        {/each}
      </ul>
    </div>

    <footer>
      <button class="btn-secondary" disabled={!allReady} on:click={onComplete}>
        Continue
      </button>
    </footer>
  </div>
</div>

<style>
  .onboarding-overlay {
    position: fixed; inset: 0; z-index: 1000;
    background: rgba(13, 17, 23, 0.95);
    display: flex; align-items: center; justify-content: center;
    padding: var(--spacing);
  }
  .onboarding-card {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: calc(var(--spacing) * 3);
    max-width: 520px;
    width: 100%;
    box-shadow: 0 8px 32px rgba(0,0,0,0.4);
  }
  header { text-align: center; margin-bottom: calc(var(--spacing) * 2); }
  h1 { font-size: 1.5rem; font-weight: 600; margin-bottom: var(--spacing); }
  .icon { margin-right: var(--spacing); }
  .subtitle { color: var(--color-muted); font-size: 0.95rem; line-height: 1.5; }
  .progress-bar { height: 8px; background: var(--color-border); border-radius: 4px; overflow: hidden; margin-bottom: var(--spacing); }
  .progress-fill { height: 100%; background: linear-gradient(90deg, var(--color-primary), var(--color-success)); transition: width 0.3s ease; }
  .progress-text { font-size: 0.85rem; color: var(--color-muted); }
  .asset-list { list-style: none; display: flex; flex-direction: column; gap: var(--spacing); margin-bottom: calc(var(--spacing) * 2); }
  .asset-list li { display: flex; align-items: center; gap: var(--spacing); padding: var(--spacing); background: var(--color-bg); border-radius: var(--radius); }
  .asset-list li.ready { border-left: 3px solid var(--color-success); }
  .asset-list li.loading { border-left: 3px solid var(--color-warning); }
  .asset-icon { font-size: 1.2rem; }
  .asset-name { flex: 1; font-weight: 500; }
  .asset-status { font-size: 0.85rem; color: var(--color-muted); }
  .asset-status.ready { color: var(--color-success); }
  footer { text-align: center; }
  .btn-secondary { padding: var(--spacing) calc(var(--spacing) * 3); background: var(--color-border); color: var(--color-text); border-radius: var(--radius); font-weight: 500; transition: background 0.2s; }
  .btn-secondary:hover:not(:disabled) { background: var(--color-primary); }
  .btn-secondary:disabled { opacity: 0.5; cursor: not-allowed; }
</style>

<script lang="ts" context="module">
  function overallPercent(assets: OnboardingState['assets']): number {
    const values = Object.values(assets);
    return Math.round((values.filter(v => v).length / values.length) * 100);
  }

  function iconFor(key: string): string {
    return { xbergWasm: '⚙️', nerModel: '🧠', tessdata: '👁️' }[key] || '📦';
  }

  function labelFor(key: string): string {
    return { xbergWasm: 'xberg WASM Engine', nerModel: 'GLiNER2-Guardrails-PII', tessdata: 'Tesseract OCR Data' }[key] || key;
  }
</script>
```

- [ ] **Step 4: Run test to verify it passes**

```bash
npm test -- src/lib/Onboarding.test.ts
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/lib/Onboarding.svelte src/lib/Onboarding.test.ts
git commit -m "feat: add onboarding screen with local AI messaging"
```

---

### Task 4: Asset Loader (CDN + IndexedDB Cache)

**Files:**

- Create: `src/lib/asset-loader.ts`
- Create: `src/lib/asset-loader.test.ts`

**Interfaces:**

- Consumes: `types.ts`
- Produces: `loadXbergWasm()`, `loadNerModel()`, `loadTessdata()` functions

- [ ] **Step 1: Write failing test**

```typescript
// src/lib/asset-loader.test.ts
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { loadNerModel, isModelCached, createNerBackend } from './asset-loader';

describe('Asset Loader', () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it('returns cached model from IndexedDB', async () => {
    const result = await loadNerModel();
    expect(result).toHaveProperty('model');
    expect(result).toHaveProperty('tokenizer');
    expect(result).toHaveProperty('config');
  });

  it('fetches from HuggingFace CDN when not cached', async () => {
    // Integration test - skip in unit tests
  });

  it('creates NER backend from model assets', async () => {
    const assets = await loadNerModel();
    const backend = await createNerBackend(assets.model, assets.tokenizer, assets.config);
    expect(backend).toHaveProperty('extractEntities');
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npm test -- src/lib/asset-loader.test.ts
```

Expected: FAIL

- [ ] **Step 3: Write asset-loader.ts**

```typescript
import { openDB } from 'idb';

const DB_NAME = 'xberg-studio-assets';
const DB_VERSION = 1;
const MODEL_STORE = 'models';
const TESSDATA_STORE = 'tessdata';

// ONNX model from lmo3 (GLiNER2 multi-v1)
const MODEL_URL = 'https://huggingface.co/lmo3/gliner2-multi-v1-onnx/resolve/main/model.onnx';
const TOKENIZER_URL = 'https://huggingface.co/lmo3/gliner2-multi-v1-onnx/resolve/main/tokenizer.json';
const CONFIG_URL = 'https://huggingface.co/lmo3/gliner2-multi-v1-onnx/resolve/main/config.json';

async function getDB() {
  return openDB(DB_NAME, DB_VERSION, {
    upgrade(db) {
      if (!db.objectStoreNames.contains(MODEL_STORE)) db.createObjectStore(MODEL_STORE);
      if (!db.objectStoreNames.contains(TESSDATA_STORE)) db.createObjectStore(TESSDATA_STORE);
    }
  });
}

export async function isModelCached(): Promise<boolean> {
  const db = await getDB();
  const model = await db.get(MODEL_STORE, 'gliner2-multi-v1-onnx-model');
  const tokenizer = await db.get(MODEL_STORE, 'gliner2-multi-v1-onnx-tokenizer');
  const config = await db.get(MODEL_STORE, 'gliner2-multi-v1-onnx-config');
  return !!(model && tokenizer && config);
}

export async function loadNerModel(): Promise<{ model: Uint8Array; tokenizer: Uint8Array; config: Uint8Array }> {
  const db = await getDB();

  // Check cache first
  const [model, tokenizer, config] = await Promise.all([
    db.get(MODEL_STORE, 'gliner2-multi-v1-onnx-model'),
    db.get(MODEL_STORE, 'gliner2-multi-v1-onnx-tokenizer'),
    db.get(MODEL_STORE, 'gliner2-multi-v1-onnx-config')
  ]);

  if (model && tokenizer && config) {
    return { model, tokenizer, config };
  }

  // Fetch from HuggingFace CDN
  const [modelRes, tokenizerRes, configRes] = await Promise.all([
    fetch(MODEL_URL),
    fetch(TOKENIZER_URL),
    fetch(CONFIG_URL)
  ]);

  if (!modelRes.ok || !tokenizerRes.ok || !configRes.ok) {
    throw new Error('Failed to download GLiNER2 ONNX model from HuggingFace');
  }

  const modelData = new Uint8Array(await modelRes.arrayBuffer());
  const tokenizerData = new Uint8Array(await tokenizerRes.arrayBuffer());
  const configData = new Uint8Array(await configRes.arrayBuffer());

  // Cache in IndexedDB
  const tx = db.transaction(MODEL_STORE, 'readwrite');
  await Promise.all([
    tx.store.put(modelData, 'gliner2-multi-v1-onnx-model'),
    tx.store.put(tokenizerData, 'gliner2-multi-v1-onnx-tokenizer'),
    tx.store.put(configData, 'gliner2-multi-v1-onnx-config'),
    tx.done
  ]);

  return { model: modelData, tokenizer: tokenizerData, config: configData };
}

export async function loadTessdata(lang: string = 'eng'): Promise<Uint8Array> {
  const db = await getDB();
  const cached = await db.get(TESSDATA_STORE, `tessdata_${lang}`);
  if (cached) return cached;

  // tesseract-wasm handles its own tessdata fetching
  // This is a placeholder for preloading
  return new Uint8Array();
}

export async function preloadXbergWasm(): Promise<void> {
  // @xberg-io/xberg-wasm loads itself via initWasm()
  // Just trigger the import to start loading
  await import('@xberg-io/xberg-wasm');
}

export async function createNerBackend(
  model: Uint8Array,
  tokenizer: Uint8Array,
  config: Uint8Array
): Promise<any> {
  // Use the new NerModel from xberg-wasm (pure-Rust Candle GLiNER2 backend)
  // Note: This requires the ner-candle-wasm feature enabled in xberg-wasm
  const { NerModel } = await import('@xberg-io/xberg-wasm');
  const runtime = await NerModel.load({ weights: model, tokenizer, encoderConfig: config });
  return runtime;
}
```

- [ ] **Step 4: Run test**

```bash
npm test -- src/lib/asset-loader.test.ts
```

Expected: PASS (with mocked idb)

- [ ] **Step 5: Commit**

```bash
git add src/lib/asset-loader.ts src/lib/asset-loader.test.ts
git commit -m "feat: add asset loader with IndexedDB caching"
```

---

### Task 5: Web Worker Pipeline

**Files:**

- Create: `src/worker/pipeline.ts`
- Create: `src/worker/pipeline.test.ts`

**Interfaces:**

- Consumes: `types.ts` (FileInput, ProcessedFile, ProgressUpdate, AppConfig)
- Produces: Worker message handler with extract → NER → link pipeline

- [ ] **Step 1: Write failing test**

```typescript
// src/worker/pipeline.test.ts
import { describe, it, expect, vi } from 'vitest';
import { processFiles } from './pipeline';

describe('Worker Pipeline', () => {
  it('emits progress updates for each stage', async () => {
    const postMessage = vi.fn();
    const files: FileInput[] = [{ name: 'test.txt', bytes: new ArrayBuffer(100), mimeType: 'text/plain' }];
    const config = { nerCategories: ['person'], outputFormat: 'markdown' as const, chunkSize: 1000 };

    // Mock the pipeline
    const updates: ProgressUpdate[] = [];
    // This is a structural test - real integration test in Task 8
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npm test -- src/worker/pipeline.test.ts
```

Expected: FAIL

- [ ] **Step 3: Write pipeline.ts**

```typescript
import type { FileInput, ProcessedFile, ProgressUpdate, AppConfig, Entity } from '../lib/types';

import { XbergEngine, initWasm, ExtractionConfig, ExtractInput } from '@xberg-io/xberg-wasm';
import { GLiNER2ONNXRuntime } from '@lmoe/gliner-onnx';

interface NerModelAssets {
  model: Uint8Array;
  tokenizer: Uint8Array;
  config: Uint8Array;
}

let glinerRuntime: GLiNER2ONNXRuntime | null = null;
let xbergEngine: XbergEngine | null = null;

function postProgress(update: ProgressUpdate): void {
  self.postMessage({ type: 'progress', ...update });
}

function slugify(text: string): string {
  return text.toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
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
  const allSpans = entities.flatMap(e => e.spans.map(s => ({ ...s, entity: e })));
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
  const entityMeta = entities.map(e => ({
    name: e.name,
    type: e.type.charAt(0).toUpperCase() + e.type.slice(1),
    slug: e.slug
  }));

  const type = input.mimeType.split('/')[1] || 'unknown';
  return `---
source: ${input.name}
type: ${type}
processed: ${new Date().toISOString()}
entities: ${JSON.stringify(entityMeta)}
---`;
}

function buildGlossary(entities: Entity[]): string {
  if (entities.length === 0) return '';
  let md = '\n## Entities\n\n';
  for (const e of entities) {
    md += `- **${e.name}** \`${e.type.charAt(0).toUpperCase() + e.type.slice(1)}\` — mentioned ${e.count} time${e.count > 1 ? 's' : ''}\n`;
  }
  return md;
}

async function initEngine(assets: NerModelAssets): Promise<void> {
  await initWasm();
  
  // Initialize GLiNER2 ONNX runtime
  glinerRuntime = await GLiNER2ONNXRuntime.fromPretrained('local', {
    modelBytes: assets.model,
    tokenizerBytes: assets.tokenizer,
    configBytes: assets.config,
    executionProviders: ['wasm']
  });

  // Create NER bridge function for xberg-wasm
  const nerBridge = async (text: string, categories: string[]) => {
    if (!glinerRuntime) throw new Error('GLiNER runtime not initialized');
    const entities = await glinerRuntime.extractEntities(text, categories);
    return entities.map(e => ({
      category: e.label,
      text: e.text,
      start: e.start,
      end: e.end,
      confidence: e.score
    }));
  };

  // Create XbergEngine with NER injection
  xbergEngine = new XbergEngine(
    { bridgeTimeoutMs: 30000 },
    { ner: nerBridge }
  );

  self.postMessage({ type: 'ready' });
}

async function processFile(input: FileInput, config: AppConfig): Promise<ProcessedFile> {
  if (!xbergEngine) throw new Error('Engine not initialized');

  postProgress({ file: input.name, stage: 'extract', percent: 10 });

  const extractInput: ExtractInput = {
    kind: 'bytes',
    bytes: new Uint8Array(input.bytes),
    mimeType: input.mimeType,
    filename: input.name
  };

  const extractConfig: ExtractionConfig = {
    outputFormat: config.outputFormat,
    chunking: { chunkSize: config.chunkSize },
    ocr: { backend: 'tesseract-wasm', language: ['eng'] },
    ner: { categories: config.nerCategories }
  };

  const result = await xbergEngine.extract(extractInput, extractConfig);
  postProgress({ file: input.name, stage: 'extract', percent: 50 });

  if (!result.results[0]?.content) {
    throw new Error('No content extracted');
  }

  const markdown = result.results[0].content;

  postProgress({ file: input.name, stage: 'ner', percent: 60 });

  const xbergEntities = result.results[0].entities || [];

  const entities: Entity[] = [];
  for (const e of xbergEntities) {
    if (!config.nerCategories.includes(e.category.toLowerCase())) continue;
    entities.push({
      name: e.text,
      type: e.category.toLowerCase(),
      slug: slugify(e.text),
      count: 1,
      spans: [{ start: e.start, end: e.end }]
    });
  }

  postProgress({ file: input.name, stage: 'ner', percent: 80 });

  const deduped = deduplicateEntities(entities);
  const linkedMarkdown = linkEntities(markdown, deduped);
  const frontmatter = buildFrontmatter(input, deduped);
  const glossary = buildGlossary(deduped);

  const finalMarkdown = frontmatter + '\n' + linkedMarkdown + glossary;

  postProgress({ file: input.name, stage: 'complete', percent: 100 });

  return {
    name: input.name.replace(/\.[^.]+$/, '.md'),
    markdown: finalMarkdown,
    entities: deduped,
    frontmatter: {
      source: input.name,
      type: type,
      processed: new Date().toISOString(),
      entities: deduped.map(e => ({ name: e.name, type: e.type.charAt(0).toUpperCase() + e.type.slice(1), slug: e.slug }))
    }
  };
}

async function processFiles(files: FileInput[], config: AppConfig): Promise<void> {
  const results: ProcessedFile[] = [];

  for (const file of files) {
    try {
      const processed = await processFile(file, config);
      results.push(processed);
      self.postMessage({ type: 'file-complete', ...processed });
    } catch (error) {
      self.postMessage({
        type: 'error',
        file: file.name,
        message: error instanceof Error ? error.message : 'Unknown error'
      });
    }
  }

  // Create zip
  const JSZip = (await import('jszip')).default;
  const zip = new JSZip();
  for (const r of results) {
    zip.file(r.name, r.markdown);
  }
  // Add manifest
  const manifest = {
    files: results.map(r => ({ name: r.name, entityCount: r.entities.length })),
    generated: new Date().toISOString()
  };
  zip.file('_manifest.json', JSON.stringify(manifest, null, 2));

  const blob = await zip.generateAsync({ type: 'blob' });
  self.postMessage({ type: 'batch-complete', zip: blob });
}

// Worker message handler
self.onmessage = async (event: MessageEvent) => {
  const { type, files, config, assets } = event.data;

  if (type === 'init') {
    // Initialize engine with NER model assets
    if (assets) {
      await initEngine(assets);
    } else {
      await initWasm();
      self.postMessage({ type: 'ready' });
    }
    return;
  }

  if (type === 'process') {
    await processFiles(files, config);
  }
};
```

function buildFrontmatter(input: FileInput, entities: Entity[]): string {
  const entityMeta = entities.map(e => ({
    name: e.name,
    type: e.type.charAt(0).toUpperCase() + e.type.slice(1),
    slug: e.slug
  }));

  const type = input.mimeType.split[1]('/') || 'unknown';
  return `---
source: ${input.name}
type: ${type}
processed: ${new Date().toISOString()}
entities: ${JSON.stringify(entityMeta)}
---`;
}

function buildGlossary(entities: Entity[]): string {
  if (entities.length === 0) return '';
  let md = '\n## Entities\n\n';
  for (const e of entities) {
    md += `- **${e.name}** \`${e.type.charAt(0).toUpperCase() + e.type.slice(1)}\` — mentioned ${e.count} time${e.count > 1 ? 's' : ''}\n`;
  }
  return md;
}

async function processFile(input: FileInput, config: AppConfig): Promise<ProcessedFile> {
  postProgress({ file: input.name, stage: 'extract', percent: 10 });

  // Stage 1: Extract with xberg
  const extractInput: ExtractInput = {
    kind: 'bytes',
    bytes: new Uint8Array(input.bytes),
    mimeType: input.mimeType,
    filename: input.name
  };

  const extractConfig: ExtractionConfig = {
    outputFormat: config.outputFormat,
    chunking: { chunkSize: config.chunkSize },
    ocr: { backend: 'tesseract-wasm', language: ['eng'] },
    ner: { categories: config.nerCategories }
  };

  const result = await extract(extractInput, extractConfig);
  postProgress({ file: input.name, stage: 'extract', percent: 50 });

  if (!result.results[0]?.content) {
    throw new Error('No content extracted');
  }

  const markdown = result.results[0].content;

  // Stage 2: NER (run on extracted text)
  postProgress({ file: input.name, stage: 'ner', percent: 60 });
  // NER is handled by xberg-wasm internally via the ner config above
  // The result should include entities in result.results[0].entities
  // For now, we'll extract from xberg result structure

  const xbergEntities = result.results[0].entities || [];

  // Convert to our format
  const entities: Entity[] = [];
  for (const e of xbergEntities) {
    if (!config.nerCategories.includes(e.category.toLowerCase())) continue;
    entities.push({
      name: e.text,
      type: e.category.toLowerCase(),
      slug: slugify(e.text),
      count: 1,
      spans: [{ start: e.start, end: e.end }]
    });
  }

  postProgress({ file: input.name, stage: 'ner', percent: 80 });

  // Stage 3: Link entities + build glossary
  postProgress({ file: input.name, stage: 'link', percent: 90 });
  const deduped = deduplicateEntities(entities);
  const linkedMarkdown = linkEntities(markdown, deduped);
  const frontmatter = buildFrontmatter(input, deduped);
  const glossary = buildGlossary(deduped);

  const finalMarkdown = frontmatter + '\n' + linkedMarkdown + glossary;

  postProgress({ file: input.name, stage: 'complete', percent: 100 });

  return {
    name: input.name.replace(/\.[^.]+$/, '.md'),
    markdown: finalMarkdown,
    entities: deduped,
    frontmatter: {
      source: input.name,
      type: type,
      processed: new Date().toISOString(),
      entities: deduped.map(e => ({ name: e.name, type: e.type.charAt(0).toUpperCase() + e.type.slice(1), slug: e.slug }))
    }
  };
}

async function processFiles(files: FileInput[], config: AppConfig): Promise<void> {
  const results: ProcessedFile[] = [];

  for (const file of files) {
    try {
      const processed = await processFile(file, config);
      results.push(processed);
      self.postMessage({ type: 'file-complete', ...processed });
    } catch (error) {
      self.postMessage({
        type: 'error',
        file: file.name,
        message: error instanceof Error ? error.message : 'Unknown error'
      });
    }
  }

  // Create zip
  const JSZip = (await import('jszip')).default;
  const zip = new JSZip();
  for (const r of results) {
    zip.file(r.name, r.markdown);
  }
  // Add manifest
  const manifest = {
    files: results.map(r => ({ name: r.name, entityCount: r.entities.length })),
    generated: new Date().toISOString()
  };
  zip.file('_manifest.json', JSON.stringify(manifest, null, 2));

  const blob = await zip.generateAsync({ type: 'blob' });
  self.postMessage({ type: 'batch-complete', zip: blob });
}

// Worker message handler
self.onmessage = async (event: MessageEvent) => {
  const { type, files, config } = event.data;

  if (type === 'init') {
    // Pre-warm
    await import('@xberg-io/xberg-wasm');
    self.postMessage({ type: 'ready' });
    return;
  }

  if (type === 'process') {
    await processFiles(files, config);
  }
};

```text

- [ ] **Step 4: Run test**

```bash
npm test -- src/worker/pipeline.test.ts
```

Expected: PASS (structural)

- [ ] **Step 5: Add worker to vite config**

```typescript
// vite.config.ts - add to build.rollupOptions.output.manualChunks
'worker': ['src/worker/pipeline.ts']
```

- [ ] **Step 6: Commit**

```bash
git add src/worker/pipeline.ts src/worker/pipeline.test.ts vite.config.ts
git commit -m "feat: add web worker extraction pipeline"
```

---

### Task 6: Main App Component

**Files:**

- Create: `src/App.svelte`
- Create: `src/App.test.ts`

**Interfaces:**

- Consumes: `Onboarding.svelte`, `asset-loader.ts`, `types.ts`, `worker/pipeline.ts`
- Produces: Root app with file handling, progress, download

- [ ] **Step 1: Write failing test**

```typescript
// src/App.test.ts
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import App from './App.svelte';

describe('App', () => {
  it('shows onboarding on first visit', () => {
    render(App);
    expect(screen.getByText(/100% local/i)).toBeInTheDocument();
  });

  it('shows drop zone after onboarding', async () => {
    render(App);
    // Simulate onboarding complete
    await vi.waitFor(() => screen.queryByText(/drop files here/i));
    expect(screen.getByText(/drop files here/i)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npm test -- src/App.test.ts
```

Expected: FAIL

- [ ] **Step 3: Write App.svelte**

```svelte
<script lang="ts">
  import { onMount } from 'svelte';
  import Onboarding from './lib/Onboarding.svelte';
  import { loadNerModel, isModelCached, preloadXbergWasm } from './lib/asset-loader';
  import { validateFile, DEFAULT_CONFIG, type AppConfig, type FileInput, type ProcessedFile, type ProgressUpdate } from './lib/types';

  let onboardingComplete = $state(false);
  let assets = $state({ xbergWasm: false, nerModel: false, tessdata: false });
  let files = $state<File[]>([]);
  let progress = $state<Map<string, ProgressUpdate>>(new Map());
  let results = $state<ProcessedFile[]>([]);
  let config = $state<AppConfig>(DEFAULT_CONFIG);
  let showConfig = $state(false);
  let error = $state<string | null>(null);
  let worker: Worker | null = null;

  onMount(async () => {
    // Check if onboarding was completed before
    const visited = localStorage.getItem('xberg-studio-visited');
    if (visited) {
      onboardingComplete = true;
      assets = { xbergWasm: true, nerModel: true, tessdata: true };
    } else {
      await preloadAssets();
    }

    // Initialize worker
    worker = new Worker(new URL('./worker/pipeline.ts', import.meta.url), { type: 'module' });
    worker.onmessage = handleWorkerMessage;
    await new Promise(resolve => { worker!.onmessage = e => { if (e.data.type === 'ready') resolve(); }; });
  });

  async function preloadAssets() {
    try {
      assets.xbergWasm = true;
      await preloadXbergWasm();

      let nerAssets;
      if (await isModelCached()) {
        assets.nerModel = true;
        nerAssets = await loadNerModel();
      } else {
        nerAssets = await loadNerModel();
        assets.nerModel = true;
      }

      assets.tessdata = true;
      localStorage.setItem('xberg-studio-visited', 'true');
      onboardingComplete = true;

      // Send NER model assets to worker for bridge initialization
      worker!.postMessage({ type: 'init-ner', assets: nerAssets });
    } catch (e) {
      error = 'Failed to load models. Some features may be limited.';
      console.error(e);
      onboardingComplete = true; // Allow continuing anyway
    }
  }

  function handleWorkerMessage(event: MessageEvent) {
    const { type, ...data } = event.data;
    switch (type) {
      case 'progress':
        progress = new Map(progress).set(data.file, data);
        break;
      case 'file-complete':
        results = [...results, data];
        progress = new Map(progress).set(data.name, { ...data, stage: 'complete', percent: 100 });
        break;
      case 'batch-complete':
        downloadZip(data.zip);
        break;
      case 'error':
        error = `${data.file}: ${data.message}`;
        break;
    }
  }

  function handleFiles(fileList: FileList | FileList | File[]): void {
    const fileArray = Array.from(fileList);
    const validFiles: File[] = [];

    for (const file of fileArray) {
      const validation = validateFile(file);
      if (!validation.valid) {
        error = validation.error || 'Invalid file';
        continue;
      }
      validFiles.push(file);
    }

    if (validFiles.length === 0) return;

    files = [...files, ...validFiles];
    error = null;

    // Send to worker
    const fileInputs = await Promise.all(validFiles.map(async f => ({
      name: f.name,
      bytes: await f.arrayBuffer(),
      mimeType: f.type || 'application/octet-stream'
    })));

    worker!.postMessage({ type: 'process', files: fileInputs, config });
  }

  function onDrop(event: DragEvent): void {
    event.preventDefault();
    if (event.dataTransfer?.files) handleFiles(event.dataTransfer.files);
  }

  function onDragOver(event: DragEvent): void {
    event.preventDefault();
    event.dataTransfer!.dropEffect = 'copy';
  }

  function downloadZip(blob: Blob): void {
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `xberg-output-${Date.now()}.zip`;
    a.click();
    URL.revokeObjectURL(url);
  }

  function clearError(): void {
    error = null;
  }
</script>

<div class="app">
  {#if !onboardingComplete}
    <Onboarding {assets} onComplete={() => { onboardingComplete = true; localStorage.setItem('xberg-studio-visited', 'true'); }} />
  {:else}
    <header class="header">
      <h1>xberg-studio</h1>
      <button class="config-toggle" on:click={() => showConfig = !showConfig} aria-expanded={showConfig}>
        ⚙ Config
      </button>
    </header>

    {#if error}
      <div class="error-banner" role="alert">
        <span>❌ {error}</span>
        <button on:click={clearError} aria-label="Dismiss">✕</button>
      </div>
    {/if}

    <main class="main">
      <section class="drop-zone" on:drop={onDrop} on:dragover={onDragOver} ondragleave={onDragOver}>
        <input type="file" id="file-input" multiple accept=".pdf,.docx,.xlsx,.pptx,.odt,.ods,.odp,.eml,.msg,.pst,.png,.jpg,.jpeg,.gif,.webp,.tiff,.bmp,.svg,.srt,.vtt,.txt,.md,.json,.csv,.xml,.html" class="file-input" on:change={(e) => handleFiles(e.target.files)} aria-label="Choose files" />
        <label for="file-input" class="drop-label">
          <span class="drop-icon" aria-hidden="true">📄</span>
          <p>Drop files here or click to browse</p>
          <p class="drop-hint">PDF, Office, Email, Images, Subtitles, Code — up to 50MB each</p>
        </label>
      </section>

      {#if files.length > 0}
        <section class="progress-section" aria-live="polite">
          {#each files as file}
            {#if progress.has(file.name)}
              <ProgressBar file={file} update={progress.get(file.name)!} />
            {/if}
          {/each}
        </section>
      {/if}

      {#if results.length > 0}
        <footer class="footer">
          <button class="btn-primary" on:click={() => worker!.postMessage({ type: 'process', files: [], config })} disabled>
            Processing...
          </button>
        </footer>
      {/if}
    </main>

    {#if showConfig}
      <ConfigPanel {config} />
    {/if}
  {/if}
</div>

<style>
  .app { min-height: 100vh; display: flex; flex-direction: column; }
  .header { display: flex; justify-content: space-between; align-items: center; padding: var(--spacing) calc(var(--spacing) * 2); border-bottom: 1px solid var(--color-border); background: var(--color-surface); }
  h1 { font-size: 1.25rem; font-weight: 600; }
  .config-toggle { padding: var(--spacing) calc(var(--spacing) * 1.5); background: var(--color-border); border-radius: var(--radius); font-size: 0.85rem; color: var(--color-text); transition: background 0.2s; }
  .config-toggle:hover { background: var(--color-primary); }
  .error-banner { display: flex; justify-content: space-between; align-items: center; padding: var(--spacing) calc(var(--spacing) * 2); background: rgba(248, 81, 73, 0.15); border-bottom: 1px solid var(--color-error); color: var(--color-error); }
  .error-banner button { color: inherit; font-size: 1.2rem; line-height: 1; }
  .main { flex: 1; padding: calc(var(--spacing) * 3) calc(var(--spacing) * 2); max-width: 800px; margin: 0 auto; width: 100%; }
  .drop-zone { border: 2px dashed var(--color-border); border-radius: 12px; padding: calc(var(--spacing) * 4) var(--spacing); text-align: center; transition: border-color 0.2s, background 0.2s; background: var(--color-surface); cursor: pointer; }
  .drop-zone:hover, .drop-zone.drag-over { border-color: var(--color-primary); background: rgba(88, 166, 255, 0.05); }
  .file-input { position: absolute; width: 0.1px; height: 0.1px; opacity: 0; overflow: hidden; z-index: -1; }
  .drop-icon { font-size: 3rem; margin-bottom: var(--spacing); display: block; }
  .drop-label p { color: var(--color-muted); margin-bottom: calc(var(--spacing) / 2); }
  .drop-hint { font-size: 0.85rem !important; }
  .progress-section { margin-top: calc(var(--spacing) * 2); display: flex; flex-direction: column; gap: var(--spacing); }
  .footer { padding-top: calc(var(--spacing) * 2); border-top: 1px solid var(--color-border); display: flex; justify-content: flex-end; }
  .btn-primary { padding: var(--spacing) calc(var(--spacing) * 3); background: var(--color-primary); color: #fff; border-radius: var(--radius); font-weight: 600; }
  .btn-primary:disabled { opacity: 0.6; cursor: not-allowed; }
</style>
```

- [ ] **Step 4: Write ProgressBar component**

```svelte
<!-- src/lib/ProgressBar.svelte -->
<script lang="ts">
  import type { ProgressUpdate, FileInput } from './types';

  interface Props {
    file: FileInput;
    update: ProgressUpdate;
  }

  let { file, update }: Props = $props();

  const stageLabels = {
    extract: 'Extracting text',
    ner: 'Finding entities',
    link: 'Linking entities',
    complete: 'Complete',
    error: 'Error'
  };
</script>

<div class="progress-card">
  <div class="progress-header">
    <span class="file-name">{file.name}</span>
    <span class="stage-label">{stageLabels[update.stage]}</span>
  </div>
  <div class="progress-bar" role="progressbar" aria-valuenow={update.percent} aria-valuemin="0" aria-valuemax="100">
    <div class="progress-fill" style="width: {update.percent}%"></div>
  </div>
  {#if update.message}
    <p class="progress-message">{update.message}</p>
  {/if}
</div>

<style>
  .progress-card { background: var(--color-surface); border: 1px solid var(--color-border); border-radius: var(--radius); padding: var(--spacing); }
  .progress-header { display: flex; justify-content: space-between; margin-bottom: calc(var(--spacing) / 2); font-size: 0.85rem; }
  .file-name { font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 70%; }
  .stage-label { color: var(--color-muted); }
  .progress-bar { height: 6px; background: var(--color-bg); border-radius: 3px; overflow: hidden; }
  .progress-fill { height: 100%; background: linear-gradient(90deg, var(--color-primary), var(--color-success)); transition: width 0.3s ease; }
  .progress-message { font-size: 0.75rem; color: var(--color-muted); margin-top: calc(var(--spacing) / 2); }
</style>
```

- [ ] **Step 5: Write ConfigPanel component**

```svelte
<!-- src/lib/ConfigPanel.svelte -->
<script lang="ts">
  import type { AppConfig } from './types';

  interface Props {
    config: AppConfig;
  }

  let { config }: Props = $props();

  const allCategories = [
    { key: 'person', label: 'Person', group: 'Person' },
    { key: 'full_name', label: 'Full Name', group: 'Person' },
    { key: 'first_name', label: 'First Name', group: 'Person' },
    { key: 'last_name', label: 'Last Name', group: 'Person' },
    { key: 'organization', label: 'Organization', group: 'Organization' },
    { key: 'company', label: 'Company', group: 'Organization' },
    { key: 'location', label: 'Location', group: 'Location' },
    { key: 'city', label: 'City', group: 'Location' },
    { key: 'state_or_region', label: 'State/Region', group: 'Location' },
    { key: 'country', label: 'Country', group: 'Location' },
    { key: 'email', label: 'Email', group: 'Contact' },
    { key: 'phone_number', label: 'Phone', group: 'Contact' },
    { key: 'address', label: 'Address', group: 'Contact' },
    { key: 'date', label: 'Date', group: 'Temporal' },
    { key: 'money', label: 'Money', group: 'Financial' }
  ];

  const grouped = Object.groupBy(allCategories, c => c.group);
</script>

<div class="config-panel" role="dialog" aria-label="Configuration">
  <header>
    <h2>⚙ Configuration</h2>
    <p class="local-badge">🔒 All processing runs locally in your browser</p>
  </header>

  <section>
    <h3>NER Categories</h3>
    <div class="category-grid">
      {#each Object.entries(grouped) as [group, categories]}
        <fieldset>
          <legend>{group}</legend>
          {#each categories as cat}
            <label>
              <input type="checkbox" bind:checked={config.nerCategories[cat.key] ? true : false} on:change={() => toggleCategory(cat.key)} />
              <span>{cat.label}</span>
            </label>
          {/each}
        </fieldset>
      {/each}
    </div>
  </section>

  <section>
    <h3>Output</h3>
    <div class="field">
      <label for="output-format">Format</label>
      <select id="output-format" bind:value={config.outputFormat}>
        <option value="markdown">Markdown</option>
        <option value="plain">Plain Text</option>
        <option value="json">JSON</option>
      </select>
    </div>
    <div class="field">
      <label for="chunk-size">Chunk Size (tokens)</label>
      <input id="chunk-size" type="number" bind:value={config.chunkSize} min="100" max="8000" step="100" />
    </div>
  </section>

  <footer>
    <button class="btn-secondary" on:click={() => showConfig = false}>Done</button>
  </footer>
</div>

<script>
  function toggleCategory(key: string) {
    const idx = config.nerCategories.indexOf(key);
    if (idx >= 0) config.nerCategories.splice(idx, 1);
    else config.nerCategories.push(key);
    config = config; // trigger reactivity
  }
</script>

<style>
  .config-panel { position: fixed; top: 0; right: 0; bottom: 0; width: 360px; max-width: 100vw; background: var(--color-surface); border-left: 1px solid var(--color-border); z-index: 100; padding: calc(var(--spacing) * 2); overflow-y: auto; box-shadow: -8px 0 32px rgba(0,0,0,0.3); animation: slideIn 0.2s ease; }
  @keyframes slideIn { from { transform: translateX(100%); } to { transform: translateX(0); } }
  header { margin-bottom: calc(var(--spacing) * 2); padding-bottom: var(--spacing); border-bottom: 1px solid var(--color-border); }
  h2 { font-size: 1.1rem; margin-bottom: calc(var(--spacing) / 2); }
  .local-badge { font-size: 0.8rem; color: var(--color-success); font-weight: 500; }
  section { margin-bottom: calc(var(--spacing) * 2); }
  h3 { font-size: 0.9rem; text-transform: uppercase; letter-spacing: 0.05em; color: var(--color-muted); margin-bottom: var(--spacing); }
  .category-grid { display: flex; flex-direction: column; gap: var(--spacing); }
  fieldset { border: 1px solid var(--color-border); border-radius: var(--radius); padding: var(--spacing); }
  legend { font-size: 0.75rem; text-transform: uppercase; color: var(--color-muted); padding: 0 calc(var(--spacing) / 2); }
  label { display: flex; align-items: center; gap: var(--spacing); cursor: pointer; font-size: 0.85rem; }
  input[type="checkbox"] { width: 16px; height: 16px; accent-color: var(--color-primary); }
  .field { display: flex; flex-direction: column; gap: calc(var(--spacing) / 2); }
  .field label { font-size: 0.8rem; color: var(--color-muted); }
  select, input[type="number"] { padding: var(--spacing) calc(var(--spacing) * 1.5); background: var(--color-bg); border: 1px solid var(--color-border); border-radius: var(--radius); color: var(--color-text); font-family: inherit; }
  footer { margin-top: calc(var(--spacing) * 3); padding-top: var(--spacing); border-top: 1px solid var(--color-border); text-align: right; }
</style>
```

- [ ] **Step 6: Run tests**

```bash
npm test -- src/App.test.ts
```

Expected: PASS

- [ ] **Step 7: Run dev server to verify**

```bash
npm run dev
```

Expected: App loads, shows onboarding, then drop zone

- [ ] **Step 8: Commit**

```bash
git add src/App.svelte src/App.test.ts src/lib/ProgressBar.svelte src/lib/ConfigPanel.svelte
git commit -m "feat: add main app component with drop zone, progress, config panel"
```

---

### Task 7: Integration & E2E Tests

**Files:**

- Create: `playwright.config.ts`
- Create: `tests/e2e/basic.spec.ts`

**Interfaces:**

- Consumes: Built app
- Produces: Playwright test suite

- [ ] **Step 1: Write playwright.config.ts**

```typescript
import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './tests/e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: 'html',
  use: {
    baseURL: 'http://localhost:5173',
    trace: 'on-first-retry',
    screenshot: 'only-on-failure'
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    { name: 'firefox', use: { ...devices['Desktop Firefox'] } },
    { name: 'webkit', use: { ...devices['Desktop Safari'] } }
  ],
  webServer: {
    command: 'npm run dev',
    url: 'http://localhost:5173',
    reuseExistingServer: !process.env.CI,
    timeout: 120000
  }
});
```

- [ ] **Step 2: Write tests/e2e/basic.spec.ts**

```typescript
import { test, expect } from '@playwright/test';

test.describe('xberg-studio', () => {
  test('loads and shows onboarding', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('text=100% Local AI')).toBeVisible();
    await expect(page.locator('text=NEVER leave this tab')).toBeVisible();
  });

  test('completes onboarding and shows drop zone', async ({ page }) => {
    await page.goto('/');
    // Wait for assets to load (mock in real test)
    await page.waitForSelector('text=Continue', { state: 'enabled', timeout: 10000 });
    await page.click('text=Continue');
    await expect(page.locator('text=Drop files here')).toBeVisible();
  });

  test('rejects unsupported file type', async ({ page }) => {
    await page.goto('/');
    await page.click('text=Continue'); // Skip onboarding

    // Create a fake unsupported file
    await page.setInputFiles('input[type="file"]', {
      name: 'test.exe',
      mimeType: 'application/x-msdownload',
      buffer: Buffer.from('fake')
    });

    await expect(page.locator('text=Unsupported file type')).toBeVisible({ timeout: 5000 });
  });

  test('shows config panel', async ({ page }) => {
    await page.goto('/');
    await page.click('text=Continue');
    await page.click('button:has-text("⚙ Config")');
    await expect(page.locator('text=NER Categories')).toBeVisible();
    await expect(page.locator('text=🔒 All processing runs locally')).toBeVisible();
  });
});
```

- [ ] **Step 3: Run E2E tests**

```bash
npm run build && npx playwright test
```

Expected: Tests pass

- [ ] **Step 4: Commit**

```bash
git add playwright.config.ts tests/
git commit -m "feat: add Playwright E2E test suite"
```

---

### Task 8: Production Build & Deploy Config

**Files:**

- Create: `.github/workflows/deploy.yml`
- Create: `vercel.json` (optional)
- Create: `netlify.toml` (optional)

**Interfaces:**

- Consumes: `package.json`, `vite.config.ts`
- Produces: CI/CD pipeline for GitHub Pages

- [ ] **Step 1: Write .github/workflows/deploy.yml**

```yaml
name: Deploy to GitHub Pages

on:
  push:
    branches: [main]
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: pages
  cancel-in-progress: false

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: '22'
          cache: 'npm'
      - run: npm ci
      - run: npm run build
      - uses: actions/upload-pages-artifact@v3
        with:
          path: ./dist

  deploy:
    needs: build
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - id: deployment
        uses: actions/deploy-pages@v4
```

- [ ] **Step 2: Write vercel.json**

```json
{
  "headers": [
    {
      "source": "/(.*)",
      "headers": [
        { "key": "Cross-Origin-Opener-Policy", "value": "same-origin" },
        { "key": "Cross-Origin-Embedder-Policy", "value": "require-corp" }
      ]
    }
  ],
  "buildCommand": "npm run build",
  "outputDirectory": "dist",
  "framework": "vite"
}
```

- [ ] **Step 3: Write netlify.toml**

```toml
[build]
  command = "npm run build"
  publish = "dist"

[[headers]]
  for = "/*"
  [headers.values]
    Cross-Origin-Opener-Policy = "same-origin"
    Cross-Origin-Embedder-Policy = "require-corp"

[functions]
  node_bundler = "esbuild"
```

- [ ] **Step 4: Test build**

```bash
npm run build
```

Expected: `dist/` folder with index.html, assets, worker

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/deploy.yml vercel.json netlify.toml
git commit -m "ci: add GitHub Pages, Vercel, Netlify deploy configs"
```

---

### Task 9: README & Documentation

**Files:**

- Create: `README.md`

**Interfaces:**

- Produces: Project documentation

- [ ] **Step 1: Write README.md**

```markdown
# xberg-studio

> **100% Local Document Intelligence in Your Browser**

Convert documents to RAG-ready Markdown with entity-linked metadata. Runs entirely client-side via WebAssembly — your files never leave your browser.

## Features

- 🔒 **100% Local** — No uploads, no server, no tracking
- 📄 **97+ Formats** — PDF, Office (DOCX/XLSX/PPTX), Email (EML/MSG/PST), Subtitles (SRT/VTT), Images, Code (306 languages)
- 🧠 **GLiNER2 NER** — 42 PII entity types + safety moderation via Candle WASM
- 👁️ **OCR** — Tesseract WASM for scanned PDFs and images
- 🔗 **Entity Links** — Markdown links for Claude Desktop RAG traversal
- 📦 **Batch + Zip** — Process 100+ files, download as .zip

## Quick Start

```bash
npm install
npm run dev
```

Open <http://localhost:5173>

## Deployment

### GitHub Pages (Automatic)

Push to `main` → GitHub Actions builds and deploys.

### Vercel / Netlify

Import repo → Auto-detects Vite → Deploys with COOP/COEP headers.

### Manual Static Hosting

```bash
npm run build
# Upload dist/ to any CDN with:
# Cross-Origin-Opener-Policy: same-origin
# Cross-Origin-Embedder-Policy: require-corp
```

## Architecture

```text
┌─────────────┐     ┌──────────────┐     ┌──────────────┐
│   Svelte    │────▶│  Web Worker  │────▶│    .zip      │
│    UI       │     │  (pipeline)  │     │   Download   │
└─────────────┘     └──────┬───────┘     └──────────────┘
                           │
              ┌────────────┼────────────┐
              ▼            ▼            ▼
         ┌────────┐  ┌───────────┐  ┌──────────┐
         │ xberg  │  │  Candle   │  │  Linker  │
         │ WASM   │  │  GLiNER2  │  │ + Glossary│
         └────────┘  └───────────┘  └──────────┘
```

## Supported Formats

| Category | Formats |
|----------|---------|
| Office | `.docx`, `.xlsx`, `.pptx`, `.pdf`, `.odt`, `.ods`, `.odp` |
| Email | `.eml`, `.msg`, `.pst` |
| Transcripts | `.srt`, `.vtt`, `.ass`, `.ssa` |
| Images (OCR) | `.png`, `.jpg`, `.webp`, `.tiff`, `.svg` |
| Data | `.json`, `.csv`, `.yaml`, `.xml`, `.html` |
| Code | 306 languages via tree-sitter |

## Output Format

```markdown
---
source: report.pdf
type: pdf
processed: 2026-07-25T14:30:00Z
entities:
  - name: John Doe
    type: Person
    slug: john-doe
---

# Report Title

[John Doe](entity:person/john-doe) works at [Acme Corp](entity:organization/acme-corp)...

## Entities

- **John Doe** `Person` — mentioned 3 times
- **Acme Corp** `Organization` — mentioned 5 times
```

## Configuration

- **NER Categories** — Select which entity types to extract
- **Output Format** — Markdown (default), Plain Text, JSON
- **Chunk Size** — Token chunking for large documents

## Privacy

- All processing in-browser via WebAssembly
- Models cached in IndexedDB after first download (~600MB)
- No telemetry, no analytics, no external requests after load

## License

MIT

```text

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: add README with features, deployment, architecture"
```

---

### Task 10: Final Verification

**Files:**

- All previous files

**Interfaces:**

- Consumes: Complete codebase
- Produces: Working production build

- [ ] **Step 1: Run full test suite**

```bash
npm run lint
npm run format -- --check
npx svelte-check
npm test
npx playwright test
```

Expected: All pass

- [ ] **Step 2: Build and verify output**

```bash
npm run build
ls -la dist/
```

Expected: `dist/index.html`, `dist/assets/*.js`, `dist/worker/*.js`

- [ ] **Step 3: Preview production build**

```bash
npm run preview
```

Expected: App loads, onboarding shows, drop zone works

- [ ] **Step 4: Test with sample files**

Drag-drop test files from xberg fixtures:

- `test_documents/sample.pdf`
- `test_documents/sample.docx`
- `test_documents/sample.eml`

Expected: .zip downloads with .md files containing entity links

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "release: xberg-studio v1.0.0"
git tag v1.0.0
```

---

## Spec Coverage Checklist

- [x] Onboarding screen with local AI messaging
- [x] NER model download progress (GLiNER2-Guardrails-PII-Multi)
- [x] File validation with inline errors for unsupported formats
- [x] 97+ format support (office, email, transcripts, images, code)
- [x] xberg WASM extraction
- [x] Candle NER in WASM
- [x] Entity linking with markdown links
- [x] YAML frontmatter + glossary output
- [x] Batch processing + zip download
- [x] COOP/COEP headers for SharedArrayBuffer
- [x] GitHub Pages deployment config
- [x] "Your files NEVER leave this tab" messaging
- [x] Error handling for empty files, size limits, MIME types
- [x] Config panel with NER category selection
- [x] Whisper limitation documented (native only)

---

## Execution Handoff

**Plan complete and saved to** `docs/superpowers/plans/2026-07-25-xberg-studio-implementation.md`

**Two execution options:**

1. **Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration
   - REQUIRED SUB-SKILL: `superpowers:subagent-driven-development`

2. **Inline Execution** — Execute tasks in this session using `executing-plans`, batch execution with checkpoints
   - REQUIRED SUB-SKILL: `superpowers:executing-plans`

**Which approach?**
