"""The six supported agents and their native MCP / CLI-fallback wiring.

Five agents have a *native* MCP integration in a user-global config file:

* Claude and Claude Agent Teams share ``~/.claude.json`` (``mcpServers``).
* Codex uses ``~/.codex/config.toml`` (``[mcp_servers.basemind]``).
* OpenCode uses ``$XDG_CONFIG_HOME/opencode/opencode.json`` (``mcp``).
* OpenCode 2 uses ``$XDG_CONFIG_HOME/opencode2/opencode.json`` (``mcp``).

Pi has no native MCP config in this setup; it is the *CLI fallback* and uses
the ``basemind`` executable directly from ``PATH``.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

from .paths import expand_template

if TYPE_CHECKING:  # pragma: no cover
    from .runners import Runner


# Desired basemind MCP entry, expressed in each agent's native config shape.
CLAUDE_MCP_ENTRY = {
    "type": "stdio",
    "command": "basemind",
    "args": ["serve"],
    "env": {},
}
CODEX_MCP_ENTRY = {"command": "basemind", "args": ["serve"]}
OPENCODE_MCP_ENTRY = {
    "type": "local",
    "command": ["basemind", "serve"],
    "enabled": True,
}

NATIVE_MODE = "native_mcp"
FALLBACK_MODE = "cli_fallback"


@dataclass(frozen=True)
class AgentSpec:
    """Static description of one supported agent."""

    id: str
    label: str
    binaries: tuple[str, ...]
    mode: str
    config_kind: str | None
    config_candidates: tuple[str, ...]
    config_key: str
    desired_entry: dict
    cli_note: str = ""
    manual_steps: tuple[str, ...] = ()


AGENTS: tuple[AgentSpec, ...] = (
    AgentSpec(
        id="claude",
        label="Claude",
        binaries=("claude",),
        mode=NATIVE_MODE,
        config_kind="json",
        config_candidates=("{home}/.claude.json",),
        config_key="mcpServers",
        desired_entry=CLAUDE_MCP_ENTRY,
        manual_steps=(
            "In an indexed Orca worktree, run: claude -p 'Use the basemind code outline tool to list this repo top-level symbols'",
            "Confirm an actual basemind MCP tool call and its result appear in the transcript (a config entry alone is not proof).",
        ),
    ),
    AgentSpec(
        id="claude-teams",
        label="Claude Agent Teams",
        binaries=("claude-teams", "claude"),
        mode=NATIVE_MODE,
        config_kind="json",
        config_candidates=("{home}/.claude.json",),
        config_key="mcpServers",
        desired_entry=CLAUDE_MCP_ENTRY,
        cli_note=(
            "No separate binary is required: Claude Agent Teams run on the claude CLI "
            "and require CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1."
        ),
        manual_steps=(
            "Set CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1 and spawn a teammate in an indexed worktree.",
            "Ask the teammate to query basemind, then confirm a real basemind MCP tool call and result in its transcript.",
        ),
    ),
    AgentSpec(
        id="codex",
        label="Codex",
        binaries=("codex",),
        mode=NATIVE_MODE,
        config_kind="toml",
        config_candidates=("{home}/.codex/config.toml",),
        config_key="mcp_servers.basemind",
        desired_entry=CODEX_MCP_ENTRY,
        manual_steps=(
            "In an indexed worktree, run: codex exec 'Use the basemind MCP tool to outline this repo'",
            "Confirm a real basemind MCP tool call and its result appear in the output.",
        ),
    ),
    AgentSpec(
        id="opencode",
        label="OpenCode",
        binaries=("opencode",),
        mode=NATIVE_MODE,
        config_kind="jsonc",
        config_candidates=(
            "{xdg}/opencode/opencode.json",
            "{xdg}/opencode/opencode.jsonc",
        ),
        config_key="mcp",
        desired_entry=OPENCODE_MCP_ENTRY,
        manual_steps=(
            "Start opencode in an indexed worktree and ask it to use basemind code search.",
            "Confirm a real basemind MCP tool call and result are shown in the session.",
        ),
    ),
    AgentSpec(
        id="opencode2",
        label="OpenCode 2",
        binaries=("opencode2",),
        mode=NATIVE_MODE,
        config_kind="jsonc",
        config_candidates=(
            "{xdg}/opencode2/opencode.json",
            "{xdg}/opencode2/opencode.jsonc",
        ),
        config_key="mcp",
        desired_entry=OPENCODE_MCP_ENTRY,
        manual_steps=(
            "Start opencode2 in an indexed worktree and ask it to use basemind code search.",
            "Confirm a real basemind MCP tool call and result are shown in the session.",
        ),
    ),
    AgentSpec(
        id="pi",
        label="Pi (CLI fallback)",
        binaries=("pi",),
        mode=FALLBACK_MODE,
        config_kind=None,
        config_candidates=(),
        config_key="",
        desired_entry={},
        cli_note="Pi has no native MCP config here; it calls the basemind CLI from PATH.",
        manual_steps=(
            "In an indexed worktree, run pi and ask it to call the basemind CLI (e.g. basemind code outline).",
            "Confirm the tool invocation and basemind output in the Pi session; PATH presence alone is not proof.",
        ),
    ),
)

AGENT_IDS = tuple(spec.id for spec in AGENTS)


def get_agent(agent_id: str) -> AgentSpec:
    """Return the spec for ``agent_id`` or raise ``ValueError``."""
    for spec in AGENTS:
        if spec.id == agent_id:
            return spec
    raise ValueError(
        f"unknown agent {agent_id!r}; expected one of: {', '.join(AGENT_IDS)}"
    )


def config_paths(spec: AgentSpec) -> list[Path]:
    """Return all candidate config paths for ``spec`` in priority order."""
    return [expand_template(template) for template in spec.config_candidates]


def resolve_config_path(spec: AgentSpec) -> Path | None:
    """Return the first existing config path, else the first candidate.

    ``None`` is returned for CLI-fallback agents, which own no config file.
    """
    paths = config_paths(spec)
    if not paths:
        return None
    for path in paths:
        if path.exists():
            return path
    return paths[0]


def detect_cli(spec: AgentSpec, runner: "Runner") -> dict:
    """Report CLI availability using the first binary found on PATH."""
    for name in spec.binaries:
        path = runner.which(name)
        if path:
            result = {
                "status": "installed",
                "binary": name,
                "path": path,
            }
            if spec.cli_note:
                result["note"] = spec.cli_note
            return result
    return {
        "status": "not_installed",
        "binary": spec.binaries[0],
        "path": None,
        **({"note": spec.cli_note} if spec.cli_note else {}),
    }
