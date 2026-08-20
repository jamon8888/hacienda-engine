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

export async function listDocuments(): Promise<PersistedDocument[]> {
  const db = await getDB();
  return db.getAll(DOCUMENTS_STORE);
}

export async function listEditedFindings(): Promise<Map<string, PiiEntity[]>> {
  const db = await getDB();
  const keys = await db.getAllKeys(EDITED_FINDINGS_STORE);
  const values = await db.getAll(EDITED_FINDINGS_STORE);
  return new Map(keys.map((key, i) => [key, values[i]] as const));
}
