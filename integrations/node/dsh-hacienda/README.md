# @hacienda/dsh-hacienda

DeepSeek Harness **bundle** (Approach A — a distribution, not a fork) that packages
hacienda-engine's harness plugin rows so they survive a web-bundle rebuild.

## What it composes

| Row | Purpose |
|---|---|
| `hacienda-host` | Host RPCs for the **Artifacts view** (`list-workspace`, `read-file-text`, `file-download-url`, `scan-artifacts`) + the `scan_folder` model tool, over `ctx.fs` (lib/host.js). |
| `llm-pi-ai-ollama` | Ollama local models via the shipped `@deepseek-ai/dsh-llm-pi-ai` adapter (Gemma selectable in the model picker; config-only). |
| `web-search-exa` + `web-fetch-http` | Exa deep search + anonymous URL fetch. `searchProvider: exa` + `tool-web` enabled. |
| *(browser roster)* | The **Files / Artifacts** view tab (`conversation.view`), client code at lib/client.js — delivered as a prebuilt client bundle by the C3 web build. |

## Layout

- `lib/host.js` — Host Cordis plugin (the working dynamic-plugin code).
- `lib/client.js` — Client Cordis plugin (the `artifacts` `conversation.view` tab).
- `lib/index.js` — main entry (Host plugin).
- `cordis.patch.yml` — the profile composition (inserts the rows above).

## Security

- The Exa key is **never** in this repo: the `web-search-exa` row reads
  `process.env.EXA_API_KEY` at boot. Ollama is keyless.
- No document content is stored in code; the Artifacts view streams from `ctx.fs`.

## Build / C3 path

To ship the browser Artifacts view, the client entry (lib/client.js) must be bundled
into the artifact the harness's `dsh-client-modules` serves under
`/plugins/hacienda-artifacts/client.js`. That is the C3 web build
(`pnpm run build:web` over this package's client entry + the DSH web frontend) —
no such build script exists yet in this package.
Until that build runs, the Host RPCs and `scan_folder` tool still work; the browser
tab is the last slice to light up. Enabling the tab additionally requires
uncommenting the `dsh.client` roster entry at the bottom of `cordis.patch.yml`
(it ships commented out on purpose — activating it before the build artifact
exists gives `dsh.client` a plugin id with nothing on disk to scan).

See `superpowers/plans/2026-08-18-dsh-hacienda-plugin-implementation.md` (Phase C3)
and `docs/superpowers/specs/2026-08-18-M2-dsh-plugin-export-and-assure.md` §13–§16.

## Installing into a profile

Add `@hacienda/dsh-hacienda` to the profile's `bundles` (after `@deepseek-ai/dsh-base`
and `@deepseek-ai/dsh-web-app`) and pnpm-install it, or use the launch script on the
bundled `hacienda` profile.
