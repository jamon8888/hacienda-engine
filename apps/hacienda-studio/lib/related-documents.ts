/**
 * Task 5.2 (spec §8 step 5, §5.1.2): the missing hop in the export — every document
 * links to the entities it mentions and every entity backlinks to its documents, but no
 * document ever links to *another document*. A reader has to go document → entity →
 * document to discover that two files share a subject, reading two files to learn one
 * fact. This computes that edge directly from data `BatchEntityRegistry` already holds:
 * no new inference, no model, pure set arithmetic over `RegistryEntity.source_documents`.
 *
 * IDF-weighted so an entity present in nearly every document (a corpus-wide constant —
 * the client on every file in a case, the vendor on every invoice) contributes ~nothing
 * to relatedness, while an entity shared by exactly two documents out of many is a
 * strong, specific signal. This is the same intuition TF-IDF applies to term frequency
 * in a document corpus, applied here to entity frequency across a document corpus.
 */
import type { RegistryEntity } from "./registry";
import { relativeDocumentLink } from "./annotate";

export interface RelatedDocument {
  docId: string;
  score: number;
  /** Display names of the entities that produced this score, ranked by their own IDF contribution — the most specific shared entity first. */
  sharedEntityNames: string[];
}

/**
 * For each document in `docIds`, the other documents it shares entities with, ranked by
 * score, highest first. A document with no scoring overlap with anything else is present
 * in the returned map with an empty array — callers can distinguish "computed, nothing
 * found" from "never asked about this document".
 *
 * `docIds` must be the full set of documents in the batch, not just the ones with
 * entities — `n = docIds.length` is what makes IDF meaningful (an entity in 3 of 4
 * documents is far more common than one in 3 of 400).
 */
export function computeRelatedDocuments(
  entities: RegistryEntity[],
  docIds: string[],
): Map<string, RelatedDocument[]> {
  const n = docIds.length;
  const perDoc = new Map<string, Map<string, { name: string; idf: number }>>();
  for (const docId of docIds) perDoc.set(docId, new Map());

  for (const entity of entities) {
    const df = entity.source_documents.length;
    // df === 0 shouldn't occur (an entity always has at least one source document), and
    // df > n would mean an entity references a document outside this batch — neither is
    // a case IDF can meaningfully score, so skip rather than divide into nonsense.
    if (df <= 0 || df > n) continue;
    const idf = Math.log(n / df);
    for (const docId of entity.source_documents) {
      perDoc.get(docId)?.set(entity.id, { name: entity.display_name, idf });
    }
  }

  const result = new Map<string, RelatedDocument[]>();
  for (let i = 0; i < docIds.length; i++) {
    const a = perDoc.get(docIds[i])!;
    const related: RelatedDocument[] = [];
    for (let j = 0; j < docIds.length; j++) {
      if (i === j) continue;
      const b = perDoc.get(docIds[j])!;
      let score = 0;
      const shared: Array<{ name: string; idf: number }> = [];
      for (const [entityId, info] of a) {
        // `info.idf === 0` for an entity present in every document (e.g. the client
        // name on every file in one case) is correct — it contributes nothing to the
        // score, but it *is* still in the intersection, so without this extra check it
        // would still land in `sharedEntityNames` even though the numeric score never
        // reflects it. Left unguarded, the exported "why are these related" reason
        // would list the one entity IDF weighting exists to make invisible.
        if (b.has(entityId) && info.idf > 0) {
          score += info.idf;
          shared.push(info);
        }
      }
      if (score > 0) {
        shared.sort((x, y) => y.idf - x.idf);
        related.push({
          docId: docIds[j],
          score,
          sharedEntityNames: shared.map((s) => s.name),
        });
      }
    }
    related.sort((x, y) => y.score - x.score);
    result.set(docIds[i], related);
  }
  return result;
}

/**
 * `computeRelatedDocuments` above returns every scoring pair, unbounded — deliberately,
 * so the cutoff decision (this function) is a separate, visible step rather than baked
 * silently into the scoring. Caps at `maxResults`, applied *after* sorting by score, so
 * the entries kept are always the strongest — never a silent-cap artifact of iteration
 * order.
 *
 * `5`, measured rather than assumed (plan §5.2: "measure ... before fixing the
 * cutoff") against `related-documents.test.ts`'s 24-document synthetic corpus — 6
 * non-overlapping entity clusters of size 2–4 plus 7 documents with no shared entity at
 * all. Unbounded per-document related-counts on that corpus: max 3, median 2, no long
 * tail — a cap of 5 never truncates anything there. That corpus is a lower bound on the
 * real risk, not proof 5 is sufficient in general: its clusters don't overlap by
 * construction, so it doesn't exercise the case that actually produces a long tail — a
 * moderately common entity (present in, say, a third of the batch) contributing a
 * small-but-nonzero score to many pairs simultaneously. `5` is chosen as a safety margin
 * for that untested case, not a value this measurement alone proves sufficient; revisit
 * once a corpus with overlapping clusters is measured.
 */
const DEFAULT_MAX_RESULTS = 5;

export function topRelatedDocuments(
  related: RelatedDocument[],
  maxResults: number = DEFAULT_MAX_RESULTS,
): RelatedDocument[] {
  return related.slice(0, maxResults);
}

/**
 * "With the reason attached (which entities are shared), not a bare link list" (plan
 * §5.2) — a link alone tells a reader *that* two documents relate, forcing them to open
 * both to learn *why*; the shared entity names let them decide whether it's worth
 * opening at all. `""` (no heading emitted) when `related` is empty, matching
 * `buildGlossary`'s convention in `worker/pipeline.ts` for the same "nothing to say"
 * case, and deliberately appended by the caller *after* the existing "## Entities"
 * section — see `worker/pipeline.ts`'s post-batch splice site for why: everything from
 * `## Entities` onward is what `lib/export-resolve.ts`'s `reExportMarkdown` already
 * treats as a static tail carried over unchanged on a redaction-edit re-export, and this
 * section is exactly as static (computed once from the whole batch, not per-edit) —
 * appending here means zero changes were needed to that slicing logic.
 */
export function buildRelatedDocumentsSection(
  related: RelatedDocument[],
  ownDocPath: string,
  docPaths: Map<string, string>,
): string {
  if (related.length === 0) return "";
  let md = "\n## Related documents\n\n";
  for (const r of related) {
    const otherPath = docPaths.get(r.docId);
    if (!otherPath) continue; // defensive: a docId with no known export path
    const link = relativeDocumentLink(ownDocPath, otherPath);
    const label = otherPath.replace(/^documents\//, "");
    md += `- [${label}](${link}) — shares ${r.sharedEntityNames.join(", ")}\n`;
  }
  return md;
}
