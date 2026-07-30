import { describe, it, expect } from "vitest";
import { effectiveFileName, isJunkFile } from "./file-filter";

describe("effectiveFileName", () => {
  it("prefers webkitRelativePath when the picker set one", () => {
    expect(
      effectiveFileName({ name: "doc.pdf", webkitRelativePath: "deal/doc.pdf" }),
    ).toBe("deal/doc.pdf");
  });

  it("falls back to name when webkitRelativePath is absent", () => {
    expect(effectiveFileName({ name: "doc.pdf" })).toBe("doc.pdf");
  });

  it("falls back to name when webkitRelativePath is empty", () => {
    expect(
      effectiveFileName({ name: "doc.pdf", webkitRelativePath: "" }),
    ).toBe("doc.pdf");
  });
});

describe("isJunkFile", () => {
  it("flags macOS and Windows folder noise", () => {
    expect(isJunkFile(".DS_Store")).toBe(true);
    expect(isJunkFile("Thumbs.db")).toBe(true);
    expect(isJunkFile("thumbs.db")).toBe(true);
    expect(isJunkFile("desktop.ini")).toBe(true);
  });

  it("flags dotfiles generally", () => {
    expect(isJunkFile(".gitignore")).toBe(true);
    expect(isJunkFile(".env")).toBe(true);
  });

  it("checks the basename of a relative path, not the whole string", () => {
    expect(isJunkFile("deal/sub/.DS_Store")).toBe(true);
    expect(isJunkFile("deal/sub/contract.pdf")).toBe(false);
  });

  it("does not flag ordinary documents", () => {
    expect(isJunkFile("contract.pdf")).toBe(false);
    expect(isJunkFile("notes.txt")).toBe(false);
  });
});
