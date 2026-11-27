#!/usr/bin/env bash
# Per-VPS-profile setup for the live CODEMAP harness.
#
# Installs the `codemap` command into the current user's ~/.local/bin, wires
# basemind into the agent configs via agent-harness, ensures this user's
# basemind daemon is up, and creates/updates the Orca "Refresh CODEMAP" automation
# in this user's own Orca host. Idempotent; --check is read-only.
#
# Must be run as the profile that owns the Orca host (pierre, jamin, candy...),
# from inside one of that profile's Orca worktrees.
set -euo pipefail

NAME="Refresh CODEMAP (fleet)"
SCHEDULE="*/15 * * * *"
PROVIDER="${CODEMAP_PROVIDER:-opencode}"
BIN_DIR="${HOME:?HOME is not set}/.local/bin"
SOURCE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

PROMPT='Refresh the live CODEMAP.md code maps for every git root known to Orca on this VPS.

1. Ensure the basemind index daemon is up: `basemind daemon ensure -q`.
2. Run `codemap fleet` from this workspace. It regenerates `docs/CODEMAP.md` in each Orca repo/worktree and skips ones that are already fresh. The installed command is `~/.local/bin/codemap`.
3. Report the final `SUMMARY refreshed=... fresh=... skipped=...` line and any `SKIP` lines verbatim.
4. Do not edit any other files, do not commit, do not create worktrees, do not message other agents. If `codemap` is missing or fails, report the error and stop.'

usage() {
  cat <<'USAGE'
setup-codemap-user.sh [--check|--apply]

  --check  (default) report what would be installed/changed; no writes
  --apply  install codemap + setup-codemap, bootstrap agents, ensure the
           basemind daemon, and create/update the Orca automation

Run as the VPS profile that owns the Orca host, inside one of its worktrees.
USAGE
}

note() { printf '[codemap-setup] %s\n' "$*"; }
fail() { printf '[codemap-setup] ERROR: %s\n' "$*" >&2; exit 1; }

orca_cli() {
  if [ -n "${ORCA_CLI_COMMAND:-}" ]; then printf '%s\n' "$ORCA_CLI_COMMAND"
  elif command -v orca-ide >/dev/null 2>&1; then printf 'orca-ide\n'
  else printf 'orca\n'; fi
}

mode="check"
case "${1:-}" in
  --check|"") mode="check" ;;
  --apply)    mode="apply" ;;
  -h|--help)  usage; exit 0 ;;
  *)          usage >&2; exit 2 ;;
esac
[ "$#" -le 1 ] || { usage >&2; exit 2; }

command -v git >/dev/null 2>&1      || fail "git is required"
command -v jq >/dev/null 2>&1       || fail "jq is required"
command -v basemind >/dev/null 2>&1 || fail "basemind is required"

if [ -f "$SOURCE_DIR/codemap.sh" ]; then
  src_codemap="$SOURCE_DIR/codemap.sh"
elif [ -f "$SOURCE_DIR/codemap" ]; then
  src_codemap="$SOURCE_DIR/codemap"
else
  fail "codemap source not found next to this script"
fi
if [ -f "$SOURCE_DIR/setup-codemap-user.sh" ]; then
  src_setup="$SOURCE_DIR/setup-codemap-user.sh"
else
  src_setup="$SOURCE_DIR/setup-codemap"
fi

repo_root=$(git -C "$PWD" rev-parse --show-toplevel 2>/dev/null) \
  || fail "run this from inside a git worktree"
ORCA=$(orca_cli)
command -v "$ORCA" >/dev/null 2>&1 || fail "Orca CLI not found: $ORCA"
"$ORCA" status --json >/dev/null 2>&1 || fail "Orca runtime is not reachable"

existing=$("$ORCA" automations list --json 2>/dev/null \
  | jq -r --arg name "$NAME" '.result.automations[]? | select(.name == $name) | .id' | head -1)

note "profile=$(id -un)  repo=$repo_root"
note "orca=$ORCA  automation=$(if [ -n "$existing" ]; then printf '%s (update)' "$existing"; else printf 'new'; fi)"
note "codemap=$BIN_DIR/codemap  schedule=$SCHEDULE  provider=$PROVIDER"

if [ "$mode" = "check" ]; then
  note "check only; no writes. Re-run with --apply to install."
  exit 0
fi

mkdir -p "$BIN_DIR"
for pair in "codemap:$src_codemap" "setup-codemap:$src_setup"; do
  name="${pair%%:*}"
  src="${pair#*:}"
  dst="$BIN_DIR/$name"
  if [ -e "$dst" ] && cmp -s "$src" "$dst"; then
    note "unchanged: $dst"
  else
    [ -e "$dst" ] && cp -a "$dst" "$dst.bak.$(date +%Y%m%d%H%M%S)"
    install -m 755 "$src" "$dst"
    note "installed: $dst"
  fi
done

if command -v agent-harness >/dev/null 2>&1; then
  agent-harness bootstrap --apply --json >/dev/null 2>&1 \
    && note "agent-harness bootstrap: applied (basemind MCP wired for all local agents)" \
    || note "agent-harness bootstrap reported a problem; inspect with: agent-harness bootstrap --check --json"
else
  note "agent-harness not installed; skipping agent MCP wiring (install the vps-harness first)"
fi

basemind daemon ensure --root "$repo_root" -q >/dev/null 2>&1 \
  && note "basemind daemon: up" \
  || note "basemind daemon ensure failed; run: basemind daemon ensure --root '$repo_root'"

if [ -n "$existing" ]; then
  "$ORCA" automations edit "$existing" \
    --name "$NAME" --trigger "$SCHEDULE" --prompt "$PROMPT" \
    --provider "$PROVIDER" --workspace current --fresh-session --enabled --json >/dev/null 2>&1 \
    && note "automation updated + enabled: $existing" \
    || fail "could not update automation $existing"
else
  created=$("$ORCA" automations create \
    --name "$NAME" --trigger "$SCHEDULE" --prompt "$PROMPT" \
    --provider "$PROVIDER" --workspace current \
    --precheck 'codemap fleet-stale' --precheck-timeout 120 \
    --fresh-session --enabled --json 2>/dev/null \
    | jq -r '.result.automation.id // empty')
  [ -n "$created" ] || fail "could not create automation (is cwd an Orca worktree?)"
  note "automation created + enabled: $created"
fi

note "done. Verify: codemap fleet-stale && $ORCA automations list --json"
