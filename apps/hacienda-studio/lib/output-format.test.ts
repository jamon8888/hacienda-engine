import { describe, expect, it } from "vitest";
import { formatExportContent, markdownToPlainText, outputFileName } from "./output-format";
import type { ProcessedFile } from "./types";

const result: ProcessedFile = {
  name: "note.md",
  markdown: "---\nsource: note.txt\n---\n\n# Redacted\n\n[EMAIL]",
  rawMarkdown: "Contact alice@example.com",
  entities: [],
  piiFindings: [],
  frontmatter: {
    source: "note.txt",
    type: "plain",
    processed: "2026-11-14T00:00:00.000Z",
    piiEntitiesFound: 1,
    entities: [],
  },
};

describe("output formats", () => {
  it("converts redacted markdown to plain text without frontmatter or glossary", () => {
    expect(
      markdownToPlainText("---\nsource: note.txt\n---\n\n# Redacted\n\n[EMAIL]\n\n## Entities\n\n- Alice"),
    ).toBe("Redacted\n\n[EMAIL]\n");
  });

  it("formats JSON from redacted content without including rawMarkdown", () => {
    const parsed = JSON.parse(formatExportContent(result, result.markdown, "json"));
    expect(parsed.content).toBe(result.markdown);
    expect(parsed.rawMarkdown).toBeUndefined();
    expect(parsed.piiEntitiesFound).toBe(1);
  });

  it("uses the selected format for downloaded filenames", () => {
    expect(outputFileName("note.md", "markdown")).toBe("note.md");
    expect(outputFileName("note.md", "plain")).toBe("note.txt");
    expect(outputFileName("note.md", "json")).toBe("note.json");
  });
});
