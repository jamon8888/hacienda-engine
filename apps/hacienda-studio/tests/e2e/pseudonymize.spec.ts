import { readFile } from "node:fs/promises";
import { test, expect, type Page } from "@playwright/test";
import JSZip from "jszip";
import { visitFresh, waitForFileRowDone } from "./fixtures";

/**
 * Track F1/F2: `redactionMode: "pseudonymize"` end to end — minting in the worker
 * (Track F2's `mintToken`, wired into `worker/pipeline.ts`), the exported markdown
 * carrying a real reversible token instead of a mask, and revealing it back through the
 * document-view `PiiPanel` (Track F1) using the same passphrase.
 */
const EMAIL = "jean.dupont@cabinet-exemple.fr";
const NOTE = `Contact Jean Dupont at ${EMAIL} regarding the deal.`;
const PASSPHRASE = "correct horse battery staple";

async function enablePseudonymizeMode(page: Page): Promise<void> {
  await page.click("button.config-toggle");
  await page.check('label:has-text("Redact PII in Output") input[type="checkbox"]');
  await page.selectOption("#redaction-mode", "pseudonymize");
  await page.fill("#pseudonym-passphrase", PASSPHRASE);
  await page.keyboard.press("Escape");
}

test.describe("redactionMode: pseudonymize (Track F1/F2)", () => {
  test("exports a reversible token instead of a mask, with no raw PII in the zip", async ({
    page,
  }) => {
    await visitFresh(page);
    await enablePseudonymizeMode(page);

    await page.setInputFiles('input[type="file"]', {
      name: "protocole.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "protocole.txt");

    const download = page.waitForEvent("download");
    await page.click("button.download-zip");
    const zip = await JSZip.loadAsync(await readFile(await (await download).path()));
    const markdown = await zip.file("documents/protocole.md")!.async("string");

    expect(markdown).not.toContain(EMAIL);
    expect(markdown).not.toContain("[EMAIL]"); // the mask-mode template — proves this isn't just masking
    // Token shape: [LABEL:key_id:base32]. The category label for "email" is "EMAIL"
    // (Track F2's categoryLabel — snake_case with underscores stripped, uppercased).
    expect(markdown).toMatch(/\[EMAIL:session:[A-Z2-7]+\]/);
  });

  test("reveals the token in the document-view panel with the same passphrase", async ({
    page,
  }) => {
    await visitFresh(page);
    await enablePseudonymizeMode(page);

    await page.setInputFiles('input[type="file"]', {
      name: "protocole.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "protocole.txt");
    // The findings panel now lives behind the click-to-open detail screen (Track K1/Phase 1).
    await page.locator('[data-file-row="protocole.txt"]').click();

    const trigger = page.locator(".pii-finding-trigger").first();
    await expect(trigger).toBeVisible();
    await expect(trigger).toContainText(/\[EMAIL:session:/);
    await trigger.click();

    await page.getByPlaceholder("Key id (default: session)").fill("session");
    await page.getByPlaceholder("Passphrase").fill(PASSPHRASE);
    await page.locator(".pii-reveal-submit").click();

    // deriveKeyHex runs PBKDF2 at 600,000 iterations (OWASP's 2023 minimum) on the main
    // thread here — the same deliberate cost lib/pseudonymize.test.ts documents needing
    // 40-60s unit-test timeouts for on this host; Playwright's 5s default is too short.
    await expect(page.locator(".pii-revealed-value")).toHaveText(EMAIL.toLowerCase(), {
      timeout: 60000,
    });
  });

  test("fails closed with the wrong passphrase", async ({ page }) => {
    await visitFresh(page);
    await enablePseudonymizeMode(page);

    await page.setInputFiles('input[type="file"]', {
      name: "protocole.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(NOTE),
    });
    await waitForFileRowDone(page, "protocole.txt");
    // The findings panel now lives behind the click-to-open detail screen (Track K1/Phase 1).
    await page.locator('[data-file-row="protocole.txt"]').click();

    await page.locator(".pii-finding-trigger").first().click();
    await page.getByPlaceholder("Key id (default: session)").fill("session");
    await page.getByPlaceholder("Passphrase").fill("wrong passphrase entirely");
    await page.locator(".pii-reveal-submit").click();

    // Same PBKDF2 cost as above — the wrong-passphrase derivation is just as expensive as
    // the correct one before it can fail closed.
    await expect(page.getByText(/wrong passphrase|mask mode/i)).toBeVisible({ timeout: 60000 });
    await expect(page.locator(".pii-revealed-value")).toHaveCount(0);
  });
});

/**
 * Track G1: "pseudonymizing an entity must not orphan its link or its glossary row."
 * Reuses the misclassified-digit-run pattern `worker/pipeline.test.ts`'s L6 regression
 * test and `egress.spec.ts`'s PII redaction contract already exercise for mask mode — a
 * card number the NER bridge misreads as a "phone" entity, so its span overlaps a PII
 * finding. `filterExportableEntities` (`worker/pipeline.ts`) only ever compares span
 * offsets, never `redact_template`'s content, so it already drops an overlapping entity
 * identically whether the redaction is a mask or a pseudonym token — no G1-specific code
 * was needed once F1 wired pseudonymization in, and this is what proves that rather than
 * asserting it from reading the source.
 */
test.describe("G1: entity linking survives pseudonymization (no orphaned links)", () => {
  test("an entity overlapping a pseudonymized span is dropped, not orphaned", async ({
    page,
  }) => {
    await visitFresh(page);
    await enablePseudonymizeMode(page);

    const CARD_NUMBER = "4111111111111111";
    const note = `Card number ${CARD_NUMBER} on file.`;

    await page.setInputFiles('input[type="file"]', {
      name: "card.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(note),
    });
    await waitForFileRowDone(page, "card.txt");

    const download = page.waitForEvent("download");
    await page.click("button.download-zip");
    const zip = await JSZip.loadAsync(await readFile(await (await download).path()));
    const markdown = await zip.file("documents/card.md")!.async("string");
    const registry = await zip.file("entities-registry.json")!.async("string");
    const glossary = await zip.file("GLOSSARY.md")!.async("string");

    // The raw card number never appears anywhere — pseudonymized in the body, and the
    // entity registry never got any per-instance mention of it in the first place.
    expect(markdown).not.toContain(CARD_NUMBER);
    expect(registry).not.toContain(CARD_NUMBER);
    expect(glossary).not.toContain(CARD_NUMBER);
    // No dangling link into a per-entity file the export filter dropped.
    expect(markdown).not.toMatch(/entities\/phone-\d+\.md/);
    // A real reversible token took the card number's place (categoryLabel strips
    // underscores from the wasm engine's snake_case "credit_card").
    expect(markdown).toMatch(/\[CREDITCARD:session:[A-Z2-7]+\]/);
  });
});
