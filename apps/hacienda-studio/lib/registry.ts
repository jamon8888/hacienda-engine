import { computeContentHash } from "./content-hash";

export interface RegistryEntity {
  id: string;
  canonical_name: string;
  display_name: string;
  type: string;
  /**
   * Task 4 (spec §8 step 4): content-derived — `hash(type|vertical|dedupKey)` — not
   * "first mention's slug" as this field used to be documented. Ordinal ids (`ent-001`)
   * and first-mention slugs renumbered or renamed on every re-export whose file order
   * changed, breaking any external reference into `entities-registry.json` or a link a
   * user made into `entities/<type>-<slug>.md`. Every spelling variant that merges into
   * one entity (via exact-key match or the alias matchers below) shares the identical
   * dedup key by construction, so this is stable across variants and across re-export
   * order — see `identityFor`. Deliberately never recomputed on alias promotion
   * (`recordAlias`): a fuller name becoming canonical/display must not change the
   * filename an existing external link points at.
   */
  slug: string;
  vertical: string;
  sector?: string;
  roles: string[];
  aliases: string[];
  source_documents: string[];
  mention_count: number;
  vertical_metadata: Record<string, any>;
  first_seen: string;
  last_seen: string;
}

export interface EntitySpan {
  start: number;
  end: number;
}

/**
 * How close together two entities were actually named. Ordered weakest-last; the numeric
 * `confidence` each band carries is a **proximity strength**, not a probability that any
 * particular relation holds — see `inferRelationships`.
 */
export type ProximityBand = "same_sentence" | "same_paragraph";

export interface RegistryRelationship {
  id: string;
  source_entity_id: string;
  target_entity_id: string;
  relationship_type: string;
  context: string;
  confidence: number;
  source_document: string;
}

/**
 * Proximity strength per band. Not probabilities — two names in one sentence are *more*
 * likely to be related than two names a paragraph apart, but neither number claims a
 * relation exists. Kept well below 1.0 so nothing downstream reads them as assertions.
 */
const PROXIMITY_CONFIDENCE: Record<ProximityBand, number> = {
  same_sentence: 0.6,
  same_paragraph: 0.3,
};

function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .substring(0, 64);
}

/**
 * The tightest pairing between any span of `a` and any span of `b`, as the half-open gap
 * `[from, to)` of text lying between them. Returns `null` when either side has no spans, or
 * when the spans overlap (which happens when one entity's match is nested inside another's —
 * there is no "between" to classify, and a nested pair is not evidence of co-occurrence).
 */
function closestGap(
  a: EntitySpan[],
  b: EntitySpan[],
): { from: number; to: number } | null {
  let best: { from: number; to: number } | null = null;
  for (const spanA of a) {
    for (const spanB of b) {
      const [first, second] =
        spanA.start <= spanB.start ? [spanA, spanB] : [spanB, spanA];
      if (first.end > second.start) continue; // overlapping / nested
      const gap = { from: first.end, to: second.start };
      if (!best || gap.to - gap.from < best.to - best.from) best = gap;
    }
  }
  return best;
}

/**
 * A blank-line-delimited paragraph has no upper bound on length, so "no blank line between
 * them" alone is not a proximity signal — two names either side of a full page of prose
 * would pass that check while being nowhere near each other. This is a hard ceiling on the
 * gap regardless of paragraph structure, chosen as roughly "a few sentences": generous
 * enough not to split a genuinely adjacent pair mid-clause, tight enough that a long
 * document's entity list does not collapse into one all-pairs blob.
 */
const MAX_PROXIMITY_GAP_CHARS = 300;

/**
 * Classify the text between two entity mentions. `null` means "not close enough to be worth
 * an edge", which `inferRelationships` treats as no relationship.
 *
 * A blank line is the paragraph boundary (markdown's own rule, and this text is markdown),
 * but is checked only after the distance cap — see `MAX_PROXIMITY_GAP_CHARS`. Sentence
 * detection is deliberately conservative — terminator followed by whitespace — so `S.A.S.`,
 * `n°4.`, and decimals split a sentence more often than they should. That biases toward
 * reporting `same_paragraph` where `same_sentence` was true, i.e. toward understating
 * proximity, which is the safe direction for a claim that lands in a compliance export.
 */
