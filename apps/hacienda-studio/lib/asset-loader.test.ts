import "fake-indexeddb/auto";
import { describe, it, expect, afterEach, vi } from "vitest";
import { openDB } from "idb";
import { fetchAsset, validateFile, loadTessdata, loadNerModel } from "./asset-loader";

function file(type: string, size = 1024): File {
  return new File([new Uint8Array(size)], "upload", { type });
}

function respondWith(
  body: string | Uint8Array<ArrayBuffer>,
  { status = 200, contentType = "application/octet-stream" } = {},
): void {
  const bytes =
    typeof body === "string" ? new TextEncoder().encode(body) : body;
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
 * Track H — the 614MB model is downloaded as concurrent byte-range requests instead of one
 * stream (see `fetchAssetInRanges` in asset-loader.ts) because a single connection's TCP/QUIC
 * slow-start ramp otherwise dominates the transfer. `jamon8888/gliner2-guardrails-pii-f16`
 * confirmed via live requests that huggingface.co's CDN honors `Range` across its redirect and
 * allows the header cross-origin — these tests cover the two branches that matters if that
 * ever stops being true.
 */
describe("fetchAsset with parallel: true", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("falls back to a plain download when the server ignores Range", async () => {
    const weights = new Uint8Array([0x00, 0x61, 0x73, 0x6d]);
    respondWith(weights);

    await expect(
      fetchAsset("/model.safetensors", { parallel: true }),
    ).resolves.toEqual(weights);
  });

  it("downloads a large, range-capable asset as concurrent chunks and reassembles it in order", async () => {
    // Must clear RANGED_DOWNLOAD_MIN_BYTES (32MB) for the parallel path to engage, and divide
    // evenly by RANGED_DOWNLOAD_CONCURRENCY (6) so each range's boundary is exact. Filling and
    // then deep-comparing the full 36MB buffer (rather than checking a sentinel byte at each
    // chunk boundary) made this test itself the bottleneck: tens of millions of per-element JS
    // loop iterations plus Chai's generic `toEqual` on large typed arrays, compounding badly
    // under this host's memory pressure.
    const concurrency = 6;
    const chunkSize = 6 * 1024 * 1024;
    const total = chunkSize * concurrency;
    const full = new Uint8Array(total);
    for (let i = 0; i < concurrency; i++) {
      full[i * chunkSize] = i + 1; // first byte of each chunk
      full[(i + 1) * chunkSize - 1] = 100 + i; // last byte of each chunk
    }

    vi.stubGlobal(
      "fetch",
      vi.fn(async (_url: string, init?: RequestInit) => {
        const range = /^bytes=(\d+)-(\d+)$/.exec(
          (init?.headers as Record<string, string> | undefined)?.Range ?? "",
        );
        if (!range)
          throw new Error("expected every request to carry a Range header");
        const start = Number(range[1]);
        const end = Number(range[2]);
        return new Response(full.slice(start, end + 1), {
          status: 206,
          headers: {
            "content-type": "application/octet-stream",
            "content-range": `bytes ${start}-${end}/${total}`,
          },
        });
      }),
    );

    const onProgress = vi.fn();
    const bytes = await fetchAsset("/model.safetensors", {
      parallel: true,
      onProgress,
    });

    expect(bytes.length).toBe(total);
    for (let i = 0; i < concurrency; i++) {
      expect(bytes[i * chunkSize]).toBe(i + 1);
      expect(bytes[(i + 1) * chunkSize - 1]).toBe(100 + i);
    }
    expect(onProgress).toHaveBeenCalled();
    const last = onProgress.mock.calls.at(-1)?.[0];
    expect(last).toEqual({ receivedBytes: total, totalBytes: total });
  });

  /**
   * Regression: a server can pass the single-byte range probe (used only to learn total size
   * and Range support) yet still fail the real ranged fetches — e.g. a redirect to a presigned
   * CDN URL whose CORS policy allows a plain GET but not the preflight a `Range` header
   * triggers. `fetchAssetInRanges` used to let that exception propagate straight out of
   * `fetchAsset`, so the whole download failed *after* the progress bar had already climbed
   * toward 100% on the chunks that did succeed — indistinguishable, from the user's side, from
   * every other download failure, and with no fallback to the plain sequential path that has no
   * `Range` header and therefore no preflight to fail on.
   */
  it("falls back to a plain download when a chunk fails after the probe succeeds", async () => {
    // 33MB: past RANGED_DOWNLOAD_MIN_BYTES (32MB) so the parallel path engages, but kept small
    // — a `toEqual` deep-compare on the full buffer is what made the neighboring large-buffer
    // test above the bottleneck of this file, so this checks length plus sentinel bytes instead.
    const total = 33 * 1024 * 1024;
    const weights = new Uint8Array(total);
    weights[0] = 0x42;
    weights[total - 1] = 0x99;
    let sawRangeRequest = false;

    vi.stubGlobal(
      "fetch",
      vi.fn(async (_url: string, init?: RequestInit) => {
        const range = (init?.headers as Record<string, string> | undefined)
          ?.Range;
        if (range) {
          sawRangeRequest = true;
          if (range === "bytes=0-0") {
            // The probe: report ranges as supported so the parallel path engages.
            return new Response(weights.slice(0, 1), {
              status: 206,
              headers: { "content-range": `bytes 0-0/${total}` },
            });
          }
          // Every real ranged fetch fails — simulates a CORS preflight rejection.
          throw new TypeError("Failed to fetch");
        }
        // No Range header: the plain sequential fallback.
        return new Response(weights, {
          status: 200,
          headers: { "content-type": "application/octet-stream" },
        });
      }),
    );

    const bytes = await fetchAsset("/model.safetensors", { parallel: true });
    expect(bytes.length).toBe(total);
    expect(bytes[0]).toBe(0x42);
    expect(bytes[total - 1]).toBe(0x99);
    expect(sawRangeRequest).toBe(true);
  });
});

