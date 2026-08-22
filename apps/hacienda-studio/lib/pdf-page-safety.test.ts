import { describe, it, expect, vi, beforeEach } from "vitest";

// `checkPdfPageSafety` dynamically imports `./pdf-thumbnail-utils` (to avoid pulling
// pdfium into every code path) — mock it so these tests exercise the page-count
// decision logic without a real PDF or the pdfium wasm engine.
const getPdfPageCount = vi.fn<(url: string) => Promise<number>>();
vi.mock("./pdf-thumbnail-utils", () => ({
  getPdfPageCount: (url: string) => getPdfPageCount(url),
}));

vi.stubGlobal("URL", {
  ...URL,
  createObjectURL: vi.fn(() => "blob:mock-url"),
  revokeObjectURL: vi.fn(),
});

import { checkPdfPageSafety } from "./asset-loader";

function pdfFile(name = "doc.pdf"): File {
  return new File([new Uint8Array(1024)], name, { type: "application/pdf" });
}

describe("checkPdfPageSafety", () => {
  beforeEach(() => {
    getPdfPageCount.mockReset();
  });

  it("passes non-PDF files through without checking page count", async () => {
    const file = new File([new Uint8Array(10)], "doc.docx", {
      type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    });
    const result = await checkPdfPageSafety(file);
    expect(result).toEqual({ valid: true });
    expect(getPdfPageCount).not.toHaveBeenCalled();
  });

  it("passes a normal-length PDF through unchanged", async () => {
    getPdfPageCount.mockResolvedValue(12);
    const result = await checkPdfPageSafety(pdfFile());
    expect(result.valid).toBe(true);
    expect(result.disableOcr).toBeUndefined();
  });

  it("disables OCR (but does not reject) for a PDF past the OCR-safe page limit", async () => {
    getPdfPageCount.mockResolvedValue(150);
    const result = await checkPdfPageSafety(pdfFile());
    expect(result.valid).toBe(true);
    expect(result.disableOcr).toBe(true);
    expect(result.warning).toMatch(/150 pages/);
  });

  it("rejects a PDF past the hard page-count cap", async () => {
    getPdfPageCount.mockResolvedValue(1000);
    const result = await checkPdfPageSafety(pdfFile());
    expect(result.valid).toBe(false);
    expect(result.error).toMatch(/1000 pages/);
  });

  it("fails open when the pdfium page-count check itself throws", async () => {
    getPdfPageCount.mockRejectedValue(new Error("corrupt PDF"));
    const result = await checkPdfPageSafety(pdfFile());
    expect(result.valid).toBe(true);
    expect(result.disableOcr).toBeUndefined();
  });
});
