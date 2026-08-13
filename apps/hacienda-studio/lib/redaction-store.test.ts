import "fake-indexeddb/auto";
import { describe, it, expect } from "vitest";
import { saveDraft, loadDraft, deleteDraft, pruneDrafts } from "./redaction-store";

/**
 * Each test uses its own content-hash key rather than deleting/clearing the DB between
 * tests — `fake-indexeddb`'s `deleteDatabase` blocks indefinitely while any connection
 * from an earlier `openDB()` call is still open (neither `idb` nor this module's
 * `getDB()`, matching `lib/asset-loader.ts`'s uncached pattern, ever calls `.close()`),
 * which is a pre-existing hang in `asset-loader.test.ts`'s identical delete-in-afterEach
 * pattern too. Unique keys sidestep it rather than fixing that unrelated test file.
 */
describe("redaction-store", () => {
  it("round-trips a draft by content hash", async () => {
    await saveDraft("hash-round-trip", "redacted content");
    await expect(loadDraft("hash-round-trip")).resolves.toBe("redacted content");
  });

  it("returns undefined for a hash with no saved draft", async () => {
    await expect(loadDraft("hash-never-saved")).resolves.toBeUndefined();
  });

  it("overwrites a previously saved draft for the same hash", async () => {
    await saveDraft("hash-overwrite", "first");
    await saveDraft("hash-overwrite", "second");
    await expect(loadDraft("hash-overwrite")).resolves.toBe("second");
  });

  it("deleteDraft removes a saved draft", async () => {
    await saveDraft("hash-delete", "to be deleted");
    await deleteDraft("hash-delete");
    await expect(loadDraft("hash-delete")).resolves.toBeUndefined();
  });

  it("deleteDraft on a hash with no saved draft is a no-op", async () => {
    await expect(deleteDraft("hash-delete-never-saved")).resolves.toBeUndefined();
  });

  it("pruneDrafts removes only drafts older than maxAgeMs", async () => {
    const originalNow = Date.now;
    try {
      Date.now = () => 1_000_000;
      await saveDraft("hash-prune-old", "stale");
      Date.now = () => 1_000_000 + 60_000;
      await saveDraft("hash-prune-fresh", "recent");
      Date.now = () => 1_000_000 + 120_000;

      await pruneDrafts(90_000);

      await expect(loadDraft("hash-prune-old")).resolves.toBeUndefined();
      await expect(loadDraft("hash-prune-fresh")).resolves.toBe("recent");
    } finally {
      Date.now = originalNow;
    }
  });
});
