/**
 * Track I3: Finder-style file grid — uses Extend UI's FileThumbnail, Hugeicons,
 * and @pierre/trees icon resolver for consistent file-type rendering.
 */
import { useEffect, useState } from "react";
import { createFileTreeIconResolver, getBuiltInSpriteSheet } from "@pierre/trees";
import { Badge } from "./ui/badge";
import { ScrollArea } from "./ui/scroll-area";
import { effectiveFileName } from "../lib/file-filter";
import { renderPdfThumbnailUrl } from "../lib/pdf-thumbnail-utils";
import { getViewerKind } from "../lib/viewer-kind";
import { FileThumbnail } from "./extend/file-thumbnail";
import type { ProcessedFile, ProgressUpdate } from "../lib/types";
import type { PiiEntity } from "../lib/pii-engine";

export interface FileRow {
  name: string;
  update: ProgressUpdate | undefined;
  result: ProcessedFile | undefined;
  error: string | undefined;
  edited: boolean;
  piiCount: number;
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

const stageLabels: Record<string, string> = {
  queued: "Queued",
  extract: "Extracting",
  ner: "Finding entities",
  pii: "Scanning PII",
  link: "Linking",
  complete: "Done",
  error: "Error",
};

/** Sprite sheet for @pierre/trees file-type icons. */
const FILE_ICON_SPRITE_SHEET = getBuiltInSpriteSheet("complete");
const { resolveIcon } = createFileTreeIconResolver({ set: "complete", colored: false });

/** File-type icon using @pierre/trees icon resolver. */
function FileTypeIcon({
  fileName,
  className,
}: {
  fileName: string;
  className?: string;
}) {
  const icon = resolveIcon("file-tree-icon-file", fileName);
  return (
    <svg
      aria-hidden="true"
      viewBox={icon.viewBox ?? "0 0 16 16"}
      className={`shrink-0 text-muted-foreground ${className ?? ""}`}
      style={
        icon.token
          ? { color: `var(--fs-file-icon-${icon.token}, var(--color-muted-foreground))` }
          : undefined
      }
    >
      <use href={`#${icon.name}`} />
    </svg>
  );
}

/** Extract extension label from filename. */
function fileExtension(fileName: string): string {
  const dot = fileName.lastIndexOf(".");
  if (dot < 0) return "";
  return fileName.slice(dot + 1).toUpperCase();
}

/** Generic preview for files without a native viewer — icon + extension badge. */
function FileGenericPreview({ fileName }: { fileName: string }) {
  const ext = fileExtension(fileName);
  return (
    <div className="flex size-full flex-col items-center justify-center gap-1.5 bg-muted/50">
      <FileTypeIcon fileName={fileName} className="size-1/3 min-h-6 min-w-6" />
      {ext && (
        <span className="rounded bg-muted px-1.5 py-0.5 text-[min(0.625rem,18cqw)] font-semibold tracking-wide uppercase text-muted-foreground">
          {ext}
        </span>
      )}
    </div>
  );
}

/** PDF thumbnail via pdfium, falling back to FileGenericPreview. */
function FilePreview({
  fileName,
  previewUrl,
}: {
  fileName: string;
  previewUrl: string | undefined;
}) {
  const [thumbnailUrl, setThumbnailUrl] = useState<string | null>(null);
  const isPdf = getViewerKind(fileName) === "pdf";

  useEffect(() => {
    if (!isPdf || !previewUrl) {
      setThumbnailUrl(null);
      return;
    }
    let cancelled = false;
    renderPdfThumbnailUrl({ url: previewUrl, pageIndex: 0, width: 120 })
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

  return (
    <FileThumbnail
      file={{ name: fileName, type: "" }}
      previewImageUrl={thumbnailUrl}
      previewContent={!thumbnailUrl ? <FileGenericPreview fileName={fileName} /> : undefined}
      className="h-full w-full"
      previewClassName="aspect-[4/3]"
    />
  );
}

/** Mini progress bar shown under each file card during processing. */
function MiniProgress({ update }: { update: ProgressUpdate }) {
  if (update.stage === "complete") return null;
  return (
    <div className="mt-2 space-y-1">
      <div className="flex items-center justify-between text-[10px] text-muted-foreground">
        <span>{stageLabels[update.stage] ?? update.stage}</span>
        <span>{update.percent}%</span>
      </div>
      <div
        className="h-1 overflow-hidden rounded-full bg-muted"
        role="progressbar"
        aria-valuenow={update.percent}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div
          className="h-full rounded-full bg-primary transition-[width] duration-300 ease-out"
          style={{ width: `${update.percent}%` }}
        />
      </div>
    </div>
  );
}

/** Entity + PII count badges shown after processing completes. */
function FileStats({
  result,
  piiCount,
  edited,
}: {
  result: ProcessedFile;
  piiCount: number;
  edited: boolean;
}) {
  return (
    <div className="mt-2 flex flex-wrap items-center gap-1.5">
      <Badge variant="secondary" size="sm">
        {result.entities.length} {result.entities.length === 1 ? "entity" : "entities"}
      </Badge>
      <Badge variant={piiCount > 0 ? "warning" : "secondary"} size="sm">
        {piiCount} PII
      </Badge>
      {edited && (
        <Badge variant="info" size="sm" className="file-edited-badge">
          edited
        </Badge>
      )}
    </div>
  );
}

export function FileBrowser({
  rows,
  openName,
  onOpen,
  onDownloadFile,
  previewUrls,
}: {
  rows: FileRow[];
  openName?: string | null;
  onOpen?: (name: string) => void;
  onDownloadFile?: (result: ProcessedFile) => void;
  previewUrls?: Map<string, string>;
}) {
  if (rows.length === 0) return null;

  return (
    <section className="file-browser mt-6 overflow-hidden rounded-lg border border-border bg-background">
      <span aria-hidden="true" className="hidden" dangerouslySetInnerHTML={{ __html: FILE_ICON_SPRITE_SHEET }} />
      <ScrollArea className="max-h-[65vh]">
        <div className="grid grid-cols-2 gap-3 p-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5">
          {rows.map((row) => {
            const clickable = row.openable && !!onOpen;
            const isOpen = openName === row.name;
            const isProcessing = !!row.update && row.update.stage !== "complete" && row.update.stage !== "error";
            const result = row.result;
            const isError = !!row.error;

            // Not a native `<button>`: the download control below must be a real,
            // independently focusable `<button>`, and a `<button>` cannot contain
            // other interactive content (the HTML content model forbids nesting
            // interactive elements). A `div[role="button"]` with the same keyboard
            // handling a native button gives for free (Enter/Space activation) lets
            // the two controls be siblings instead.
            return (
              <div
                key={row.name}
                data-file-row={row.name}
                role="button"
                tabIndex={clickable ? 0 : -1}
                aria-disabled={!clickable}
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
                  "group flex flex-col rounded-lg border bg-card p-2 text-left transition-[border-color,background-color,box-shadow] " +
                  (clickable
                    ? "cursor-pointer hover:border-primary/50 hover:bg-accent/50 hover:shadow-sm"
                    : "cursor-default opacity-70") +
                  (isOpen ? " border-primary ring-1 ring-primary/20" : " border-border")
                }
              >
                {/* Thumbnail area */}
                <div className="relative aspect-[4/3] overflow-hidden rounded-md bg-muted">
                  {result ? (
                    <FilePreview
                      fileName={row.name}
                      previewUrl={previewUrls?.get(result.name)}
                    />
                  ) : isError ? (
                    <div className="flex h-full flex-col items-center justify-center gap-1.5">
                      <span className="text-3xl" aria-hidden="true">⚠️</span>
                      <span className="text-[10px] text-destructive">Failed</span>
                    </div>
                  ) : isProcessing ? (
                    <div className="flex h-full flex-col items-center justify-center gap-2">
                      <FileTypeIcon fileName={row.name} className="size-1/3 min-h-6 min-w-6" />
                      <div className="w-3/4">
                        <div className="h-1 overflow-hidden rounded-full bg-muted">
                          <div
                            className="h-full rounded-full bg-primary transition-[width] duration-300"
                            style={{ width: `${row.update?.percent ?? 0}%` }}
                          />
                        </div>
                      </div>
                    </div>
                  ) : (
                    <FileGenericPreview fileName={row.name} />
                  )}

                  {/* Download overlay on hover */}
                  {result && onDownloadFile && (
                    <div className="absolute inset-0 flex items-center justify-center bg-background/60 opacity-0 backdrop-blur-sm transition-opacity group-hover:opacity-100">
                      <button
                        type="button"
                        aria-label={`Download ${row.name}`}
                        title="Download this file"
                        className="rounded-full bg-primary p-2 text-primary-foreground shadow-md hover:bg-primary/90"
                        onClick={(e) => {
                          e.stopPropagation();
                          onDownloadFile(result);
                        }}
                      >
                        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="size-4">
                          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                          <polyline points="7 10 12 15 17 10" />
                          <line x1="12" y1="15" x2="12" y2="3" />
                        </svg>
                      </button>
                    </div>
                  )}
                </div>

                {/* File name */}
                <div className="mt-2 min-w-0">
                  <p className="truncate text-xs font-medium" title={row.name}>
                    {row.name}
                  </p>
                </div>

                {/* Progress bar (during processing) */}
                {isProcessing && row.update && (
                  <MiniProgress update={row.update} />
                )}

                {/* Error message */}
                {isError && (
                  <p className="mt-1 truncate text-[10px] text-destructive" title={row.error}>
                    {row.error}
                  </p>
                )}

                {/* Stats (after processing) */}
                {result && (
                  <FileStats result={result} piiCount={row.piiCount} edited={row.edited} />
                )}
              </div>
            );
          })}
        </div>
      </ScrollArea>
    </section>
  );
}
