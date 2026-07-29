import type { NerCategory } from "./types";

export interface BridgeEntity {
  category: NerCategory;
  text: string;
  start: number;
  end: number;
  confidence: number;
}

const FRENCH_MONTHS =
  "janvier|f[ée]vrier|mars|avril|mai|juin|juillet|ao[uû]t|septembre|octobre|novembre|d[ée]cembre";
const ENGLISH_MONTHS =
  "january|february|march|april|may|june|july|august|september|october|november|december";

const EMAIL_PATTERN = /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b/g;
const PHONE_PATTERN = /(\+?\d{1,3}[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}/g;
const URL_PATTERN = /https?:\/\/[^\s]+/g;
/**
 * compromise's date extraction lives in the separate compromise-dates plugin,
 * so `doc.dates()` is undefined here and calling it threw for every document
 * with the category enabled. This covers the ISO and French numeric/long forms
 * that appear in the target corpus, which the English-only plugin would not
 * have handled anyway.
 */
const DATE_PATTERN = new RegExp(
  [
    "\\b\\d{4}-\\d{2}-\\d{2}\\b",
    "\\b\\d{1,2}[/.]\\d{1,2}[/.]\\d{2,4}\\b",
    `\\b\\d{1,2}(?:er)? (?:${FRENCH_MONTHS}|${ENGLISH_MONTHS}) \\d{4}\\b`,
  ].join("|"),
  "gi",
);

function collectPattern(
  entities: BridgeEntity[],
  category: NerCategory,
  text: string,
  pattern: RegExp,
  confidence: number,
): void {
  pattern.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    entities.push({
      category,
      text: match[0],
      start: match.index,
      end: match.index + match[0].length,
      confidence,
    });
  }
}

/**
 * compromise reports matched terms without offsets. Recording `start: 0, end:
 * term.length` — as this bridge used to — makes the caller splice entity links
 * over the opening characters of the document instead of over the mention, so
 * locate each occurrence for real.
 */
function collectTerms(
  entities: BridgeEntity[],
  category: NerCategory,
  text: string,
  terms: string[],
  confidence: number,
): void {
  for (const term of terms) {
    const trimmed = term.trim();
    if (trimmed.length === 0) continue;
    let from = text.indexOf(trimmed);
    while (from !== -1) {
      entities.push({
        category,
        text: trimmed,
        start: from,
        end: from + trimmed.length,
        confidence,
      });
      from = text.indexOf(trimmed, from + trimmed.length);
    }
  }
}

/**
 * The NER bridge the engine calls back into. Every category offered in the UI
 * must be handled here and must name a variant of the engine's entity
 * vocabulary — the engine rejects the whole result otherwise, which the app
 * reports as an opaque "Unknown error" against the document.
 */
export async function extractEntities(
  text: string,
  categories: string[],
): Promise<BridgeEntity[]> {
  const wanted = new Set(categories);
  const entities: BridgeEntity[] = [];

  if (wanted.has("email")) {
    collectPattern(entities, "email", text, EMAIL_PATTERN, 0.9);
  }
  if (wanted.has("phone")) {
    collectPattern(entities, "phone", text, PHONE_PATTERN, 0.8);
  }
  if (wanted.has("url")) {
    collectPattern(entities, "url", text, URL_PATTERN, 0.9);
  }
  if (wanted.has("date")) {
    collectPattern(entities, "date", text, DATE_PATTERN, 0.8);
  }

  const needsNlp = ["person", "organization", "location", "money", "percent"];
  if (!needsNlp.some((category) => wanted.has(category))) return entities;

  const compromise = await import("compromise");
  const doc = compromise.default(text);

  if (wanted.has("person")) {
    collectTerms(entities, "person", text, doc.people().out("array"), 0.9);
  }
  if (wanted.has("organization")) {
    collectTerms(
      entities,
      "organization",
      text,
      doc.organizations().out("array"),
      0.9,
    );
  }
  if (wanted.has("location")) {
    collectTerms(entities, "location", text, doc.places().out("array"), 0.9);
  }
  if (wanted.has("money")) {
    collectTerms(entities, "money", text, doc.money().out("array"), 0.8);
  }
  if (wanted.has("percent")) {
    collectTerms(
      entities,
      "percent",
      text,
      doc.percentages().out("array"),
      0.8,
    );
  }

  return entities;
}
