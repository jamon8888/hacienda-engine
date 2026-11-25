# Hacienda Claude Code Mod renderer

This Mod targets Claude Code's `ui.render` site for `AssistantMessage` on the
Claude Desktop **Code tab**. Anthropic's contract supplies markdown `text` and
read-only `isFirstOfReply` / `onScreen`; a rewritten `text` changes drawing
only. This implementation leaves those props and the stored transcript intact,
calling `next(event)` unless a local projection explicitly replaces at least
one token. Revealed originals are emitted as plain `Text` children in gray, so
HTML-looking input is text and is never interpreted as markup.

## Install

Configure Hacienda's gateway first using
[`docs/claude-code-gateway-install.md`](../docs/claude-code-gateway-install.md).
The Mod supports the generated helper credential mode and the user-level
`ANTHROPIC_AUTH_TOKEN` env mode. Then build and install the documented
Claude Code plugin/Mod from a local marketplace:

```sh
npm --prefix integrations/claude-code-mod run package
claude plugin marketplace add ./dist/hacienda-claude-marketplace
claude plugin install hacienda-projection-renderer@hacienda-local
```

Then restart Claude Desktop and open a **Code** session. The marketplace is
registered for the current user by the CLI and appears under Desktop **Plugins
→ Add plugin**. Full bootstrap requires Claude Code v2.1.326 or later. Remove the
marketplace and plugin with `claude plugin marketplace remove hacienda-local`
and `claude plugin uninstall hacienda-projection-renderer`.

The generated package uses Anthropic's documented plugin manifest and Mods hook
module format. Current docs require Claude Code v2.1.275 or later for
`/plugin install` in a session.

## Bridge contract

The renderer does not open sockets, fetch URLs, or perform network discovery.
The Mod hook bootstraps and polls the local adapter; it fails closed and leaves
content pseudonymized if any precondition fails. The bridge contract is:

1. Establish a short-lived Claude session with Hacienda through a **same-device
   loopback** service and authenticate it out of band. Never accept arbitrary
   remote URLs or use browser-origin trust alone as authentication.
2. Create `createProjectionSession({ sessionId, origin, expiresAt, verifyEvent })`
   with an exact loopback HTTP origin, an opaque session id, and an authenticator
   that verifies each event's capability/signature and session/tour authorization.
3. Deliver mapping events only as `{ type: 'Mapping', origin, sessionId, token,
   original }` through `acceptMapEvent`. Events fail closed on origin mismatch,
   session mismatch, failed authentication, or expiry. Keep the projection
   session in memory only and call `revoke()` on session end/revocation.
4. Keep the projection session as an in-memory bridge before the renderer runs.
   Its `project(text)` result is ephemeral segments `{text, revealed}`. The
   adapter/server must not persist or log originals; only Claude's visual
   renderer receives the projected values. Never send them to Messages,
   transcript storage, later turns, tools, or remote services.

## Desktop bootstrap boundary

The packaged Mod now uses the documented APIs directly and requires Claude Code
v2.1.326 or later for `$.process.hookEnv`. Its `prompt.submit` hook bootstraps
Hacienda before the prompt is dispatched. It gets user settings/environment
through `$.settings.read` and `$.env.get`; helper mode invokes only the exact
Hacienda-generated `~/.claude/hacienda-gateway-key.sh` path with
`$.process.run` (no shell), while env mode reads `ANTHROPIC_AUTH_TOKEN`.
Unknown helper paths and missing credentials fail closed.

The old SSE route cannot be consumed because `$.http.fetch` buffers its body.
The Mod instead polls one bounded response before each `AssistantMessage` draw:

```text
GET /mod/poll?session=<opaque session id>&limit=32
Authorization: Bearer <documents:process credential>
X-Hacienda-Projection-Correlation: <non-secret correlation>
X-Hacienda-Projection-Capability: <session capability>

{ "events": [{ "type": "Mapping", "origin": "<exact loopback origin>",
  "sessionId": "<session id>", "token": "<pseudonym>",
  "original": "<value>", "capability": "<capability>" }],
  "expired": false, "closed": false }
```

The server atomically drains at most 32 queued events, drops mappings above
4096 UTF-8 bytes, and caps the JSON response at 256 KiB. The hook checks those
limits again and rejects non-2xx, malformed, expired, closed, or mismatched
session/origin/capability responses. Every request starts at the numeric
loopback origin configured for Hacienda; bootstrap must return that same exact
origin. The `/mod/*` routes issue no redirects, and the Mod requests
`redirect: 'error'`; the official fetch API otherwise follows up to five
redirects and strips authorization/capability headers on cross-origin hops.
No URL from an event is used for a request. Any error clears in-memory mappings
and keeps pseudonyms visible.

The Node host adapter remains available for trusted embedding hosts, but the
Desktop Mod no longer imports it. A Claude Desktop Code-tab smoke test of the
new prompt-submit bootstrap, bounded polling, and visual-only rendering remains
required.

Official format and API references: [Mods overview](https://code.claude.com/docs/en/plugins/mods/overview),
[Mods API](https://code.claude.com/docs/en/plugins/mods/api),
[Mods reference](https://code.claude.com/docs/en/plugins/mods/reference),
[plugin installation and Desktop](https://code.claude.com/docs/en/plugins/install),
and [Claude Desktop Code tab](https://code.claude.com/docs/en/desktop).
