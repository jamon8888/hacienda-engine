import { readFile } from "node:fs/promises";
import { test, expect, type Page } from "@playwright/test";
import JSZip from "jszip";

/**
 * Track D1: assert that Studio's config toggles actually change output, not just that
 * the app renders. `pipeline.spec.ts` already proves the happy path works end-to-end;
 * this suite proves each toggle is load-bearing — the exact gap that let
 * `enablePiiDetection`/`redactPiiInOutput` sit dead until Track L6 wired them.
 *
 * `enableTranscription`'s toggle-effect assertion is deliberately not here: it needs a
 * real audio fixture and a real Whisper model download, which is Track D3's job, not
 * duplicated here. See the plan's D1 entry for the explicit dependency.
 *
 * `enabledVerticals`'s toggle-effect assertion is also deliberately not here: the only
 * NER categories Studio ever detects are person/organization/location/date/time/money/
 * percent/email/phone/url (`NerCategory`), and a vertical's terms (`earnout`, `portco`,
 * `spa`, ...) only resolve when an NER hit's *text* happens to match one, which the
 * fallback compromise.js bridge does not deterministically produce — this suite runs
 * against the real in-browser NER, not a mocked one. `lib/verticals/dictionary.test.ts`
 * asserts the actual mechanism the toggle drives instead: building the dictionary from a
 * subset of `enabledVerticals` excludes the other verticals' terms.
 */

const NOTE_WITH_PII = "Contact Jean Dupont at jean.dupont@cabinet-exemple.fr regarding the file.";

async function skipOnboarding(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem("xberg-studio-visited", "true");
  });
}

async function openConfigPanel(page: Page): Promise<void> {
  await page.click('button:has-text("Config")');
}

async function processAndGetMarkdown(
  page: Page,
  fileName: string,
  contents: string,
): Promise<{ zip: JSZip; markdown: string }> {
  const download = page.waitForEvent("download");
  // Not `input[type="file"]`: Track I1 added a second file input
  // (`#folder-input`, for folder selection) with the same type, so that
  // selector now matches two elements.
  await page.setInputFiles("#file-input", {
    name: fileName,
    mimeType: "text/plain",
    buffer: Buffer.from(contents),
  });
  const zip = await JSZip.loadAsync(await readFile(await (await download).path()));
  // Track I2: documents live under `documents/` in the exported vault, not at zip root.
  const mdName = `documents/${fileName.replace(/\.[^.]+$/, ".md")}`;
  const markdown = await zip.file(mdName)!.async("string");
  return { zip, markdown };
}

test.describe("config toggles have effect (Track D1)", () => {
  test("enablePiiDetection: off finds nothing, on finds the IBAN/email", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector("#file-input:not([disabled])");

    await openConfigPanel(page);
    await page.getByLabel("Enable PII Detection").uncheck();

    const { markdown: off } = await processAndGetMarkdown(page, "toggle-pii-off.txt", NOTE_WITH_PII);
    expect(off).toContain("pii_entities_found: 0");

    await page.getByLabel("Enable PII Detection").check();

    const { markdown: on } = await processAndGetMarkdown(page, "toggle-pii-on.txt", NOTE_WITH_PII);
    expect(on).not.toContain("pii_entities_found: 0");
  });

  test("redactPiiInOutput: off leaves the email in the body, on masks it", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector("#file-input:not([disabled])");

    await openConfigPanel(page);
    // enablePiiDetection defaults on; redactPiiInOutput defaults off.
    const { markdown: unredacted } = await processAndGetMarkdown(page, "toggle-redact-off.txt", NOTE_WITH_PII);
    expect(unredacted).toContain("jean.dupont@cabinet-exemple.fr");

    await page.getByLabel("Redact PII in Output").check();

    const { markdown: redacted } = await processAndGetMarkdown(page, "toggle-redact-on.txt", NOTE_WITH_PII);
    expect(redacted).not.toContain("jean.dupont@cabinet-exemple.fr");
  });
});
