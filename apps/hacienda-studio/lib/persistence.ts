/**
 * Cross-session persistence for the document library (redesign Track: see
 * `apps/hacienda-studio` redesign plan). Separate IndexedDB database from
 * `lib/asset-loader.ts`'s `xberg-studio-assets` — different lifecycle (a user clearing
 * their processed documents should never touch cached model weights, and vice versa).
 *
 * Deliberately persists only what the mockup's own "Original not in this session" banner
 * implies should survive a reload: the processed `ProcessedFile` (markdown, rawMarkdown,
 * entities, piiFindings, frontmatter) and any manual finding edits layered on top of it.
 * Never the raw `File` bytes — those stay in-memory for the tab's lifetime only, same as
 * today. Every write here is write-through from the same events that already update
 * React state (`file-complete`, `handleAddFinding`, `handleRemoveFinding`, delete), not a
 * separate sync pass.
 */
import { openDB, type DBSchema, type IDBPDatabase } from "idb";
import type { ProcessedFile } from "./types";
import type { PiiEntity } from "./pii-engine";

const DB_NAME = "hacienda-studio-library";
const DB_VERSION = 1;
const DOCUMENTS_STORE = "documents";
const EDITED_FINDINGS_STORE = "editedFindings";

export interface PersistedDocument {
  result: ProcessedFile;
  /** When this record was written — surfaced in the Documents list, not load-bearing. */
  processedAt: number;
}

interface LibraryDB extends DBSchema {
  documents: {
    key: string;
    value: PersistedDocument;
  };
  editedFindings: {
    key: string;
    value: PiiEntity[];
  };
}

let dbPromise: Promise<IDBPDatabase<LibraryDB>> | null = null;

function getDB(): Promise<IDBPDatabase<LibraryDB>> {
  if (!dbPromise) {
    dbPromise = openDB<LibraryDB>(DB_NAME, DB_VERSION, {
      upgrade(db) {
        if (!db.objectStoreNames.contains(DOCUMENTS_STORE)) {
          db.createObjectStore(DOCUMENTS_STORE);
        }
        if (!db.objectStoreNames.contains(EDITED_FINDINGS_STORE)) {
          db.createObjectStore(EDITED_FINDINGS_STORE);
        }
      },
    });
  }
  return dbPromise;
}

/** Keyed by `ProcessedFile.name` — unique per batch, same key `App.tsx` already uses. */
export async function saveDocument(result: ProcessedFile): Promise<void> {
  const db = await getDB();
  await db.put(DOCUMENTS_STORE, { result, processedAt: Date.now() }, result.name);
}

export async function saveEditedFindings(
  name: string,
  findings: PiiEntity[],
): Promise<void> {
  const db = await getDB();
  await db.put(EDITED_FINDINGS_STORE, findings, name);
}

/** Removes a document and any edits layered on it. Both stores or neither. */
export async function deleteDocument(name: string): Promise<void> {
  const db = await getDB();
  const tx = db.transaction([DOCUMENTS_STORE, EDITED_FINDINGS_STORE], "readwrite");
  await Promise.all([
    tx.objectStore(DOCUMENTS_STORE).delete(name),
    tx.objectStore(EDITED_FINDINGS_STORE).delete(name),
  ]);
  await tx.done;
}

/** Drops the whole library — used by a "Clear all" action, not called automatically. */
export async function clearLibrary(): Promise<void> {
  const db = await getDB();
  const tx = db.transaction([DOCUMENTS_STORE, EDITED_FINDINGS_STORE], "readwrite");
  await Promise.all([
    tx.objectStore(DOCUMENTS_STORE).clear(),
    tx.objectStore(EDITED_FINDINGS_STORE).clear(),
  ]);
  await tx.done;
}

/** A record written by an earlier build can be missing a field a newer `ProcessedFile`
 * shape added — `pages/DocumentDetail.tsx` reads `result.rawMarkdown`/`result.entities`
 * directly off whatever `listDocuments` returns and throws on `undefined`. Not full
 * schema validation (that's a bigger lift for a browser-only, single-writer store): just
 * the fields this app's own code actually dereferences without a guard. */
function isCompletePersistedDocument(value: unknown): value is PersistedDocument {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  const result = v.result as Record<string, unknown> | undefined;
  return (
    typeof v.processedAt === "number" &&
    typeof result === "object" &&
    result !== null &&
    typeof result.name === "string" &&
    typeof result.markdown === "string" &&
    typeof result.rawMarkdown === "string" &&
    Array.isArray(result.entities) &&
    Array.isArray(result.piiFindings) &&
    typeof result.frontmatter === "object" &&
    result.frontmatter !== null
  );
}

export async function listDocuments(): Promise<PersistedDocument[]> {
  const db = await getDB();
  const all = await db.getAll(DOCUMENTS_STORE);
  return all.filter((doc) => {
    if (isCompletePersistedDocument(doc)) return true;
    console.warn("[persistence] Skipping malformed persisted document (written by an older build?):", doc);
    return false;
  });
}

export async function listEditedFindings(): Promise<Map<string, PiiEntity[]>> {
  const db = await getDB();
  // A single read-only transaction, not separate `getAllKeys`/`getAll` calls (each of
  // which opened its own transaction) — a `saveEditedFindings`/`deleteDocument` landing
  // between those two calls could change the store's length or order, so the old code
  // could zip a key from one snapshot with a value from another and attach one
  // document's PII edits to a different document's name.
  const tx = db.transaction(EDITED_FINDINGS_STORE, "readonly");
  const entries = new Map<string, PiiEntity[]>();
  let cursor = await tx.store.openCursor();
  while (cursor) {
    entries.set(cursor.key, cursor.value);
    cursor = await cursor.continue();
  }
  await tx.done;
  return entries;
}
