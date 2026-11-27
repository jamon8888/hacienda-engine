"""Read-only ``doctor`` health report.

An absent *optional* agent CLI is reported as ``not_installed`` and never
fails the doctor.  Only the required tools (basemind, Orca, Python) can make
the overall status ``fail``.
"""

from __future__ import annotations

import sys
from typing import TYPE_CHECKING

from .agents import AGENTS, FALLBACK_MODE, detect_cli, resolve_config_path
from .merge import ConfigError, entry_satisfied, loads_json_config, parse_toml, toml_lookup
from .version import (
    BASEMIND_EXACT,
    BASEMIND_EXACT_STR,
    ORCA_MIN,
    ORCA_MIN_STR,
    __version__,
)

if TYPE_CHECKING:  # pragma: no cover
    from .runners import Runner


SCHEMA = "agent-harness/doctor/1"


def parse_version(text: str) -> tuple[int, ...] | None:
    """Extract a dotted numeric version from free-form CLI output."""
    import re

    match = re.search(r"(\d+)\.(\d+)(?:\.(\d+))?", text)
    if not match:
        return None
    return tuple(int(part) if part else 0 for part in match.groups())


def _pad(value: tuple[int, ...], length: int) -> tuple[int, ...]:
    return value + (0,) * (length - len(value))


def _version_check(
    runner: "Runner",
    command: list[str],
    *,
    exact: tuple[int, ...] | None = None,
    minimum: tuple[int, ...] | None = None,
) -> dict:
    code, out, err = runner.run(command)
    combined = f"{out}\n{err}".strip()
    binary = command[0]
    if code in (127, 124, 125):
        return {"status": "missing", "version": None, "detail": combined or f"{binary} unavailable"}
    if code != 0:
        return {"status": "error", "version": None, "detail": combined or f"{binary} exited {code}"}
    version = parse_version(combined)
    if version is None:
        return {"status": "error", "version": None, "detail": f"could not parse {binary} version: {combined[:200]}"}
    result: dict = {"status": "ok", "version": ".".join(str(p) for p in version)}
    target = exact or minimum or ()
    if _pad(version, len(target)) < _pad(target, len(target)):
        result["status"] = "version_mismatch"
    if exact is not None and _pad(version, len(exact)) != _pad(exact, len(exact)):
        result["status"] = "version_mismatch"
        result["detail"] = f"validated against basemind {BASEMIND_EXACT_STR}"
    return result


def _integration(spec, basemind_available: bool) -> dict:
    """Report the agent's MCP integration status without writing anything."""
    base = {"mode": spec.mode}
    if spec.mode == FALLBACK_MODE:
        status = "ok" if basemind_available else "not_installed"
        return {
            **base,
            "status": status,
            "detail": spec.cli_note or "Uses the basemind CLI from PATH.",
        }

    path = resolve_config_path(spec)
    base["config_path"] = str(path) if path else None
    if path is None or not path.exists():
        return {**base, "status": "not_configured", "detail": "config file not present yet"}

    try:
        text = path.read_text(encoding="utf-8")
        if spec.config_kind == "toml":
            current = toml_lookup(parse_toml(text), spec.config_key)
        else:
            data = loads_json_config(text, jsonc=(spec.config_kind == "jsonc"))
            container = data.get(spec.config_key)
            current = container.get("basemind") if isinstance(container, dict) else None
    except (OSError, UnicodeError, ConfigError) as exc:
        return {**base, "status": "config_error", "detail": str(exc)}

    if entry_satisfied(current, spec.desired_entry):
        return {**base, "status": "ok", "detail": "basemind MCP entry present"}
    return {**base, "status": "needs_bootstrap", "detail": "basemind MCP entry missing or incorrect"}


def doctor(runner: "Runner") -> dict:
    """Build the full read-only doctor report."""
    basemind = _version_check(runner, ["basemind", "--version"], exact=BASEMIND_EXACT)
    orca = _version_check(runner, ["orca", "--version"], minimum=ORCA_MIN)
    python_version = sys.version_info[:3]
    python = {
        "status": "ok" if python_version >= (3, 11) else "version_mismatch",
        "version": ".".join(str(part) for part in python_version),
        "detail": "stdlib tomllib requires Python >= 3.11",
    }

    required = {"basemind": basemind, "orca": orca, "python": python}
    basemind_available = basemind["status"] in ("ok", "version_mismatch")

    agents = []
    counts = {"ok": 0, "not_installed": 0, "needs_bootstrap": 0, "warnings": 0, "failures": 0}
    for spec in AGENTS:
        cli = detect_cli(spec, runner)
        integration = _integration(spec, basemind_available)
        agents.append(
            {
                "id": spec.id,
                "label": spec.label,
                "cli": cli,
                "integration": integration,
            }
        )
        if cli["status"] == "not_installed":
            counts["not_installed"] += 1
        if integration["status"] == "ok":
            counts["ok"] += 1
        elif integration["status"] == "needs_bootstrap":
            counts["needs_bootstrap"] += 1
        elif integration["status"] in ("config_error", "not_installed"):
            counts["warnings"] += 1

    for item in required.values():
        if item["status"] in ("missing", "error"):
            counts["failures"] += 1
        elif item["status"] == "version_mismatch":
            counts["warnings"] += 1

    if counts["failures"]:
        overall = "fail"
    elif counts["warnings"]:
        overall = "degraded"
    else:
        overall = "ok"

    return {
        "schema": SCHEMA,
        "harness_version": __version__,
        "status": overall,
        "required": {
            **required,
            "basemind": {**basemind, "expected": BASEMIND_EXACT_STR},
            "orca": {**orca, "minimum": ORCA_MIN_STR},
        },
        "agents": agents,
        "summary": counts,
        "note": "Absent optional agent CLIs are reported as not_installed and do not fail the doctor.",
    }


def doctor_exit_code(report: dict) -> int:
    """Return 0 for ok/degraded, 1 for fail."""
    return 1 if report["status"] == "fail" else 0
