/**
 * Task 6 (spec §8 step 6): `buildEntityFile` today tells a reader *where* an entity
 * appears, never *what is said about it* — reading it answers "which documents mention
 * Acme SAS" but not "what does this corpus say about Acme SAS", which is the query a
 * knowledge base actually exists to serve. These three functions compute the pieces
 * `lib/zip-export.ts`'s `buildEntityFile` renders into the dossier: quoted context per
 * mention, ranked co-occurring entities, and an observed date range — each a standalone,
 * testable computation over data the pipeline already has, no new inference or model call.
 *
 * **Depends on Task 3, and this is load-bearing, not incidental:** every function here
 * that reads document text must be given the *exported* (post-redaction) markdown, never
 * `rawMarkdown`. A pseudonymized entity's `display_name` is already its token by the time
 * these run (`filterExportableEntities`), so searching the exported text for that token
 * can only ever find the token — the real surface form was spliced out of that string
 * before this module ever sees it. Searching `rawMarkdown` instead would defeat Task 3
 * entirely: the whole reason a dossier is safe to enrich with quoted prose is that the
 * prose it quotes from has already had redaction applied.
 */
import type { RegistryEntity, RegistryRelationship } from "./registry";

/**
 * Every mention of `needle` (an entity's `display_name` — its real name, or its token
 * under pseudonymize mode) in `markdown`, as a whitespace-collapsed snippet of up to
 * `contextChars` characters either side. Capped at `maxSnippets` so one heavily-mentioned
 * entity in one document can't make its dossier grow unboundedly — see
 * `entity-dossier.test.ts`'s size-delta measurement for why the cap exists and what it
 * costs.
 *
 * Matches `needle` as a literal substring, not a word-boundary-aware search: an entity's
 * `display_name` is typically a multi-word proper noun or an unambiguous pseudonym token,
 * so a substring match is unlikely to land mid-word by accident, but it is not
 * impossible — a short surname or acronym entity could, in principle, substring-match
 * inside an unrelated longer word elsewhere in the document. Accepted as a known,
 * low-probability limitation of a dossier-enrichment feature, not a correctness
 * requirement this function is meant to guarantee airtight.
 */
export function extractQuotedContext(
  markdown: string,
  needle: string,
  maxSnippets: number,
  contextChars: number = 120,
): string[] {
  if (!needle) return [];
  const snippets: string[] = [];
  let searchFrom = 0;
  while (snippets.length < maxSnippets) {
    const idx = markdown.indexOf(needle, searchFrom);
    if (idx === -1) break;
    const start = Math.max(0, idx - contextChars);
    const end = Math.min(markdown.length, idx + needle.length + contextChars);
    let snippet = markdown.slice(start, end).replace(/\s+/g, " ").trim();
    if (start > 0) snippet = "…" + snippet;
    if (end < markdown.length) snippet = snippet + "…";
    snippets.push(snippet);
    searchFrom = idx + needle.length;
  }
  return snippets;
}

export interface CoOccurringEntity {
  entity: RegistryEntity;
  confidence: number;
  context: string;
}

/**
 * The other entities `entityId` has a `co_occurs_with` edge to (Task 1), ranked by
 * confidence — highest (closest proximity) first — and capped at `maxResults`.
 *
 * Deduped by the *other* entity's id, keeping its highest-confidence edge: the same pair
 * can co-occur in more than one document (`inferRelationships` runs once per document),
 * producing one `RegistryRelationship` per document they share — listing the same
 * co-occurring entity two or three times in a row would be noise, not signal.
 */
export function rankCoOccurringEntities(
  entityId: string,
  entities: RegistryEntity[],
  relationships: RegistryRelationship[],
  maxResults: number,
): CoOccurringEntity[] {
  const byId = new Map(entities.map((e) => [e.id, e]));
  const bestByOtherId = new Map<string, CoOccurringEntity>();
  for (const r of relationships) {
    let otherId: string | null = null;
    if (r.source_entity_id === entityId) otherId = r.target_entity_id;
    else if (r.target_entity_id === entityId) otherId = r.source_entity_id;
    if (!otherId) continue;
    const other = byId.get(otherId);
    if (!other) continue;
    const existing = bestByOtherId.get(otherId);
    if (!existing || r.confidence > existing.confidence) {
      bestByOtherId.set(otherId, { entity: other, confidence: r.confidence, context: r.context });
    }
  }
  return Array.from(bestByOtherId.values())
    .sort((a, b) => b.confidence - a.confidence)
    .slice(0, maxResults);
}

/**
 * Strict ISO `YYYY-MM-DD` only — the same scoping decision `lib/zip-export.ts`'s
 * `buildTimelineIndex` makes, and for the identical reason (this corpus is bilingual;
 * `lib/ner-bridge.ts`'s `DATE_PATTERN` detects French and English, numeric and
 * spelled-out dates, and comparing those formats against each other correctly is a real
 * parsing project this feature does not attempt). Duplicated here rather than imported
 * from `zip-export.ts` to avoid a circular import (`zip-export.ts` imports from this
 * module for `buildEntityFile`'s new content) — if this pattern ever changes, both
 * copies need updating together.
 */
const ISO_DATE_PATTERN = /^\d{4}-\d{2}-\d{2}$/;

export interface ObservedDateRange {
  earliest: string;
  latest: string;
}

/**
 * The earliest and latest ISO-parseable date entity co-occurring with `entityId`, or
 * `null` if none exist. Reuses the same `co_occurs_with` edges `rankCoOccurringEntities`
 * does, filtered to `type === "date"` — not a second inference pass, the same data
 * viewed differently.
 *
 * Degrades safely under pseudonymize mode without any special-casing: a `date` entity
 * that was itself redacted and retained (Task 3.1's rule applies to every entity type,
 * not just person/organization) has a token, not an ISO string, as its `display_name` —
 * `ISO_DATE_PATTERN` simply never matches a token, so a pseudonymized date silently
 * contributes nothing to the range rather than needing to be detected and excluded.
 */
export function computeObservedDateRange(
  entityId: string,
  entities: RegistryEntity[],
  relationships: RegistryRelationship[],
): ObservedDateRange | null {
  const byId = new Map(entities.map((e) => [e.id, e]));
  const isoDates: string[] = [];
  for (const r of relationships) {
    let otherId: string | null = null;
    if (r.source_entity_id === entityId) otherId = r.target_entity_id;
    else if (r.target_entity_id === entityId) otherId = r.source_entity_id;
    if (!otherId) continue;
    const other = byId.get(otherId);
    if (!other || other.type !== "date") continue;
    const value = other.display_name.trim();
    if (ISO_DATE_PATTERN.test(value)) isoDates.push(value);
  }
  if (isoDates.length === 0) return null;
  isoDates.sort(); // ISO strings sort lexicographically == chronologically
  return { earliest: isoDates[0], latest: isoDates[isoDates.length - 1] };
}
