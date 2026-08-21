import { Archive, Trash2, Play } from "lucide-react";
import { FileUpload } from "@/components/extend/file-upload";
import { Button } from "@/components/ui/button";
import { effectiveFileName } from "@/lib/file-filter";
import { exportDocumentsZip } from "@/lib/export-zip";
import type { ProcessedFile, ProgressUpdate } from "@/lib/types";

const UPLOAD_ACCEPT =
  ".pdf,.docx,.xlsx,.pptx,.odt,.ods,.odp,.eml,.msg,.pst,.png,.jpg,.jpeg,.gif,.webp,.tiff,.bmp,.svg,.srt,.vtt,.txt,.md,.json,.csv,.xml,.html,.mp3,.wav,.m4a,.ogg,.flac,.aac,.mp4,.mov,.webm,.mkv";

const STAGE_ORDER: ProgressUpdate["stage"][] = [
  "extract",
  "ner",
  "pii",
  "link",
  "complete",
];

const STAGE_LABELS: Record<ProgressUpdate["stage"], string> = {
  queued: "queued",
  "wasm-load": "wasm-load",
  extract: "extract",
  transcribe: "transcribe",
  ner: "ner",
  pii: "pii",
  link: "link",
  complete: "complete",
  error: "error",
};

function StagePills({ update }: { update: ProgressUpdate | undefined }) {
  const currentIndex = update ? STAGE_ORDER.indexOf(update.stage) : -1;
  return (
    <div className="flex flex-wrap gap-1.5">
      {STAGE_ORDER.map((stage, i) => {
        const done = update?.stage === "complete" || (currentIndex >= 0 && i < currentIndex);
        const active = update?.stage === stage && stage !== "complete";
        return (
          <span
            key={stage}
            className={
              "rounded-md border px-2 py-0.5 font-mono text-[11px] " +
              (done
                ? "border-emerald-500/40 font-medium text-emerald-500"
                : active
                  ? "border-primary font-medium text-primary"
                  : "border-border text-muted-foreground")
            }
          >
            {STAGE_LABELS[stage]}
          </span>
        );
      })}
    </div>
  );
}

