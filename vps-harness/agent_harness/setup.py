"""``setup --root <repo>``: validate repo index and tools, and fail closed.

No global configuration is touched here.  The command only verifies that a
repository is a git worktree, that basemind is installed with the full tool
set this harness relies on, and that basemind reports a non-empty index for
that specific root.
"""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import TYPE_CHECKING

from .version import __version__

if TYPE_CHECKING:  # pragma: no cover
    from .runners import Runner


SCHEMA = "agent-harness/setup/1"

REQUIRED_COMMANDS = ("admin", "code", "docs", "git", "memory", "overview", "status")


def _check(name: str, ok: bool, detail: str) -> dict:
    return {"name": name, "status": "ok" if ok else "fail", "detail": detail}


def setup_repo(root: str, runner: "Runner") -> dict:
    """Validate ``root`` and return a structured report (never writes)."""
    try:
        resolved = Path(root).expanduser().resolve(strict=True)
    except (OSError, RuntimeError):
        resolved = Path(root).expanduser()
    checks: list[dict] = []

    if resolved.is_dir():
        checks.append(_check("root_exists", True, str(resolved)))
    else:
        checks.append(_check("root_exists", False, f"not a directory: {resolved}"))

    code, out, err = runner.run(["git", "-C", str(resolved), "rev-parse", "--is-inside-work-tree"])
    is_git = code == 0 and out.strip() == "true"
    checks.append(_check("git_worktree", is_git, "git worktree detected" if is_git else (err.strip() or out.strip() or "not a git worktree")))

    code, out, err = runner.run(["basemind", "--version"])
    basemind_ok = code == 0
    checks.append(_check("basemind_installed", basemind_ok, (out or err).strip() or "basemind unavailable"))

    code, out, err = runner.run(["basemind", "--help"])
    help_text = f"{out}\n{err}"
    missing_commands = sorted(
        command for command in REQUIRED_COMMANDS if not re.search(rf"\b{command}\b", help_text)
    )
    tools_ok = code == 0 and not missing_commands
    checks.append(
        _check(
            "basemind_commands",
            tools_ok,
            "required CLI commands present" if tools_ok else f"missing commands: {', '.join(missing_commands) or 'none (basemind --help failed)'}",
        )
    )

    code, out, err = runner.run(["basemind", "--root", str(resolved), "status", "--json"])
    file_count = None
    index_detail = (err or out).strip() or "no status output"
    if code == 0:
        try:
            status = json.loads(out)
            file_count = status.get("file_count")
            index_detail = f"file_count={file_count}"
        except (json.JSONDecodeError, AttributeError) as exc:
            index_detail = f"could not parse basemind status: {exc}"
    index_ok = (
        isinstance(file_count, int)
        and not isinstance(file_count, bool)
        and file_count > 0
    )
    checks.append(_check("basemind_index", index_ok, index_detail))

    failed = [check for check in checks if check["status"] == "fail"]
    return {
        "schema": SCHEMA,
        "harness_version": __version__,
        "root": str(resolved),
        "status": "ok" if not failed else "fail",
        "fail_closed": True,
        "checks": checks,
        "summary": {"passed": len(checks) - len(failed), "failed": len(failed)},
        "note": "setup is read-only and fails closed: any failed check exits non-zero.",
    }


def setup_exit_code(report: dict) -> int:
    """Return 0 only when every check passed."""
    return 0 if report["status"] == "ok" else 1
