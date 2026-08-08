import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test, expect, type Page } from "@playwright/test";
import JSZip from "jszip";
import { mockNerModelAssets, skipOnboarding } from "./fixtures";

const FIXTURES_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), "fixtures");

/**
 * Hosts the app is permitted to contact. Everything else is a compliance
 * failure, not a bug: clients are avocats and experts-comptables bound by
 * secret professionnel (loi n° 71-1130 art. 66-5), and the product's claim is
 * that document bytes, extracted text, entity values and pseudonym mappings
 * never leave the browser. A background request to a CDN while a client
 * contract is open would be a GDPR Chapter V transfer and a breach of that
 * duty — so this is asserted, not reviewed by eye.
 *
 * Only asset downloads are allowed, and only before any document is opened.
 */
const ALLOWED_HOSTS = new Set(["localhost", "127.0.0.1", "huggingface.co"]);

const CONTRACT = [
  "PROTOCOLE D'ACCORD — Acme SAS (SIREN 552 100 554) cède à Beta SARL",
  "l'intégralité des titres. Contact: Maître Jean Dupont,",
  "jean.dupont@cabinet-exemple.fr, IBAN FR7630006000011234567890189.",
].join(" ");

function recordExternalRequests(page: Page): string[] {
  const external: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.protocol === "blob:" || url.protocol === "data:") return;
    if (ALLOWED_HOSTS.has(url.hostname)) return;
    external.push(`${request.method()} ${request.url()}`);
  });
  return external;
}

test.describe("network egress", () => {
  test("contacts no host outside the allowlist while processing a document", async ({
    page,
  }) => {
    const external = recordExternalRequests(page);

    await skipOnboarding(page);
    await page.goto("/");
    // Not `.drop-zone`: it renders while the worker is still compiling the
    // WASM module, and the input is disabled until the handshake lands.
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    const download = page.waitForEvent("download");
    await page.setInputFiles('input[type="file"]', {
      name: "protocole.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(CONTRACT),
    });
    await download;

    expect(external).toEqual([]);
  });

  // Track K regression guard: `DocxViewerPreview`/`PDFViewer` pull in large,
  // previously-unaudited third-party packages (`@extend-ai/react-docx`,
  // `@embedpdf/*`) — this proves their rendering path (including the pdfium
  // WASM fetch) stays on `'self'`/blob: and never reaches a CDN like
  // jsdelivr, closing the Gate 1 egress loop end-to-end.
  test("contacts no host outside the allowlist while previewing docx/pdf documents", async ({
    page,
  }) => {
    // Two full uploads (worker extraction + viewer bundle load) back to back, on a dev
    // server that cold-optimizes each viewer's worker dependency on first use —
    // comfortably exceeds the suite's default 120s per-test timeout on a slow/first run.
    test.setTimeout(240000);
    const external = recordExternalRequests(page);

    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    const docxBuffer = await readFile(path.join(FIXTURES_DIR, "note.docx"));
    const docxDownload = page.waitForEvent("download");
    await page.setInputFiles('input[type="file"]', {
      name: "note.docx",
      mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      buffer: docxBuffer,
    });
    await docxDownload;
    await expect(page.locator('[aria-label="Open DOCX actions"]')).toBeVisible({
      timeout: 15000,
    });

    const pdfBuffer = await readFile(path.join(FIXTURES_DIR, "note.pdf"));
    const pdfDownload = page.waitForEvent("download");
    await page.setInputFiles('input[type="file"]', {
      name: "note.pdf",
      mimeType: "application/pdf",
      buffer: pdfBuffer,
    });
    await pdfDownload;
    await expect(page.locator('[data-slot="pdf-viewer"]')).toBeVisible({ timeout: 15000 });

    expect(external).toEqual([]);
  });

  test("ships no reference to a third-party CDN", async ({ page }) => {
    // Onboarding is not skipped here (this test only checks the initial DOM), but the
    // main thread's preloadAssets() still fires and would otherwise reach the real
    // model host in the background — mock it for the same reason every other test does.
    await mockNerModelAssets(page);
    await page.goto("/");

    const remoteReferences = await page.evaluate(() =>
      [
        ...document.querySelectorAll(
          "link[href], script[src], img[src], iframe[src]",
        ),
      ]
        .map((element) => element.getAttribute("href") ?? element.getAttribute("src") ?? "")
        .filter((value) => /^https?:\/\//.test(value)),
    );

    expect(remoteReferences).toEqual([]);
  });
});

/**
 * Track A2's *Check*: redacting the document is not the whole contract if the same
 * secret ships a second time in entities-registry.json or the KG export. Reuses the
 * CONTRACT fixture above — it names an IBAN that both the markdown redaction and
 * the entity-linking pass would otherwise touch (Track F4).
 */
test.describe("PII redaction export contract", () => {
  test("redacted output ships no IBAN in the markdown or the entity/KG export", async ({
    page,
  }) => {
    await skipOnboarding(page);
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])', { state: "attached" });

    await page.click("button.config-toggle");
    await page.check(
      'label:has-text("Redact PII in Output") input[type="checkbox"]',
    );
    await page.keyboard.press("Escape");

    const download = page.waitForEvent("download");
    await page.setInputFiles('input[type="file"]', {
      name: "protocole.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(CONTRACT),
    });

    const zip = await JSZip.loadAsync(
      await readFile(await (await download).path()),
    );

    const IBAN = "FR7630006000011234567890189";
    const files = [
      "documents/protocole.md",
      "GLOSSARY.md",
      "entities-registry.json",
      "kg-export/neo4j.cypher",
      "kg-export/networkx.json",
      "kg-export/rdf.ttl",
    ];
    for (const name of files) {
      const contents = await zip.file(name)!.async("string");
      expect(contents, `${name} must not contain the redacted IBAN`).not.toContain(
        IBAN,
      );
    }

    // Track I2's entities/ files are per-entity, not enumerable by a fixed
    // name — check every one that was actually produced, same contract as
    // the fixed-name files above. Excludes the `entities/` directory entry
    // itself (`zip.file()` returns null for a directory, not a file).
    const entityFiles = Object.keys(zip.files).filter(
      (name) => name.startsWith("entities/") && !zip.files[name].dir,
    );
    for (const name of entityFiles) {
      const contents = await zip.file(name)!.async("string");
      expect(contents, `${name} must not contain the redacted IBAN`).not.toContain(
        IBAN,
      );
    }

    const markdown = await zip.file("documents/protocole.md")!.async("string");
    expect(markdown).toContain("[IBAN:****]");
  });
});
