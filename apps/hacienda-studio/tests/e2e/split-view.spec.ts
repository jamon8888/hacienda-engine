import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test, expect, type Page } from "@playwright/test";
import { visitFresh, waitForFileRowDone } from "./fixtures";

/**
 * Track K1/K2: clicking a viewer-compatible `FileBrowser` row opens a side-by-side split
 * view (native document viewer on the left, a free-text-editable redacted-markdown buffer
 * on the right — `components/RedactedEditor.tsx`). Unlike `finder-and-editing.spec.ts`'s
 * PII editing (which operates on `rawMarkdown` offsets via `piiFindings`/`entities`), the
 * redacted pane here is a plain independent text buffer: selecting a range and clicking
 * "Redact selection" does a text splice, not a re-render keyed to detector output.
 */
const FIXTURES_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), "fixtures");

async function uploadFixture(
  page: Page,
  fileName: string,
  mimeType: string,
): Promise<void> {
  const buffer = await readFile(path.join(FIXTURES_DIR, fileName));
  await page.setInputFiles('input[type="file"]', { name: fileName, mimeType, buffer });
  await waitForFileRowDone(page, fileName);
}

/** Selects `.cm-redacted-editor`'s content by character offset, same technique as
 * `finder-and-editing.spec.ts`'s `selectRange` for `.cm-pii-editor`. */
async function selectRedactedRange(page: Page, from: number, to: number): Promise<void> {
  const editor = page.locator(".cm-redacted-editor .cm-content");
  await editor.click();
  await page.keyboard.press("Control+Home");
  for (let i = 0; i < from; i++) await page.keyboard.press("ArrowRight");
  for (let i = 0; i < to - from; i++) await page.keyboard.press("Shift+ArrowRight");
}

test.describe("Side-by-side split view (Track K1/K2)", () => {
  test("opens the split view with the native viewer and a redacted editor", async ({ page }) => {
    await visitFresh(page);
    await uploadFixture(
      page,
      "note.docx",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    );

    const row = page.locator('[data-file-row="note.docx"]');
    await expect(row).toBeVisible();
    await row.click();

    await expect(page.locator('[aria-label="Open DOCX actions"]')).toBeVisible({
      timeout: 15000,
    });
    await expect(page.locator(".cm-redacted-editor")).toBeVisible();

    await page.locator(".close-split-view").click();
    await expect(page.locator(".cm-redacted-editor")).toHaveCount(0);
  });

  test("Redact selection splices a category token into the redacted buffer", async ({ page }) => {
    await visitFresh(page);
    await uploadFixture(
      page,
      "note.docx",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    );

    await page.locator('[data-file-row="note.docx"]').click();
    await expect(page.locator(".cm-redacted-editor")).toBeVisible();

    const before = (await page.locator(".cm-redacted-editor .cm-content").innerText()).trim();
    expect(before.length).toBeGreaterThan(0);

    await selectRedactedRange(page, 0, 4);
    await page.selectOption(".redact-category", "other");
    await page.locator(".redact-selection").click();

    const after = (await page.locator(".cm-redacted-editor .cm-content").innerText()).trim();
    expect(after).toContain("[OTHER]");
    expect(after).not.toBe(before);
  });
});
