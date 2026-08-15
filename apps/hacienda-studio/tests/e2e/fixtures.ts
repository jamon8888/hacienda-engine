import type { Page } from "@playwright/test";

/**
 * The default e2e suite must never fetch the real GLiNER2 model over the network: it is a
 * ~614 MB download (minutes even on a fast connection — the Phase 0 baseline measurement
 * against the prior ~1.2 GB F32 weights took over 7 minutes at this environment's ~2.9 MB/s;
 * the F16 weights below are roughly half that, still not something a test suite should fetch),
 * and once `worker/pipeline.ts`'s `initEngine()` stopped blocking the worker-ready handshake
 * on it (so the UI isn't frozen for that long on every uncached session), the fetch can now
 * overlap real document processing — which trips `egress.spec.ts`'s "no network while a
 * document is open" compliance assertion even though the request target (public model
 * weights, asset-loader.ts's own ALLOWED_HOSTS-equivalent) carries no client data.
 *
 * Routing the three asset URLs to tiny local bytes makes every test deterministic and fast
 * while still exercising the real fallback path: `NerModel.load()` rejects on non-model
 * bytes exactly like it would on a corrupted download, so `initNerBackend()`'s existing
 * catch sets `nerRuntime = null` and `selectNerBridge` falls back to the regex/compromise
 * bridge — the same path a real user hits whenever the neural backend fails to load. A
 * separate, manually-triggered smoke test (not part of this suite) is the place to exercise
 * the real download and the real neural backend.
 */
export async function mockNerModelAssets(page: Page): Promise<void> {
  await page.route(
    "https://huggingface.co/jamon8888/gliner2-guardrails-pii-f16/resolve/main/**",
    (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/octet-stream",
        body: Buffer.from([0, 1, 2, 3]),
      }),
  );
}

/**
 * Same rationale as `mockNerModelAssets` above, for `@remotion/whisper-web`'s own model
 * download (`ggml-{model}.bin`, `get-model-url.ts`): the real `tiny.en` weights alone are
 * ~74MB (`SIZES` in `@remotion/whisper-web/dist/constants.js`), so fetching them for real in
 * this suite would be exactly the slow, non-deterministic network dependency the NER mock
 * above exists to avoid — independent of whether a given sandbox can even reach
 * huggingface.co at all (this one cannot; see `audio.spec.ts`'s header).
 *
 * The fake bytes this returns are not a valid ggml model, so they only stand in for
 * *reaching* `downloadWhisperModel()`/`WhisperBridge.load()` on the main thread without a
 * real fetch — proving `canUseWhisperWeb()` no longer rejects on `window` being undefined
 * (Track D3's bug). Exercising a real download against genuinely valid model weights, and
 * real inference against them, is — like the NER model's real-download path above — left to
 * a separate, manually-triggered run against real network access, not this suite.
 */
export async function mockWhisperModelAssets(page: Page): Promise<void> {
  await page.route(
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/**",
    (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/octet-stream",
        body: Buffer.from([0, 1, 2, 3]),
      }),
  );
}

/** Mocks NER assets and marks onboarding as already dismissed, but does not navigate. */
export async function skipOnboarding(page: Page): Promise<void> {
  await mockNerModelAssets(page);
  await page.addInitScript(() => {
    localStorage.setItem("xberg-studio-visited", "true");
  });
}

/** `skipOnboarding` plus the navigation and worker-ready wait nearly every spec needs. */
export async function visitFresh(page: Page): Promise<void> {
  await skipOnboarding(page);
  await page.goto("/");
  await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });
}
