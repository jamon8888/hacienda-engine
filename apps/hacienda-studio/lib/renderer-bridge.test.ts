import { describe, expect, it, vi } from "vitest";
import {
  createDesktopRendererBridge,
  createRendererBridge,
  toOpenOriginalInvokePayload,
  type OpenOriginalReferences,
} from "./renderer-bridge";

const references: OpenOriginalReferences = {
  documentRef: "doc-opaque-1",
  versionRef: "version-opaque-1",
  originalRef: "original-opaque-1",
  viewerRef: "viewer-opaque-1",
  capabilityRef: "capability-opaque-1",
};

describe("desktop renderer bridge", () => {
  it("builds an invoke payload from refs without carrying raw PII or content", async () => {
    const invoke = vi.fn(async () => undefined);
    const bridge = createDesktopRendererBridge(invoke);

    await bridge.openOriginal(references);

    expect(invoke).toHaveBeenCalledWith("open-original", {
      document_ref: "doc-opaque-1",
      version_ref: "version-opaque-1",
      original_ref: "original-opaque-1",
      viewer_ref: "viewer-opaque-1",
      capability_ref: "capability-opaque-1",
    });
    const payload = JSON.stringify(toOpenOriginalInvokePayload(references));
    expect(payload).not.toContain("Alex Martin");
    expect(payload).not.toContain("rawMarkdown");
    expect(payload).not.toContain("originalContent");
  });

  it("fails closed when Tauri invoke is not available", async () => {
    const bridge = createRendererBridge({});

    expect(bridge.canOpenOriginal).toBe(false);
    await expect(bridge.openOriginal(references)).rejects.toThrow(
      "Desktop original viewer is unavailable",
    );
  });
});
