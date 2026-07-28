import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  base: "/",
  plugins: [svelte()],
  optimizeDeps: {
    exclude: ["@remotion/whisper-web"],
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
