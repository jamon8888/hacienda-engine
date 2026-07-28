# xberg-studio — WASM Web App for RAG-Ready Document Conversion

**Date:** 2026-07-25  
**Status:** Draft  
**Author:** opencode

## Overview

A client-side Svelte web app that converts documents to RAG-ready Markdown files with entity-linked metadata using xberg WASM for extraction, GLiNER2-Guardrails-PII-Multi via Candle for NER, and tesseract-wasm for OCR. Supports 97+ formats including office documents (.docx, .xlsx, .pptx), email (.eml, .msg, .pst), transcripts (.srt, .vtt), PDFs, images, and code. Output is downloadable as a .zip of `.md` files with YAML frontmatter and entity glossaries, optimized for Claude Desktop RAG workflows.

## Goals

1. Convert any document format to clean Markdown using xberg WASM
2. Support major office formats: .docx, .xlsx, .pptx, .pdf, .odt, .ods, .odp
3. Support email formats: .eml, .msg, .pst (headers, body, attachments)
4. Support transcripts: .srt, .vtt (subtitle extraction); audio/video via Whisper — **native only (requires xberg CLI/REST API, not in WASM)**
5. Extract named entities (people, orgs, places, PII) using GLiNER2-Guardrails-PII-Multi via Candle in WASM
6. Link entities inline as markdown links for Claude Desktop traversal
7. Output RAG-ready file system with metadata and glossaries
8. Run entirely client-side — no server required
9. Deployable to static hosting (GitHub Pages, etc.)

## Architecture

### Stack

| Component | Technology |
|-----------|------------|
| UI | Svelte + Vite + TypeScript |
| Extraction | xberg WASM (CDN) |
| NER | Candle NER in WASM + `fastino/GLiNER2-Guardrails-PII-Multi` |
| OCR | `tesseract-wasm` npm package (multi-language fallback) |
| Packaging | JSZip (CDN) |
| Deployment | Static site (any CDN/host) |

### Architecture Diagram

```text
┌─────────────────────────────────────────────────────────┐
│  Svelte UI (main thread)                                │
│  ┌─────────────┐  ┌──────────┐  ┌───────────────────┐  │
│  │  Drag-Drop   │  │ Progress │  │ Config Panel      │  │
│  │  Zone        │  │ Bars     │  │ (NER categories,  │  │
│  │              │  │          │  │  output format)   │  │
│  └──────┬──────┘  └────┬─────┘  └───────────────────┘  │
│         │              │                                │
│         ▼              ▼                                │
│  ┌─────────────────────────────────────────────────┐    │
│  │              Web Worker                         │    │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────────┐  │    │
│  │  │ xberg    │→ │Candle    │→ │ Entity       │  │    │
│  │  │ WASM     │  │ GLiNER2  │  │ Linker       │  │    │
│  │  │ extract  │  │ NER      │  │ + Glossary   │  │    │
│  │  └──────────┘  └──────────┘  └──────────────┘  │    │
│  └─────────────────────────────────────────────────┘    │
│         │                                                │
│         ▼                                                │
│  ┌─────────────┐                                         │
│  │  .zip       │                                         │
│  │  Download   │                                         │
│  └─────────────┘                                         │
└─────────────────────────────────────────────────────────┘
```

### Streaming Pipeline

Three-stage streaming pipeline in a Web Worker:

**Stage 1: Extract (xberg WASM)**

- File arrives as `ArrayBuffer` or `ReadableStream`
- xberg WASM `extract()` produces markdown chunks
- For large files (>10MB): process in page/chunk boundaries
- **Email handling:** Extracts headers, body (HTML→Markdown), attachments metadata
- **Transcript handling:** Extracts timestamped text segments from .srt/.vtt

**Stage 2: NER (Candle GLiNER2)**

- `fastino/GLiNER2-Guardrails-PII-Multi` model loaded via `Gliner2Candle::from_bytes()`
- Model weights fetched from CDN, cached in browser (IndexedDB)
- F16 dtype on wasm32 for memory efficiency (~600MB)
- Supports 42 PII entity types + safety moderation
- Zero-shot: pass any entity labels at inference time

**Stage 3: Link + Glossary**

- String-replace entity spans with markdown links
- Slugify function: `Acme Corp` → `acme-corp`
- Track entity frequency for glossary ranking
- Deduplicate: same entity at different offsets → same link

### Worker Message Protocol

