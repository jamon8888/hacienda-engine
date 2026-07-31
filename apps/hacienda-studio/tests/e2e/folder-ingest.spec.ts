import { readFile } from "node:fs/promises";
import { test, expect, type Page } from "@playwright/test";
import JSZip from "jszip";

/**
 * Track I1: a folder selection carries `File.webkitRelativePath`
 * (`"subfolder/report.txt"`), which Studio must carry all the way into the
 * exported vault so the folder's structure survives — the same guarantee
 * `pipeline.spec.ts` proves for a flat single-file drop, but for the second
 * input (`#folder-input`) and with a subfolder in play.
 */

async function skipOnboarding(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem("xberg-studio-visited", "true");
  });
}

test.describe("folder ingest (Track I1)", () => {
  test("preserves a folder's subdirectory structure under documents/ in the export", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector("#folder-input:not([disabled])");

    const download = page.waitForEvent("download");
    // Playwright sets `webkitRelativePath` from `name` on a `webkitdirectory`
    // input when `name` contains a `/` — mirroring what the browser does when
    // a real folder is selected.
    await page.setInputFiles("#folder-input", [
      {
        name: "contracts/2024/protocole.txt",
        mimeType: "text/plain",
        buffer: Buffer.from("Acme SAS acquired Beta SARL."),
      },
    ]);

    const zip = await JSZip.loadAsync(await readFile(await (await download).path()));

    expect(Object.keys(zip.files)).toContain("documents/contracts/2024/protocole.md");
    const markdown = await zip.file("documents/contracts/2024/protocole.md")!.async("string");
    expect(markdown).toContain("source: contracts/2024/protocole.txt");
  });
});
