/**
 * Shared by `worker/pipeline.ts` (per-document, per-mention slugs used to merge
 * repeated mentions of the same name within one document) and `lib/registry.ts`
 * (the canonical, cross-document slug that becomes an entity's filename in the
 * exported vault, `entities/<slug>.md` — Track I2). Both need the exact same
 * function, or two mentions of "Acme SAS" in different documents could compute
 * different slugs and silently split into two files.
 */
export function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .substring(0, 64);
}
