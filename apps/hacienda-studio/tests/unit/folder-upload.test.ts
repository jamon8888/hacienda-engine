import { describe, it, expect } from "vitest";
import { traverseDataTransferItems, flattenFolderFiles, withRelativePath } from "@/lib/folder-upload";

function makeFileEntry(name: string): unknown {
  return {
    isFile: true,
    isDirectory: false,
    name,
    file(success: (f: File) => void) {
      const f = new File(["hello"], name, { type: "application/pdf" });
      success(f);
    },
  };
}

function makeReader(children: unknown[]): { readEntries: (cb: (e: unknown[]) => void) => void } {
  let done = false;
  return {
    readEntries(cb: (e: unknown[]) => void) {
      if (!done) {
        done = true;
        cb(children);
      } else {
        cb([]);
      }
    },
  };
}

describe("folder-upload", () => {
  it("traverses folder entry recursively", async () => {
    const fileEntry = makeFileEntry("doc.pdf");
    const subDir = {
      isFile: false,
      isDirectory: true,
      name: "sub",
      createReader: () => makeReader([fileEntry]),
    };
    const rootEntry = {
      isFile: false,
      isDirectory: true,
      name: "myFolder",
      createReader: () => makeReader([subDir]),
    };
    const mockItem = {
      kind: "file",
      getAsFile: () => null,
      webkitGetAsEntry: () => rootEntry,
    } as unknown as DataTransferItem;

    const files = await traverseDataTransferItems([mockItem]);
    expect(files.length).toBe(1);
    // traverse should synthesize webkitRelativePath containing sub/
    const rel = (files[0] as unknown as { webkitRelativePath: string }).webkitRelativePath;
    expect(rel).toContain("sub/");
  });

  it("flatten preserves relative path for zip export", () => {
    const f = new File(["hello"], "doc.pdf", { type: "application/pdf" });
    Object.defineProperty(f, "webkitRelativePath", { value: "myFolder/doc.pdf", writable: false, configurable: true });
    expect(flattenFolderFiles([f])[0].name).toBe("myFolder/doc.pdf");
  });

  it("withRelativePath keeps effectiveFileName for docPath", () => {
    const f = new File(["hello"], "doc.pdf", { type: "application/pdf" });
    Object.defineProperty(f, "webkitRelativePath", { value: "myFolder/sub/doc.pdf", writable: false, configurable: true });
    const mapped = withRelativePath([f]);
    expect(mapped[0].name).toBe("myFolder/sub/doc.pdf");
  });
});
