export interface FolderEntry {
  path: string;
  file: File;
}

export async function traverseFolder(entry: FileSystemDirectoryEntry, parentPath = ""): Promise<FolderEntry[]> {
  const results: FolderEntry[] = [];
  const reader = entry.createReader();
  const currentPath = parentPath ? `${parentPath}/${entry.name}` : entry.name;

  const readEntries = async (): Promise<void> => {
    const entries = await new Promise<FileSystemEntry[]>((resolve, reject) => {
      reader.readEntries(resolve, reject);
    });
    if (!entries.length) return;
    for (const e of entries) {
      if (e.isFile) {
        const fileEntry = e as FileSystemFileEntry;
        const file = await new Promise<File>((resolve, reject) => fileEntry.file(resolve, reject));
        results.push({ path: `${currentPath}/${file.name}`, file });
      } else if (e.isDirectory) {
        results.push(...await traverseFolder(e as FileSystemDirectoryEntry, currentPath));
      }
    }
    await readEntries();
  };

  await readEntries();
  return results;
}

export function handleFolderDrop(items: DataTransferItemList): Promise<FolderEntry[]> {
  const files: Promise<FolderEntry[]>[] = [];
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (item.kind === "file") {
      const entry = (item as any).webkitGetAsEntry?.();
      if (entry && entry.isDirectory) {
        files.push(traverseFolder(entry));
      } else if (entry && entry.isFile) {
        const file = item.getAsFile();
        if (file) files.push(Promise.resolve([{ path: file.name, file }]));
      }
    }
  }
  return Promise.all(files).then(arr => arr.flat());
}

/** Traverses dropped directory entries and returns files with relative paths. */
export async function traverseDataTransferItems(items: readonly DataTransferItem[]): Promise<File[]> {
  const entries = await handleFolderDrop(items as DataTransferItemList);
  return entries.map(({ path, file }) => {
    Object.defineProperty(file, "webkitRelativePath", { value: path, configurable: true });
    return file;
  });
}

/** Converts folder files to files whose names include their relative paths. */
export function flattenFolderFiles(files: readonly File[]): File[] {
  return files.map(file => {
    const path = file.webkitRelativePath || file.name;
    return path === file.name ? file : new File([file], path, { type: file.type, lastModified: file.lastModified });
  });
}

/** Applies each file's relative path to its effective filename. */
export function withRelativePath(files: readonly File[]): File[] {
  return flattenFolderFiles(files);
}
