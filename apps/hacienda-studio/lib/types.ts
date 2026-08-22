import type { PiiEntity } from "./pii-engine";

export interface FileInput {
  name: string;
  bytes: ArrayBuffer;
  type: string;
  /**
   * Set by `lib/asset-loader.ts`'s `checkPdfPageSafety` for PDFs whose page count makes
   * full-document OCR a memory risk (xberg-wasm's OCR batch-size throttle,
   * `adapt_batch_size_to_memory`, is a no-op in the browser build — see that function's
   * `get_available_memory()`, which only implements the syscall for native Linux/macOS
   * targets and always returns 0 on wasm32). Native text extraction still runs; only the
   * scanned-page OCR fallback is skipped for this file.
   */
  disableOcr?: boolean;
  /**
   * Set by `lib/asset-loader.ts`'s `checkPdfPageSafety` for PDFs — the pdfium page count
   * it already computed, reused by `lib/pdf-liteparse.ts` to decide `parse()` vs
   * `openBatchSession()` without a second page-count pass. `undefined` for non-PDFs, or
   * if the probe failed and `checkPdfPageSafety` failed open.
   */
  pdfPageCount?: number;
}

export interface Entity {
  name: string;
  type: string;
  slug: string;
  count: number;
  spans: Array<{ start: number; end: number }>;
  vertical?: string;
  sector?: string;
  roles?: string[];
}

export interface ProcessedFile {
  name: string;
  markdown: string;
  /**
   * Track F3: the extracted/transcribed text *before* `renderAnnotatedMarkdown` splices in
   * entity links and PII redaction, and before frontmatter is prepended. `piiFindings`'
   * `start`/`end` are offsets into exactly this string, not into `markdown` — every splice
   * that produces `markdown` shifts what those offsets would point at. The editor (Track
   * F3) needs this pairing to highlight the right text; `markdown` alone can't provide it.
   */
  rawMarkdown: string;
  entities: Entity[];
  /**
   * Track F1: the PII findings for this document, so the UI can render a reveal panel
   * against them. `redact_template` is either the format-preserving mask (`"[EMAIL]"`) or,
   * under `redactionMode: "pseudonymize"`, the reversible token actually spliced into
   * `markdown` — same field, so the panel never has to know which mode produced it. Spans
   * (`start`/`end`) are offsets into `rawMarkdown`, not `markdown` — see above.
   * Empty when `enablePiiDetection` is off.
   */
  piiFindings: PiiEntity[];
  frontmatter: {
    source: string;
    type: string;
    processed: string;
    /** Count from the `hacienda-wasm` PII engine; 0 when detection is disabled. */
    piiEntitiesFound: number;
    entities: Array<{
      name: string;
      type: string;
      slug: string;
      vertical?: string;
      sector?: string;
      roles?: string[];
    }>;
  };
}

export interface ProgressUpdate {
  file: string;
  /**
   * `"queued"` is set before a file's stages begin (5-screen UI flow's processing queue
   * screen). `"transcribe"` is posted from `App.tsx` directly, not relayed through the
   * worker's `postProgress` — the worker only ever emits `progress` messages for stages it
   * runs itself, but transcription now runs on the main thread (see
   * `worker/transcribe-bridge.ts`'s header for why), and `App.tsx` already owns the
   * `progress` state this feeds. Its `percent`/`message` come from `WhisperBridge`'s
   * `onProgress` (resample and transcribe phases), so this is real progress, not a fixed
   * placeholder — same guarantee every other stage here already gives the UI.
   */
  stage: "queued" | "extract" | "transcribe" | "ner" | "pii" | "link" | "complete" | "error" | "wasm-load";
  percent: number;
  message?: string;
}

/**
 * Mirrors the `WasmEntityCategory` enum. The engine rejects the whole NER
 * result when a bridge returns a name outside this set — the failure surfaces
 * as an opaque "Unknown error" against the document — so the vocabulary is
 * pinned in the type system rather than left as `string`.
 */
export type NerCategory =
  | "person"
  | "organization"
  | "location"
  | "date"
  | "time"
  | "money"
  | "percent"
  | "email"
  | "phone"
  | "url";

export interface AppConfig {
  nerCategories: NerCategory[];
  outputFormat: "markdown" | "plain" | "json";
  chunkSize: number;
  enableTranscription: boolean;
  transcriptionModel:
    "tiny.en" | "tiny" | "base.en" | "base" | "small.en" | "small";
  transcriptionLanguage: string;
  translateToEnglish: boolean;
  enablePiiDetection: boolean;
  redactPiiInOutput: boolean;
  enabledVerticals: ("m&a" | "financial_services" | "shared")[];
  /**
   * Track F1/F2: `"mask"` is the existing format-preserving `[EMAIL]`-style redaction.
   * `"pseudonymize"` replaces each finding with a reversible `lib/pseudonymize.ts` token
   * instead — only takes effect when `redactPiiInOutput` is also set, and only when
   * `pseudonymPassphrase` is non-empty (an empty passphrase silently falls back to mask
   * rather than producing tokens no one can derive the key to reveal).
   */
  redactionMode: "mask" | "pseudonymize";
  /** Session-only. Never persisted, never sent anywhere but this tab's own worker. */
  pseudonymPassphrase: string;
  /** Travels with a token (`[CATEGORY:key_id:...]`) so the same passphrase, entered again
   * later, can be told which id to derive against — matters once key rotation exists. */
  pseudonymKeyId: string;
  /**
   * Which engine extracts PDF text. `"liteparse"` (default) routes PDFs through
   * `@llamaindex/liteparse-wasm` (PDFium-backed) — see `docs/superpowers/specs/
   * 2026-08-22-liteparse-pdf-extraction-design.md` for why: xberg-wasm's `pdf_oxide`
   * backend has open crash-class bugs and no bounded-memory large-document path, both
   * fixed by switching engines for PDF specifically. `"xberg"` is kept as an internal
   * escape hatch (regression testing, a fast rollback if LiteParse regresses on some
   * document class) but is intentionally not exposed in ConfigPanel — this was a
   * config-gated rollout while under evaluation; it's the default now. Non-PDF formats
   * always go through xberg-wasm regardless of this flag — LiteParse's wasm build has no
   * DOCX/XLSX/PPTX support (that requires a native LibreOffice subprocess, not available
   * in-browser).
   */
  pdfEngine: "xberg" | "liteparse";
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
  nerCategories: ["person", "organization", "location", "email", "phone"],
  outputFormat: "markdown",
  chunkSize: 1000,
  enableTranscription: false,
  transcriptionModel: "tiny.en",
  transcriptionLanguage: "auto",
  translateToEnglish: false,
  enablePiiDetection: true,
  redactPiiInOutput: false,
  enabledVerticals: ["m&a", "financial_services", "shared"],
  redactionMode: "mask",
  pseudonymPassphrase: "",
  pseudonymKeyId: "session",
  pdfEngine: "liteparse",
};
