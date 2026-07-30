import { readFile } from "node:fs/promises";
import { test, expect, type Page } from "@playwright/test";
import JSZip from "jszip";

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

    await page.addInitScript(() => {
      localStorage.setItem("xberg-studio-visited", "true");
    });
    await page.goto("/");
    // Not `.drop-zone`: it renders while the worker is still compiling the
    // WASM module, and the input is disabled until the handshake lands.
    await page.waitForSelector('input[type="file"]:not([disabled])');

    const download = page.waitForEvent("download");
    await page.setInputFiles('input[type="file"]', {
      name: "protocole.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(CONTRACT),
    });
    await download;

    expect(external).toEqual([]);
  });

  test("ships no reference to a third-party CDN", async ({ page }) => {
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
    await page.addInitScript(() => {
      localStorage.setItem("xberg-studio-visited", "true");
    });
    await page.goto("/");
    await page.waitForSelector('input[type="file"]:not([disabled])');

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
      "protocole.md",
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

    const markdown = await zip.file("protocole.md")!.async("string");
    expect(markdown).toContain("[IBAN:****]");
  });
});
