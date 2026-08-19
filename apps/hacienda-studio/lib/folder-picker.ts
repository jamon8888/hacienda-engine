/**
 * Cross-browser folder picker.
 *
 * On Chromium browsers (Chrome, Edge, Opera) we use the File System Access API
 * (`showDirectoryPicker()`) which opens a **real** "Select Folder" dialog on
 * macOS — unlike `webkitdirectory` which silently opens a confusing file picker.
 *
 * On other browsers (Safari, Firefox) we fall back to the classic
 * `webkitdirectory` trick, which works but the UX is poor on macOS.
 */

export interface FolderPickResult {
  files: File[];
  directoryName: string;
}

/**
 * Returns true when the browser supports the File System Access API's
 * `showDirectoryPicker()`.
 */
export function supportsDirectoryPicker(): boolean {
  return "showDirectoryPicker" in globalThis;
}

/**
 * Recursively read all files from a `FileSystemDirectoryHandle`.
 * Skips hidden files/dirs (names starting with `.`) and common OS junk.
 */
async function readDirectoryEntries(
  dirHandle: FileSystemDirectoryHandle,
  prefix: string,
): Promise<File[]> {
  const files: File[] = [];
  for await (const entry of dirHandle.values()) {
    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
    // Skip hidden files, Thumbs.db, .DS_Store, etc.
    const base = entry.name.toLowerCase();
    if (
      base.startsWith(".") ||
      base === "thumbs.db" ||
      base === "desktop.ini"
    ) {
      continue;
    }
    if (entry.kind === "file") {
      const file = await entry.getFile();
      // Create a new File with the relative path as `webkitRelativePath`
      // so downstream code (effectiveFileName, zip export) works correctly.
      const withPath = new File([file], path, {
        type: file.type,
        lastModified: file.lastModified,
      });
      // The browser doesn't let us set webkitRelativePath directly, but
      // we store it as a non-standard property for effectiveFileName().
      Object.defineProperty(withPath, "webkitRelativePath", {
        value: path,
        writable: false,
      });
      files.push(withPath);
    } else if (entry.kind === "directory") {
      files.push(...(await readDirectoryEntries(entry, path)));
    }
  }
  return files;
}

/**
 * Open the native "Select Folder" dialog and return all files inside it.
 *
 * Uses `showDirectoryPicker()` on Chromium (gives a real folder dialog on macOS)
 * and falls back to a hidden `<input webkitdirectory>` on other browsers.
 */
export async function pickFolder(): Promise<FolderPickResult | null> {
  if (supportsDirectoryPicker()) {
    try {
      const dirHandle = await showDirectoryPicker({ mode: "read" });
      const files = await readDirectoryEntries(dirHandle, "");
      return { files, directoryName: dirHandle.name };
    } catch (err: unknown) {
      // User cancelled the dialog
      if (err instanceof Error && err.name === "AbortError") return null;
      throw err;
    }
  }

  // Fallback: webkitdirectory input (Safari, Firefox, older Chromium)
  return webkitDirectoryFallback();
}

/**
 * Fallback folder picker using a hidden `<input webkitdirectory>`.
 * Returns a promise that resolves when the user picks a folder.
 */
function webkitDirectoryFallback(): Promise<FolderPickResult | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input") as HTMLInputElement & {
      // Nonstandard: not in TS's DOM lib, and not covered by
      // @types/wicg-file-system-access (that package types the separate,
      // standards-track File System Access API, not this legacy attribute).
      webkitdirectory: boolean;
    };
    input.type = "file";
    input.setAttribute("webkitdirectory", "");
    // On Safari, the attribute must also be set as a property
    input.webkitdirectory = true;
    input.multiple = true;
    input.style.display = "none";

    let resolved = false;

    input.addEventListener("change", () => {
      if (resolved) return;
      resolved = true;
      document.body.removeChild(input);

      const fileList = Array.from(input.files ?? []);
      const first = fileList[0];
      if (!first) {
        resolve(null);
        return;
      }

      // Extract directory name from the first file's webkitRelativePath
      const firstPath = first.webkitRelativePath || first.name;
      const directoryName = firstPath.split("/")[0] || "folder";

      resolve({ files: fileList, directoryName });
    });

    // If the user closes the dialog without picking, some browsers fire
    // 'cancel' but not 'change'. There's no 'cancel' event on file inputs,
    // so we use a timeout to detect the dialog was dismissed.
    input.addEventListener("cancel", () => {
      if (resolved) return;
      resolved = true;
      document.body.removeChild(input);
      resolve(null);
    });

    document.body.appendChild(input);
    input.click();

    // Fallback timeout: if nothing happens in 5 minutes, assume dismissed
    setTimeout(() => {
      if (!resolved) {
        resolved = true;
        if (input.parentNode) document.body.removeChild(input);
        resolve(null);
      }
    }, 300_000);
  });
}
