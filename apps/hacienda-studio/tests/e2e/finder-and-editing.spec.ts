import { readFile } from "node:fs/promises";
import { test, expect, type Page } from "@playwright/test";
import JSZip from "jszip";

/**
 * Track I3/I4: the Finder-like per-file list (`components/FileBrowser.tsx`) and
 * per-document PII add/remove editing (`App.tsx`'s `handleAddFinding`/`handleRemoveFinding`,
 * re-splicing through `lib/annotate.ts`'s `renderAnnotatedMarkdown` on export).
 */
const EMAIL = "jean.dupont@cabinet-exemple.fr";
const EMPLOYEE_ID = "458213";
const NOTE = `Contact Jean Dupont at ${EMAIL}. Employee id ${EMPLOYEE_ID} confirmed.`;

async function visitFresh(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem("xberg-studio-visited", "true");
  });
  await page.goto("/");
  await page.waitForSelector('input[type="file"]:not([disabled])');
}

/** Selects `NOTE[from:to]` by character offset — precise, unlike a pixel-based double-click. */
async function selectRange(page: Page, from: number, to: number): Promise<void> {
  const editor = page.locator(".cm-pii-editor .cm-content");
  await editor.click();
  await page.keyboard.press("Control+Home");
  for (let i = 0; i < from; i++) await page.keyboard.press("ArrowRight");
  for (let i = 0; i < to - from; i++) await page.keyboard.press("Shift+ArrowRight");
}

test.describe("FileBrowser (Track I3)", () => {
  test("shows the processed file with its entity and PII counts", async ({ page }) => {
    await visitFresh(page);

    const download = page.waitForEvent("download");
    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await download;

    const row = page.locator('[data-file-row="note.txt"]');
    await expect(row).toBeVisible();
    await expect(row).toContainText("Done");
    await expect(row).toContainText("PII");
    await expect(row.locator(".file-edited-badge")).toHaveCount(0);
  });
});

test.describe("Per-document PII editing (Track I4)", () => {
  test("removing a finding un-redacts it in the re-exported file", async ({ page }) => {
    await visitFresh(page);

    const download = page.waitForEvent("download");
    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await download;

    const findingCountBefore = await page.locator(".pii-finding-trigger").count();
    expect(findingCountBefore).toBeGreaterThan(0);

    await page.locator('[aria-label="Remove finding 1"]').click();

    await expect(page.locator('[data-file-row="note.txt"] .file-edited-badge')).toBeVisible();

    const exportDownload = page.waitForEvent("download");
    await page.locator(".export-edited").click();
    const exported = await readFile(await (await exportDownload).path(), "utf-8");

    // The removed finding's original text is no longer redacted in the export.
    expect(exported).toContain(EMAIL);
  });

  test("marking a selection as PII redacts it in the re-exported file", async ({ page }) => {
    await visitFresh(page);

    const download = page.waitForEvent("download");
    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await download;

    const from = NOTE.indexOf(EMPLOYEE_ID);
    const to = from + EMPLOYEE_ID.length;
    await selectRange(page, from, to);

    await page.selectOption(".pii-add-category", "other");
    await page.locator(".pii-mark-selection").click();

    const pill = page.locator(`.cm-pii-pill[data-pii-category="other"]`);
    await expect(pill).toBeVisible();
    await expect(pill).toHaveText(EMPLOYEE_ID);

    await expect(page.locator('[data-file-row="note.txt"] .file-edited-badge')).toBeVisible();

    const exportDownload = page.waitForEvent("download");
    await page.locator(".export-edited").click();
    const exported = await readFile(await (await exportDownload).path(), "utf-8");

    expect(exported).not.toContain(EMPLOYEE_ID);
    expect(exported).toContain("[OTHER]");
  });
});
