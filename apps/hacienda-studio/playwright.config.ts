import { defineConfig, devices } from "@playwright/test";

/**
 * Every defect this suite has failed to catch so far existed in only one of the
 * two builds — asset URLs the dev server happened to resolve and the production
 * bundle did not. Set E2E_TARGET=preview to run against the built output.
 */
const isPreview = process.env.E2E_TARGET === "preview";
const port = isPreview ? 4173 : 5173;

export default defineConfig({
  testDir: "./tests/e2e",
  // Booting the worker compiles a 48 MB WASM module before a document can be
  // processed, which does not fit in the 30s default on a cold cache.
  timeout: 120000,
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: "html",
  use: {
    baseURL: `http://localhost:${port}`,
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "firefox", use: { ...devices["Desktop Firefox"] } },
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
  ],
  webServer: {
    command: isPreview ? "npm run build && npm run preview" : "npm run dev",
    url: `http://localhost:${port}`,
    reuseExistingServer: !process.env.CI,
    timeout: 300000,
  },
});
