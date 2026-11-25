# Installing the Hacienda gateway for Claude Code and Claude Desktop

This document describes the supported installation path for pointing **Claude Code** at the
existing Hacienda `POST /v1/messages` gateway, and what that does — and does not — mean for
**Claude Desktop**. It is based on the current official Anthropic documentation (linked at
the end). The Codex installer (`hacienda codex install`) and its `config.toml` are a separate,
unchanged path.

## What the installer does

```sh
# Recommended: the credential never appears in settings.json.
export HACIENDA_CLAUDE_GATEWAY_TOKEN='<a Hacienda bearer token with documents:process>'
hacienda claude install --credential-mode helper

# Alternative: put the value in the user env block (file is written 0600).
hacienda claude install --credential-mode env

# Fully explicit paths / sources:
hacienda claude install \
  --settings /Users/me/.claude/settings.json \
  --gateway-url http://127.0.0.1:8787 \
  --credential-file /path/to/secret \
  --credential-mode helper
```

`hacienda claude install`:

1. Reads `~/.claude/settings.json` (or `$CLAUDE_CONFIG_DIR/settings.json`, or `--settings`).
2. Merges `env.ANTHROPIC_BASE_URL = <gateway URL>` and the credential placement into that
   document. **Every unrelated top-level key and every unrelated `env` variable is preserved**
   (`permissions`, `hooks`, `model`, `$schema`, user env vars, …). Only Hacienda's own keys are
   added or replaced, and switching modes removes the other credential key so two credential
   sources are never active at once. Invalid JSON / a non-object `env` fails closed.
3. Writes the document atomically with owner-only (`0600`) permissions, so a partially written
   file cannot produce a Claude Code *Settings Error*.

Arguments:

| Flag | Default | Meaning |
| --- | --- | --- |
| `--settings PATH` | `~/.claude/settings.json` | User settings file to merge into. |
| `--gateway-url URL` | `http://127.0.0.1:8787` | Gateway **origin/base**; Claude Code appends `/v1/messages`. Must be http(s), no embedded credentials, query or fragment. |
| `--credential-mode MODE` | `env` | `helper` (recommended) or `env`. |
| `--credential-env NAME` | `HACIENDA_CLAUDE_GATEWAY_TOKEN` | Name of the env var read **at install time**. Distinct from `HACIENDA_GATEWAY_TOKEN`, which is the Codex gateway's local secret. |
| `--credential-file PATH` | — | Read the credential from a file instead; takes precedence over the env var. |

## Safe credential handling

There is **no default credential value in the code**. The value is read at install time from
the named environment variable or from `--credential-file`, is required to be non-empty, is
never printed to stdout/stderr, and is held in a type whose `Debug` is redacted.

- `helper` mode (recommended): writes a `0600` secret file
  (`~/.claude/hacienda-gateway-token`), a `0700` helper script
  (`~/.claude/hacienda-gateway-key.sh`), and sets `apiKeyHelper` to that script. The helper
  reads the env var at runtime, falling back to the secret file (which is how a Desktop app
  launched from the dock, without shell exports, still gets the credential). **The secret is
  never stored in `settings.json`.** This is the Claude Code mechanism intended for
  vault-sourced or rotating credentials.
- `env` mode: writes the value directly to `env.ANTHROPIC_AUTH_TOKEN` in the `0600` settings
  file. This is the documented persistent configuration, but the plaintext value now lives on
  disk, so prefer `helper` where that matters.
- Never place a gateway credential in a project's `.claude/settings.json` or
  `.claude/settings.local.json`: those files are committed/shared and, for project files, env
  values only apply after the folder is trusted. Use the **user** file for a credential the
  CLI must see before first-run setup.

The helper's env-var name is validated to `[A-Za-z_][A-Za-z0-9_]*` and the script path is
shell-quoted, so a crafted flag cannot inject shell commands. The script prints **only** the
credential (no banners or logs) and exits non-zero when no credential is available.

## Supported authentication behaviour

The Hacienda gateway is an Anthropic-compatible provider. Its `/v1/messages` route is guarded
by the shared bearer middleware and requires the `documents:process` capability
(`hacienda-api/src/routes.rs`, `hacienda-core/src/auth/authz.rs`). The middleware extracts the
token only from `Authorization: Bearer <token>`.

| Claude Code setting | Header it sends | Works against Hacienda? |
| --- | --- | --- |
| `ANTHROPIC_AUTH_TOKEN` (installer default) | `Authorization: Bearer …` | **Yes** — this is what the gateway reads. |
| `apiKeyHelper` (helper mode) | Both `Authorization` and `x-api-key` | **Yes** — the `Authorization` copy is the one Hacienda uses. Helper output is cached ~5 minutes; keep it free of extra output. |
| `ANTHROPIC_API_KEY` | `x-api-key` | Only if something in front of Hacienda maps it to a bearer token. The in-tree middleware does **not** read `x-api-key`, so the installer does not use this variable. |

Other documented behaviour worth knowing:

