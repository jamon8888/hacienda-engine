import type { ProcessedFile } from "./types";
import type { ConversionMode } from "./conversion";
import { DETECTION_CATEGORIES } from "./pii-categories";

export type TreatmentMode = "pseudonymize" | "anonymize";

export interface Session {
  readonly id: string;
  readonly name: string;
  readonly createdAt: Date;
  readonly files: ProcessedFile[];
  readonly detectionSelection: Set<string>;
  readonly treatmentMode: TreatmentMode;
  readonly conversionMode: ConversionMode;
}

function generateId(): string {
  const maybeCrypto = globalThis.crypto;
  if (
    maybeCrypto !== undefined &&
    typeof maybeCrypto.randomUUID === "function"
  ) {
    return maybeCrypto.randomUUID();
  }
  // Fallback for non-secure contexts / older runtimes — not cryptographically strong
  // but sufficient for UI key uniqueness; never used across FFI or persisted as auth.
  const randomPart = Math.random().toString(36).slice(2, 10);
  const timePart = Date.now().toString(36);
  return `session-${timePart}-${randomPart}`;
}

/**
 * Crée une nouvelle session avec les 14 catégories cochées par défaut
 * (PR, MAIL, PHON, CIE, CID, ACT, ADR, LOC, CP, CARD, IBAN, URL, FILE, REF).
 * `treatmentMode` par défaut `pseudonymize` (actif dans le screenshot),
 * `conversionMode` par défaut `markdown` (Format Markdown actif).
 */
export function createSession(name: string): Session {
  const defaults = new Set<string>();
  for (const group of DETECTION_CATEGORIES) {
    for (const cat of group.categories) {
      if (cat.defaultSelected) {
        defaults.add(cat.code);
      }
    }
  }
  return {
    id: generateId(),
    name,
    createdAt: new Date(),
    files: [],
    detectionSelection: defaults,
    treatmentMode: "pseudonymize",
    conversionMode: "markdown",
  };
}
