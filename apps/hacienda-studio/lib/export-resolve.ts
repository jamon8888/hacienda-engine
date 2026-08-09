/**
 * Track K/Phase 2: resolves what content actually exports for a result, given the two
 * independent edit surfaces `App.tsx` maintains — Track I4's `editedFindings` (add/remove
 * findings, replayed through `reExportMarkdown`) and Track K2's `redactedDrafts` (free-text
 * edits to the split view's redacted-markdown pane, seeded from I4 state at first-open but
 * independent of it thereafter). `reExportMarkdown` lives here (not `App.tsx`) so this
 * module's `resolveExportContent` — used by the new "Download redacted zip" button's
 * override map — and `App.tsx`'s existing single-file "Export edited file" button share one
 * implementation instead of two copies drifting apart.
 */
import { renderAnnotatedMarkdown } from "./annotate";
import type { ProcessedFile } from "./types";
import type { PiiEntity } from "./pii-engine";

/**
 * Re-splices `result.rawMarkdown` against the (possibly edited) `findings`, reusing the
 * exact function `worker/pipeline.ts` used to build the original export. Frontmatter and
 * the local "## Entities" glossary are carried over unchanged from the original export
 * rather than regenerated: `pii_entities_found` and the entity list can go stale relative
 * to add/remove edits (an added finding won't retroactively drop an entity mention from the
 * glossary, a removed one won't add a mention back) — an explicit, documented scope cut.
 */
export function reExportMarkdown(result: ProcessedFile, findings: PiiEntity[]): string {
  const docPath = "documents/" + result.name;
  const linked = renderAnnotatedMarkdown(result.rawMarkdown, result.entities, findings, docPath);
  const frontmatterMatch = result.markdown.match(/^---\n[\s\S]*?\n---/);
  const glossaryMatch = result.markdown.match(/\n## Entities\n\n[\s\S]*$/);
  const frontmatter = frontmatterMatch ? frontmatterMatch[0] : "";
  const glossary = glossaryMatch ? glossaryMatch[0] : "";
  return frontmatter + "\n" + linked + glossary;
}

/**
 * K2 wins once it exists, because `redactedDraftFor` (App.tsx) already seeds K2 *from* I4
 * state at first-open time. Known, pre-existing edge case: if a user opens the split view
 * (seeding K2), then later edits I4 findings via the Findings tab without touching the K2
 * buffer, K2 does not retroactively re-splice — export reflects K2 as last edited, not
 * current I4 state. This is intentional (auto-re-seeding K2 would silently discard manual
 * redacted-markdown edits), not a bug to fix here.
 */
export function resolveExportContent(
  result: ProcessedFile,
  redactedDrafts: Map<string, string>,
  editedFindings: Map<string, PiiEntity[]>,
): string {
  const draft = redactedDrafts.get(result.name);
  if (draft !== undefined) return draft;
  const edited = editedFindings.get(result.name);
  if (edited !== undefined) return reExportMarkdown(result, edited);
  return result.markdown;
}
