# Claude Code — Hacienda Engine

Read [AGENTS.md](AGENTS.md) first for the product map, crate boundaries, security constraints, development commands, and documentation pointers. It is the shared source of truth for this repository; keep project-wide guidance there rather than copying it here.

## Tools in this environment

- Use **Basemind** for local Hacienda code, symbols/references, indexed documents, and Git history. Run `basemind status --json` if index-backed results look empty or stale. Its code-file count is not every file: ignored/generated or unsupported files may be skipped, while documents use a separate search tier.
- Use **Context7** for current upstream crate/framework/API documentation. Resolve the exact library ID before querying; include the version when the question depends on a particular version. Do not use Context7 to answer what this checkout implements.
- If either MCP service is unavailable, use its CLI where possible and report any missing evidence; do not treat an empty index as proof that a symbol or document is absent.

## Working in this checkout

- Check `docs/CODEMAP.md` for the generated high-level map. It is machine-generated and ignored by Git; refresh it with `codemap generate --root .` when stale.
- Check `git status --short` before editing. If working across linked worktrees, inspect each worktree's status independently; uncommitted files are not shared between them.
- Run focused tests for the changed package first. Use `task verify` for the repo gate when practical; its exact steps are in `Taskfile.yml`.
- Consult `docs/configuration.md` before touching config, API authentication, non-loopback binds, pseudonymisation, or key rotation. Consult `hacienda-api/README.md` before changing audit/API evidence contracts.

## Claude-specific setup

MCP servers and Context7 are configured per user or runtime, not by this repository. This repository has no `.claude/settings.json` hook configuration; do not assume Claude-specific hooks are installed. Use `basemind rescan <path>` when an edited file is missing or stale in the index.
