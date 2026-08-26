import { effectiveFileName } from "./file-filter";

function isFileEntry(entry: FileSystemEntry): entry is FileSystemFileEntry {
  return "file" in entry;
}

function isDirectoryEntry(entry: FileSystemEntry): entry is FileSystemDirectoryEntry {
  return "createReader" in entry;
}

function fileFromEntry(entry: FileSystemFileEntry): Promise<File> {
  return new Promise((resolve, reject) => {
    entry.file(resolve, reject);
  });
}

function readBatch(reader: FileSystemDirectoryReader): Promise<FileSystemEntry[]> {
  return new Promise((resolve, reject) => {
    reader.readEntries(resolve, reject);
  });
}

async function readAllEntries(reader: FileSystemDirectoryReader): Promise<FileSystemEntry[]> {
  const all: FileSystemEntry[] = [];
  while (true) {
    const batch = await readBatch(reader);
    if (batch.length === 0) break;
    for (const entry of batch) {
      all.push(entry);
    }
  }
  return all;
}

/**
 * Recursively traverse a FileSystemEntry, collecting files with synthesized
 * `webkitRelativePath` so the downstream `effectiveFileName -> ProcessedFile.name -> folderOf`
 * chain preserves the folder structure for zip export and library grouping.
 */
export async function traverseEntry(entry: FileSystemEntry, path: string, out: File[]): Promise<void> {
  if (entry.isFile && isFileEntry(entry)) {
    const file = await fileFromEntry(entry);
    const relative = path + file.name;
    try {
      Object.defineProperty(file, "webkitRelativePath", {
        value: relative,
        writable: false,
        configurable: true,
      });
    } catch {
      // Some environments make webkitRelativePath non-configurable; preserve best-effort
    }
    out.push(file);
    return;
  }
  if (entry.isDirectory && isDirectoryEntry(entry)) {
    const reader = entry.createReader();
    const entries = await readAllEntries(reader);
    const nextPath = path + entry.name + "/";
    for (const child of entries) {
      await traverseEntry(child, nextPath, out);
    }
  }
}

/**
 * For each DataTransferItem, try webkitGetAsEntry. If an entry exists, recurse via
 * traverseEntry; otherwise if kind=file, push the underlying File.
 * Mirrors the browser's `DataTransferItem.webkitGetAsEntry` -> `FileSystemDirectoryEntry`
 * flow used by folder drag-and-drop.
 */
export async function traverseDataTransferItems(items: DataTransferItem[]): Promise<File[]> {
  const files: File[] = [];
  for (const item of items) {
    const getter = item.webkitGetAsEntry;
    const entry = typeof getter === "function" ? getter.call(item) : null;
    if (entry !== null && entry !== undefined) {
      await traverseEntry(entry, "", files);
      continue;
    }
    if (item.kind === "file") {
      const file = item.getAsFile();
      if (file !== null) {
        files.push(file);
      }
    }
  }
  return files;
}

export interface FolderFile {
  readonly name: string;
  readonly file: File;
}

/**
 * Map File -> FolderFile keeping webkitRelativePath as name for docPath.
 * Already done via effectiveFileName but exposed for folder-upload consumers.
 */
export function withRelativePath(files: File[]): FolderFile[] {
  const out: FolderFile[] = [];
  for (const file of files) {
    const name = effectiveFileName(file);
    out.push({ name, file });
  }
  return out;
}

/**
 * Alias kept for the task's test contract: flatten preserves relative path for zip export.
 * Delegates to withRelativePath so both names remain valid.
 */
export function flattenFolderFiles(files: File[]): FolderFile[] {
  return withRelativePath(files);
}
