import { test, expect } from "@playwright/test";
import { visitFresh, waitForFileRowDone } from "./fixtures";

/**
 * Track F3: CodeMirror renders in the real app with a PII span decorated, and — the
 * plan's own Check — typing before that span in a real browser (not the headless
 * `lib/pii-decorations.test.ts` unit test) doesn't misplace its highlight.
 */
const NOTE =
  "Contact Jean Dupont at jean.dupont@cabinet-exemple.fr regarding the deal.";

test.describe("MarkdownEditor PII decorations (Track F3)", () => {
  test("renders the document with the detected PII span highlighted", async ({ page }) => {
    await visitFresh(page);

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "note.txt");
    // The editor now lives behind the click-to-open detail screen (Track K1/Phase 1).
    await page.locator('[data-file-row="note.txt"]').click();

    const pill = page.locator(".cm-pii-pill").first();
    await expect(pill).toBeVisible();
    await expect(pill).toHaveText("jean.dupont@cabinet-exemple.fr");
    await expect(pill).toHaveAttribute("data-pii-category", "email");
  });

  test("typing before the span shifts it without corrupting the highlighted text", async ({
    page,
  }) => {
    await visitFresh(page);

    await page.setInputFiles('input[type="file"]', {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "note.txt");
    // The editor now lives behind the click-to-open detail screen (Track K1/Phase 1).
    await page.locator('[data-file-row="note.txt"]').click();

    const editor = page.locator(".cm-pii-editor .cm-content");
    await expect(page.locator(".cm-pii-pill").first()).toBeVisible();

    // Click at the very start of the document and type text before the PII span.
    await editor.click({ position: { x: 2, y: 2 } });
    await page.keyboard.press("Home");
    await page.keyboard.type("Please note: ");

    const pill = page.locator(".cm-pii-pill").first();
    await expect(pill).toHaveText("jean.dupont@cabinet-exemple.fr");
    await expect(editor).toContainText("Please note: Contact Jean Dupont");
  });
});
