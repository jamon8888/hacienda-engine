import { test, expect, type Page } from "@playwright/test";

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