```typescript
// main → worker
{ type: 'init', config: { nerCategories: [...] } }
{ type: 'process', files: [{ name, bytes, mimeType }] }
{ type: 'cancel' }

// worker → main
{ type: 'progress', file: string, stage: 'extract'|'ner'|'link', percent: number }
{ type: 'file-complete', name: string, markdown: string, entities: Entity[] }
{ type: 'batch-complete', zip: Blob }
{ type: 'error', file: string, message: string }
```

## Output Format

Each processed file becomes a `.md` file:

```markdown
---
source: report.pdf
type: pdf
processed: 2026-07-25T14:30:00Z
entities:
  - name: John Doe
    type: Person
    slug: john-doe
  - name: Acme Corp
    type: Organization
    slug: acme-corp
  - name: New York
    type: Location
    slug: new-york
---

# Report Title

Content here with [John Doe](entity:person/john-doe) working at 
[Acme Corp](entity:organization/acme-corp) in [New York](entity:location/new-york)...

## Entities

- **John Doe** `Person` — mentioned 3 times
- **Acme Corp** `Organization` — mentioned 5 times  
- **New York** `Location` — mentioned 2 times
```

### Entity Link Format

- Links: `[Entity Name](entity:type/slug)` e.g., `[Acme Corp](entity:organization/acme-corp)`
- Case-insensitive: `entity:Person/John-Doe` and `entity:person/john-doe` both resolve
- Glossary sorted by frequency (most mentioned first)
- Duplicate entities merged with count

### Zip Structure

```text
output.zip
├── report.md
├── document.md
├── presentation.md
└── _manifest.json    ← optional: index of all files + entity counts
```

## NER Configuration

### Model: `fastino/GLiNER2-Guardrails-PII-Multi`

| Property | Value |
|----------|-------|
| Parameters | 0.3B (300M encoder) |
| Architecture | DeBERTa-v2 + span scoring heads |
| Format | Safetensors (1.23 GB F32) |
| WASM runtime | Candle (F16, ~600MB) |
| Entity types | 42 PII types + safety moderation |
| Languages | EN, FR, ES, DE, IT, PT, NL |
| License | Apache 2.0 |

### Supported Entity Categories

| Group | Labels |
|-------|--------|
| Person | `person`, `full_name`, `first_name`, `last_name` |
| Contact | `email`, `phone_number`, `address`, `street_address` |
| Location | `city`, `state_or_region`, `postal_code`, `country` |
| Organization | `organization`, `company` |
| Financial | `credit_card`, `bank_account`, `ssn` |
| Temporal | `date_of_birth`, `date` |

### UI Config Panel

```svelte
<ConfigPanel>
  NER Categories: [x] Person [x] Organization [x] Location
                  [x] Email [x] Phone [ ] Date [ ] Money
  Output Format:  [Markdown ▾]
  Chunk Size:     [1000] tokens
</ConfigPanel>
```

## OCR Configuration

### Fallback: `tesseract-wasm` npm package

- Multi-language support (eng, fra, deu, spa, etc.)
- Tessdata fetched on demand from CDN
- Used when native Tesseract WASM is unavailable or multi-language is needed

### Usage

```typescript
import { enableOcr } from "@xberg-io/xberg-wasm";

await enableOcr(); // registers fallback OCR backend

const result = await extract(input, {
  ocr: { backend: "tesseract-wasm", language: ["eng"] }
});
```

## UI Design

### Onboarding Screen (First Visit)

On first load, show a full-screen onboarding modal before the drag-drop zone:

