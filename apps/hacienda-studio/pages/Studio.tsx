import { Archive, Trash2, Play, Check, Loader2 } from "lucide-react";
import { FileUpload } from "@/components/extend/file-upload";
import { Button } from "@/components/ui/button";
import { effectiveFileName } from "@/lib/file-filter";
import { exportDocumentsZip } from "@/lib/export-zip";
import type { ProcessedFile, ProgressUpdate } from "@/lib/types";

const UPLOAD_ACCEPT =
  ".pdf,.docx,.xlsx,.pptx,.odt,.ods,.odp,.eml,.msg,.pst,.png,.jpg,.jpeg,.gif,.webp,.tiff,.bmp,.svg,.srt,.vtt,.txt,.md,.json,.csv,.xml,.html,.mp3,.wav,.m4a,.ogg,.flac,.aac,.mp4,.mov,.webm,.mkv";

// Screenshot shows 6 pipeline pills — keep order exactly as rendered
const STAGE_ORDER = ["extract", "ocr", "chunk", "ner", "pii", "redact"] as const;
type DisplayStage = (typeof STAGE_ORDER)[number];

const STAGE_LABELS: Record<DisplayStage, string> = {
  extract: "extract",
  ocr: "ocr",
  chunk: "chunk",
  ner: "ner",
  pii: "pii",
  redact: "redact",
};

function mapToDisplayIndex(update: ProgressUpdate | undefined): number {
  if (!update) return -1;
  switch (update.stage) {
    case "queued":
    case "wasm-load":
    case "error":
      return -1;
    case "extract":
      // 10% start → extract active, 50% finish → extract done, ocr/chunk will be marked done on next stage
      return 0;
    case "transcribe":
      // transcription runs where OCR would for audio/video — light up ocr pill
      return 1;
    case "ner":
      return 3;
    case "pii":
      return 4;
    case "link":
      return 5;
    case "complete":
      return STAGE_ORDER.length; // all done
    default:
      return -1;
  }
}

