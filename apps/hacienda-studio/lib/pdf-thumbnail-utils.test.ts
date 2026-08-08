import { describe, it, expect, vi } from "vitest";

/**
 * Regression: @embedpdf/pdfium defaults to fetching pdfium.wasm from jsdelivr,
 * and font-fallback fetches per script from the same CDN. Either one is a
 * real document-content-egress violation the moment a PDF viewer goes live,
 * since this app's core product claim is zero network calls during
 * processing. See egress audit in
 * superpowers/specs/2026-07-28-hacienda-studio-client-app-design.md §11
 * gate 1.
 */
describe("pdf-thumbnail-utils egress", () => {
  it("resolves the pdfium wasm to a same-origin bundled asset, never a CDN", async () => {
    const { PDFIUM_WASM_URL } = await import("./pdf-thumbnail-utils");

    expect(PDFIUM_WASM_URL).not.toMatch(/jsdelivr/i);
    expect(PDFIUM_WASM_URL).toMatch(/pdfium\.wasm$/);
  });

  it("disables CDN font-fallback fetches when creating the pdfium engine", async () => {
    const createPdfiumEngine = vi.fn(async () => ({}));
    vi.doMock("@embedpdf/engines/pdfium-worker-engine", () => ({
      createPdfiumEngine,
    }));

    const { loadSharedPdfEngine, PDFIUM_WASM_URL } = await import(
      "./pdf-thumbnail-utils"
    );
    await loadSharedPdfEngine();

    expect(createPdfiumEngine).toHaveBeenCalledWith(PDFIUM_WASM_URL, {
      fontFallback: null,
    });

    vi.doUnmock("@embedpdf/engines/pdfium-worker-engine");
  });
});
