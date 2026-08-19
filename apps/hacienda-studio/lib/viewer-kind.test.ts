import { describe, it, expect } from "vitest";
import { getViewerKind } from "./viewer-kind";

describe("getViewerKind", () => {
  it("returns the matching kind for every supported extension", () => {
    expect(getViewerKind("a.pdf")).toBe("pdf");
    expect(getViewerKind("a.docx")).toBe("docx");
    expect(getViewerKind("a.doc")).toBe("doc");
    expect(getViewerKind("a.xlsx")).toBe("xlsx");
    expect(getViewerKind("a.xls")).toBe("xls");
    expect(getViewerKind("a.pptx")).toBe("pptx");
    expect(getViewerKind("a.ppt")).toBe("ppt");
  });

  it("is case-insensitive on the extension", () => {
    expect(getViewerKind("A.PDF")).toBe("pdf");
  });

  it("returns null for a name with no extension", () => {
    expect(getViewerKind("README")).toBeNull();
  });

  it("returns null for an unrecognized extension", () => {
    expect(getViewerKind("a.txt")).toBeNull();
  });

  /**
   * DetailScreen.tsx has no native-viewer branch for OpenDocument formats — a
   * non-null ViewerKind for these would open a split view whose native-viewer
   * pane silently renders blank instead of falling back to the markdown-only
   * view. Must return null so `hasViewer` (DetailScreen.tsx) stays false.
   */
  it("returns null for OpenDocument formats (no native viewer exists)", () => {
    expect(getViewerKind("a.odt")).toBeNull();
    expect(getViewerKind("a.ods")).toBeNull();
    expect(getViewerKind("a.odp")).toBeNull();
  });
});
