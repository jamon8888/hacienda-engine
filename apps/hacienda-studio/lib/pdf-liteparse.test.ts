import { describe, expect, it, vi } from "vitest";
import type { OCREngine } from "tesseract-wasm";
import { toLiteParseOcrEngine } from "./pdf-liteparse";

// `createImageBitmap` doesn't exist in vitest's `node` environment (this file's
// `recognize()` decodes PNG bytes with it, same as `worker/pipeline.ts`'s
// `selectOcrBridge` already does) — stub it so the bbox-mapping logic under test can
// actually run end to end instead of throwing before it gets there.
function stubCreateImageBitmap(): void {
  (globalThis as { createImageBitmap?: unknown }).createImageBitmap = vi
    .fn()
    .mockResolvedValue({ close: vi.fn() });
}

function fakeOcrEngine(
  lines: Array<{ text: string; confidence: number; rect: { left: number; top: number; right: number; bottom: number } }>,
): OCREngine {
  return {
    loadImage: vi.fn(),
    getTextBoxes: vi.fn().mockReturnValue(lines),
    clearImage: vi.fn(),
  } as unknown as OCREngine;
}

describe("toLiteParseOcrEngine", () => {
  it("maps tesseract-wasm line boxes to LiteParse's [x1,y1,x2,y2] bbox tuples", async () => {
    stubCreateImageBitmap();
    const engine = fakeOcrEngine([
      { text: "Hello", confidence: 0.97, rect: { left: 10, top: 20, right: 80, bottom: 40 } },
    ]);
    const adapter = toLiteParseOcrEngine(engine);

    const result = await adapter.recognize(new Uint8Array([1, 2, 3]), 100, 100, "eng");

    expect(result).toEqual([
      { text: "Hello", confidence: 0.97, bbox: [10, 20, 80, 40] },
    ]);
  });

  it("returns one entry per detected line, preserving order", async () => {
    stubCreateImageBitmap();
    const engine = fakeOcrEngine([
      { text: "Line one", confidence: 0.9, rect: { left: 0, top: 0, right: 50, bottom: 10 } },
      { text: "Line two", confidence: 0.85, rect: { left: 0, top: 10, right: 60, bottom: 20 } },
    ]);
    const adapter = toLiteParseOcrEngine(engine);

    const result = await adapter.recognize(new Uint8Array([1]), 100, 100, "eng");

    expect(result.map((r) => r.text)).toEqual(["Line one", "Line two"]);
  });

  it("clears the loaded image and frees the bitmap even when getTextBoxes throws", async () => {
    stubCreateImageBitmap();
    const engine = {
      loadImage: vi.fn(),
      getTextBoxes: vi.fn().mockImplementation(() => {
        throw new Error("boom");
      }),
      clearImage: vi.fn(),
    } as unknown as OCREngine;
    const adapter = toLiteParseOcrEngine(engine);

    await expect(adapter.recognize(new Uint8Array([1]), 10, 10, "eng")).rejects.toThrow("boom");
    expect(engine.clearImage).toHaveBeenCalledOnce();
  });
});
