import type { PiiEntity } from "./pii-engine";

export interface FileInput {
  name: string;
  bytes: ArrayBuffer;
  type: string;
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
  entities: Entity[];
  /**
   * Track F1: the PII findings for this document, so the UI can render a reveal panel
   * against them. `redact_template` is either the format-preserving mask (`"[EMAIL]"`) or,
   * under `redactionMode: "pseudonymize"`, the reversible token actually spliced into
   * `markdown` — same field, so the panel never has to know which mode produced it.
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
  stage: "extract" | "ner" | "pii" | "link" | "complete" | "error";
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
};
