import { describe, it, expect, afterEach, vi } from "vitest";
import { fetchAsset, validateFile } from "./asset-loader";

function file(type: string, size = 1024): File {
  return new File([new Uint8Array(size)], "upload", { type });
}

function respondWith(
  body: string | Uint8Array<ArrayBuffer>,
  { status = 200, contentType = "application/octet-stream" } = {},
): void {
  const bytes = typeof body === "string" ? new TextEncoder().encode(body) : body;
  vi.stubGlobal(
    "fetch",
    vi.fn(
      async () =>
        new Response(bytes, {
          status,
          headers: { "content-type": contentType },
        }),
    ),
  );
}

/**
 * Regression: a URL that does not resolve is answered by the SPA fallback with
 * index.html and HTTP 200. `response.ok` was therefore true, and the HTML
 * reached the WebAssembly loader as model weights — surfacing much later as
 * "expected magic word 00 61 73 6d, found 3c 21 64 6f". This has bitten the
 * vertical taxonomies, the xberg WASM binary and the model URL, so the guard
 * has to hold for both the honest and the mislabelled case.
 */
describe("fetchAsset", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns the body when the response is a real asset", async () => {
    const weights = new Uint8Array([0x00, 0x61, 0x73, 0x6d]);
    respondWith(weights);

    await expect(fetchAsset("/model.safetensors")).resolves.toEqual(weights);
  });

  it("rejects an SPA fallback that answers HTML with HTTP 200", async () => {
    respondWith("<!doctype html><html></html>", { contentType: "text/html" });

    await expect(fetchAsset("/model.safetensors")).rejects.toThrow(
      /returned HTML/,
    );
  });

  it("rejects HTML that is mislabelled with a binary content type", async () => {
    respondWith("<!doctype html><html></html>");

    await expect(fetchAsset("/model.safetensors")).rejects.toThrow(
      /begins with '<'/,
    );
  });

  it("reports the status when the request fails outright", async () => {
    respondWith("not found", { status: 404, contentType: "text/plain" });

    await expect(fetchAsset("/model.safetensors")).rejects.toThrow(/HTTP 404/);
  });

  it("accepts JSON assets, which huggingface.co serves as text/plain", async () => {
    respondWith('{"hidden_size":768}', { contentType: "text/plain" });

    const bytes = await fetchAsset("/encoder_config/config.json");

    expect(JSON.parse(new TextDecoder().decode(bytes))).toEqual({
      hidden_size: 768,
    });
  });
});

/**
 * Track A3: audio/video were rejected outright before an upload ever reached the
 * worker. `SUPPORTED_MIME_PREFIXES`/`validateFile` used to be duplicated verbatim
 * in lib/types.ts (now removed, unimported, dead) — this is the copy `App.svelte`
 * actually calls.
 */
describe("validateFile", () => {
  it("accepts an audio file", () => {
    expect(validateFile(file("audio/mpeg"))).toEqual({ valid: true });
  });

  it("accepts a video file", () => {
    expect(validateFile(file("video/mp4"))).toEqual({ valid: true });
  });

  it("still rejects an empty file", () => {
    expect(validateFile(file("audio/mpeg", 0))).toEqual({
      valid: false,
      error: "File is empty",
    });
  });

  it("still rejects a file over 50MB", () => {
    expect(validateFile(file("audio/mpeg", 51 * 1024 * 1024))).toEqual({
      valid: false,
      error: "File too large (>50MB)",
    });
  });

  it("still rejects an unsupported type", () => {
    expect(validateFile(file("application/x-executable"))).toEqual({
      valid: false,
      error: "Unsupported file type: application/x-executable",
    });
  });
});
