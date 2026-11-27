#!/usr/bin/env bash
# Generate a live, agent-facing docs/CODEMAP.md from the current code state.
#
# Subcommands:
#   generate [--root DIR]   write DIR/docs/CODEMAP.md (only when content changes)
#   stale [--root DIR]      exit 0 when regeneration is needed, 1 when fresh
#   targets                 list unique git roots known to Orca (repos + worktrees)
#   fleet                   regenerate CODEMAP.md in every target
#   fleet-stale             exit 0 when any target is stale (Orca automation precheck)
#
# Requires: git, jq, basemind; `targets`/`fleet` also need the Orca CLI.
set -euo pipefail

usage() {
  cat <<'USAGE'
codemap.sh <generate|stale|targets|fleet|fleet-stale> [--root DIR]

  generate     write <root>/docs/CODEMAP.md from basemind overview + git state
  stale        exit 0 if CODEMAP.md is missing or the source fingerprint changed
  targets      print unique git roots from `orca repo list` + `orca worktree list`
  fleet        regenerate every target, printing REFRESHED / FRESH / SKIP lines
  fleet-stale  exit 0 if any target is stale (used as an Orca --precheck)
USAGE
}

die() {
  printf 'codemap: %s\n' "$*" >&2
  exit 2
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

orca_cli() {
  if [ -n "${ORCA_CLI_COMMAND:-}" ]; then
    printf '%s\n' "$ORCA_CLI_COMMAND"
  elif command -v orca-ide >/dev/null 2>&1; then
    printf 'orca-ide\n'
  else
    printf 'orca\n'
  fi
}

normalize_root() {
  local path="${1:-$PWD}"
  [ -d "$path" ] || die "not a directory: $path"
  git -C "$path" rev-parse --show-toplevel 2>/dev/null || die "not a git worktree: $path"
}

# Hash HEAD plus the content of every tracked/untracked non-ignored file except
# the generated doc itself. Stable across runs with no code change.
fingerprint() {
  local root="$1" head
  head=$(git -C "$root" rev-parse HEAD)
  {
    printf '%s\n' "$head"
    (
      cd "$root"
      git ls-files --cached --others --exclude-standard -z -- ':(exclude)docs/CODEMAP.md' \
        | xargs -0 -r sha256sum --
    )
  } | sha256sum | cut -d' ' -f1
}

stored_fingerprint() {
  local out="$1"
  [ -f "$out" ] || return 1
  sed -n 's/^<!-- codemap:fingerprint:\([0-9a-f]*\) -->$/\1/p' "$out" | head -1
}

ensure_index() {
  local root="$1" count
  count=$(basemind --root "$root" status --json 2>/dev/null | jq -r '.file_count // 0' || printf '0')
  if [ "${count:-0}" = "0" ]; then
    # `scan` fights the daemon's lock; `rescan` with no paths forwards to it.
    basemind --root "$root" rescan -q >/dev/null 2>&1 || true
  fi
}

cmd_generate() {
  local root="$1" out fp tmp ov recent dirty
  need git
  need jq
  need basemind
  root=$(normalize_root "$root")
  out="$root/docs/CODEMAP.md"

  ensure_index "$root"
  fp=$(fingerprint "$root")
  ov=$(basemind --root "$root" overview --json 2>/dev/null) \
    || die "basemind overview failed for $root"
  recent=$(basemind --root "$root" git recent --json 2>/dev/null) \
    || recent='{"commits":[]}'
  dirty=$(git -C "$root" status --porcelain --untracked-files=normal -- ':(exclude)docs/CODEMAP.md' | wc -l | tr -d ' ')

  tmp=$(mktemp "${TMPDIR:-/tmp}/codemap.XXXXXX")
  {
    printf '<!-- codemap:fingerprint:%s -->\n' "$fp"
    printf '# Code map — %s\n\n' "$(basename "$root")"
    cat <<'INTRO'
> **Generated file.** Rebuilt from the working tree and the basemind index by
> `codemap.sh generate`. Do not edit by hand. The Orca automation
> "Refresh CODEMAP" keeps this current fleet-wide.

## Read this first

- This is the live, agent-facing snapshot of the code. Read it before opening source files.
- Query the index instead of grepping the tree:
  `basemind code outline <path>`, `basemind code symbols --name <name>`,
  `basemind code grep <pattern>`, `basemind code refs <name>`,
  `basemind docs search "<concept>"`.
- Refresh one repo with `codemap generate --root <repo>`; all Orca targets with `codemap fleet`.

INTRO
    printf '## Repository at a glance\n\n'
    printf '%s\n' "$ov" | jq -r '"- **Files:** \(.files // 0) · **Lines:** \(.lines // 0) · **Bytes:** \(.bytes // 0)"'
    printf '%s\n' "$ov" | jq -r --arg dirty "$dirty" '"- **Working tree:** \($dirty) non-clean path(s)"'
    printf '%s\n' "$ov" | jq -r '.routes // {} | "- **HTTP routes:** \(.count // 0) across \(.files // 0) files (\(.methods // {} | to_entries | map("\(.key)=\(.value)") | join(", ")))"'

    printf '\n### Languages\n\n| Language | Files | Lines | Bytes |\n|---|---:|---:|---:|\n'
    printf '%s\n' "$ov" | jq -r '.languages[]? | "| \(.language) | \(.files) | \(.lines) | \(.bytes) |"'

    printf '\n### Layout\n\n| Path | Files | Lines | Language |\n|---|---:|---:|---|\n'
    printf '%s\n' "$ov" | jq -r '.layout[]? | "| `\(.path)` | \(.files) | \(.lines) | \(.language) |"'

    printf '\n### Build manifests\n\n| Manifest | Ecosystem | Workspace members |\n|---|---|---|\n'
    printf '%s\n' "$ov" | jq -r '.manifests[]? | select(((.members // []) | length) > 0) | "| `\(.path)` | \(.ecosystem // "-") | \(.members | length): \(.members | join(", ")) |"'

    printf '\n### Entry points (first 50)\n\n| Kind | Name | Path |\n|---|---|---|\n'
    printf '%s\n' "$ov" | jq -r '.entry_points[:50][]? | "| \(.kind) | \(.name) | `\(.path)` |"'

    printf '\n### Largest files (top 10)\n\n| Path | Lines |\n|---|---:|\n'
    printf '%s\n' "$ov" | jq -r '.largest[:10][]? | "| `\(.path)` | \(.lines) |"'

    printf '\n### Most imported (top 10)\n\n| Path | Importers |\n|---|---:|\n'
    printf '%s\n' "$ov" | jq -r '.most_imported[:10][]? | "| `\(.path)` | \(.importers) |"'

    printf '\n### Naming conventions\n\n| Language | File case | Symbol cases |\n|---|---|---|\n'
    printf '%s\n' "$ov" | jq -r '.naming[]? | "| \(.language) | \(if .files.case then "\(.files.case) (\(.files.percent)%)" else "-" end) | \(.symbols // {} | to_entries | map("\(.key)=\(.value.case)") | join(", ")) |"'

    printf '\n### Recent commits (top 10)\n\n'
    printf '%s\n' "$recent" | jq -r '.commits[:10][]? | "- `\(.[0:12])` \(.[13:])"'
  } > "$tmp"

  if [ -f "$out" ] && cmp -s "$tmp" "$out"; then
    rm -f "$tmp"
    printf 'FRESH %s\n' "$root"
    return 0
  fi

  mkdir -p "$(dirname "$out")"
  mv "$tmp" "$out"
  # The daemon normally sees the write; this also registers non-hot workspaces.
  basemind --root "$root" rescan docs/CODEMAP.md -q >/dev/null 2>&1 || true
  printf 'REFRESHED %s\n' "$root"
}

cmd_stale() {
  local root="$1" out stored current
  root=$(normalize_root "$root")
  out="$root/docs/CODEMAP.md"
  [ -f "$out" ] || return 0
  stored=$(stored_fingerprint "$out" || true)
  [ -n "$stored" ] || return 0
  current=$(fingerprint "$root")
  [ "$stored" != "$current" ]
}

cmd_targets() {
  need jq
  local orca
  orca=$(orca_cli)
  {
    "$orca" repo list --json 2>/dev/null | jq -r '.result.repos[]?.path // empty'
    "$orca" worktree list --limit 10000 --json 2>/dev/null | jq -r '.result.worktrees[]?.path // empty'
  } | while IFS= read -r path; do
    [ -n "$path" ] || continue
    [ -d "$path" ] || continue
    git -C "$path" rev-parse --is-inside-work-tree >/dev/null 2>&1 || continue
    printf '%s\n' "$path"
  done | sort -u
}

cmd_fleet() {
  local refreshed=0 fresh=0 skipped=0 path out
  while IFS= read -r path; do
    if ! cmd_stale "$path"; then
      printf 'FRESH %s\n' "$path"
      fresh=$((fresh + 1))
      continue
    fi
    if out=$(cmd_generate "$path" 2>&1); then
      printf '%s\n' "$out"
      refreshed=$((refreshed + 1))
    else
      printf 'SKIP %s: %s\n' "$path" "$(printf '%s' "$out" | tail -1)"
      skipped=$((skipped + 1))
    fi
  done < <(cmd_targets)
  printf 'SUMMARY refreshed=%d fresh=%d skipped=%d\n' "$refreshed" "$fresh" "$skipped"
}

cmd_fleet_stale() {
  local path any=0
  while IFS= read -r path; do
    if cmd_stale "$path"; then
      printf 'STALE %s\n' "$path"
      any=1
    fi
  done < <(cmd_targets)
  [ "$any" -eq 1 ]
}

main() {
  if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ] || [ "$#" -eq 0 ]; then
    usage
    [ "$#" -eq 0 ] && return 2
    return 0
  fi
  local cmd="$1"
  shift
  local root="$PWD"
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --root)
        [ "$#" -ge 2 ] || die "--root needs a path"
        root="$2"
        shift 2
        ;;
      -h|--help)
        usage
        return 0
        ;;
      *)
        die "unknown argument: $1"
        ;;
    esac
  done

  case "$cmd" in
    generate)   cmd_generate "$root" ;;
    stale)      cmd_stale "$root" ;;
    targets)    cmd_targets ;;
    fleet)      cmd_fleet ;;
    fleet-stale) cmd_fleet_stale ;;
    "")         usage; return 2 ;;
    *)          die "unknown subcommand: $cmd" ;;
  esac
}

main "$@"
