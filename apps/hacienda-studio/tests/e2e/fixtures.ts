import { expect, type Page } from "@playwright/test";

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

/**
 * Waits for a file's `FileBrowser` row to reach "Done" — the processing-complete signal
 * most specs actually want, as opposed to `waitForEvent("download")`, which today happens
 * to fire at the same moment only because the batch zip still auto-downloads on completion.
 * `fileName` must match `data-file-row`, i.e. `effectiveFileName(file)` (the input file's
 * own name, not `frontmatter.source`).
 *
 * 60s timeout, not the `waitForEvent("download")` this replaces: that relied on Playwright's
 * 120s per-test default with no explicit budget of its own. The full extract→ner→pii pipeline
 * measurably exceeds a naive 15s on this CPU-constrained host under parallel worker contention
 * (see office-viewers/pseudonymize flakes during Phase 0 migration) — 60s matches the existing
 * PBKDF2-derivation wait in pseudonymize.spec.ts for the same reason.
 */
export async function waitForFileRowDone(page: Page, fileName: string): Promise<void> {
  const row = page.locator(`[data-file-row="${fileName}"]`);
  await expect(row).toContainText("Done", { timeout: 60000 });
}
