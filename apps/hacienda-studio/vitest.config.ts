import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // Playwright owns the browser-level tests under tests/e2e.
    include: ["lib/**/*.{test,spec}.ts", "worker/**/*.{test,spec}.ts"],
    environment: "node",
  },
});
