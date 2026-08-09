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
import { useEffect, useState } from "react";
import { Badge } from "./ui/badge";
import { ScrollArea } from "./ui/scroll-area";
import { effectiveFileName } from "../lib/file-filter";
import { FILE_ICON_SPRITE_SHEET, resolveFileIcon } from "../lib/file-icons";
import { renderPdfThumbnailUrl } from "../lib/pdf-thumbnail-utils";
import { getViewerKind } from "../lib/viewer-kind";
import type { ProcessedFile, ProgressUpdate } from "../lib/types";
import type { PiiEntity } from "../lib/pii-engine";

export interface FileRow {
  name: string;
  update: ProgressUpdate | undefined;
  result: ProcessedFile | undefined;
  error: string | undefined;
  edited: boolean;
  piiCount: number;
  // Any completed result is openable — not just ones with a native Extend UI viewer.
  // `DetailScreen` renders a split view (native viewer + RedactedEditor) when a viewer
  // exists, and a findings-only fallback (MarkdownEditor + PiiPanel, Track I4) otherwise —
  // gating this on `getViewerKind(...) !== null` would make plain-text/unsupported-viewer
  // files permanently unreachable once findings-editing lives behind a click-to-open
  // screen instead of always being rendered inline.
  openable: boolean;
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
      openable: !!result,
    };
  });
}

function statusLabel(row: FileRow): string {
  if (row.error) return row.error;
  if (row.result) return "Done";
  if (row.update) return `${row.update.stage} (${row.update.percent}%)`;
  return "Queued";
}

/** Renders the `@pierre/trees` sprite once per page — `<use href="#...">` below
 * resolves against it regardless of which row rendered first. */
function FileIconSpriteSheet() {
  return (
    <span
      aria-hidden="true"
      className="hidden"
      dangerouslySetInnerHTML={{ __html: FILE_ICON_SPRITE_SHEET }}
    />
  );
}

function FileTypeIcon({ fileName }: { fileName: string }) {
  const icon = resolveFileIcon(fileName);
  return (
    <svg
      aria-hidden="true"
      viewBox={icon.viewBox ?? "0 0 16 16"}
      className="size-5 shrink-0 text-muted-foreground"
    >
      <use href={`#${icon.name}`} />
    </svg>
  );
}

/**
 * Real page-1 thumbnail for PDFs (the pdfium machinery `pdf-viewer.tsx`'s own sidebar
 * already uses — see `lib/pdf-thumbnail-utils.ts`), falling back to the plain file-type
 * icon while the render is in flight or if it fails. Per the plan's scope decision,
 * non-PDF viewer-capable types (docx/xlsx/pptx) get the icon only, no thumbnail.
 */
function FileThumbnail({ fileName, previewUrl }: { fileName: string; previewUrl: string | undefined }) {
  const [thumbnailUrl, setThumbnailUrl] = useState<string | null>(null);
  const isPdf = getViewerKind(fileName) === "pdf";

  useEffect(() => {
    if (!isPdf || !previewUrl) {
      setThumbnailUrl(null);
      return;
    }
    let cancelled = false;
    renderPdfThumbnailUrl({ url: previewUrl, pageIndex: 0, width: 40 })
      .then((url) => {
        if (!cancelled) setThumbnailUrl(url);
      })
      .catch(() => {
        if (!cancelled) setThumbnailUrl(null);
      });
    return () => {
      cancelled = true;
    };
  }, [isPdf, previewUrl]);

  if (thumbnailUrl) {
    return (
      <img
        src={thumbnailUrl}
        alt=""
        aria-hidden="true"
        className="h-8 w-6 shrink-0 rounded-sm border border-border object-cover"
      />
    );
  }
  return (
    <span className="flex size-5 shrink-0 items-center justify-center">
      <FileTypeIcon fileName={fileName} />
    </span>
  );
}

export function FileBrowser({
  rows,
  openName,
  onOpen,
  previewUrls,
}: {
  rows: FileRow[];
  openName?: string | null;
  onOpen?: (name: string) => void;
  /** Keyed by `ProcessedFile.name` (the `.md` output name) — same map `App.tsx` threads
   * into `DetailScreen`'s viewer, reused here for PDF thumbnails. */
  previewUrls?: Map<string, string>;
}) {
  if (rows.length === 0) return null;
  return (
    <section className="file-browser mt-6 overflow-hidden rounded-md border border-border">
      <FileIconSpriteSheet />
      <ScrollArea className="max-h-[60vh]">
        <ul>
          {rows.map((row) => {
            const clickable = row.openable && !!onOpen;
            const isOpen = openName === row.name;
            return (
              <li
                key={row.name}
                data-file-row={row.name}
                role={clickable ? "button" : undefined}
                tabIndex={clickable ? 0 : undefined}
                aria-pressed={clickable ? isOpen : undefined}
                onClick={clickable ? () => onOpen(row.name) : undefined}
                onKeyDown={
                  clickable
                    ? (e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          onOpen(row.name);
                        }
                      }
                    : undefined
                }
                className={
                  "flex items-center justify-between gap-3 border-b border-border px-3 py-2 text-sm last:border-b-0" +
                  (clickable ? " cursor-pointer hover:bg-muted" : "") +
                  (isOpen ? " bg-muted" : "")
                }
              >
                <div className="flex min-w-0 items-center gap-2">
                  {row.error ? (
                    <span aria-hidden="true">⚠️</span>
                  ) : row.result ? (
                    <FileThumbnail
                      fileName={row.name}
                      previewUrl={previewUrls?.get(row.result.name)}
                    />
                  ) : row.update ? (
                    <span aria-hidden="true">⏳</span>
                  ) : (
                    <span aria-hidden="true">⋯</span>
                  )}
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
                      <Badge variant={row.piiCount > 0 ? "warning" : "secondary"} size="sm">
                        {row.piiCount} PII
                      </Badge>
                    </>
                  )}
                  {row.edited && (
                    <Badge variant="info" size="sm" className="file-edited-badge">
                      edited
                    </Badge>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      </ScrollArea>
    </section>
  );
}
