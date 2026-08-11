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
 * Waits for a file's `FileBrowser` row to reach completion — the processing-complete signal
 * most specs actually want. The progress bar is cleared 1s after batch-complete (see
 * App.tsx's batch-complete handler), so we wait for the PII badge to appear (which is
 * always rendered as "N PII" or "0 PII") and the row to be visible.
 * `fileName` must match `data-file-row`, i.e. `effectiveFileName(file)`.
 *
 * 60s timeout: the full extract→ner→pii pipeline measurably exceeds a naive 15s on this
 * CPU-constrained host under parallel worker contention — 60s matches the existing
 * PBKDF2-derivation wait in pseudonymize.spec.ts for the same reason.
 */
export async function waitForFileRowDone(page: Page, fileName: string): Promise<void> {
  const row = page.locator(`[data-file-row="${fileName}"]`);
  await expect(row).toBeVisible({ timeout: 60000 });
  // PII badge is always present as "N PII" or "0 PII"
  await expect(row).toContainText("PII", { timeout: 60000 });
}

/** Waits for the file browser screen to appear (after batch-complete fires). */
export async function waitForBrowserScreen(page: Page): Promise<void> {
  await expect(page.locator('[aria-label="File browser"]')).toBeVisible({ timeout: 15000 });
}

/** Waits for the detail screen to appear after clicking a file row. */
export async function waitForDetailScreen(page: Page): Promise<void> {
  await expect(page.locator('section[aria-label="File detail"]')).toBeVisible({ timeout: 15000 });
}
