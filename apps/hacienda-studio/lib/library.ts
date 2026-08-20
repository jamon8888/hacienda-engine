/**
 * Groups processed documents into the folder tree the Documents page's sidebar shows.
 * `ProcessedFile.name` already carries the folder-relative path for directory uploads
 * (see `lib/file-filter.ts`'s `effectiveFileName`) — e.g. `"Téléchargements/README.md"` —
 * so no separate path field is persisted; the grouping is derived from the name alone,
 * same as `worker/pipeline.ts` already relies on for the export zip's folder structure.
 */
import type { ProcessedFile } from "./types";
import type { PiiEntity } from "./pii-engine";

export const ROOT_FOLDER = "";

export interface LibraryDocument {
  result: ProcessedFile;
  findings: PiiEntity[];
  folder: string;
  baseName: string;
}

export interface LibraryFolder {
  name: string;
  documents: LibraryDocument[];
}

export function folderOf(name: string): string {
  const idx = name.lastIndexOf("/");
  return idx === -1 ? ROOT_FOLDER : name.slice(0, idx);
}

export function baseNameOf(name: string): string {
  const idx = name.lastIndexOf("/");
  return idx === -1 ? name : name.slice(idx + 1);
}

export function groupByFolder(documents: LibraryDocument[]): LibraryFolder[] {
  const byFolder = new Map<string, LibraryDocument[]>();
  for (const doc of documents) {
    const list = byFolder.get(doc.folder) ?? [];
    list.push(doc);
    byFolder.set(doc.folder, list);
  }
  return [...byFolder.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([name, docs]) => ({ name, documents: docs }));
}