```text
┌─────────────────────────────────────────────────────────┐
│                                                         │
│      🔒  xberg-studio — 100% Local AI in Your Browser  │
│                                                         │
│      Your files NEVER leave this tab.                   │
│      All processing runs locally via WebAssembly.       │
│                                                         │
│      ┌─────────────────────────────────────────────┐   │
│      │  Preparing local models...                  │   │
│      │  ████████████░░░░░░░░░░  Downloading (45%)   │   │
│      │  • xberg WASM engine        ✓ Cached        │   │
│      │  • GLiNER2-Guardrails-PII   ↓ 600 MB        │   │
│      │  • Tesseract OCR (tessdata)   ✓ Bundled      │   │
│      └─────────────────────────────────────────────┘   │
│                                                         │
│      [Cancel]                              [Continue]  │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

**Behavior:**

- Shows on first visit (detected via `localStorage` flag)
- Downloads GLiNER2 model (~600 MB F16) to IndexedDB cache
- Pre-warms xberg WASM and tesseract-wasm
- Progress bar with per-asset status
- "Continue" disabled until all assets cached
- User can cancel → falls back to on-demand loading

### Main Screen (After Onboarding)

Minimal Svelte app with drag-drop zone and hidden config:

```text
┌─────────────────────────────────────────────────────────┐
│  xberg-studio                           [⚙ Config]     │
├─────────────────────────────────────────────────────────┤
│                                                         │
│         ┌───────────────────────────────────┐           │
│         │                                   │           │
│         │     Drop files here               │           │
│         │     or click to browse            │           │
│         │                                   │           │
│         │     PDF, DOCX, images, etc.       │           │
│         └───────────────────────────────────┘           │
│                                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │ Processing... report.pdf                        │    │
│  │ ████████████░░░░░░░░░░░  Extract (60%)          │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  [Download .zip]                                        │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### Config Panel (Collapsible)

```svelte
<ConfigPanel>
  <h3>🔒 All processing runs locally in your browser</h3>
  NER Categories: [x] Person [x] Organization [x] Location
                  [x] Email [x] Phone [ ] Date [ ] Money
  Output Format:  [Markdown ▾]
  Chunk Size:     [1000] tokens
</ConfigPanel>
```

### State Management

- `files: File[]` — input files
- `progress: Map<string, { stage, percent }>` — per-file progress
- `results: Map<string, { markdown, entities }>` — completed results
- `config: { nerCategories, outputFormat, chunkSize }` — user settings
- `onboardingComplete: boolean` — tracks first-visit onboarding

## Error Handling

| Error | Recovery |
|-------|----------|
| WASM load fails | Show error, suggest CDN fallback URL |
| NER model download fails | Retry once, then continue without NER |
| OCR model download fails | Continue without OCR, mark files as "OCR skipped" |
| File too large (>50MB) | Show warning, suggest splitting |
| **Unsupported format** | **Show inline error with supported formats list, reject file** |
| **Invalid MIME type** | **Show error: "File type not supported. Allowed: PDF, Office, Email, Images, Subtitles, Code"** |
| **Empty file (0 bytes)** | **Show error: "File is empty. Please select a valid file."** |
| Out of memory | Suggest reducing chunk size in config |

### File Validation on Upload

```typescript
const SUPPORTED_MIME_TYPES = [
  'application/pdf',
  'application/vnd.openxmlformats-officedocument.wordprocessingml.document', // .docx
  'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', // .xlsx
  'application/vnd.openxmlformats-officedocument.presentationml.presentation', // .pptx
  'application/msword', // .doc
  'application/vnd.ms-excel', // .xls
  'application/vnd.ms-powerpoint', // .ppt
  'application/vnd.oasis.opendocument.text', // .odt
  'application/vnd.oasis.opendocument.spreadsheet', // .ods
  'application/vnd.oasis.opendocument.presentation', // .odp
  'message/rfc822', // .eml
  'application/vnd.ms-outlook', // .msg
  'application/vnd.ms-pki.stl', // .pst
  'text/plain', 'text/csv', 'text/markdown', 'text/html',
  'application/json', 'application/xml',
  'image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/tiff', 'image/svg+xml',
  'text/srt', 'text/vtt',
  // ... plus code MIME types
];

function validateFile(file: File): { valid: boolean; error?: string } {
  if (file.size === 0) return { valid: false, error: 'File is empty' };
  if (file.size > 50 * 1024 * 1024) return { valid: false, error: 'File too large (>50MB)' };
  if (!SUPPORTED_MIME_TYPES.some(t => file.type.startsWith(t.split('/')[0]))) {
    return { valid: false, error: `Unsupported file type: ${file.type || file.name}` };
  }
  return { valid: true };
}
```

### Inline Error Display

```text
┌─────────────────────────────────────────────────────────┐
│  ❌  Unsupported file: presentation.key                 │
│                                                         │
│  Allowed formats: PDF, DOCX, XLSX, PPTX, ODT, ODS,     │
│  ODP, EML, MSG, PST, PNG, JPG, TIFF, SVG, SRT, VTT,   │
│  and 306 programming languages.                        │
│                                                         │
│  [Dismiss]                                              │
└─────────────────────────────────────────────────────────┘
```

