# Hacienda Secrecy Client

The Tauri renderer is a small React/Tiptap conversation surface for the existing
`stream-claude`, `stream-codex`, and `open-original` commands. Provider requests
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
or transcript persistence. Prompts are cleared from the editor on submission
and never added to history. Provider or response failures show a content-free
fail-closed message.

PII tokens are canonical Tiptap JSON with the `haciendaSensitive` mark and are
presented with their category as visible text and an accessible label, plus
distinct border patterns. The `open-original` bridge contract accepts only the
five opaque references (`document_ref`, `version_ref`, `original_ref`,
`viewer_ref`, `capability_ref`), but this runtime does not supply references from
an authorized, expiring, audited viewer session. The UI reports the viewer as
unavailable instead of offering an unconnected reveal flow. The backend also
exposes no renderer command for loading prior conversations.

The manual selection action is shown as unavailable because this runtime has no
`manual-pseudonymize-selection` command. No selection is annotated or replaced
in the renderer. The current backend also has no append-only versioned document
save command, so the app does not persist edits; any future save path must accept
token-only canonical JSON.

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
packaging. `npm run check` is a non-interactive command contract check for the
provider DTO shapes, opaque viewer payload, malformed responses, and unavailable
Tauri bridge.

The desktop provider runtime remains fail-closed if its gateway and local
redactor are not configured. `open-original` also fails closed unless a native
viewer session has registered and authorized the supplied references.
