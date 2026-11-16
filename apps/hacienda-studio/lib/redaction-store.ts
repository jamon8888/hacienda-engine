/**
 * Track K/Phase 4: IndexedDB-backed autosave for the split view's redacted-markdown draft
 * (`App.tsx`'s `redactedDrafts`), so an edit survives a reload. A separate DB from
 * `lib/asset-loader.ts`'s `xberg-studio-assets` (model/tessdata cache) and
 * `AuditHandle`'s `hacienda-studio-audit` (an append-only audit log, wrong shape for a
 * mutable draft) — same `idb` `openDB` pattern as `asset-loader.ts`'s `getDB()`.
 */
import { openDB, type DBSchema, type IDBPDatabase } from "idb";
import type { TokenizedEditorEnvelope } from "./tokenized-editor";

const DB_NAME = "xberg-studio-drafts";
const DB_VERSION = 2;
const STORE = "redacted-drafts";
const TOKENIZED_STORE = "tokenized-envelopes";

interface DraftRecord {
  redactedMarkdown: string;
  savedAt: number;
}

interface DraftsDB extends DBSchema {
  "redacted-drafts": {
    key: string;
    value: DraftRecord;
  };
  "tokenized-envelopes": {
    key: string;
    value: { envelope: TokenizedEditorEnvelope; savedAt: number };
  };
}

// Not cached at module scope (unlike a naive singleton) — same reasoning as
// `lib/asset-loader.ts`'s `getDB()`: a cached, never-closed connection blocks any later
// `indexedDB.deleteDatabase` (used by this module's own tests, and by a user clearing
// storage) until the page reloads.
function getDB(): Promise<IDBPDatabase<DraftsDB>> {
  return openDB<DraftsDB>(DB_NAME, DB_VERSION, {
    upgrade(db) {
      if (!db.objectStoreNames.contains(STORE)) db.createObjectStore(STORE);
      if (!db.objectStoreNames.contains(TOKENIZED_STORE))
        db.createObjectStore(TOKENIZED_STORE);
    },
  });
}

function assertTokenOnlyEnvelope(envelope: TokenizedEditorEnvelope): void {
  const serialized = JSON.stringify(envelope);
  if (
    /"(?:plaintext|rawValue|originalText|secretValue)"\s*:/.test(serialized)
  ) {
    throw new Error("Tokenized editor persistence rejects plaintext fields");
  }
  for (const node of envelope.editor.content.flatMap(
    (block) => block.content ?? [],
  )) {
    const marked = node.marks?.some(
      (mark) => mark.type === "haciendaSensitive",
    );
    if (marked && !/^\[[A-Z0-9_]+:[^\]]+\]$|^\[[A-Z0-9_]+\]$/.test(node.text)) {
      throw new Error("Sensitive editor text must be represented by a token");
    }
  }
}

/** LanceDB-compatible seam: only the validated tokenized envelope crosses this store. */
export async function saveTokenizedEnvelope(
  key: string,
  envelope: TokenizedEditorEnvelope,
): Promise<void> {
  assertTokenOnlyEnvelope(envelope);
  const db = await getDB();
  await db.put(TOKENIZED_STORE, { envelope, savedAt: Date.now() }, key);
}

export async function loadTokenizedEnvelope(
  key: string,
): Promise<TokenizedEditorEnvelope | undefined> {
  const db = await getDB();
  return (await db.get(TOKENIZED_STORE, key))?.envelope;
}

export async function saveDraft(
  contentHash: string,
  redactedMarkdown: string,
): Promise<void> {
  const db = await getDB();
  await db.put(STORE, { redactedMarkdown, savedAt: Date.now() }, contentHash);
}

export async function loadDraft(
  contentHash: string,
): Promise<string | undefined> {
  const db = await getDB();
  const record = await db.get(STORE, contentHash);
  return record?.redactedMarkdown;
}

export async function deleteDraft(contentHash: string): Promise<void> {
  const db = await getDB();
  await db.delete(STORE, contentHash);
}

/**
 * Drops drafts older than `maxAgeMs`. The redacted markdown this store holds is
 * user-editable free text — a user can leave identifiers in it, or paste unredacted
 * content into it — so unbounded local retention is a retention risk this app cannot
 * otherwise let the user undo. Call on startup rather than only offering a manual
 * "clear" control, so a draft is bounded by default even if nobody ever looks for one.
 */
export async function pruneDrafts(maxAgeMs: number): Promise<void> {
  const db = await getDB();
  const cutoff = Date.now() - maxAgeMs;
  const tx = db.transaction(STORE, "readwrite");
  let cursor = await tx.store.openCursor();
  while (cursor) {
    if (cursor.value.savedAt < cutoff) await cursor.delete();
    cursor = await cursor.continue();
  }
  await tx.done;
}

/** Remove every saved redacted draft while leaving the document library untouched. */
export async function clearDrafts(): Promise<void> {
  const db = await getDB();
  await db.clear(STORE);
}
