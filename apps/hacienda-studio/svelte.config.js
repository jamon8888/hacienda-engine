import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

// svelte-check loads vite.config.ts to discover the preprocessor, and that load
// fails here because the config imports node builtins. Declaring it separately
// keeps `npm run check` working; vite.config.ts picks it up automatically.
export default {
  preprocess: vitePreprocess(),
};
