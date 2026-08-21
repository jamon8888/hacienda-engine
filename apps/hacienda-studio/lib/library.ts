/**
 * Groups processed documents into the folder tree the Documents page's sidebar shows.
 * `ProcessedFile.name` already carries the folder-relative path for directory uploads
 * (see `lib/file-filter.ts`'s `effectiveFileName`) — e.g. `"Téléchargements/README.md"` —
 * so no separate path field is persisted; the grouping is derived from the name alone,
 * same as `worker/pipeline.ts` already relies on for the export zip's folder structure.
 */
import type { ProcessedFile } from "./types";
import type { PiiEntity } from "./pii-engine";
import type { FileSystemFileItem } from "@/components/extend/file-system";

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

const EXTENSION_CONTENT_TYPES: Record<string, string> = {
  pdf: "application/pdf",
  docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  pptx: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  svg: "image/svg+xml",
};

function contentTypeFor(name: string): string | undefined {
  const ext = name.split(".").pop()?.toLowerCase();
  return ext ? EXTENSION_CONTENT_TYPES[ext] : undefined;
}

/**
 * Maps the library onto `components/extend/file-system.tsx`'s `FileSystem` component —
 * the macOS-Finder-style grid/list browser this redesign's Documents page uses. Folders
 * fall out of `path`'s slashes for free (the component infers them), so this is just a
 * per-document projection, not a second grouping pass.
 */
export function toFileSystemItems(documents: LibraryDocument[]): FileSystemFileItem[] {
  return documents.map((doc) => ({
    kind: "file",
    path: doc.result.name,
    name: doc.baseName,
    contentType: contentTypeFor(doc.result.name),
    size: new Blob([doc.result.markdown]).size,
    metadata: { findings: String(doc.findings.length) },
  }));
}
