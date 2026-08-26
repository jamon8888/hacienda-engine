import { defineConfig } from "vitest/config";
import { fileURLToPath, URL } from "node:url";

export default defineConfig({
  resolve: {
    alias: {
      "@": fileURLToPath(new URL(".", import.meta.url)),
    },
  },
  test: {
    // Playwright owns the browser-level tests under tests/e2e.
    include: [
      "lib/**/*.{test,spec}.ts",
      "worker/**/*.{test,spec}.ts",
      "tests/unit/**/*.{test,spec}.{ts,tsx}",
    ],
    environment: "node",
    environmentMatchGlobs: [["tests/unit/app-shell.*", "jsdom"]],
    setupFiles: ["./tests/setup.ts"],
  },
});
