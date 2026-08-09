/**
 * Screen 3 of 5: processing queue. Near-verbatim relocation of `App.tsx`'s per-file
 * progress-bar section — shown while the worker is still working through the current
 * batch, replaced by the file-browser screen once every file settles (`batch-complete`).
 */
import { ProgressBar } from "../ProgressBar";
import { effectiveFileName } from "../../lib/file-filter";
import type { ProgressUpdate } from "../../lib/types";

export function QueueScreen({
  files,
  progress,
  completedCount,
}: {
  files: File[];
  progress: Map<string, ProgressUpdate>;
  completedCount: number;
}) {
  return (
    <section aria-label="Processing queue" className="mx-auto w-full max-w-[800px] flex-1 px-6 py-12">
      <p className="batch-summary text-center text-sm text-muted-foreground" aria-live="polite">
        {completedCount} / {files.length} processed
      </p>
      <section className="mt-6 flex flex-col gap-3" aria-live="polite">
        {files.map((file) => {
          const key = effectiveFileName(file);
          const update = progress.get(key);
          if (!update) return null;
          return <ProgressBar key={key} file={{ name: key }} update={update} />;
        })}
      </section>
    </section>
  );
}
