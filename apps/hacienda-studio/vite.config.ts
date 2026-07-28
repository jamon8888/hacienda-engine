import { createReadStream, existsSync, statSync } from "node:fs";
import { dirname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const appRoot = dirname(fileURLToPath(import.meta.url));
const workspaceRoot = join(appRoot, "..", "..");

/**
 * npm workspaces hoists dependencies to the workspace root, leaving
 * apps/hacienda-studio/node_modules empty. Vite's dev server roots at the app
 * directory, so the `/node_modules/@xberg-io/xberg-wasm/…` URL that the wasm
 * glue emits misses and the SPA fallback answers with index.html. Serve those
 * requests from the hoisted location instead. Production builds are unaffected
 * — Rollup resolves and emits the .wasm itself.
 */
function serveHoistedNodeModules(): Plugin {
  return {
    name: "serve-hoisted-node-modules",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const url = req.url?.split("?")[0];
        if (!url?.startsWith("/node_modules/")) return next();

        const abs = normalize(join(workspaceRoot, url));
        if (!abs.startsWith(workspaceRoot)) return next();
        if (existsSync(join(appRoot, url))) return next();
        if (!existsSync(abs) || !statSync(abs).isFile()) return next();

        if (abs.endsWith(".wasm")) res.setHeader("Content-Type", "application/wasm");
        createReadStream(abs).pipe(res);
      });
    },
  };
}

export default defineConfig({
  base: "/",
  plugins: [svelte(), serveHoistedNodeModules()],
  optimizeDeps: {
    // Both packages resolve their .wasm via new URL(..., import.meta.url).
    // Pre-bundling rewrites that base to .vite/deps/, so the request misses
    // and the dev server's SPA fallback answers with index.html — WebAssembly
    // then rejects it ("expected magic word 00 61 73 6d, found 3c 21 64 6f").
    exclude: ["@remotion/whisper-web", "@xberg-io/xberg-wasm"],
  },
  build: {
    target: "esnext",
    outDir: "../../dist/apps/hacienda-studio",
    emptyOutDir: true,
    rollupOptions: {
      output: {
        manualChunks: {
          "xberg-wasm": ["@xberg-io/xberg-wasm"],
          vendor: ["jszip", "idb"],
        },
      },
    },
  },
  server: {
    headers: {
      "Cross-Origin-Opener-Policy": "same-origin",
      "Cross-Origin-Embedder-Policy": "require-corp",
    },
    proxy: {
      "/api/huggingface": {
        target: "https://huggingface.co",
        changeOrigin: true,
        followRedirects: true,
        rewrite: (path) => path.replace(/^\/api\/huggingface/, ""),
      },
    },
  },
  worker: {
    format: "es",
    plugins: () => [],
  },
});
