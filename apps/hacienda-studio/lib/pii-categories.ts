import type { PiiCategoryWire } from "./pii-engine";

export const ADDABLE_CATEGORIES = [
  "person",
  "email",
  "phone",
  "address",
  "organization",
  "other",
] as const;

export type AddableCategory = (typeof ADDABLE_CATEGORIES)[number];

// Before `as const`, `ADDABLE_CATEGORIES` was a plain `string[]`, so
// `ADDABLE_CATEGORIES[0]` typed as `string | undefined` under `noUncheckedIndexedAccess` —
// RedactedEditor/MarkdownEditor seeded `useState` with that value directly, which is a type
// error anywhere `spliceRedaction`/`onAddFinding` require a plain `string`. `as const` turns
// the array into a fixed-length readonly tuple, so indexing it with a literal `0` is exact
// (no `undefined` in the type) — but exporting the default explicitly, rather than having
// every call site reach for `[0]`, keeps the "what's the default category" decision in one
// place. RedactedEditor/MarkdownEditor initialize `category` with this value.
export const DEFAULT_ADDABLE_CATEGORY: AddableCategory = ADDABLE_CATEGORIES[0];

/** Narrows a `<select>`'s `e.target.value` (always `string`) without an `as` assertion. */
export function isAddableCategory(value: string): value is AddableCategory {
  for (const c of ADDABLE_CATEGORIES) {
    if (c === value) return true;
  }
  return false;
}

// ---------------------------------------------------------------------------
// Task 1 — 27 catégories du screenshot → wire PiiCategoryWire
// ---------------------------------------------------------------------------

export interface PiiCategoryDef {
  readonly code: string;
  readonly label: string;
  readonly wire: PiiCategoryWire;
  readonly defaultSelected: boolean;
}

export interface DetectionCategoryGroup {
  readonly title: string;
  readonly categories: readonly PiiCategoryDef[];
}

/**
 * 5 groupes → 27 catégories exactes du screenshot.
 * 14 cochées par défaut (PR, MAIL, PHON, CIE, CID, ACT, ADR, LOC, CP, CARD, IBAN, URL, FILE, REF).
 * Wire shapes mirror `hacienda-core/src/pii/types.rs:PiiCategory` serde representation:
 * - bare snake_case string for unit variants
 * - `{ custom: "<Label>" }` for Custom(String)
 */
export const DETECTION_CATEGORIES: readonly DetectionCategoryGroup[] = [
  {
    title: "DONNÉES PERSONNELLES",
    categories: [
      { code: "PR", label: "Nom de personne", wire: "person", defaultSelected: true },
      { code: "MAIL", label: "Adresse e-mail", wire: "email", defaultSelected: true },
      { code: "PHON", label: "Numéro de téléphone", wire: "phone_number", defaultSelected: true },
      { code: "AGE", label: "Âge", wire: { custom: "Age" }, defaultSelected: false },
      { code: "TR", label: "Titre / Civilité", wire: { custom: "Title" }, defaultSelected: false },
    ],
  },
  {
    title: "DONNÉES D'ENTREPRISES",
    categories: [
      { code: "CIE", label: "Nom d'entreprise", wire: "organization", defaultSelected: true },
      { code: "CID", label: "Identifiant entreprise", wire: { custom: "CID" }, defaultSelected: true },
      { code: "ACT", label: "Activité / Secteur", wire: { custom: "ACT" }, defaultSelected: true },
      { code: "PROD", label: "Produit / Service", wire: { custom: "Product" }, defaultSelected: false },
    ],
  },
  {
    title: "DONNÉES DE LOCALISATION",
    categories: [
      { code: "ADR", label: "Adresse postale", wire: "address", defaultSelected: true },
      { code: "LOC", label: "Commune / Localité", wire: { custom: "Commune" }, defaultSelected: true },
      { code: "CP", label: "Code postal", wire: { custom: "PostalCode" }, defaultSelected: true },
      { code: "GEO", label: "Géolocalisation", wire: { custom: "Geolocation" }, defaultSelected: false },
    ],
  },
  {
    title: "DONNÉES FINANCIÈRES",
    categories: [
      { code: "CARD", label: "Carte bancaire", wire: "credit_card", defaultSelected: true },
      { code: "IBAN", label: "IBAN", wire: "iban", defaultSelected: true },
      { code: "BIC", label: "BIC / SWIFT", wire: "swift_bic", defaultSelected: false },
    ],
  },
  {
    title: "DONNÉES DIVERSES",
    categories: [
      { code: "URL", label: "URL / Lien", wire: "url", defaultSelected: true },
      { code: "FILE", label: "Nom de fichier / Dossier", wire: { custom: "Filename" }, defaultSelected: true },
      { code: "REF", label: "Référence", wire: { custom: "Reference" }, defaultSelected: true },
      { code: "IP", label: "Adresse IP", wire: "ip_address", defaultSelected: false },
      { code: "DT", label: "Date", wire: { custom: "Date" }, defaultSelected: false },
      { code: "TEMP", label: "Date temporelle", wire: { custom: "Temporal" }, defaultSelected: false },
      { code: "ORG", label: "Organisation diverse", wire: { custom: "Organization" }, defaultSelected: false },
      { code: "NIR", label: "NIR / Sécurité sociale", wire: { custom: "NIR" }, defaultSelected: false },
      { code: "SIREN", label: "Numéro SIREN", wire: { custom: "SIREN" }, defaultSelected: false },
      { code: "SIRET", label: "Numéro SIRET", wire: { custom: "SIRET" }, defaultSelected: false },
      { code: "TVA", label: "Numéro TVA intracommunautaire", wire: { custom: "TVA" }, defaultSelected: false },
    ],
  },
];

/**
 * Map badge code (PR, MAIL, ...) → PiiCategoryWire shape consumed by `hacienda-wasm` / `hacienda-core`.
 * Returns undefined for unknown codes rather than throwing, so callers can guard.
 */
export function categoryToWire(code: string): PiiCategoryWire | undefined {
  for (const group of DETECTION_CATEGORIES) {
    for (const cat of group.categories) {
      if (cat.code === code) return cat.wire;
    }
  }
  return undefined;
}

/**
 * Totaux utiles pour la modale (14 sur 27).
 */
export const TOTAL_CATEGORIES: number = (() => {
  let n = 0;
  for (const g of DETECTION_CATEGORIES) n += g.categories.length;
  return n;
})();

export const DEFAULT_SELECTED_CODES: readonly string[] = (() => {
  const out: string[] = [];
  for (const g of DETECTION_CATEGORIES) {
    for (const c of g.categories) {
      if (c.defaultSelected) out.push(c.code);
    }
  }
  return out;
})();
