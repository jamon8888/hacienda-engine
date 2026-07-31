import { mkdtemp, mkdir, writeFile, rm, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, basename } from "node:path";
import { test, expect, type Page } from "@playwright/test";
import JSZip from "jszip";

const NOTE =
  "Contact Jean Dupont at jean.dupont@cabinet-exemple.fr regarding the deal.";

async function skipOnboarding(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem("xberg-studio-visited", "true");
  });
}

/**
 * Track I1: folder ingest. Builds a folder with a nested supported document
 * plus the OS/unsupported noise a real folder contains, uploads it through
 * the folder picker (Playwright's directory-path `setInputFiles`, which
 * populates `webkitRelativePath` the same way a real browser directory
 * picker does), and checks the export against all three of I1's claims:
 * the source tree survives into the output, junk/unsupported files are
 * skipped rather than failing the batch, and the batch still completes.
 */
test.describe("folder upload (Track I1)", () => {
  test("preserves the source tree and skips junk/unsupported files", async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });

    const dir = await mkdtemp(join(tmpdir(), "hacienda-folder-upload-"));
    try {
      await mkdir(join(dir, "sub"), { recursive: true });
      await writeFile(join(dir, "sub", "note.txt"), NOTE);
      await writeFile(join(dir, ".DS_Store"), "junk");
      await writeFile(join(dir, "mystery.xyz"), "unsupported extension");

      await skipOnboarding(page);
      await page.goto("/");
      await page.waitForSelector('input[type="file"]:not([disabled])');

      // Switch the single `#file-input` into directory-picking mode before
      // Playwright can populate `webkitRelativePath` on the selected files.
      await page.click("button.mode-toggle");
      await expect(page.locator("input#file-input")).toHaveAttribute(
        "webkitdirectory",
        "",
      );

      const download = page.waitForEvent("download");
      await page.setInputFiles('input[type="file"]', dir);

      const skipNotice = page.locator(".skip-notice");
      await expect(skipNotice).toContainText("Skipped");
      await expect(skipNotice).toContainText("1 unsupported");
      await expect(skipNotice).toContainText("1 system");

      const zip = await JSZip.loadAsync(
        await readFile(await (await download).path()),
      );
      const entryNames = Object.keys(zip.files);

      const root = basename(dir);
      const notePath = entryNames.find((name) =>
        name.endsWith(`documents/${root}/sub/note.md`),
      );
      expect(notePath, entryNames.join(", ")).toBeTruthy();
      expect(
        entryNames.some((name) => name.includes("mystery")),
      ).toBe(false);
      expect(
        entryNames.some((name) => name.includes("DS_Store")),
      ).toBe(false);

      const markdown = await zip.file(notePath!)!.async("string");
      expect(markdown).toContain(`source: ${root}/sub/note.txt`);
      expect(markdown).toContain("jean.dupont@cabinet-exemple.fr");

      expect(errors).toEqual([]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
