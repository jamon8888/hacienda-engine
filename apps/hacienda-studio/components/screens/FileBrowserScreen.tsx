/**
 * Screen 4 of 5: the processed-batch file list. Near-verbatim relocation of `App.tsx`'s
 * `<FileBrowser>` section — `openName` is always unset here since an "open" file now
 * routes to its own `DetailScreen` instead of expanding inline. Adds "Add more files",
 * the `browser → upload` transition the old single-page layout didn't need (uploading
 * more used to just mean dropping onto the same still-visible drop zone).
 */
import { FileBrowser, type FileRow } from "../FileBrowser";

export function FileBrowserScreen({
  rows,
  onOpen,
  onAddMore,
  onDownloadZip,
  previewUrls,
}: {
  rows: FileRow[];
  onOpen: (name: string) => void;
  onAddMore: () => void;
  onDownloadZip: () => void;
  previewUrls: Map<string, string>;
}) {
  // Only files that finished processing have content worth zipping — matches
  // `FileRow.openable`, which is also gated on a completed `result`.
  const hasCompletedFiles = rows.some((r) => r.openable);
  return (
    <section aria-label="File browser" className="mx-auto w-full max-w-[800px] flex-1 px-6 py-12">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold text-muted-foreground">Files ({rows.length})</h2>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="download-zip rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
            onClick={onDownloadZip}
            disabled={!hasCompletedFiles}
          >
            Download redacted zip
          </button>
          <button
            type="button"
            className="add-more-files rounded-md bg-muted px-3 py-1.5 text-xs text-foreground hover:bg-primary hover:text-primary-foreground"
            onClick={onAddMore}
          >
            Add more files
          </button>
        </div>
      </div>
      <FileBrowser rows={rows} openName={null} onOpen={onOpen} previewUrls={previewUrls} />
    </section>
  );
}
