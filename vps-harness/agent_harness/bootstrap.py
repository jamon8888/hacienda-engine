"""Narrow, idempotent ``bootstrap --check|--apply`` of agent MCP configs.

``--check`` performs no writes and reports exactly what would change.
``--apply`` merges only the managed basemind entry, writes a timestamped
backup first, and re-verifies afterwards.  Agents whose optional CLI is
absent are reported as ``not_installed`` and never cause a failure.
"""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING

from .agents import AGENTS, FALLBACK_MODE, detect_cli, resolve_config_path
from .merge import ConfigError, backup_and_write, merge_json_config, merge_toml_config, sha256_file
from .version import __version__

if TYPE_CHECKING:  # pragma: no cover
    from .runners import Runner


SCHEMA = "agent-harness/bootstrap/1"


def _desired_text(spec, text: str) -> str | None:
    """Return the merged text for ``spec``, or ``None`` if already correct."""
    if spec.config_kind == "toml":
        return merge_toml_config(text, spec.config_key, spec.desired_entry)
    return merge_json_config(
        text,
        spec.config_key,
        spec.desired_entry,
        jsonc=(spec.config_kind == "jsonc"),
    )


def plan_agent(spec, runner: "Runner") -> dict:
    """Compute a read-only plan for one agent."""
    cli = detect_cli(spec, runner)
    result: dict = {
        "id": spec.id,
        "label": spec.label,
        "mode": spec.mode,
        "cli": cli,
    }

    if spec.mode == FALLBACK_MODE:
        result.update(
            {
                "status": "cli_fallback",
                "config_path": None,
                "change": False,
                "detail": spec.cli_note or "No native MCP config; uses basemind CLI.",
            }
        )
        return result

    path = resolve_config_path(spec)
    result["config_path"] = str(path) if path else None
    if path is None or not path.exists():
        if cli["status"] == "not_installed":
            result.update(
                {
                    "status": "skipped_not_installed",
                    "change": False,
                    "detail": "CLI absent and no existing config; nothing to edit.",
                }
            )
        else:
            result.update(
                {
                    "status": "config_missing",
                    "change": bool(path),
                    "detail": "Config does not exist; --apply would create it with only the basemind entry.",
                }
            )
            if path:
                result["_new_text"] = _desired_text(spec, "{}" if spec.config_kind != "toml" else "")
        return result

    try:
        text = path.read_text(encoding="utf-8")
        new_text = _desired_text(spec, text)
    except (OSError, UnicodeError, ConfigError) as exc:
        result.update({"status": "config_error", "change": False, "detail": str(exc)})
        return result

    if new_text is None:
        result.update(
            {"status": "already_configured", "change": False, "detail": "basemind MCP entry already present"}
        )
    else:
        result.update(
            {"status": "needs_change", "change": True, "detail": "basemind MCP entry will be added/updated"}
        )
        result["_new_text"] = new_text
        result["_source_hash"] = sha256_file(path)
    return result


def verify_agent(spec) -> bool:
    """Re-read an agent config and confirm the desired entry is present."""
    if spec.mode == FALLBACK_MODE:
        return True
    from .merge import entry_satisfied, loads_json_config, parse_toml, toml_lookup

    path = resolve_config_path(spec)
    if path is None or not path.exists():
        return False
    try:
        text = path.read_text(encoding="utf-8")
        if spec.config_kind == "toml":
            current = toml_lookup(parse_toml(text), spec.config_key)
        else:
            data = loads_json_config(text, jsonc=(spec.config_kind == "jsonc"))
            container = data.get(spec.config_key)
            current = container.get("basemind") if isinstance(container, dict) else None
    except (OSError, UnicodeError, ConfigError):
        return False
    return entry_satisfied(current, spec.desired_entry)


def bootstrap(mode: str, runner: "Runner") -> dict:
    """Run ``bootstrap`` in ``check`` or ``apply`` mode."""
    if mode not in ("check", "apply"):
        raise ValueError(f"invalid bootstrap mode {mode!r}")

    agents: list[dict] = []
    counts = {"applied": 0, "unchanged": 0, "skipped": 0, "errors": 0}

    for spec in AGENTS:
        plan = plan_agent(spec, runner)
        new_text = plan.pop("_new_text", None)
        source_hash = plan.pop("_source_hash", None)

        if plan["status"] == "config_error":
            counts["errors"] += 1
            agents.append(plan)
            continue

        if mode == "apply" and new_text is not None:
            config_path = Path(plan["config_path"])
            if (
                source_hash is not None
                and config_path.exists()
                and sha256_file(config_path) != source_hash
            ):
                plan["status"] = "conflict"
                plan["detail"] = "config changed after planning; rerun --apply to replan"
                counts["errors"] += 1
                agents.append(plan)
                continue
            try:
                backup = backup_and_write(config_path, new_text)
                plan["backup"] = backup
                plan["verified"] = verify_agent(spec)
                plan["status"] = "applied" if plan["verified"] else "write_failed"
                counts["applied" if plan["verified"] else "errors"] += 1
            except (OSError, ConfigError) as exc:
                plan["status"] = "write_failed"
                plan["detail"] = str(exc)
                counts["errors"] += 1
        elif mode == "apply":
            plan["verified"] = verify_agent(spec) if spec.mode != FALLBACK_MODE else True
            counts["unchanged"] += 1
        elif plan["change"]:
            counts["skipped"] += 1
        else:
            counts["unchanged"] += 1

        agents.append(plan)

    return {
        "schema": SCHEMA,
        "harness_version": __version__,
        "mode": mode,
        "read_only": mode == "check",
        "status": "error" if counts["errors"] else "ok",
        "agents": agents,
        "summary": counts,
        "note": "check is read-only; apply backs up before each write and is idempotent.",
    }


def bootstrap_exit_code(report: dict) -> int:
    """Return 0 when no drift remains, 1 when ``--check`` found work."""
    if report["status"] == "error":
        return 2
    if report["mode"] == "check":
        return 1 if report["summary"]["skipped"] else 0
    return 0 if report["status"] == "ok" else 2
