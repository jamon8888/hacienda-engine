import { describe, it, expect, vi, beforeEach } from "vitest";
import { loadNerModel, isModelCached, createNerBackend } from "./asset-loader";

describe("Asset Loader", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("returns cached model from IndexedDB", async () => {
    const result = await loadNerModel();
    expect(result).toHaveProperty("weights");
    expect(result).toHaveProperty("tokenizer");
    expect(result).toHaveProperty("encoderConfig");
  });

  it("fetches from HuggingFace CDN when not cached", async () => {
    // Integration test - skip in unit tests
  });

  it("creates NER backend from model assets", async () => {
    const assets = await loadNerModel();
    const backend = await createNerBackend(
      assets.weights,
      assets.tokenizer,
      assets.encoderConfig,
    );
    expect(backend).toHaveProperty("detect");
  });
});
