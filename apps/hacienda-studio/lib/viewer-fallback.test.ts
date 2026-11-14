import { describe, expect, it } from "vitest";
import { getViewerPlan } from "./viewer-fallback";

describe("viewer fallback seam", () => {
  it("uses the existing native viewer for supported PDF and Office formats", () => {
    expect(getViewerPlan("contract.pdf", "# [TITLE:k1:t1]")).toMatchObject({
      kind: "pdf",
      usesMarkdownFallback: false,
    });
    expect(getViewerPlan("contract.docx", "# [TITLE:k1:t1]")).toMatchObject({
      kind: "docx",
      usesMarkdownFallback: false,
    });
  });

  it("renders tokenized Markdown and exposes an explicit desktop-open link when authorized", () => {
    const plan = getViewerPlan("message.eml", "# [PERSON:k1:p1]", {
      canOpenOriginal: true,
    });

    expect(plan).toMatchObject({
      kind: null,
      usesMarkdownFallback: true,
      originalLink: { label: "Ouvrir l'original", available: true },
    });
    expect(plan.markdown).toContain("[PERSON:k1:p1]");
    expect(plan.markdown).not.toContain("Alex");
  });

  it("does not offer an original link without a validated desktop capability", () => {
    const plan = getViewerPlan("message.eml", "# [PERSON:k1:p1]");

    expect(plan.originalLink).toEqual({
      label: "Ouvrir l'original",
      available: false,
    });
  });
});
