// Self-contained flat config: no root eslint.config.js exists in this repo to
// extend (apps/hacienda-studio references eslint in package.json but has no
// committed config either), so this package brings its own minimal setup
// rather than depend on something that doesn't exist.
import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    ignores: ["dist/**", "src/_generated/**", "coverage/**"],
  },
);
