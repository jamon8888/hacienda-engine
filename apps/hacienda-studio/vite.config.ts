import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import { fileURLToPath, URL } from "node:url";

const CROSS_ORIGIN_ISOLATION = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

export default defineConfig({
  base: "/",
  plugins: [
    // Must come before `react()` — the router plugin injects code the React plugin's
    // babel transform then needs to see. No `src/` in this app, so `routesDirectory`/
    // `generatedRouteTree` point at the package root instead of the plugin's defaults.
    tanstackRouter({
      target: "react",
      autoCodeSplitting: true,
      routesDirectory: "./routes",
      generatedRouteTree: "./routeTree.gen.ts",
    }),
    react(),
  ],
  // Mirrors tsconfig.json's "@/*" path and hacienda-private's own components.json
  // alias convention, so ported components need no import-path changes.
  resolve: {
    alias: {
      "@": fileURLToPath(new URL(".", import.meta.url)),
    },
  },
  optimizeDeps: {
    // All of these packages resolve their .wasm via new URL(..., import.meta.url).
    // Pre-bundling rewrites that base to .vite/deps/, so the request misses
    // and the dev server's SPA fallback answers with index.html — WebAssembly
    // then rejects it ("expected magic word 00 61 73 6d, found 3c 21 64 6f").
    // Docx/pptx/xlsx viewers ship their own import-workers that Vite's
    // pre-bundler tries to bundle as regular ESM, producing the
    // `…/docx-import-worker.js?worker_file&type=module` MIME-block you saw
    // (the worker is fetched as a plain script in .vite/deps/).
    //
    // `@llamaindex/liteparse-wasm` (PDF extraction) was missing from this list: Vite only
    // discovers it mid-session, on first actual PDF upload, then re-optimizes and reloads —
    // any worker that already grabbed a wasm reference before that reload is left pointing
    // at a mangled URL, and the very next PDF hits `Uncaught RuntimeError: unreachable`
    // inside the corrupted module instead of a clean "file not found".
    exclude: [
      "@remotion/whisper-web",
      "@xberg-io/xberg-wasm",
      "hacienda-wasm",
      "@extend-ai/react-docx",
      "@extend-ai/react-pptx",
      "@extend-ai/react-xlsx",
      "@embedpdf/engines",
      "tesseract-wasm",
      "@llamaindex/liteparse-wasm",
    ],
    // UMD/CommonJS bundles without ESM default (`module.exports = …`) throw
    // `doesn't provide an export named: 'default'` when served raw:
    // - `utif/UTIF.js` → `image-render.ts:3:7 import UTIF from 'utif'`
    // - `regl/dist/regl.js` → `index.js:11073 import regl from 'regl'`
    // - `react-dom/server.browser.js` (CommonJS in react-dom@18) → `editor.tsx:3:9`
    // Forcing them through esbuild's pre-bundle creates a synthetic default.
    include: ["utif", "pako", "regl", "react-dom", "react-dom/client", "react-dom/server"],
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
  // Self-hosted proxy for Hugging Face XET — lets the dev server follow the
  // 302 → XET redirect without CORS/405 issues and without requiring the user
  // to clear IndexedDB manually. Production still hits huggingface.co directly
  // (MODEL_BASE env), dev hits the proxy autonomously.
  server: {
    headers: CROSS_ORIGIN_ISOLATION,
    host: true,
    proxy: {
      '/hf-model': {
        target: 'https://huggingface.co',
        changeOrigin: true,
        secure: true,
        proxyTimeout: 600000,
        timeout: 600000,
        rewrite: (path) => path.replace(/^\/hf-model/, ''),
        configure: (proxy) => {
          proxy.on('proxyReq', (proxyReq) => {
            proxyReq.removeHeader('origin');
          });
          proxy.on('proxyRes', (proxyRes, req, res) => {
            const location = proxyRes.headers['location'];
            if (location && typeof location === 'string' && location.startsWith('/')) {
              proxyRes.headers['location'] = '/hf-model' + location;
            }
          });
        },
      },
    },
  },
  preview: { headers: CROSS_ORIGIN_ISOLATION },
  worker: {
    format: "es",
    plugins: () => [],
  },
});
