/**
 * Track I1: folder ingest. Pure helpers extracted from `App.svelte` so the
 * "which files get processed and what does the user see when some don't"
 * logic is unit-testable without a browser.
 */

/**
 * The relative path a file was selected with — `File.webkitRelativePath`
 * when selected via a folder input, or just `File.name` for a flat
 * selection. Used both as this file's output path under `documents/` and as
 * the unique key progress tracking keys on: two files in different
 * subfolders can share a basename (`a/report.pdf` and `b/report.pdf`), so
 * `File.name` alone collides once folder ingest is possible.
 */
export function fileRelativePath(file: File): string {
  const webkitPath = (file as unknown as { webkitRelativePath?: string }).webkitRelativePath;
  return webkitPath && webkitPath.length > 0 ? webkitPath : file.name;
}

export interface SkippedFile {
  name: string;
  reason: string;
}

export interface PartitionedFiles {
  valid: File[];
  skipped: SkippedFile[];
}

/**
 * Splits a raw file selection into files Studio can process and files it
 * can't. A real folder drop mixes valid documents with `.DS_Store`,
 * thumbnails, lock files, etc. — the previous behaviour (a single shared
 * `error` string each invalid file overwrote) meant only the last skipped
 * file was ever visible, and gave no way to tell "3 files skipped" from
 * "everything failed".
 */
export function partitionFiles(
  files: File[],
  validate: (file: File) => { valid: boolean; error?: string },
): PartitionedFiles {
  const valid: File[] = [];
  const skipped: SkippedFile[] = [];
  for (const file of files) {
    const result = validate(file);
    if (result.valid) {
      valid.push(file);
    } else {
      skipped.push({ name: fileRelativePath(file), reason: result.error || "Invalid file" });
    }
  }
  return { valid, skipped };
}

/**
 * Formats `partitionFiles`'s skipped list into one banner message. Names
 * every file when there are only a few, but a folder can skip dozens
 * (OS metadata, thumbnails) — a wall of filenames is worse than a count.
 */
export function formatSkippedMessage(skipped: SkippedFile[]): string | null {
  if (skipped.length === 0) return null;
  if (skipped.length <= 3) {
    return skipped.map((s) => `${s.name}: ${s.reason}`).join("; ");
  }
  const shown = skipped
    .slice(0, 3)
    .map((s) => s.name)
    .join(", ");
  return `${skipped.length} files skipped (unsupported): ${shown}, and ${skipped.length - 3} more`;
}
