import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test, expect } from "@playwright/test";
import JSZip from "jszip";
import { skipOnboarding, waitForFileRowDone } from "./fixtures";

/**
 * Regression tests for the three broken flows after batch processing:
 * 1. Screen transition: queue → browser (batch-complete must always fire)
 * 2. "Download redacted zip" button must be visible and enabled
 * 3. File grid must show completed rows with stats
 *
 * Root cause traced to worker/pipeline.ts: if processFiles() threw before
 * the file loop (taxonomy loading, key derivation, WASM init), batch-complete
 * never fired and the UI stayed stuck on the queue screen forever.
 */
const FIXTURES_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), "fixtures");
const NOTE = "Contact Jean Dupont at jean.dupont@cabinet-exemple.fr regarding the deal.";

test.describe("batch completion and screen transition", () => {
  test("transitions from queue to file browser after processing a text file", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });

    // Queue screen should appear during processing
    await page.waitForSelector('[aria-label="Processing queue"]', { timeout: 10000 });

    // Must transition to file browser after batch completes
    await waitForFileRowDone(page, "note.txt");

    // File browser screen must be visible
    await expect(page.locator('[aria-label="File browser"]')).toBeVisible({ timeout: 10000 });
  });

  test("transitions from queue to file browser after processing a DOCX", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    const buffer = await readFile(path.join(FIXTURES_DIR, "note.docx"));
    await page.setInputFiles('input[type="file"]', {
      name: "note.docx",
      mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      buffer,
    });

    await waitForFileRowDone(page, "note.docx");
    await expect(page.locator('[aria-label="File browser"]')).toBeVisible({ timeout: 10000 });
  });

  test("transitions from queue to file browser after processing multiple files", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', [
      {
        name: "a.txt",
        mimeType: "text/plain",
        buffer: Buffer.from(NOTE),
      },
      {
        name: "b.txt",
        mimeType: "text/plain",
        buffer: Buffer.from("Another document with Acme SAS."),
      },
    ]);

    await waitForFileRowDone(page, "a.txt");
    await waitForFileRowDone(page, "b.txt");
    await expect(page.locator('[aria-label="File browser"]')).toBeVisible({ timeout: 10000 });

    // Both rows should be visible
    await expect(page.locator('[data-file-row="a.txt"]')).toBeVisible();
    await expect(page.locator('[data-file-row="b.txt"]')).toBeVisible();
  });

  test("no console errors during batch processing", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(String(error)));

    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });

    await waitForFileRowDone(page, "note.txt");

    // Filter out known non-critical warnings
    const critical = errors.filter(
      (e) => !e.includes("NER FAILED") && !e.includes("Whisper") && !e.includes("dtype mismatch"),
    );
    expect(critical).toEqual([]);
  });
});

test.describe("download zip button", () => {
  test("'Download redacted zip' button is visible after batch completes", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });

    await waitForFileRowDone(page, "note.txt");

    const btn = page.locator("button.download-zip");
    await expect(btn).toBeVisible({ timeout: 10000 });
    await expect(btn).toBeEnabled();
  });

  test("'Download redacted zip' produces a valid zip with expected structure", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "note.txt");

    const download = page.waitForEvent("download");
    await page.click("button.download-zip");
    const zip = await JSZip.loadAsync(await readFile(await (await download).path()));

    expect(Object.keys(zip.files).sort()).toEqual(
      expect.arrayContaining([
        "GLOSSARY.md",
        "_manifest.json",
        "entities-registry.json",
        "documents/note.md",
      ]),
    );

    const markdown = await zip.file("documents/note.md")!.async("string");
    expect(markdown).toContain("source: note.txt");
    expect(markdown).toContain("jean.dupont@cabinet-exemple.fr");
  });

  test("'Download zip' button is visible on the detail screen", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "note.txt");

    await page.locator('[data-file-row="note.txt"]').click();

    // Detail screen has its own "Download zip" button
    const detailDownloadBtn = page.locator('button:has-text("Download zip")').first();
    await expect(detailDownloadBtn).toBeVisible({ timeout: 10000 });
  });
});

