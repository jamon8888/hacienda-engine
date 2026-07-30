import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const CROSS_ORIGIN_ISOLATION = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

export default defineConfig({
  base: "/",
  plugins: [svelte()],
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
