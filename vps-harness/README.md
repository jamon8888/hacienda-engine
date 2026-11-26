# Shared user-level VPS agent harness

Stdlib-only Python harness intended for installation under
`~/.local/share/agent-harness`. It wires the basemind MCP server into the
user-level config of six supported agents and
reports the state of the VPS without ever mutating config during inspection.

## Commands

```sh
agent-harness doctor --json                 # read-only health report (JSON)
agent-harness bootstrap --check --json      # read-only config drift report
agent-harness bootstrap --apply --json      # narrow merge + backup, idempotent
agent-harness setup --root /path/to/repo    # validate repo index/tools, fail closed
agent-harness self-test --agent claude      # live proof or actionable `unsupported`
agent-harness install                       # install versioned package + launchers
agent-harness agentic rules --json          # strict Rust/TypeScript rules for prompts
agent-harness agentic plan --spec-file s.json --json   # isolated task plan
agent-harness agentic compact --kind cargo --input out.txt --json
agent-harness agentic gatekeeper --root /repo          # verify_changes() gate
```

Run from this source tree with `bin/agent-harness <command>` or
`python3 -m agent_harness <command>` (the latter needs `vps-harness/` on
`PYTHONPATH`).

## Agents

| id | mode | native config (user-global) |
|---|---|---|
| `claude` | native MCP | `~/.claude.json` → `mcpServers.basemind` |
| `claude-teams` | native MCP | same `~/.claude.json` (Claude Agent Teams) |
| `codex` | native MCP | `~/.codex/config.toml` → `[mcp_servers.basemind]` |
| `opencode` | native MCP | `$XDG_CONFIG_HOME/opencode/opencode.json(c)` → `mcp.basemind` |
| `opencode2` | native MCP | `$XDG_CONFIG_HOME/opencode2/opencode.json(c)` → `mcp.basemind` |
| `pi` | CLI fallback | none; calls `basemind` from `PATH` |

## Behaviour guarantees

* `doctor` checks basemind 0.34.0, Orca >= 1.4.263 and the six CLIs. An absent
  optional CLI is `not_installed` and never fails the doctor.
* `bootstrap --check` performs no writes. `--apply` merges only the managed
  entry, writes a timestamped backup before each write, and re-verifies.
  JSON/JSONC configs are re-serialized when a change is needed (so JSONC
  comments are normalized); the untouched original is always kept in the
  backup, and an already-correct config is never rewritten.
* `setup --root` is read-only and fails closed when the repo is not a git
  worktree, basemind lacks the full tool set, or the index is empty.
* `self-test` reports `unsupported` (non-zero) unless an actual live prover
  supplies proof. Static detection is explicitly labelled as non-proof.
* `install` copies a versioned package to the data dir, repoints `current`, and
  symlinks `~/.local/bin/agent-harness`. No sudo; re-running is idempotent.

## Agentic planning helpers (0.2.0)

`agent-harness agentic` emits prompts, plan data, compact diagnostics, and a
gatekeeper command. It does not launch or coordinate agents; an external
orchestrator must assign roles and enforce write boundaries. The gatekeeper
runs repository checks, but prompts and role declarations alone are not access
controls:

* a plan describing worktree isolation, shared build caches, and separate test-author / implementer roles;
* Rust/TypeScript rules injected into both prompts (`unwrap`/`expect`/`panic`/
  `unsafe` and `any`/`@ts-ignore`/non-null banned; property-based tests required);
* a generic gatekeeper: static checks and unit tests, followed by diff-targeted
  mutation commands when a diff exists. Its language tools must be installed; use
  the root `Taskfile.yml` and package scripts as the authoritative Hacienda gate.
* a circuit breaker (5 correction loops, reflection after two identical errors,
  escalation afterwards) and JSON log compaction down to file/line/mutant.

All 91 unit tests are offline. `bin/gatekeeper --print-plan` shows the exact stage
commands without running them.

## Tests

```sh
cd vps-harness && python3 -m unittest discover -s tests -t . -v
```

All tests are offline: external tools are replaced with an in-memory runner
and every path is redirected to a temporary HOME/XDG tree.
