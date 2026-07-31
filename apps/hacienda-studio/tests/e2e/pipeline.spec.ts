import { readFile } from "node:fs/promises";
import { test, expect, type Page } from "@playwright/test";
import JSZip from "jszip";

const NOTE = [
  "Contact Jean Dupont at jean.dupont@cabinet-exemple.fr.",
  "Acme SAS acquired Beta SARL for a purchase price of 4.2M EUR.",
  "IBAN FR7630006000011234567890189.",
].join(" ");

async function skipOnboarding(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem("xberg-studio-visited", "true");
  });
}

/**
 * The rest of the e2e suite only asserts that the app renders, so two defects
 * that broke every single document survived it: the vertical taxonomies were
 * fetched from a URL that resolved to index.html, and the xberg WASM binary was
 * requested from a path npm workspace hoisting had emptied. Neither surfaces
 * until a file is actually pushed through the worker.
 *
 * The progress card is not the thing to assert on — the app clears it a second
 * after the batch finishes, so it exists for barely a second and any assertion
 * against it races the teardown. The exported zip is the durable artifact.
 */
test.describe("document pipeline", () => {
  test("processes a text file through to a downloadable export", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });

    await skipOnboarding(page);
    await page.goto("/");
    // Not `.drop-zone`: it renders while the worker is still compiling the
    // WASM module, and the input is disabled until the handshake lands.
    await page.waitForSelector("#file-input:not([disabled])");

    const download = page.waitForEvent("download");
    // Not `input[type="file"]`: Track I1 added a second file input
    // (`#folder-input`, for folder selection) with the same type, so that
    // selector now matches two elements.
    await page.setInputFiles("#file-input", {
      name: "note.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });

    const zip = await JSZip.loadAsync(await readFile(await (await download).path()));
    // Track I2: the export is a navigable vault, not a flat bundle — documents
    // live under `documents/`, entities get their own page, and README.md/
    // GLOSSARY.md are the agent's entry points into it.
    expect(Object.keys(zip.files).sort()).toEqual(
      expect.arrayContaining([
        "_manifest.json",
        "entities-registry.json",
        "documents/note.md",
        "README.md",
        "GLOSSARY.md",
      ]),
    );

    const markdown = await zip.file("documents/note.md")!.async("string");
    expect(markdown).toContain("source: note.txt");
    expect(markdown).toContain("jean.dupont@cabinet-exemple.fr");

    expect(errors).toEqual([]);
  });
});
