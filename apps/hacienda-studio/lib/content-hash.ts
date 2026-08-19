/**
 * Track K/Phase 4: autosave keys drafts by the original file's bytes, not its name — a
 * re-export from Word with different bytes is a new file with no draft, and (unlike a
 * filename) this can't collide across a folder upload's sibling directories.
 */
export async function computeContentHash(bytes: ArrayBuffer): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}
