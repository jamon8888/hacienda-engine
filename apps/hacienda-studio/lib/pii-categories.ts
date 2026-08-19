export const ADDABLE_CATEGORIES = [
  "person",
  "email",
  "phone",
  "address",
  "organization",
  "other",
] as const;

export type AddableCategory = (typeof ADDABLE_CATEGORIES)[number];

// Before `as const`, `ADDABLE_CATEGORIES` was a plain `string[]`, so
// `ADDABLE_CATEGORIES[0]` typed as `string | undefined` under `noUncheckedIndexedAccess` —
// RedactedEditor/MarkdownEditor seeded `useState` with that value directly, which is a type
// error anywhere `spliceRedaction`/`onAddFinding` require a plain `string`. `as const` turns
// the array into a fixed-length readonly tuple, so indexing it with a literal `0` is exact
// (no `undefined` in the type) — but exporting the default explicitly, rather than having
// every call site reach for `[0]`, keeps the "what's the default category" decision in one
// place. RedactedEditor/MarkdownEditor initialize `category` with this value.
export const DEFAULT_ADDABLE_CATEGORY: AddableCategory = ADDABLE_CATEGORIES[0];

/** Narrows a `<select>`'s `e.target.value` (always `string`) without an `as` assertion. */
export function isAddableCategory(value: string): value is AddableCategory {
  return (ADDABLE_CATEGORIES as readonly string[]).includes(value);
}