- A settings-file `env` value overrides a shell export of the same variable. Run `/status` in
  Claude Code: the **Status** tab shows the `Anthropic base URL` line and which credential
  variable is active (`Auth token` / `API key`).
- Managed settings (MDM, server-managed, `managed-settings.json`) override user settings. An
  administrator-distributed gateway configuration wins over anything this installer writes.
- If a saved claude.ai login and a gateway credential are both active, Claude Code warns
  `auth may not work as expected`; use `/logout` to keep only the gateway credential.
  `ANTHROPIC_AUTH_TOKEN` takes precedence immediately; `ANTHROPIC_API_KEY` needs one-time
  approval.
- Remote Control and voice dictation require a claude.ai identity and are unavailable while a
  credential or `ANTHROPIC_BASE_URL` is set.
- Claude Code watches settings files and reloads edits to `env` and `apiKeyHelper`, but restart
  the CLI/Desktop to be certain the gateway route is applied.

The gateway itself must continue to fail closed: it validates inspectable content,
pseudonymises before any provider egress, forwards `anthropic-version`/`anthropic-beta` and
`cache_control` unchanged, and refuses bodies it cannot inspect. Installing a client config
never relaxes that boundary.

## Claude Desktop: what is and is not inherited

Claude Desktop's **Code tab runs Claude Code**, and the desktop app reads the same Claude Code
settings files as the terminal (`~/.claude/settings.json`, MCP servers, hooks, plugins). So the
user settings written here reach the embedded Claude Code for those general settings, including
the `apiKeyHelper` script.

However, the current official documentation is explicit: **the Desktop app reads gateway
*routing* from its third-party inference configuration, not from `ANTHROPIC_BASE_URL` or
`settings.json`.** To route the Desktop app through Hacienda you must configure that separately:

1. In Claude Desktop, enable **Help → Troubleshooting → Enable Developer Mode** (this restarts
   the app and adds a Developer menu).
2. Open **Developer → Configure Third-Party Inference…**, set **Inference provider = Gateway**,
   and enter the same base URL as `ANTHROPIC_BASE_URL` (`http://127.0.0.1:8787` for the local
   gateway). Choose a credential kind:
   - **Static API key** — a Hacienda bearer token placed in the managed profile;
   - **Credential helper** (`inferenceCredentialHelper`) — an absolute path to an executable
     that prints the credential, for example the `hacienda-gateway-key.sh` generated above;
   - **Interactive sign-in / OIDC** — per-user identity-provider tokens, if the gateway
     validates them.
   Alternatively, an administrator deploys the configuration via MDM or the Enterprise Admin
   Console; a managed configuration takes precedence and makes the local form read-only.
3. Fully quit and reopen Claude Desktop. Configuration changes take effect at launch.

With a gateway configuration active, Anthropic-hosted cloud environments disappear from the
selector and Remote Control is unavailable. SSH sessions in beta need Claude Desktop
1.40609.0+ and a gateway reachable from the remote host.

**Known limitation:** `hacienda claude install` configures the Claude Code user settings file,
not Claude Desktop's third-party inference configuration. The Desktop local config lives in a
`configLibrary/` directory whose format is written by the app's configuration window (the app
does not read an exported JSON config dropped into that directory), so there is no documented,
stable file to write from a CLI. The Desktop step above is therefore deliberately manual. This
is a client-configuration limitation only: the existing `/v1/messages` gateway already
implements the required Messages protocol, and no claim is made here about general Desktop Chat
rendering or about the Claude Mod `ui.render` surface.

## Verify

```sh
# 1. Direct protocol/credential check (Claude Code's verification request).
export ANTHROPIC_BASE_URL=http://127.0.0.1:8787  # same value as --gateway-url
curl -sS -w '\n%{http_code}\n' -X POST "$ANTHROPIC_BASE_URL/v1/messages" \
  -H "Authorization: Bearer $HACIENDA_CLAUDE_GATEWAY_TOKEN" \
  -H 'anthropic-version: 2023-06-01' -H 'content-type: application/json' \
  -d '{"model":"claude-sonnet-4-6","max_tokens":1,"messages":[{"role":"user","content":"."}]}'
```

In Claude Code, run `/status` and confirm the
`Anthropic base URL` and credential lines, then send a test message. For Desktop, confirm
requests arrive at Hacienda on `/v1/messages` with streaming and the required headers.

## Sources

- [Connect Claude Code to an LLM gateway](https://code.claude.com/docs/en/llm-gateway-connect)
- [Settings files and precedence](https://code.claude.com/docs/en/settings)
- [Gateway compatibility guide](https://code.claude.com/docs/en/llm-gateway-protocol)
- [Deploy Claude Desktop on 3P with an LLM gateway](https://claude.com/docs/third-party/claude-desktop/gateway)
- [Claude Desktop 3P configuration reference](https://claude.com/docs/third-party/claude-desktop/configuration)
- [Claude Desktop 3P configuration implementation](https://claude.com/docs/third-party/claude-desktop/configuration-format)