function formatBytes(bytes: number): string {
  return bytes < 1024 * 1024
    ? `${Math.max(1, Math.round(bytes / 1024))} KB`
    : `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function Studio({
  workerReady,
  folderMode,
  onToggleFolderMode,
  onFilesAccepted,
  pendingFiles,
  onRemovePending,
  onClearPending,
  onProcessQueue,
  files,
  progress,
  results,
  fileErrors,
  onOpenDocument,
}: {
  workerReady: boolean;
  folderMode: boolean;
  onToggleFolderMode: (e: React.MouseEvent) => void;
  onFilesAccepted: (files: File[]) => void;
  /** Files dropped/picked but not yet sent to the worker — reviewable and removable
   * before the user confirms with "Process N files", matching the mockup's two-step
   * upload flow (queue, then process) instead of processing on drop. */
  pendingFiles: File[];
  onRemovePending: (index: number) => void;
  onClearPending: () => void;
  onProcessQueue: () => void;
  files: File[];
  progress: Map<string, ProgressUpdate>;
  results: ProcessedFile[];
  fileErrors: Map<string, string>;
  onOpenDocument: (name: string) => void;
}) {
  const resultsByInput = new Map(results.map((r) => [r.frontmatter.source, r] as const));
  const processedCount = files.filter(
    (f) => progress.get(effectiveFileName(f))?.stage === "complete",
  ).length;
  const isProcessing = files.length > 0;

  return (
    <div className="flex flex-1 flex-col px-6 py-10">
      <div className="mb-4 flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-semibold">Add documents</h1>
          <p className="text-sm text-muted-foreground">Files never leave this browser tab.</p>
        </div>
        <div className="flex items-center gap-3">
          {isProcessing && (
            <span className="text-sm text-muted-foreground">{processedCount} processed</span>
          )}
          {results.length > 0 && (
            <Button
              size="sm"
              variant="outline"
              onClick={() => exportDocumentsZip(results.map((result) => ({ result })))}
            >
              <Archive className="size-4" /> Download folder
            </Button>
          )}
        </div>
      </div>

      <FileUpload
        className="drop-zone"
        id="file-input"
        disabled={!workerReady}
        multiple
        filterAccept={false}
        showFileList={false}
        showBorderBeam={workerReady}
        webkitdirectory={folderMode}
        accept={folderMode ? undefined : UPLOAD_ACCEPT}
        inputAriaLabel={folderMode ? "Choose a folder" : "Choose files"}
        title={
          workerReady
            ? folderMode
              ? "Drop a folder here or click to browse"
              : "Drop files here or click to browse"
            : "Starting the local engine…"
        }
        description="PDF, Office, Email, Images, Audio/Video, Subtitles, Code — up to 50MB each"
        onFilesAccepted={onFilesAccepted}
      />
      <button
        type="button"
        className="mode-toggle mx-auto mt-4 block bg-transparent text-xs text-muted-foreground underline hover:text-primary disabled:cursor-not-allowed disabled:opacity-60"
        disabled={!workerReady}
        onClick={onToggleFolderMode}
      >
        {folderMode ? "or choose individual files" : "or choose a folder"}
      </button>

      {pendingFiles.length > 0 && (
        <section aria-labelledby="studio-pending-heading" className="mt-8 rounded-lg border border-border">
          <h2 id="studio-pending-heading" className="sr-only">
            Selected files
          </h2>
          <ul>
            {pendingFiles.map((file, i) => (
              <li
                key={`${effectiveFileName(file)}-${i}`}
                className="flex items-center justify-between gap-3 border-b border-border px-4 py-3 last:border-b-0"
              >
                <span className="truncate text-sm">{effectiveFileName(file)}</span>
                <div className="flex items-center gap-3">
                  <span className="text-xs text-muted-foreground">{formatBytes(file.size)}</span>
                  <button
                    type="button"
                    aria-label={`Remove ${effectiveFileName(file)}`}
                    className="inline-flex h-11 w-11 items-center justify-center rounded-md text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                    onClick={() => onRemovePending(i)}
                  >
                    <Trash2 className="size-4" />
                  </button>
                </div>
              </li>
            ))}
          </ul>
          <div className="flex items-center justify-between px-4 py-3">
            <button
              type="button"
              className="text-sm text-muted-foreground hover:text-foreground"
              onClick={onClearPending}
            >
              Clear all
            </button>
            <Button onClick={onProcessQueue}>
              <Play className="size-4" /> Process {pendingFiles.length} file
              {pendingFiles.length === 1 ? "" : "s"}
            </Button>
          </div>
        </section>
      )}

      {isProcessing && (
        <section
          aria-labelledby="studio-processing-heading"
          className="mt-8 flex flex-col gap-2"
          aria-live="polite"
        >
          <h2 id="studio-processing-heading" className="sr-only">
            Processing
          </h2>
          {files.map((file) => {
            const key = effectiveFileName(file);
            const update = progress.get(key);
            const result = resultsByInput.get(key);
            const error = fileErrors.get(key);
            return (
              <div
                key={key}
                className={
                  "rounded-lg border bg-card p-4 " +
                  (error ? "border-destructive/50" : "border-border")
                }
              >
                <div className="flex items-center justify-between gap-3">
                  <button
                    type="button"
                    className="truncate text-left text-sm font-medium disabled:cursor-default"
                    disabled={!result}
                    onClick={() => result && onOpenDocument(result.name)}
                  >
                    {key}
                  </button>
                  <span className="text-xs text-muted-foreground">
                    {error ? "Failed" : update?.stage === "complete" ? "Complete" : update ? "Processing" : "Queued"}
                  </span>
                </div>
                {error ? (
                  <p className="mt-2 text-xs text-destructive">{error}</p>
                ) : (
                  <>
                    <div className="mt-3">
                      <StagePills update={update} />
                    </div>
                    {update?.message && (
                      <p className="mt-2 text-xs text-muted-foreground">{update.message}</p>
                    )}
                  </>
                )}
              </div>
            );
          })}
        </section>
      )}
    </div>
  );
}
