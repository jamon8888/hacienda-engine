import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";

const CROSS_ORIGIN_ISOLATION = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

export default defineConfig({
  base: "/",
  plugins: [react()],
  // Mirrors tsconfig.json's "@/*" path and hacienda-private's own components.json
  // alias convention, so ported components need no import-path changes.
  resolve: {
    alias: {
      "@": fileURLToPath(new URL(".", import.meta.url)),
    },
  },
  optimizeDeps: {
    // All three packages resolve their .wasm via new URL(..., import.meta.url).
    // Pre-bundling rewrites that base to .vite/deps/, so the request misses
    // and the dev server's SPA fallback answers with index.html — WebAssembly
    // then rejects it ("expected magic word 00 61 73 6d, found 3c 21 64 6f").
    exclude: ["@remotion/whisper-web", "@xberg-io/xberg-wasm", "hacienda-wasm"],
  },
  build: {
    target: "esnext",
    // Inside the package so Turborepo can cache it — task outputs are resolved
    // relative to the package directory.
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      output: {
        manualChunks: {
          "xberg-wasm": ["@xberg-io/xberg-wasm"],
          "hacienda-wasm": ["hacienda-wasm"],
          vendor: ["jszip", "idb"],
        },
      },
    },
  },
  // SharedArrayBuffer requires cross-origin isolation. These have to be real
  // HTTP headers — the equivalent <meta http-equiv> tags are ignored by
  // browsers — so whatever serves dist/ in production must send them too.
  server: { headers: CROSS_ORIGIN_ISOLATION },
  preview: { headers: CROSS_ORIGIN_ISOLATION },
  worker: {
    format: "es",
    plugins: () => [],
  },
});