/**
 * Track A3: audio/video were rejected outright before an upload ever reached the
 * worker. `SUPPORTED_MIME_PREFIXES`/`validateFile` used to be duplicated verbatim
 * in lib/types.ts (now removed, unimported, dead) — this is the copy `App.tsx`
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

/**
 * Regression: `loadTessdata` used to be a stub that returned an empty `Uint8Array` on a
 * cache miss without ever fetching anything — the onboarding screen marked the "Tesseract
 * OCR Data" row ready while no `.traineddata` bytes existed anywhere. `fake-indexeddb/auto`
 * (already used by audit-handle.test.ts) gives a spec-compliant IndexedDB so the cache-hit
 * path can be exercised for real, not mocked.
 */
describe("loadTessdata", () => {
  // `getDB()` (asset-loader.ts) opens a connection per call and never closes it — fine for a
  // long-lived browser tab, but it means `indexedDB.deleteDatabase` here would block forever
  // waiting for a versionchange on a connection nothing ever closes. Clearing the object
  // store instead only needs a same-version transaction, which doesn't require exclusivity.
  afterEach(async () => {
    vi.unstubAllGlobals();
    const db = await openDB("xberg-studio-assets", 1);
    try {
      // Guards against a DB opened (by this very call, absent an upgrade callback) with
      // no "tessdata" store yet — possible if a test's own setup throws before ever
      // calling loadTessdata (which creates both stores). clear() would throw
      // NotFoundError in that case; without the try/finally that would also skip
      // db.close(), leaking the connection into the next test.
      await db.clear("tessdata");
    } finally {
      db.close();
    }
  });

  it("fetches and caches the language file on a cache miss", async () => {
    const traineddata = new Uint8Array([1, 2, 3, 4]);
    const fetchMock = vi.fn(
      async (_url: string) =>
        new Response(traineddata, {
          status: 200,
          headers: { "content-type": "application/octet-stream" },
        }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await expect(loadTessdata("eng")).resolves.toEqual(traineddata);

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0][0]).toContain("eng.traineddata");
  });

  it("skips the network on a cache hit", async () => {
    const traineddata = new Uint8Array([5, 6, 7, 8]);
    const fetchMock = vi.fn(
      async () =>
        new Response(traineddata, {
          status: 200,
          headers: { "content-type": "application/octet-stream" },
        }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await loadTessdata("eng");
    await expect(loadTessdata("eng")).resolves.toEqual(traineddata);

    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});


/**
 * `loadNerModel`'s storage-quota preflight (Track: CodeRabbit review on PR #98/#99).
 *
 * Covers the discriminated-union contract (`{ ok: true, assets }` /
 * `{ ok: false, reason: "insufficient-storage", ... }`) — expected conditions never throw
 * here — and the specific ordering bug CodeRabbit caught: the quota check ran before the
 * cache lookup, so a user whose model was already cached and needed no write at all could
 * still be rejected for "insufficient storage".
 */
describe("loadNerModel storage-quota preflight", () => {
  afterEach(async () => {
    vi.unstubAllGlobals();
    const db = await openDB("xberg-studio-assets", 1);
    try {
      await db.clear("models");
    } finally {
      db.close();
    }
  });

  function stubQuota(quotaBytes: number, usageBytes: number): void {
    vi.stubGlobal("navigator", {
      ...globalThis.navigator,
      storage: {
        estimate: async () => ({ quota: quotaBytes, usage: usageBytes }),
      },
    });
  }

  it("returns an insufficient-storage result instead of throwing when quota is too low", async () => {
    stubQuota(100 * 1024 * 1024, 0); // 100MB free, model needs ~620MB
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    const result = await loadNerModel();

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.reason).toBe("insufficient-storage");
      expect(result.message).toMatch(/storage/i);
    }
    // The whole point of a preflight: no ~600MB download was attempted.
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("checks the cache before the quota preflight, not after", async () => {
    // Pre-populate the cache directly, bypassing the network entirely.
    const db = await openDB("xberg-studio-assets", 1, {
      upgrade(db) {
        if (!db.objectStoreNames.contains("models")) db.createObjectStore("models");
      },
    });
    await db.put("models", new Uint8Array([1]), "gliner2-guardrails-pii-model");
    await db.put("models", new Uint8Array([2]), "gliner2-guardrails-pii-tokenizer");
    await db.put("models", new Uint8Array([3]), "gliner2-guardrails-pii-config");
    db.close();

    // Quota deliberately reports far too little space — if the preflight ran before the
    // cache lookup, this alone would fail the call despite nothing needing to be written.
    stubQuota(1, 0);
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    const result = await loadNerModel();

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.assets.model).toEqual(new Uint8Array([1]));
    }
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
