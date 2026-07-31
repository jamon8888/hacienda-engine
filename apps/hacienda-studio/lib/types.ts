export interface FileInput {
  name: string;
  bytes: ArrayBuffer;
  type: string;
  /**
   * Vault-relative path this file was selected with (Track I1) — `File.name`
   * for a flat selection, or `File.webkitRelativePath` when selected via a
   * folder input, e.g. `"contracts/2024/nda.pdf"`. Drives the output path
   * under `documents/` so a folder's structure survives into the export,
   * and doubles as the unique key progress tracking keys on, since two
   * files in different subfolders can share a basename.
   */
  relativePath: string;
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
  transcriptionModel: "tiny.en" | "tiny" | "base.en" | "base" | "small.en" | "small";
  transcriptionLanguage: string;
  translateToEnglish: boolean;
  enablePiiDetection: boolean;
  redactPiiInOutput: boolean;
  enabledVerticals: ("m&a" | "financial_services" | "shared")[];
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
};