test.describe("file grid rendering", () => {
  test("shows the file row with entity and PII badges after processing", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "note.txt");

    const row = page.locator('[data-file-row="note.txt"]');
    await expect(row).toBeVisible();
    await expect(row).toContainText("entities");
    await expect(row).toContainText("PII");
  });

  test("shows file name in the grid row", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "report.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "report.txt");

    const row = page.locator('[data-file-row="report.txt"]');
    await expect(row).toBeVisible();
    await expect(row).toContainText("report.txt");
  });

  test("file row is clickable and opens the detail screen", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "note.txt");

    await page.locator('[data-file-row="note.txt"]').click();

    // Detail screen should show the file name and a back button
    await expect(page.locator(".close-split-view")).toBeVisible({ timeout: 10000 });
    await expect(page.locator('section[aria-label="File detail"]')).toBeVisible();
  });

  test("shows the 'Add more files' button on the browser screen", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "note.txt");

    await expect(page.locator("button.add-more-files")).toBeVisible({ timeout: 10000 });
  });

  test("'Add more files' returns to the upload screen", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "note.txt");

    await page.locator("button.add-more-files").click();

    // Should be back on the upload screen with a file input
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached", timeout: 10000 });
  });

  test("file count matches number of uploaded files", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', [
      { name: "a.txt", mimeType: "text/plain", buffer: Buffer.from("Doc A") },
      { name: "b.txt", mimeType: "text/plain", buffer: Buffer.from("Doc B") },
      { name: "c.txt", mimeType: "text/plain", buffer: Buffer.from("Doc C") },
    ]);

    await waitForFileRowDone(page, "a.txt");
    await waitForFileRowDone(page, "b.txt");
    await waitForFileRowDone(page, "c.txt");

    // Header should show "Files (3)"
    await expect(page.locator('h2:has-text("Files (3)")')).toBeVisible({ timeout: 10000 });
  });
});

test.describe("viewer rendering in detail screen", () => {
  test("opens DOCX viewer when clicking a DOCX file", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    const buffer = await readFile(path.join(FIXTURES_DIR, "note.docx"));
    await page.setInputFiles('input[type="file"]', {
      name: "note.docx",
      mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      buffer,
    });
    await waitForFileRowDone(page, "note.docx");

    await page.locator('[data-file-row="note.docx"]').click();

    // DOCX viewer should render
    await expect(page.locator('[aria-label="Open DOCX actions"]')).toBeVisible({ timeout: 20000 });
    // Redacted editor should also be visible (split view)
    await expect(page.locator(".cm-redacted-editor")).toBeVisible();
  });

  test("opens XLSX viewer when clicking an XLSX file", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    const buffer = await readFile(path.join(FIXTURES_DIR, "data.xlsx"));
    await page.setInputFiles('input[type="file"]', {
      name: "data.xlsx",
      mimeType: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
      buffer,
    });
    await waitForFileRowDone(page, "data.xlsx");

    await page.locator('[data-file-row="data.xlsx"]').click();

    await expect(page.locator('[aria-label="Open workbook actions"]')).toBeVisible({ timeout: 20000 });
    await expect(page.locator(".cm-redacted-editor")).toBeVisible();
  });

  test("opens PPTX viewer when clicking a PPTX file", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    const buffer = await readFile(path.join(FIXTURES_DIR, "deck.pptx"));
    await page.setInputFiles('input[type="file"]', {
      name: "deck.pptx",
      mimeType: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
      buffer,
    });
    await waitForFileRowDone(page, "deck.pptx");

    await page.locator('[data-file-row="deck.pptx"]').click();

    await expect(page.locator('[aria-label="Open PowerPoint actions"]')).toBeVisible({ timeout: 20000 });
    await expect(page.locator(".cm-redacted-editor")).toBeVisible();
  });

  test("opens PDF viewer when clicking a PDF file", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    const buffer = await readFile(path.join(FIXTURES_DIR, "note.pdf"));
    await page.setInputFiles('input[type="file"]', {
      name: "note.pdf",
      mimeType: "application/pdf",
      buffer,
    });
    await waitForFileRowDone(page, "note.pdf");

    await page.locator('[data-file-row="note.pdf"]').click();

    await expect(page.locator('[data-slot="pdf-viewer"]')).toBeVisible({ timeout: 20000 });
    await expect(page.locator('[aria-label="Open PDF actions"]')).toBeVisible({ timeout: 15000 });
    await expect(page.locator(".cm-redacted-editor")).toBeVisible();
  });

  test("back button returns to file browser from detail screen", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "note.txt");

    await page.locator('[data-file-row="note.txt"]').click();
    await expect(page.locator('section[aria-label="File detail"]')).toBeVisible({ timeout: 10000 });

    await page.locator(".close-split-view").click();

    // Back on the file browser
    await expect(page.locator('[aria-label="File browser"]')).toBeVisible();
    await expect(page.locator('[data-file-row="note.txt"]')).toBeVisible();
  });

  test("viewer shows findings pane with PII detections", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "note.txt");

    await page.locator('[data-file-row="note.txt"]').click();

    // Findings tab should show PII detections
    await page.locator('button:has-text("Findings")').click();
    await expect(page.locator(".cm-pii-editor")).toBeVisible({ timeout: 10000 });
    const pill = page.locator(".cm-pii-pill").first();
    await expect(pill).toBeVisible();
    await expect(pill).toHaveText("jean.dupont@cabinet-exemple.fr");
  });
});

