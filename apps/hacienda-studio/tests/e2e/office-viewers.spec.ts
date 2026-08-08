import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test, expect } from "@playwright/test";
import { visitFresh } from "./fixtures";

/**
 * Phase B/C: docx/xlsx/pptx/pdf preview viewers wired alongside the existing
 * Markdown editor/PII panel (`App.tsx`'s `results.map(...)` block). Each fixture is a
 * minimal but real Office/PDF document (generated via LibreOffice, not hand-rolled bytes)
 * so the vendored `components/extend/*-viewer.tsx` actually has content to render.
 */
const FIXTURES_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), "fixtures");

async function uploadFixture(
  page: import("@playwright/test").Page,
  fileName: string,
  mimeType: string,
): Promise<void> {
  const buffer = await readFile(path.join(FIXTURES_DIR, fileName));
  const download = page.waitForEvent("download");
  await page.setInputFiles('input[type="file"]', { name: fileName, mimeType, buffer });
  await download;
}

test.describe("Office document viewers (Track K)", () => {
  test("renders the DOCX viewer toolbar after upload", async ({ page }) => {
    await visitFresh(page);
    await uploadFixture(
      page,
      "note.docx",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    );

    await expect(page.locator('[aria-label="Open DOCX actions"]')).toBeVisible({
      timeout: 15000,
    });
  });

  test("renders the XLSX viewer toolbar after upload", async ({ page }) => {
    await visitFresh(page);
    await uploadFixture(
      page,
      "data.xlsx",
      "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    );

    await expect(page.locator('[aria-label="Open workbook actions"]')).toBeVisible({
      timeout: 15000,
    });
  });

  test("renders the PPTX viewer toolbar after upload", async ({ page }) => {
    await visitFresh(page);
    await uploadFixture(
      page,
      "deck.pptx",
      "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    );

    await expect(page.locator('[aria-label="Open PowerPoint actions"]')).toBeVisible({
      timeout: 15000,
    });
  });

  test("does not replace the Markdown editor/PII panel", async ({ page }) => {
    await visitFresh(page);
    await uploadFixture(
      page,
      "note.docx",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    );

    await expect(page.locator('[aria-label="Open DOCX actions"]')).toBeVisible({
      timeout: 15000,
    });
    await expect(page.locator(".cm-pii-editor")).toBeVisible();
  });
});

test.describe("PDF viewer (Track K)", () => {
  test("renders the PDF viewer toolbar and page count after upload", async ({ page }) => {
    await visitFresh(page);
    await uploadFixture(page, "note.pdf", "application/pdf");

    await expect(page.locator('[data-slot="pdf-viewer"]')).toBeVisible({ timeout: 15000 });
    await expect(page.locator('[aria-label="Open PDF actions"]')).toBeVisible({
      timeout: 15000,
    });
    await expect(page.locator("text=of 1")).toBeVisible({ timeout: 15000 });
  });
});
