/**
 * Track I4: split out of `worker/pipeline.ts` so the re-splice logic is importable from
 * `App.tsx` (main thread) without pulling in that module's top-level `self.onmessage =`
 * assignment — harmless in a Worker, but in the main thread `self` is `window`, and
 * assigning `window.onmessage` at import time would silently hijack any real `postMessage`
 * listener the page has. `worker/pipeline.ts` re-exports everything here unchanged, so
 * nothing importing from `"./pipeline"` (tests included) needs to know this split exists.
 */
import type { Entity } from "./types";
import type { PiiEntity } from "./pii-engine";

interface RenderSpan {
  start: number;
  end: number;
  render: (matchedText: string) => string;
}

/**
 * Track I2 supersedes G2's in-document anchors: `entities/<type>-<slug>.md`
 * is a real file, followable by anything that reads a markdown folder
 * (Claude Desktop with filesystem access, Obsidian, a plain `ls`), not just
 * a same-page fragment. Type-prefixed rather than the plan's literal
 * `entities/acme-sas.md` example, because a bare slug collides across types
 * (an `organization` and a `location` can slugify to the same string) — the
 * same collision risk G2's anchor scheme already had to account for.
 */
export function entityFileName(entity: Pick<Entity, "type" | "slug">): string {
  return `${entity.type}-${entity.slug}.md`;
}

/**
 * Every document lives at `documents/<relative path>.md`; every entity file
 * lives at `entities/<type>-<slug>.md`, one directory below the zip root.
 * `docPath` is always `documents/...`, so the number of `../` segments
 * needed to reach the zip root is exactly `docPath`'s segment count minus
 * one (the filename itself doesn't count, "documents" does).
 */
export function relativeEntityLink(
  docPath: string,
  entity: Pick<Entity, "type" | "slug">,
): string {
  const ups = "../".repeat(docPath.split("/").length - 1);
  return `${ups}entities/${entityFileName(entity)}`;
}

/** The inverse direction: from `entities/<file>.md` back to a `documents/...` path. */
export function relativeDocLink(docPath: string): string {
  return `../${docPath}`;
}

/**
 * Task 5.2 (spec §8 step 5): a real relative path from one `documents/...` file to
 * another, not `relativeEntityLink`'s simpler "count segments, go up N times" scheme —
 * that scheme only works because every entity file lives at the *same* fixed depth
 * (`entities/`, one directory below the zip root). Two documents can live at *different*
 * depths from each other: `lib/folder-picker.ts` preserves a folder upload's nested
 * structure via `webkitRelativePath`, so `documents/2024/msa.md` and
 * `documents/minutes/board.md` are both real, valid paths in the same batch. Both
 * arguments are `documents/...`-prefixed paths (matching every other function in this
 * file), not bare filenames.
 */
export function relativeDocumentLink(fromDocPath: string, toDocPath: string): string {
  const fromDir = fromDocPath.split("/").slice(0, -1);
  const toParts = toDocPath.split("/");
  let common = 0;
  while (
    common < fromDir.length &&
    common < toParts.length - 1 &&
    fromDir[common] === toParts[common]
  ) {
    common++;
  }
  const ups = "../".repeat(fromDir.length - common);
  const downs = toParts.slice(common).join("/");
  return ups + downs;
}

/**
 * Entity-link spans and PII-redaction spans, both computed against the same
 * unmodified `markdown`, merged into a single splice pass (Track F4/L7). PII wins
 * on overlap: an entity-link span whose range overlaps a redaction span is dropped
 * rather than spliced, so a redacted span's raw text is never duplicated into a
 * link's visible text or its anchor target — which is what corrupted output before
 * (PII detection ran on already-linked markdown, so a match inside link syntax got
 * rewritten in both places it appeared). Spans are applied right-to-left so each
 * splice leaves earlier offsets in `markdown` valid for the rest of the pass.
 *
 * Only handles the markdown body's splice order. Whether an entity's frontmatter/
 * glossary/registry row should itself be suppressed when its span was redacted is
 * a separate decision `processFile` makes before entities ever reach here — see
 * the "exportableEntities" filter in `worker/pipeline.ts` (Track A2).
 */
export function renderAnnotatedMarkdown(
  markdown: string,
  entities: Entity[],
  piiFindings: PiiEntity[],
  docPath: string,
): string {
  const piiSpans: RenderSpan[] = piiFindings.map((f) => ({
    start: f.start,
    end: f.end,
    render: () => f.redact_template,
  }));

  const overlapsPii = (span: { start: number; end: number }) =>
    piiSpans.some((p) => span.start < p.end && p.start < span.end);

  const entitySpans: RenderSpan[] = entities
    .flatMap((e) =>
      e.spans.map((s) => ({
        start: s.start,
        end: s.end,
        render: (matched: string) =>
          `[${matched}](${relativeEntityLink(docPath, e)})`,
      })),
    )
    .filter((s) => !overlapsPii(s));

  const spans = [...entitySpans, ...piiSpans].sort((a, b) => b.start - a.start);

  let result = markdown;
  for (const span of spans) {
    const before = result.slice(0, span.start);
    const matched = result.slice(span.start, span.end);
    const after = result.slice(span.end);
    result = before + span.render(matched) + after;
  }
  return result;
}

/**
 * Track K2: the side-by-side redacted-markdown pane's "Redact selection" button. Unlike
 * `renderAnnotatedMarkdown` above, this operates on the pane's own free-text buffer, not
 * `rawMarkdown` offsets — once a user has typed into that buffer it has no correspondence
 * to `piiFindings`/`entities` positions any more, so redaction there is a plain text splice,
 * not a span re-render keyed to detector output.
 */
export function spliceRedaction(
  text: string,
  from: number,
  to: number,
  category: string,
): string {
  return text.slice(0, from) + `[${category.toUpperCase()}]` + text.slice(to);
}
