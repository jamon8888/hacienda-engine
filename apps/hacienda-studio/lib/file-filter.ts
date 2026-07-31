/**
 * A directory picker (`webkitdirectory`) sets `webkitRelativePath`
 * (`"my-folder/sub/doc.pdf"`); a plain file picker or a single-file drag
 * leaves it empty, and `.name` (the basename) is all there is. Everything
 * downstream — the worker's `input.name`, the zip entry path, the progress
 * Map key — must agree on which one it's using, so this is the one place
 * that decides.
 */
export function effectiveFileName(file: {
  name: string;
  webkitRelativePath?: string;
}): string {
  return file.webkitRelativePath && file.webkitRelativePath.length > 0
    ? file.webkitRelativePath
    : file.name;
}

const JUNK_BASENAME_PATTERN = /^(thumbs\.db|desktop\.ini)$/i;

/**
 * A real folder contains OS noise no picker filters out: `.DS_Store`,
 * `Thumbs.db`, dotfiles. These aren't "unsupported files" to report — a user
 * dropping a folder never chose to include them and reporting them as
 * skipped would be alarm noise on every single folder upload.
 */
export function isJunkFile(nameOrRelativePath: string): boolean {
  const base = nameOrRelativePath.split("/").pop() ?? nameOrRelativePath;
  return base.startsWith(".") || JUNK_BASENAME_PATTERN.test(base);
}
