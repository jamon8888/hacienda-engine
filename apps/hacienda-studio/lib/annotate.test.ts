import { describe, it, expect } from "vitest";
import { spliceRedaction, relativeDocumentLink } from "./annotate";

describe("spliceRedaction", () => {
  it("replaces the selected range with an uppercased category token", () => {
    const text = "Contact Jean Dupont for details.";
    const from = text.indexOf("Jean Dupont");
    const to = from + "Jean Dupont".length;

    expect(spliceRedaction(text, from, to, "person")).toBe(
      "Contact [PERSON] for details.",
    );
  });

  it("leaves text outside the selection untouched", () => {
    const text = "abcdef";
    expect(spliceRedaction(text, 2, 4, "other")).toBe("ab[OTHER]ef");
  });

  it("inserts at a collapsed selection without deleting anything", () => {
    const text = "abcdef";
    expect(spliceRedaction(text, 3, 3, "email")).toBe("abc[EMAIL]def");
  });
});

describe("relativeDocumentLink (Task 5.2)", () => {
  it("links two documents in the same directory with a bare filename, no ../", () => {
    expect(relativeDocumentLink("documents/2024/msa.md", "documents/2024/amendment.md")).toBe(
      "amendment.md",
    );
  });

  it("links two flat (non-nested) documents with a bare filename", () => {
    expect(relativeDocumentLink("documents/msa.md", "documents/invoice.md")).toBe("invoice.md");
  });

  it("goes up and back down for sibling subdirectories", () => {
    expect(relativeDocumentLink("documents/2024/msa.md", "documents/minutes/board.md")).toBe(
      "../minutes/board.md",
    );
  });

  it("descends into a subdirectory from a flat document", () => {
    expect(relativeDocumentLink("documents/msa.md", "documents/2024/amendment.md")).toBe(
      "2024/amendment.md",
    );
  });

  it("ascends from a subdirectory to a flat document", () => {
    expect(relativeDocumentLink("documents/2024/amendment.md", "documents/msa.md")).toBe(
      "../msa.md",
    );
  });

  it("handles differing nesting depth on both sides", () => {
    expect(
      relativeDocumentLink("documents/a/b/deep.md", "documents/x/shallow.md"),
    ).toBe("../../x/shallow.md");
  });
});