function classifyProximity(
  text: string,
  from: number,
  to: number,
): ProximityBand | null {
  if (to - from > MAX_PROXIMITY_GAP_CHARS) return null;
  const between = text.slice(from, to);
  if (/\n[ \t]*\n/.test(between)) return null;
  if (/[.!?]["')\]]?\s/.test(between)) return "same_paragraph";
  return "same_sentence";
}

/**
 * French and English honorifics stripped before comparing person names for alias matching
 * — "M. Dupont" and "Jean Dupont" must reduce to the same surname token to be recognised as
 * possibly the same person. Deliberately not stripped from `display_name`/`canonical_name`
 * themselves (only from the comparison copy `personNameTokens` returns): the registry still
 * shows whichever surface form the document actually used.
 */
const HONORIFIC_PATTERN =
  /^(monsieur|madame|mademoiselle|ma[iî]tre|docteur|professeur|mister|misses|miss|mme|mlle|me|dr|pr|prof|mr|mrs|ms|m)\.?\s+/i;

function personNameTokens(name: string): string[] {
  return name
    .replace(HONORIFIC_PATTERN, "")
    .replace(/[.,;:]+$/, "")
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
}

/**
 * Two person-name token lists are treated as the same individual only if their last token
 * (the surname, in both French and English "given name(s) surname" order) matches exactly
 * *and* the shorter list's tokens are all present in the longer one — "Dupont" ⊂ "Jean
 * Dupont", but "Marie Dupont" ⊄ "Jean Dupont" despite sharing a surname. This is deliberately
 * conservative: it only merges a bare or partial form into a fuller one, never two distinct
 * full names.
 */
function isSubsetMatch(a: string[], b: string[]): boolean {
  if (a.length === 0 || b.length === 0) return false;
  if (a[a.length - 1] !== b[b.length - 1]) return false;
  const [shorter, longer] = a.length <= b.length ? [a, b] : [b, a];
  return shorter.every((t) => longer.includes(t));
}

/**
 * Normalise organisation names for comparison: lower-case, remove punctuation, collapse whitespace.
 */
function normalizeOrgName(name: string): string {
  return name
    .toLowerCase()
    .replace(/[.,;:]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

/**
 * Strip trailing legal suffixes that are not semantically distinguishing — a loop, not a
 * single strip, so cascading suffixes ("Acme Holdings Ltd Inc") reduce fully rather than
 * leaving one behind. Doubles as Task 4's dedup-key/id-stability input for organisations
 * (`addEntity`'s `stableKeyForId`): "Acme", "Acme SAS" and "ACME S.A.S." must all strip to
 * the same base for both alias matching (below) and the content-hashed id (`identityFor`)
 * to treat them as one entity.
 */
function stripLegalSuffixes(name: string): string {
  const suffixes = new Set(["sas", "sarl", "sa", "ltd", "gmbh", "inc"]);
  let tokens = normalizeOrgName(name).split(" ");
  while (tokens.length > 1) {
    const last = tokens[tokens.length - 1];
    if (suffixes.has(last)) {
      tokens.pop();
    } else {
      break;
    }
  }
  return tokens.join(" ");
}

/**
 * Content-derived identity for a registry entity: `hash(stableKey)`, truncated to 12 hex
 * chars (48 bits — ample margin against collision for any realistic batch size, short
 * enough for a readable filename). `stableKey` is built by the caller from
 * `type|vertical|normalizedOrDedupedName` — stable across re-export in a different
 * document order, because every spelling variant that merges into one entity (exact-key
 * match or an alias matcher) produces the identical `stableKey` by construction, so
 * whichever variant happens to be seen first yields the same id any other would have.
 *
 * SHA-256 via `lib/content-hash.ts` rather than a lighter non-cryptographic hash (contrast
 * `worker/pipeline.ts`'s `tokenSlug`, Task 3): this id is meant to be a stable reference
 * other tooling can rely on, not a cosmetic filename shortener, and a real batch's entity
 * count is not so small that a 32-bit hash's collision odds are obviously negligible.
 */
async function identityFor(stableKey: string): Promise<{ id: string; slug: string }> {
  const digest = await computeContentHash(new TextEncoder().encode(stableKey).buffer);
  const slug = digest.slice(0, 12);
  return { id: `ent-${slug}`, slug };
}

export class BatchEntityRegistry {
  private entities: Map<string, RegistryEntity> = new Map();
  private relationships: RegistryRelationship[] = [];
  private batchId: string;
  private entityKeyMap: Map<string, string> = new Map();
  private docEntityMap: Map<string, string[]> = new Map();
  /**
   * `docId -> entityId -> spans`, the offsets each entity was matched at *within that
   * document's* text. Deliberately a side map rather than a field on `RegistryEntity`:
   * `RegistryEntity` is serialised wholesale into `entities-registry.json`, and spans are
   * both per-document (the same entity has different offsets in every file) and useless to
   * a bundle reader. `inferRelationships` needs them to tell "named in the same sentence"
   * apart from "both appear somewhere in a 50-page PDF"; nothing else does.
   */
  private docEntitySpans: Map<string, Map<string, EntitySpan[]>> = new Map();

  constructor() {
    this.batchId = `batch-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  }

  async addEntity(
    entity: {
      name: string;
      type: string;
      slug: string;
      count: number;
      spans: Array<{ start: number; end: number }>;
    },
    metadata: { vertical: string; sector?: string; roles?: string[] },
    docId: string,
  ): Promise<RegistryEntity> {
    // Normalize entity name for deduplication (remove trailing punctuation)
    const normalizedName = entity.name.replace(/[.,;:]+$/, "").toLowerCase();
    const key = `${normalizedName}|${entity.type}|${metadata.vertical}`;
    const existingId = this.entityKeyMap.get(key);

    if (existingId) {
      const existing = this.entities.get(existingId)!;
      // Task 4 (spec §8 step 4): even an exact-dedup-key repeat can differ in casing or
      // trailing punctuation ("acme sas" vs "Acme SAS" both normalize to the same key) —
      // record it as an alias and promote to the longest surface form seen, same rule
      // `recordAlias` applies for the fuzzy-matched (person/org) branches below. Skipped
      // when byte-identical to the current display name, the common repeat-mention case.
      if (entity.name !== existing.display_name) {
        this.recordAlias(existing, entity.name);
      }
      // Bug fix (Task 1): this branch used to update `source_documents` but never
      // `docEntityMap`, so an entity first seen in doc-001 and seen again in doc-002 was
      // absent from doc-002's entity list. Two consequences, both silent: `document_entities`
      // in `entities-registry.json` under-reported every repeat appearance, and
      // `inferRelationships(doc-002)` could not see the entity at all — so co-occurrences
      // involving any previously-seen entity were simply never emitted. Recording it here
      // makes both correct, and is a prerequisite for proximity scoring being meaningful.
      this.recordMention(existing, docId, entity.spans);
      return existing;
    }

    if (entity.type === "person") {
      const tokens = personNameTokens(entity.name);
      const match = this.findPersonAliasMatch(tokens, metadata.vertical);
      if (match) {
        this.recordMention(match, docId, entity.spans);
        this.recordAlias(match, entity.name);
        this.entityKeyMap.set(key, match.id);
        return match;
      }
    }

    if (entity.type === "organization") {
      const base = stripLegalSuffixes(entity.name);
      const match = this.findOrgAliasMatch(base, metadata.vertical);
      if (match) {
        this.recordMention(match, docId, entity.spans);
        this.recordAlias(match, entity.name);
        this.entityKeyMap.set(key, match.id);
        return match;
      }
    }

    const stableKeyForId = `${entity.type}|${metadata.vertical}|${entity.type === "organization" ? stripLegalSuffixes(entity.name) : normalizedName}`;
    const { id, slug } = await identityFor(stableKeyForId);
    const registryEntity: RegistryEntity = {
      id,
      canonical_name: entity.name,
      display_name: entity.name,
      type: entity.type,
      slug,
      vertical: metadata.vertical,
      sector: metadata.sector,
      roles: metadata.roles || [],
      aliases: [],
      source_documents: [docId],
      mention_count: 1,
      vertical_metadata: metadata,
      first_seen: new Date().toISOString(),
      last_seen: new Date().toISOString(),
    };

    this.entities.set(id, registryEntity);
    this.entityKeyMap.set(key, id);

    // Track document -> entity mapping
    this.registerInDocument(id, docId, entity.spans);

    return registryEntity;
  }

  /** Shared by the exact-key and person/org-alias match branches of `addEntity`. */
  private recordMention(existing: RegistryEntity, docId: string, spans: EntitySpan[]) {
    if (!existing.source_documents.includes(docId)) {
      existing.source_documents.push(docId);
    }
    existing.mention_count++;
    existing.last_seen = new Date().toISOString();
    this.registerInDocument(existing.id, docId, spans);
  }

  /**
   * Finds the one existing person entity (same vertical) whose name `tokens` could name the
   * same individual — see `isSubsetMatch`. Returns `null` on zero *or on more than one*
   * candidate: with both "Jean Dupont" and "Marie Dupont" already registered, "M. Dupont"
   * alone does not say which one it names, and guessing wrong in a legal export is worse than
   * leaving it as its own unmerged entity.
   */
  private findPersonAliasMatch(tokens: string[], vertical: string): RegistryEntity | null {
    if (tokens.length === 0) return null;
    let match: RegistryEntity | null = null;
    for (const candidate of this.entities.values()) {
      if (candidate.type !== "person" || candidate.vertical !== vertical) continue;
      if (!isSubsetMatch(tokens, personNameTokens(candidate.canonical_name))) continue;
      if (match) return null;
      match = candidate;
    }
    return match;
  }

  private findOrgAliasMatch(baseName: string, vertical: string): RegistryEntity | null {
    const normalizedBase = normalizeOrgName(baseName);
    let match: RegistryEntity | null = null;
    for (const candidate of this.entities.values()) {
      if (candidate.type !== "organization" || candidate.vertical !== vertical) continue;
      const candidateBase = stripLegalSuffixes(candidate.canonical_name);
      if (normalizeOrgName(candidateBase) !== normalizedBase) continue;
      if (match) return null;
      match = candidate;
    }
    return match;
  }

  /**
   * Folds `rawName` into `existing` as an alias — unless it carries more of the name than
   * `existing.canonical_name` does (e.g. "M. Dupont" registered first, "Jean Dupont" seen
   * later), in which case the fuller form is promoted to canonical/display and the bare form
   * that used to be there becomes the alias instead. Either way the registry always shows the
   * most complete surface form seen so far, with the rest recorded, not discarded.
   *
   * "Fuller" is character length, not `personNameTokens(...).length` (word count) as an
   * earlier, person-only version of this function used — reconciled to character length
   * (Task 4 §8 step 4) because this function now also promotes organisation aliases, and
   * word count cannot distinguish "Acme SAS" from "ACME S.A.S." (both 2 tokens), silently
   * freezing promotion after the first multi-word variant — caught by
   * `registry.test.ts`'s "merges 'Acme', 'Acme SAS', and 'ACME S.A.S.'" test, which
   * expects the longest to win. Character length gives the correct answer for both the
   * person and organisation cases already covered by this suite.
   *
   * Deliberately does **not** touch `existing.slug`/`existing.id` on promotion (a deviation
   * from this function's pre-reconciliation form, which called `slugify(surfaceForm)` here):
   * both are content-hashed at creation time from the entity's *dedup key*, specifically so
   * they stay stable regardless of which spelling variant a reader or an external link saw
   * first — recomputing the slug from whichever name becomes canonical would silently
   * reintroduce that instability for every entity that ever gets an alias promoted, which is
   * the exact failure mode Task 4 exists to eliminate.
   */
  private recordAlias(existing: RegistryEntity, rawName: string) {
    const surfaceForm = rawName.trim();
    if (surfaceForm.length > existing.canonical_name.length) {
      if (
        existing.display_name !== surfaceForm &&
        !existing.aliases.includes(existing.display_name)
      ) {
        existing.aliases.push(existing.display_name);
      }
      existing.canonical_name = surfaceForm;
      existing.display_name = surfaceForm;
      return;
    }
    if (surfaceForm !== existing.display_name && !existing.aliases.includes(surfaceForm)) {
      existing.aliases.push(surfaceForm);
    }
  }

  /**
   * Records that `entityId` occurs in `docId`, at `spans`. Idempotent on the id (an entity
   * mentioned twice in one document appears once in `docEntityMap`) while accumulating every
   * span, which is what proximity scoring needs.
   */
  private registerInDocument(
    entityId: string,
    docId: string,
    spans: EntitySpan[],
  ) {
    if (!this.docEntityMap.has(docId)) {
      this.docEntityMap.set(docId, []);
    }
    const ids = this.docEntityMap.get(docId)!;
    if (!ids.includes(entityId)) ids.push(entityId);

    if (!this.docEntitySpans.has(docId)) {
      this.docEntitySpans.set(docId, new Map());
    }
    const spansById = this.docEntitySpans.get(docId)!;
    const existing = spansById.get(entityId) ?? [];
    spansById.set(entityId, [...existing, ...spans]);
  }

  addRelationship(
    sourceId: string,
    targetId: string,
    type: string,
    context: string,
    confidence: number,
    docId: string,
  ) {
    this.relationships.push({
      id: `rel-${String(this.relationships.length + 1).padStart(3, "0")}`,
      source_entity_id: sourceId,
      target_entity_id: targetId,
      relationship_type: type,
      context,
      confidence,
      source_document: docId,
    });
  }

  /**
   * Emit co-occurrence edges for `docId`, scored by how close together the two entities were
   * actually named in `text`.
   *
   * This replaces a scheme that asserted `works_for` (confidence 0.7) for every person that
   * happened to share a document with an organisation, `partner_of` (0.5) for every
   * organisation pair, and `contact_email` (0.7/0.8) for every email near either — none of
   * which was ever observed in the text. Those claims were written verbatim into
   * `entities-registry.json` and all three `kg-export/` serialisations, where a reader has no
   * way to tell an asserted relation from a coincidence of layout. A 40-person board pack
   * produced roughly 200 false employment claims from a single file.
   *
   * What is emitted now is only what co-occurrence can actually support: **one undirected
   * `co_occurs_with` edge per entity pair**, carrying the proximity band it was observed at.
   * `confidence` is a *proximity strength*, not a probability of any relation.
   *
   * Document-level co-occurrence is deliberately **not** emitted. It is already fully
   * recoverable from `document_entities` (every entity of a document is listed there), so an
   * edge for it would be redundant, and at O(n²) it is where the noise came from: 45 entities
   * in one file is 990 pairs. Only same-sentence and same-paragraph pairs — where proximity
   * carries real signal — become edges.
   *
   * `text` must be the same string the spans are offsets into (`ProcessedFile.rawMarkdown`).
   * Omitting it degrades to emitting nothing rather than to guessing.
   */
  inferRelationships(docId: string, text?: string) {
    if (!text) return;
    const entityIds = this.docEntityMap.get(docId) || [];
    const spansById = this.docEntitySpans.get(docId);
    if (!spansById) return;

    for (let i = 0; i < entityIds.length; i++) {
      for (let j = i + 1; j < entityIds.length; j++) {
        const aId = entityIds[i];
        const bId = entityIds[j];
        const aSpans = spansById.get(aId) ?? [];
        const bSpans = spansById.get(bId) ?? [];

        const closest = closestGap(aSpans, bSpans);
        if (!closest) continue;

        const band = classifyProximity(text, closest.from, closest.to);
        if (!band) continue; // farther apart than a paragraph — see the doc comment

        this.addRelationship(
          aId,
          bId,
          "co_occurs_with",
          band === "same_sentence"
            ? "Named in the same sentence"
            : "Named in the same paragraph",
          PROXIMITY_CONFIDENCE[band],
          docId,
        );
      }
    }
  }

  toJSON() {
    return {
      batch_id: this.batchId,
      processed_at: new Date().toISOString(),
      total_documents: [
        ...new Set(
          Array.from(this.entities.values()).flatMap((e) => e.source_documents),
        ),
      ].length,
      vertical_taxonomy_version: "1.0.0",
      entity_registry: {
        entities: Array.from(this.entities.values()),
        relationships: this.relationships,
      },
      document_entities: Object.fromEntries(this.docEntityMap),
      vertical_summary: this.getVerticalSummary(),
    };
  }

  getVerticalSummary() {
    const summary: Record<
      string,
      { entity_count: number; relationship_count: number }
    > = {};
    for (const e of this.entities.values()) {
      if (!summary[e.vertical]) {
        summary[e.vertical] = { entity_count: 0, relationship_count: 0 };
      }
      summary[e.vertical].entity_count++;
    }
    for (const r of this.relationships) {
      const sourceEntity = this.entities.get(r.source_entity_id);
      if (sourceEntity && summary[sourceEntity.vertical]) {
        summary[sourceEntity.vertical].relationship_count++;
      }
    }
    return summary;
  }

  getEntities() {
    return Array.from(this.entities.values());
  }
  getRelationships() {
    return this.relationships;
  }
  getBatchId() {
    return this.batchId;
  }
}