## Dependencies

| Package | Version | Purpose |
|---------|---------|---------|
| `@xberg-io/xberg-wasm` | latest | Document extraction (includes Candle NER) |
| `tesseract-wasm` | latest | OCR fallback for multi-language support |
| `jszip` | latest | Zip file creation |
| `svelte` | 5.x | UI framework |
| `vite` | 6.x | Build tool |

### Model Assets (fetched from HuggingFace CDN)

| Asset | Source | Size |
|-------|--------|------|
| GLiNER2-Guardrails-PII-Multi | `fastino/GLiNER2-Guardrails-PII-Multi` | ~600MB (F16) |
| Tesseract tessdata | `tesseract-wasm` CDN | ~4MB per language |

## Testing Strategy

- **Unit tests:** Entity linking, slug generation, glossary builder
- **Integration tests:** Full pipeline with test fixtures (PDF, DOCX, PNG)
- **E2E tests:** Playwright tests for drag-drop, progress, download
- **Performance:** Benchmark extraction + NER on 1MB, 10MB, 50MB files

## Deployment

- Static site — works on any CDN or GitHub Pages
- All assets loaded from CDN (xberg WASM, NER model, tesseract-wasm)
- No server required — 100% client-side
- **COOP/COEP headers required** for SharedArrayBuffer (Web Worker support):
  - GitHub Pages: `_headers` file or meta tags
  - Netlify/Vercel: `netlify.toml` or `vercel.json` headers config
  - Custom server: `Cross-Origin-Opener-Policy: same-origin` + `Cross-Origin-Embedder-Policy: require-corp`

## Supported File Formats

The app supports all 97 formats that xberg WASM handles, including:

### Office Documents

| Category | Formats |
|----------|---------|
| **Word Processing** | `.docx`, `.docm`, `.doc`, `.dotx`, `.dotm`, `.dot`, `.odt`, `.pages` |
| **Spreadsheets** | `.xlsx`, `.xlsm`, `.xlsb`, `.xls`, `.xla`, `.xlam`, `.xltm`, `.xltx`, `.xlt`, `.ods`, `.numbers` |
| **Presentations** | `.pptx`, `.pptm`, `.ppt`, `.ppsx`, `.potx`, `.potm`, `.pot`, `.odp`, `.key` |
| **PDF** | `.pdf` (text + OCR for scanned) |
| **eBooks** | `.epub`, `.fb2` |

### Email & Archives

| Category | Formats |
|----------|---------|
| **Email** | `.eml`, `.msg`, `.pst` — full headers, body (HTML/plain), attachments |
| **Archives** | `.zip`, `.tar`, `.tgz`, `.gz`, `.7z` — recursive extraction |

### Transcripts & Subtitles

| Category | Formats |
|----------|---------|
| **Subtitle** | `.srt`, `.vtt`, `.ass`, `.ssa` — timestamped text extraction |
| **Transcription** | Audio/video via Whisper ONNX (`.mp3`, `.m4a`, `.wav`, `.webm`, `.mp4`) — **native only, not available in WASM** |

### Images (OCR-Enabled)

| Category | Formats |
|----------|---------|
| **Raster** | `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.bmp`, `.tiff`, `.tif` |
| **Vector** | `.svg` |

### Web & Data

| Category | Formats |
|----------|---------|
| **Markup** | `.html`, `.htm`, `.xhtml`, `.xml` |
| **Structured** | `.json`, `.yaml`, `.yml`, `.toml`, `.csv`, `.tsv` |
| **Text** | `.txt`, `.md`, `.markdown`, `.djot`, `.mdx`, `.rst`, `.org`, `.rtf` |

### Code

| Category | Formats |
|----------|---------|
| **Source** | 306 programming languages via tree-sitter |

## Success Criteria

1. ✅ Drag-drop files → get .zip of .md files with entity links
2. ✅ NER extracts people, orgs, locations, emails, phones
3. ✅ Entity links work in Claude Desktop RAG
4. ✅ OCR works on scanned PDFs and images
5. ✅ Email files (.eml, .msg, .pst) extract headers, body, and attachments
6. ✅ Transcripts (.srt, .vtt) extract timestamped text
7. ✅ Office files (.docx, .xlsx, .pptx) extract full content with structure
8. ✅ Processes 100+ files without crashing
9. ✅ Deploys to GitHub Pages with zero config
