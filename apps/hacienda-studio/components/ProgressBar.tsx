import type { ProgressUpdate } from "../lib/types";

const stageLabels: Record<ProgressUpdate["stage"], string> = {
  queued: "Queued",
  extract: "Extracting text",
  transcribe: "Transcribing audio",
  ner: "Finding entities",
  pii: "Scanning for PII",
  link: "Linking entities",
  complete: "Complete",
  error: "Error",
};

export function ProgressBar({
  file,
  update,
}: {
  /** Named loosely so both a DOM File and a worker FileInput satisfy it. */
  file: { name: string };
  update: ProgressUpdate;
}) {
  return (
    <div className="rounded-md border border-border bg-card p-3 text-card-foreground">
      <div className="mb-1.5 flex justify-between text-sm">
        <span className="max-w-[70%] overflow-hidden text-ellipsis whitespace-nowrap font-medium">
          {file.name}
        </span>
        <span className="text-muted-foreground">{stageLabels[update.stage]}</span>
      </div>
      <div
        className="h-1.5 overflow-hidden rounded-full bg-muted"
        role="progressbar"
        aria-valuenow={update.percent}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div
          className="h-full bg-primary transition-[width] duration-300 ease-out"
          style={{ width: `${update.percent}%` }}
        />
      </div>
      {update.message && (
        <p className="mt-1.5 text-xs text-muted-foreground">{update.message}</p>
      )}
    </div>
  );
}
