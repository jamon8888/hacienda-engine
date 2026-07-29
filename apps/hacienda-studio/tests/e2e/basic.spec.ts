import { test, expect } from "@playwright/test";

test.describe("xberg-studio", () => {
  test("loads and shows onboarding", async ({ page }) => {
    // Clear visited flag so onboarding appears
    await page.addInitScript(() => {
      localStorage.removeItem("xberg-studio-visited");
    });
    await page.goto("/");
    await page.waitForLoadState("domcontentloaded");
    await page.waitForSelector(".onboarding-overlay", { timeout: 20000 });
    await expect(page.locator("text=100% Local AI")).toBeVisible();
    await expect(page.locator("text=NEVER leave this tab")).toBeVisible();
    // Continue button should exist but be disabled (assets not loaded)
    const btn = page.locator("text=Continue");
    await expect(btn).toBeVisible();
    await expect(btn).toBeDisabled();
  });

  test("skips onboarding when visited and shows drop zone", async ({
    page,
  }) => {
    // Pre-set visited flag to skip onboarding
    await page.addInitScript(() => {
      localStorage.setItem("xberg-studio-visited", "true");
    });
    await page.goto("/");
    await page.waitForLoadState("domcontentloaded");
    // Onboarding should NOT appear; drop zone should be visible
    await page.waitForSelector(".drop-zone", { timeout: 10000 });
    await expect(page.locator(".onboarding-overlay")).not.toBeVisible();
    // The drop zone announces itself as busy until the worker handshake lands.
    await page.waitForSelector('input[type="file"]:not([disabled])');
    await expect(page.locator("text=Drop files here")).toBeVisible();
  });

  test("rejects unsupported file type", async ({ page }) => {
    await page.addInitScript(() => {
      localStorage.setItem("xberg-studio-visited", "true");
    });
    await page.goto("/");
    await page.waitForLoadState("domcontentloaded");
    await page.waitForSelector('input[type="file"]:not([disabled])');

    await page.setInputFiles('input[type="file"]', {
      name: "test.exe",
      mimeType: "application/x-msdownload",
      buffer: Buffer.from("fake"),
    });

    await expect(page.locator("text=Unsupported file type")).toBeVisible({
      timeout: 5000,
    });
  });

  test("shows config panel", async ({ page }) => {
    await page.addInitScript(() => {
      localStorage.setItem("xberg-studio-visited", "true");
    });
    await page.goto("/");
    await page.waitForLoadState("domcontentloaded");
    await page.waitForSelector(".drop-zone", { timeout: 10000 });

    await page.click('button:has-text("Config")');
    await expect(page.locator("text=NER Categories")).toBeVisible();
    await expect(
      page.locator("text=All processing runs locally"),
    ).toBeVisible();
  });
});
