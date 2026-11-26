# Live CODEMAP harness (Orca + basemind)

`docs/CODEMAP.md` is a generated, agent-facing snapshot of the current code state. It is
rebuilt from the working tree and the Basemind index, ignored by Git, and shared by every
agent that opens the repo. This document describes the optional Orca automation that keeps
it current for a user profile.

## Pieces

| Piece | Scope | Role |
|---|---|---|
| `scripts/codemap.sh` | installable per profile | Generates `docs/CODEMAP.md`, fingerprints source, enumerates Orca git roots |
| `~/.local/bin/codemap` | per profile | Installed command (`codemap generate\|stale\|targets\|fleet\|fleet-stale`) |
| `scripts/setup-codemap-user.sh` / `~/.local/bin/setup-codemap` | per profile | Idempotent one-command setup (`--check` / `--apply`) |
| basemind comms daemon | per profile, machine-global for that user | Always-on index watcher; agents reach it through their MCP config |
| Orca automation `Refresh CODEMAP (fleet)` | per profile's Orca host | Cron `*/15 * * * *`, skips runs when no source changed |

Orca targets are the git roots returned by `orca repo list` plus `orca worktree list`
(unique paths). The target set is profile-specific and changes as repos and worktrees
are added or removed. A raw filesystem scan is deliberately **not** used — it would pick
up vendored/nested checkouts.

## Setup for every VPS profile

Each OS profile (`pierre`, `jamin`, `candy`) owns its own Orca host, basemind daemon and
agent configs. Homes are mode `750` and there is no passwordless sudo, so one account
cannot install for another: **run this as each profile, from one of that profile's Orca
worktrees.**

```sh
# once per profile, as that user, inside an Orca worktree
setup-codemap --check     # read-only: shows automation state and what will be installed
setup-codemap --apply     # install codemap, wire agents, ensure daemon, enable automation
```

If `setup-codemap` is not on `PATH` yet, copy `scripts/codemap.sh` and
`scripts/setup-codemap-user.sh` from this repo into that profile's `~/.local/bin` as
`codemap` and `setup-codemap`, then run the commands above. The vps-harness install
(`./vps-harness/bin/agent-harness install`) is a prerequisite for the agent-MCP step;
without it that step is skipped with a warning and the rest still works.

`--apply` is idempotent: existing binaries are backed up before replacement, and an
existing automation is updated and enabled rather than duplicated.

The automation provider defaults to `opencode` (the agent already authenticated in this
profile). Override with `CODEMAP_PROVIDER=claude` / `codex` / etc. when a profile uses a
different agent. Pick one that needs **no interactive first-run login**: a cron cannot
answer a login or theme picker, and the run will stay stuck. Claude on a fresh profile is
a known trap — it asks for a login method before reading the prompt.

## How the loop stays fresh

1. An agent edits code → the basemind daemon (or the repo's PostToolUse/pre-commit hooks)
   re-indexes the affected workspace.
2. On the cron tick, Orca runs `codemap fleet-stale` as a precheck. It hashes HEAD plus
   every tracked/untracked non-ignored file except `docs/CODEMAP.md` in each target. If
   nothing changed, the run is skipped (`skipped_precheck`) in seconds.
3. If any target is stale, the automation agent calls `basemind daemon ensure` then
   `codemap fleet`, which rewrites only changed docs and reports
   `SUMMARY refreshed=… fresh=… skipped=…`.
4. Writing the doc triggers another basemind rescan, so the generated file is itself
   searchable through `basemind docs search`.

A full fleet pass depends on the number and size of that profile's worktrees; steady-state
checks are quick when no source fingerprint has changed.

## Operating it

```sh
codemap generate --root /path/to/repo   # refresh one repo now
codemap stale --root /path/to/repo      # exit 0 = needs refresh, 1 = fresh
codemap targets                         # the git roots Orca knows for this profile
codemap fleet                           # refresh all targets
codemap fleet-stale                     # exit 0 if any target needs refresh

orca automations list --json
orca automations runs --id <id> --json
orca automations run <id>               # manual run; bypasses the precheck
orca automations edit <id> --disabled   # pause the loop
```

To change cadence, run `setup-codemap --apply` after editing `SCHEDULE` in
`scripts/setup-codemap-user.sh`, or edit the automation directly:
`orca automations edit <id> --trigger '30 * * * *'`.

## Troubleshooting

- **`codemap: unknown subcommand: --help`** — an older installed copy; rerun
  `setup-codemap --apply` (it reinstalls both commands from the repo).
- **`basemind overview failed`** — the index for that worktree is empty. Run
  `basemind --root <repo> rescan` (not `scan`, which fights the daemon's lock);
  `codemap generate` attempts this automatically.
- **`basemind` reports 0.33.x after an npm install** — an older `~/.local/bin/basemind`
  shadows the npm-global one. Replace it with a symlink to
  `~/.npm-global/bin/basemind` (back up the old file first).
- **Automation skipped every run** — expected when no source changed. Use
  `orca automations run <id>` to force one.
- **`setup-codemap` fails with `run this from inside a git worktree`** — start in an Orca
  worktree; the automation is bound to that workspace.
- **Agents don't see basemind** — run `agent-harness bootstrap --check` / `--apply` for
  that profile; the harness merges the MCP entry into each agent's user-global config.
