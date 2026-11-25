# Hacienda adapter for the Claude Code Mod

`claude-mod/` is a render-only Mod: its `ui.render` hook never opens a socket or
fetches a URL. This package supplies the local Mod adapter (`mod-adapter.js`)
and the packaging step that produces a standalone Claude Code plugin marketplace.
The supported bootstrap runs entirely inside the documented Mods API — no Node.js
built-ins, no host-side `globalThis` bridge, and no separate host process. It
requires Claude Code **v2.1.326 or later** for `$.process.hookEnv`.

See [`claude-mod/README.md`](../../claude-mod/README.md) for the bridge contract and
[the gateway install guide](../../docs/claude-code-gateway-install.md) for configuring
the loopback gateway and credential first. Build the distributable marketplace with:

```sh
npm --prefix integrations/claude-code-mod run package
```

## Supported bootstrap: `prompt.submit` + bounded `/mod/poll`

The packaged hook (`plugin/hooks/register.js`) registers `createModProjection($)`
on `prompt.submit`, before Claude Code dispatches the user message, so a
projection session exists before redaction begins:

- It reads the current session from `$.process.hookEnv()` and the gateway URL,
  correlation hint, and credential from `$.settings.read` / `$.env.get`.
- The gateway URL must be an exact numeric HTTP loopback origin
  (`http://127.0.0.1:8787` by default); remote URLs and browser-origin trust are
  never accepted.
- The bearer comes from `ANTHROPIC_AUTH_TOKEN` or from invoking only the exact
  Hacienda-generated `~/.claude/hacienda-gateway-key.sh` helper via
  `$.process.run` (no shell). Unknown helper paths and missing credentials fail
  closed.
- It POSTs `/mod/session` with the bearer, correlation hint, and origin, then
  validates the echoed loopback origin, opaque session id, short-lived
  capability, expiry, and correlation before creating the in-memory projection
  session.

Because `$.http.fetch` buffers response bodies, the `/mod/events` SSE stream
cannot be consumed by the Mod. The hook instead performs a bounded one-shot poll
before each `AssistantMessage` draw:

```text
GET /mod/poll?session=<opaque session id>&limit=32
Authorization: Bearer <documents:process credential>
Origin: <exact loopback origin>
X-Hacienda-Projection-Correlation: <non-secret correlation>
X-Hacienda-Projection-Capability: <session capability>

{ "events": [{ "type": "Mapping", "origin": "<exact loopback origin>",
  "sessionId": "<session id>", "token": "<pseudonym>",
  "original": "<value>", "capability": "<capability>" }],
  "expired": false, "closed": false }
```

The server atomically drains at most 32 mappings, drops originals above 4096
UTF-8 bytes, and caps the JSON body at 256 KiB. Every request uses the numeric
loopback origin returned by bootstrap; the hook sets `redirect: 'error'` and the
routes never redirect, so the bearer and capability headers cannot be stripped
by a cross-origin hop. Any transport, authentication, redirect, parse, size,
expiry, or session/origin/capability mismatch clears in-memory mappings and the
Mod keeps the pseudonym. Revealed values are visual-only: they never change the
stored transcript, model context, provider traffic, or later turns.

## Server-side route inventory

Four loopback-only routes are registered in the API route table, all behind the
same `documents:process` bearer guard as `/v1/messages`:

- `POST /mod/session` mints an opaque session id, a short-lived capability, and
  the exact loopback origin from the validated `Host` header. It binds the
  session to the authenticated tenant **and** principal and makes it the single
  projection target.
- `GET /mod/events?session=…` streams `Mapping` events over a capability-bound
  channel. Each frame carries `{type, origin, sessionId, token, original,
  capability}` and the renderer's `verifyEvent` rejects a frame whose capability
  or session does not match. This route is retained for local-capable hosts; the
  Desktop Mod uses the bounded poll instead because `$.http.fetch` buffers the
  streamed body.
- `GET /mod/poll?session=…&limit=32` is the Mods API transport: it atomically
  drains at most 32 mappings and returns `{events, expired, closed}` as bounded
  JSON. It requires the same bearer, correlation, origin, and capability checks
  and never redirects.
- `POST /mod/revoke?session=…` drops the session and its in-memory mappings.

`/v1/messages` resolves tokens minted during request/response redaction via a
server-side reveal context scoped to the caller's tenant and publishes them to
that one active session's in-memory queue. It does **not** alter the event
frames, the model output, or the stored transcript. If correlation is missing,
ambiguous, expired, or of the wrong principal, no mapping is published and the
Mod keeps the pseudonym.

This is a dedicated store, not a reuse of `ViewerSessionStore`: the Codex viewer
is a cookie-bound browser page with a globally active tab, while a Mod has no
cookie jar and must never receive another principal's originals through a
shared "active tab" slot.

## `ANTHROPIC_CUSTOM_HEADERS` assessment

Official Claude Code environment docs define `ANTHROPIC_CUSTOM_HEADERS` as
newline-separated `Name: Value` pairs (a single string with `\n` in JSON
settings). Claude Code sends those headers on its inference requests to the
configured gateway; in the `env` block of a settings file they apply from that
scope, and headers that look like credentials, org/tenant, routing, or
API-behaviour controls need approval when delivered by server-managed settings.
<https://code.claude.com/docs/en/env-vars>
<https://code.claude.com/docs/en/llm-gateway-connect>

Consequences for this adapter:

- It **can** carry a stable, per-process *correlation hint* (`X-Hacienda-
  Projection-Correlation`). The value is non-secret and installed into user
  settings; the gateway's `safe_routing_headers` whitelist means it is never
  forwarded to the Anthropic provider.
- It **cannot** be the session authenticator. It is static for the settings
  scope, readable by any process that can read the settings file, and replayable
  by any local process. Authentication therefore comes from the `documents:process`
  bearer plus a server-minted, single-use capability; the correlation value only
  selects which session a mapping may target, and ambiguous correlations fail
  closed.
- A value that needs MDM approval must not be assumed. The Hacienda header is
  deliberately named as a correlation hint, not a credential/org/routing control.
- If official runtime support for custom headers is absent (older Claude Code,
  a managed profile that strips the value, or a Desktop third-party inference
  configuration that does not forward it), the Mod has no correlation to publish
  against. It fails closed: no bridge is installed and the Mod keeps pseudonyms.
  Nothing in this package weakens tenant/session isolation to work around that.

## Token format bridge

Hacienda pseudonyms are `[EMAIL:k1:…]`; the Mod token scanner recognises
`EMAIL_K1_…`. `mod-adapter.js` normalises tokens only for the in-memory projection
session and restores the exact wire token for every unrevealed segment. The
original value appears only in `revealed` segments of the rendered tree.

## Tests

```sh
node --test integrations/claude-code-mod/test/
```
