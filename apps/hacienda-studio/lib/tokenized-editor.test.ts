import { describe, expect, it } from "vitest";
import {
  applyManualRedaction,
  redactMetadata,
  textFromTokenizedDocument,
  type TokenizedEditorDocument,
} from "./tokenized-editor";

const documentWithSensitiveSelection: TokenizedEditorDocument = {
  type: "doc",
  content: [
    {
      type: "paragraph",
      content: [{ type: "text", text: "Contact Alex about the matter." }],
    },
  ],
};

describe("tokenized editor seams", () => {
  it("redacts title and metadata before building the editor envelope", () => {
    const issued: string[] = [];
    const metadata = redactMetadata(
      {
        title: "Alex Martin — merger",
        filename: "alex-martin.docx",
        authors: ["Alex Martin"],
        subject: "Project Falcon",
        keywords: ["Alex", "merger"],
      },
      ({ value, tokenClass }) => {
        issued.push(`${tokenClass}:${value}`);
        return `${tokenClass}-opaque-${issued.length}`;
      },
    );

    expect(metadata.title).toBe("[TITLE:title-opaque-1]");
    expect(metadata.filename).toBe("[FILENAME:filename-opaque-2]");
    expect(metadata.authors).toEqual(["[PERSON:person-opaque-3]"]);
    expect(metadata.subject).toBe("[MATTER:matter-opaque-4]");
    expect(metadata.keywords).toEqual([
      "[KEYWORD:keyword-opaque-5]",
      "[KEYWORD:keyword-opaque-6]",
    ]);
    expect(JSON.stringify(metadata)).not.toContain("Alex Martin");
    expect(issued).toContain("title:Alex Martin — merger");
  });

  it("manually anonymizes an undetected selection and sends plaintext only to the vault seam", () => {
    const vaultValues: string[] = [];
    const next = applyManualRedaction({
      document: documentWithSensitiveSelection,
      from: 8,
      to: 12,
      category: "person",
      mode: "pseudonymize",
      issueToken: ({ value }) => {
        vaultValues.push(value);
        return "person-opaque-1";
      },
    });

    expect(textFromTokenizedDocument(next)).toBe(
      "Contact [PERSON:person-opaque-1] about the matter.",
    );
    expect(JSON.stringify(next)).not.toContain("Alex");
    expect(vaultValues).toEqual(["Alex"]);
  });

  it("manually masks a selection without creating a rehydratable value", () => {
    const next = applyManualRedaction({
      document: documentWithSensitiveSelection,
      from: 8,
      to: 12,
      category: "person",
      mode: "manual-mask",
    });

    expect(textFromTokenizedDocument(next)).toBe(
      "Contact [PERSON] about the matter.",
    );
    expect(JSON.stringify(next)).not.toContain("Alex");
  });
});
