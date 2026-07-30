import { readFile } from "node:fs/promises";
import { test, expect, type Page } from "@playwright/test";
import JSZip from "jszip";

/**
 * Track D1: "one test where the output differs with the flag on and off" for
 * each toggle that already existed in the UI as dead configuration before
 * Tracks A/L wired it up. `enableTranscription`'s effect is asserted in
 * tests/e2e/audio.spec.ts (Track D3) instead of duplicated here — that test
 * already needs an audio fixture and a fresh page per toggle state, so it is
 * the natural place to exercise it. `enabledVerticals`'s effect is asserted
 * at the `VerticalDictionary` level in lib/verticals/dictionary.test.ts —
 * deterministic, unlike depending on the heuristic NER bridge extracting a
 * taxonomy term as a named entity in a full pipeline run.
 */
const NOTE =
  "Contact Jean Dupont at jean.dupont@cabinet-exemple.fr regarding the deal.";

async function visitFresh(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem("xberg-studio-visited", "true");
  });
  await page.goto("/");
  await page.waitForSelector('input[type="file"]:not([disabled])');
}

/**
 * Does not navigate — call `visitFresh` (and toggle any config) first. A
 * second `page.goto` here would reload the page and discard whatever config
 * change the caller just made in the (in-memory, not persisted) Svelte state.
 */
async function uploadAndDownload(
  page: Page,
  fileName: string,
): Promise<{ markdown: string; frontmatter: string }> {
  const download = page.waitForEvent("download");
  await page.setInputFiles('input[type="file"]', {
    name: fileName,
    mimeType: "text/plain",
    buffer: Buffer.from(NOTE),
  });

  const zip = await JSZip.loadAsync(
    await readFile(await (await download).path()),
  );
  const markdown = await zip
    .file("documents/" + fileName.replace(/\.[^.]+$/, ".md"))!
    .async("string");
  const frontmatter = markdown.split("---")[1] ?? "";
  return { markdown, frontmatter };
}

test.describe("enablePiiDetection", () => {
  test("off: no PII is detected at all", async ({ page }) => {
    await visitFresh(page);
    await page.click("button.config-toggle");
    await page.uncheck(
      'label:has-text("Enable PII Detection") input[type="checkbox"]',
    );
    await page.keyboard.press("Escape");

    const { markdown, frontmatter } = await uploadAndDownload(
      page,
      "off.txt",
    );

    expect(frontmatter).toContain("pii_entities_found: 0");
    expect(markdown).toContain("jean.dupont@cabinet-exemple.fr");
  });

  test("on (default): PII is detected and counted", async ({ page }) => {
    await visitFresh(page);

    const { frontmatter } = await uploadAndDownload(page, "on.txt");

    expect(frontmatter).not.toContain("pii_entities_found: 0");
    expect(frontmatter).toMatch(/pii_entities_found: [1-9]\d*/);
  });
});

test.describe("redactPiiInOutput", () => {
  test("off (default): detected PII is counted but left in the markdown", async ({
    page,
  }) => {
    await visitFresh(page);

    const { markdown, frontmatter } = await uploadAndDownload(
      page,
      "scan-only.txt",
    );

    expect(frontmatter).toMatch(/pii_entities_found: [1-9]\d*/);
    expect(markdown).toContain("jean.dupont@cabinet-exemple.fr");
  });

  test("on: detected PII is masked in the markdown", async ({ page }) => {
    await visitFresh(page);
    await page.click("button.config-toggle");
    await page.check(
      'label:has-text("Redact PII in Output") input[type="checkbox"]',
    );
    await page.keyboard.press("Escape");

    const { markdown } = await uploadAndDownload(page, "redacted.txt");

    expect(markdown).not.toContain("jean.dupont@cabinet-exemple.fr");
    expect(markdown).toContain("[EMAIL]");
  });
});
