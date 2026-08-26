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
    environmentMatchGlobs: [
      ["tests/unit/app-shell.*", "jsdom"],
      ["tests/unit/nouvelle-session.*", "jsdom"],
      ["tests/unit/detection-modal.*", "jsdom"],
      ["tests/unit/processed-files.*", "jsdom"],
      ["tests/unit/folder-upload.*", "jsdom"],
      ["tests/unit/editor-layout.*", "jsdom"],
      ["tests/unit/interactive-editor.*", "jsdom"],
      ["tests/unit/pseudonyms.*", "jsdom"],
      ["tests/unit/restoration.*", "jsdom"],
      ["tests/unit/texte-simple.*", "jsdom"],
      ["tests/unit/conservation.*", "jsdom"],
      ["tests/unit/accueil.*", "jsdom"],
    ],
    setupFiles: ["./tests/setup.ts"],
  },
});