function StagePills({ update }: { update: ProgressUpdate | undefined }) {
  const activeIndex = mapToDisplayIndex(update);
  const isComplete = update?.stage === "complete";
  return (
    <div className="flex flex-wrap gap-1.5">
      {STAGE_ORDER.map((stage, i) => {
        const done = isComplete || (activeIndex >= 0 && i < activeIndex);
        const isDone = done || (activeIndex >= 3 && i < 3);
        const active = !isComplete && i === activeIndex;
        return (
          <span
            key={stage}
            className={
              "rounded border px-1.5 py-0.5 font-mono text-[10px] leading-none tracking-wide " +
              (isDone
                ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-400"
                : active
                  ? "border-amber-500/40 bg-amber-500/15 text-amber-300"
                  : "border-white/[0.08] bg-white/[0.02] text-white/30")
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
  const isProcessing = files.length > 0;
  const processedCount = files.filter((f) => progress.get(effectiveFileName(f))?.stage === "complete").length;
  const totalCount = files.length;

  return (
    <div className="flex flex-1 flex-col bg-[#070a10] text-foreground">
      <div className="mx-auto flex w-full max-w-[720px] flex-col px-4 py-6 sm:px-6 sm:py-8">
        {/* ── Upload — always visible, stays on the same page as progress ── */}
        <div className="flex flex-col">
          <div className="mb-4 flex items-center justify-between">
            <div>
              <h1 className="text-xl font-semibold tracking-tight sm:text-2xl">Add documents</h1>
              <p className="mt-1 text-xs text-white/50 sm:text-sm">Files never leave this browser tab.</p>
            </div>
            <div className="flex items-center gap-2">
              {results.length > 0 && (
                <Button
                  size="sm"
                  variant="outline"
                  className="h-8 border-white/10 bg-white/[0.04] hover:bg-white/[0.08]"
                  onClick={() => exportDocumentsZip(results.map((result) => ({ result })))}
                >
                  <Archive className="size-3.5" /> Download
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
            className="mode-toggle mx-auto mt-3 block bg-transparent text-xs text-white/40 underline decoration-white/20 underline-offset-2 hover:text-white/70 disabled:cursor-not-allowed disabled:opacity-60"
            disabled={!workerReady}
            onClick={onToggleFolderMode}
          >
            {folderMode ? "or choose individual files" : "or choose a folder"}
          </button>

          {pendingFiles.length > 0 && (
            <section aria-labelledby="studio-pending-heading" className="mt-6 overflow-hidden rounded-xl border border-white/[0.06] bg-[#0f1419]">
              <h2 id="studio-pending-heading" className="sr-only">Selected files</h2>
              <ul>
                {pendingFiles.map((file, i) => (
                  <li
                    key={`${effectiveFileName(file)}-${i}`}
                    className="flex items-center justify-between gap-3 border-b border-white/[0.04] px-4 py-3 last:border-b-0"
                  >
                    <span className="truncate font-mono text-xs text-white/80">{effectiveFileName(file)}</span>
                    <div className="flex items-center gap-2">
                      <span className="font-mono text-[11px] text-white/30">{formatBytes(file.size)}</span>
                      <button
                        type="button"
                        aria-label={`Remove ${effectiveFileName(file)}`}
                        className="inline-flex size-7 items-center justify-center rounded-full bg-white/[0.04] text-white/40 hover:bg-destructive/15 hover:text-destructive"
                        onClick={() => onRemovePending(i)}
                      >
                        <Trash2 className="size-3.5" />
                      </button>
                    </div>
                  </li>
                ))}
              </ul>
              <div className="flex items-center justify-between bg-white/[0.02] px-4 py-3">
                <button type="button" className="text-xs text-white/50 hover:text-white" onClick={onClearPending}>
                  Clear all
                </button>
                <Button size="sm" className="h-8 bg-white px-4 text-xs font-medium text-[#070a10] hover:bg-white/90" onClick={onProcessQueue}>
                  <Play className="size-3.5" /> Process {pendingFiles.length} file{pendingFiles.length === 1 ? "" : "s"}
                </Button>
              </div>
            </section>
          )}
        </div>

        {/* ── Processing — stays in the SAME upload page, exactly like the screenshot ── */}
        {isProcessing && (
          <div className="mt-8 flex flex-col">
            <div className="mb-4">
              <h2 className="text-[15px] font-semibold tracking-tight">Processing</h2>
              <p className="mt-1 text-xs text-white/50">
                {processedCount} of {totalCount} finished · each file runs through the full pipeline independently.
              </p>
            </div>

            <div className="flex flex-col gap-2.5" aria-live="polite" aria-label="Processing queue">
              {files.map((file) => {
                const key = effectiveFileName(file);
                const update = progress.get(key);
                const result = resultsByInput.get(key);
                const error = fileErrors.get(key);
                const percent = update?.percent ?? (error ? 0 : 0);
                const isComplete = update?.stage === "complete";
                const isQueued = !update || update.stage === "queued" || update.stage === "wasm-load";
                const isError = !!error || update?.stage === "error";
                const statusLabel = isError ? "Failed" : isComplete ? "Complete" : isQueued ? "Queued" : update?.message || stageToHuman(update?.stage);
                const displayPercent = isComplete ? 100 : isQueued ? 0 : Math.round(percent);
                return (
                  <div
                    key={key}
                    className={
                      "rounded-lg border p-3.5 " +
                      (isError
                        ? "border-destructive/30 bg-[#1a1214]"
                        : isComplete
                          ? "border-white/[0.06] bg-[#151a24]"
                          : "border-white/[0.06] bg-[#151a24]")
                    }
                  >
                    <div className="flex items-center gap-2.5">
                      <span
                        className={
                          "flex size-3.5 shrink-0 items-center justify-center rounded-full text-[10px] " +
                          (isComplete
                            ? "bg-emerald-500/15 text-emerald-400 ring-1 ring-emerald-500/30"
                            : isError
                              ? "bg-destructive/15 text-destructive ring-1 ring-destructive/20"
                              : isQueued
                                ? "bg-white/[0.04] text-white/20 ring-1 ring-white/[0.06]"
                                : "bg-amber-500/15 text-amber-400 ring-1 ring-amber-500/30")
                        }
                        aria-hidden
                      >
                        {isComplete ? <Check className="size-2.5" strokeWidth={3} /> : isQueued ? <span className="size-1.5 rounded-full bg-white/20" /> : <Loader2 className="size-2.5 animate-spin" />}
                      </span>
                      <button
                        type="button"
                        className="min-w-0 flex-1 truncate text-left font-mono text-xs font-medium text-white/90 hover:text-white disabled:cursor-default disabled:text-white/90"
                        disabled={!result || isError}
                        onClick={() => result && onOpenDocument(result.name)}
                        title={key}
                      >
                        {key}
                      </button>
                      <span className="shrink-0 font-mono text-[11px] tabular-nums text-white/40">{displayPercent}%</span>
                    </div>

                    <div className="mt-3 h-1 overflow-hidden rounded-full bg-white/[0.06]">
                      <div
                        className="h-full rounded-full bg-[#e8a33d] transition-[width] duration-500 ease-out"
                        style={{ width: `${displayPercent}%` }}
                      />
                    </div>

                    <div className="mt-2.5">
                      <StagePills update={update} />
                    </div>
                    <p className="mt-1.5 truncate font-mono text-[10px] leading-none text-white/30">{statusLabel}</p>
                    {isError && <p className="mt-2 break-words text-xs leading-relaxed text-destructive">{error}</p>}
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {pendingFiles.length > 0 && isProcessing && (
          <div className="mt-3 rounded-lg border border-amber-500/15 bg-amber-500/[0.04] px-4 py-3 text-xs text-amber-200/70">
            {pendingFiles.length} more file{pendingFiles.length === 1 ? "" : "s"} in review queue — not yet sent to the pipeline.
          </div>
        )}
      </div>
    </div>
  );
}

function stageToHuman(stage: ProgressUpdate["stage"] | undefined): string {
  switch (stage) {
    case "extract":
      return "Extracting content";
    case "transcribe":
      return "Transcribing audio";
    case "ner":
      return "Scoring entities across chunks";
    case "pii":
      return "Detecting PII";
    case "link":
      return "Redacting & linking";
    default:
      return "Processing";
  }
}
