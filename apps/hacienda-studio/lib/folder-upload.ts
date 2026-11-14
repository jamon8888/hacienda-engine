export interface FolderEntry {
  path: string;
  file: File;
}

export async function traverseFolder(entry: FileSystemDirectoryEntry): Promise<FolderEntry[]> {
  const results: FolderEntry[] = [];
  const reader = entry.createReader();

  const readEntries = async (): Promise<void> => {
    return new Promise((resolve, reject) => {
      reader.readEntries(async (entries, err) => {
        if (err) return reject(err);
        if (!entries.length) return resolve();
        for (const e of entries) {
          if (e.isFile) {
            const fileEntry = e as FileSystemFileEntry;
            const file = await new Promise<File>((resolve, reject) => fileEntry.file(resolve, reject));
            results.push({ path: entry.name + "/" + file.name, file });
          } else if (e.isDirectory) {
            results.push(...await traverseFolder(e as FileSystemDirectoryEntry));
          }
        }
        await readEntries();
      });
    });
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