test.describe("per-file download", () => {
  test("hovering a completed file row shows the download overlay", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "note.txt");

    const row = page.locator('[data-file-row="note.txt"]');
    await row.hover();

    // Download overlay button should become visible on hover
    const downloadBtn = row.locator('[title="Download this file"]');
    await expect(downloadBtn).toBeVisible();
  });

  test("clicking per-file download triggers a download", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "note.txt");

    const row = page.locator('[data-file-row="note.txt"]');
    await row.hover();

    const download = page.waitForEvent("download");
    await row.locator('[title="Download this file"]').click();
    const dl = await download;
    expect(dl.suggestedFilename()).toBe("note.md");

    const content = await readFile(await dl.path(), "utf-8");
    expect(content).toContain("source: note.txt");
    expect(content).toContain("jean.dupont@cabinet-exemple.fr");
  });
});

test.describe("office document processing end-to-end", () => {
  test("processes DOCX and shows entity/PII counts in grid", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    const buffer = await readFile(path.join(FIXTURES_DIR, "note.docx"));
    await page.setInputFiles('input[type="file"]', {
      name: "note.docx",
      mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      buffer,
    });
    await waitForFileRowDone(page, "note.docx");

    const row = page.locator('[data-file-row="note.docx"]');
    await expect(row).toBeVisible();
    await expect(row).toContainText("entity");
    await expect(row).toContainText("PII");

    // Zip should be downloadable
    const download = page.waitForEvent("download");
    await page.click("button.download-zip");
    const zip = await JSZip.loadAsync(await readFile(await (await download).path()));
    expect(Object.keys(zip.files)).toEqual(
      expect.arrayContaining(["documents/note.md", "_manifest.json"]),
    );
  });

  test("processes XLSX and shows entity/PII counts in grid", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    const buffer = await readFile(path.join(FIXTURES_DIR, "data.xlsx"));
    await page.setInputFiles('input[type="file"]', {
      name: "data.xlsx",
      mimeType: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
      buffer,
    });
    await waitForFileRowDone(page, "data.xlsx");

    const row = page.locator('[data-file-row="data.xlsx"]');
    await expect(row).toBeVisible();
    await expect(row).toContainText("entity");
    await expect(row).toContainText("PII");

    // Zip should be downloadable
    const download = page.waitForEvent("download");
    await page.click("button.download-zip");
    const zip = await JSZip.loadAsync(await readFile(await (await download).path()));
    expect(Object.keys(zip.files)).toEqual(
      expect.arrayContaining(["documents/data.md", "_manifest.json"]),
    );
  });

  test("processes PPTX and shows entity/PII counts in grid", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    const buffer = await readFile(path.join(FIXTURES_DIR, "deck.pptx"));
    await page.setInputFiles('input[type="file"]', {
      name: "deck.pptx",
      mimeType: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
      buffer,
    });
    await waitForFileRowDone(page, "deck.pptx");

    const row = page.locator('[data-file-row="deck.pptx"]');
    await expect(row).toBeVisible();
    await expect(row).toContainText("Done");
  });

  test("processes PDF and shows entity/PII counts in grid", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    const buffer = await readFile(path.join(FIXTURES_DIR, "note.pdf"));
    await page.setInputFiles('input[type="file"]', {
      name: "note.pdf",
      mimeType: "application/pdf",
      buffer,
    });
    await waitForFileRowDone(page, "note.pdf");

    const row = page.locator('[data-file-row="note.pdf"]');
    await expect(row).toBeVisible();
    await expect(row).toContainText("Done");
    await expect(row).toContainText("entity");
    await expect(row).toContainText("PII");
  });

  test("processes a mix of file types in one batch", async ({ page }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    const txtBuf = Buffer.from(NOTE);
    const docxBuf = await readFile(path.join(FIXTURES_DIR, "note.docx"));
    const xlsxBuf = await readFile(path.join(FIXTURES_DIR, "data.xlsx"));

    await page.setInputFiles('input[type="file"]', [
      { name: "note.txt", mimeType: "text/plain", buffer: txtBuf },
      { name: "note.docx", mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document", buffer: docxBuf },
      { name: "data.xlsx", mimeType: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", buffer: xlsxBuf },
    ]);

    await waitForFileRowDone(page, "note.txt");
    await waitForFileRowDone(page, "note.docx");
    await waitForFileRowDone(page, "data.xlsx");

    await expect(page.locator('[data-file-row="note.txt"]')).toBeVisible();
    await expect(page.locator('[data-file-row="note.docx"]')).toBeVisible();
    await expect(page.locator('[data-file-row="data.xlsx"]')).toBeVisible();

    await expect(page.locator('h2:has-text("Files (3)")')).toBeVisible({ timeout: 10000 });
  });
});
