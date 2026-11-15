# Hacienda Secrecy Client

The Tauri renderer is a small React/Tiptap conversation surface for the existing
`stream-claude`, `stream-codex`, and local viewer commands. Provider requests
keep the Rust DTO's exact `conversation_id`, `context_id`, `prompt`, and `context`
fields inside the command's `request` argument. The renderer keeps the same
ephemeral conversation and context IDs while switching providers and across
turns.

The message composer uses Tiptap 2.27.3 and the same `haciendaSensitive` mark
schema as Hacienda Studio. The validated response chunks become canonical,
token-only Tiptap JSON and the read-only history renders that JSON. The renderer
sends the current editor text plus prior turns in the Tauri `context` DTO. Prior
user messages and tokenized assistant replies are retained only in renderer
memory for conversation continuity; they are cleared when the app closes or
reloads. The renderer does not implement redaction, cryptography, HTTP requests,
or transcript persistence. Provider send stays disabled until
`local-pii-status` reports `provider_send_enabled: true`, which requires both the
complete local model redactor and a configured gateway transport. This status
checks local configuration, not provider network reachability. Prompts are cleared
from the editor on submission and never added to history. Provider or response
failures show a content-free fail-closed message.

PII tokens are canonical Tiptap JSON with the `haciendaSensitive` mark and are
presented with their category as visible text and an accessible label, plus
distinct border patterns. The `open-original` bridge contract accepts only the
five opaque references (`document_ref`, `version_ref`, `original_ref`,
`viewer_ref`, `capability_ref`), but this runtime does not supply references from
an authorized, expiring, audited original viewer session. Tokenized provider
responses can instead open in the separate Tauri local viewer, which loads its
own `?viewer=local` route and requests token-only content from
`local-viewer-content`. A reveal requires an explicit click and calls
`reveal-local-token` for that token alone; plaintext stays in ephemeral viewer
state and the window closes on its 59-second local timeout or on session errors.
The backend exposes no command for loading prior conversations.

Manual selection pseudonymization calls `pseudonymize-selection` with the
selected text and category. On success, the editor replaces the selection with
the returned token and marks it with the canonical `manual` provenance. Raw
selection text is not added to conversation history. The current renderer has no
document save flow; it does not persist edits, and any future save path must
accept append-only, versioned, token-only JSON.

## Run and package

Run these commands from this directory:

```sh
npm ci
npm run check
npm run build
npm run tauri:dev
npm run tauri:build
```

The app has its own `package-lock.json`. Tauri's config starts Vite on
`127.0.0.1:1420` for development and builds the static frontend into `dist/` for
packaging. Both Tauri commands first stage the matching ONNX Runtime archive,
verify its pinned SHA-256, and include the native libraries and notices as app
resources. The app points Xberg at that bundled runtime before any local model
loads. Archives are cached under `~/.cache/hacienda-secrecy-client/onnxruntime/`
(override with `HACIENDA_ORT_CACHE`); Python 3.10 or later is required for staging.
Embedding weights are fetched into private app data when the user activates local
search. GLiNER weights are installed separately with
`scripts/install_gliner2_pii.py`; neither model checkpoint is included in the app
bundle. `npm run check`
is a non-interactive command contract check for the
provider DTO shapes, empty and malformed references, local viewer and manual
command payloads, detector gating, and malformed Tauri responses. TypeScript
also checks the JavaScript bridge DTO annotations and the shared mark provenance
union.

The desktop provider runtime remains fail-closed if local detection is not
complete. `open-original` also fails closed unless a native viewer session has
registered and authorized the supplied references.
