/**
 * Track I3: a Finder-like per-file list — status (extracting/done/error), entity and PII
 * counts once a file finishes, and an "edited" badge once Track I4 editing has touched a
 * file's findings. Deliberately not a port of hacienda-private's `file-system.tsx`: that
 * component is ~4,600 lines built on `@base-ui/react` + Tailwind v4 (see
 * `components/ui/README.md` for why that toolkit generation isn't compatible with this
 * app's classic Radix-based shadcn setup), and porting it would mean rewriting a file
 * browser's worth of tree/selection/virtualization logic that this app's actual use case —
 * one flat batch of files per run, not a persistent project tree — doesn't need. This is
 * the scoped equivalent: the same per-file state the plan asks for, in a list this app
 * already had the pieces for (`ProgressBar`'s `progress` map, `results`, the new I4 edit
 * state), not a vendored subtree.
 */
import { effectiveFileName } from "../lib/file-filter";
import type { ProcessedFile, ProgressUpdate } from "../lib/types";
import type { PiiEntity } from "../lib/pii-engine";

export interface FileRow {
  name: string;
  update: ProgressUpdate | undefined;
  result: ProcessedFile | undefined;
  error: string | undefined;
  edited: boolean;
  piiCount: number;
}

export function buildFileRows(
  files: File[],
  progress: Map<string, ProgressUpdate>,
  results: ProcessedFile[],
  fileErrors: Map<string, string>,
  editedNames: Set<string>,
  findingsByName: Map<string, PiiEntity[]>,
): FileRow[] {
  const resultsByInputName = new Map(
    results.map((r) => [r.frontmatter.source, r] as const),
  );
  return files.map((file) => {
    const name = effectiveFileName(file);
    const result = resultsByInputName.get(name);
    const piiCount = result
      ? (findingsByName.get(result.name)?.length ?? result.piiFindings.length)
      : 0;
    return {
      name,
      update: progress.get(name),
      result,
      error: fileErrors.get(name),
      edited: result ? editedNames.has(result.name) : false,
      piiCount,
    };
  });
}

function statusIcon(row: FileRow): string {
  if (row.error) return "⚠️";
  if (row.result) return "✅";
  if (row.update) return "⏳";
  return "⋯";
}

function statusLabel(row: FileRow): string {
  if (row.error) return row.error;
  if (row.result) return "Done";
  if (row.update) return `${row.update.stage} (${row.update.percent}%)`;
  return "Queued";
}

export function FileBrowser({ rows }: { rows: FileRow[] }) {
  if (rows.length === 0) return null;
  return (
    <section className="file-browser mt-6 overflow-hidden rounded-md border border-border">
      <ul>
        {rows.map((row) => (
          <li
            key={row.name}
            data-file-row={row.name}
            className="flex items-center justify-between gap-3 border-b border-border px-3 py-2 text-sm last:border-b-0"
          >
            <div className="flex min-w-0 items-center gap-2">
              <span aria-hidden="true">{statusIcon(row)}</span>
              <span className="truncate font-medium">{row.name}</span>
            </div>
            <div className="flex shrink-0 items-center gap-3 text-xs text-muted-foreground">
              <span>{statusLabel(row)}</span>
              {row.result && (
                <>
                  <span>
                    {row.result.entities.length}{" "}
                    {row.result.entities.length === 1 ? "entity" : "entities"}
                  </span>
                  <span>{row.piiCount} PII</span>
                </>
              )}
              {row.edited && (
                <span className="file-edited-badge rounded bg-primary/15 px-1.5 py-0.5 text-primary">
                  edited
                </span>
              )}
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
