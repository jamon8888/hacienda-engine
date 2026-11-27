"""``self-test --agent <id>``: live proof or an actionable ``unsupported``.

Static detection (binary on PATH, config entry present) is collected but is
*explicitly not proof*.  A live prover is only available when one is supplied
in-process (tests) or via ``AGENT_HARNESS_SELFTEST_PROVER``.  Without live
proof the command returns ``unsupported`` and exits non-zero, with concrete
manual steps for the requested agent.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
from typing import TYPE_CHECKING, Callable

from .agents import detect_cli, get_agent
from .health import _integration
from .version import __version__

if TYPE_CHECKING:  # pragma: no cover
    from .runners import Runner


SCHEMA = "agent-harness/selftest/1"

Prover = Callable[[str], dict]


def _subprocess_prover(path: str, runner: "Runner") -> Prover:
    """Build a prover that executes ``path <agent>`` and expects JSON."""

    def prove(agent_id: str) -> dict:
        code, out, err = runner.run([path, agent_id], timeout=120)
        if code != 0:
            return {"proven": False, "error": err.strip() or f"prover exited {code}"}
        try:
            payload = json.loads(out)
        except json.JSONDecodeError as exc:
            return {"proven": False, "error": f"prover did not return JSON: {exc}"}
        return payload if isinstance(payload, dict) else {"proven": False, "error": "prover JSON is not an object"}

    return prove


def _prover_from_env(runner: "Runner") -> Prover | None:
    configured = os.environ.get("AGENT_HARNESS_SELFTEST_PROVER")
    if not configured:
        return None
    if not Path(configured).exists():
        return lambda agent_id: {"proven": False, "error": f"prover not found: {configured}"}
    return _subprocess_prover(configured, runner)


def _static_detection(spec, runner: "Runner", basemind_available: bool) -> dict:
    """Collect static signals, clearly labelled as non-proof."""
    return {
        "cli": detect_cli(spec, runner),
        "integration": _integration(spec, basemind_available),
        "is_proof": False,
        "note": "Static detection only; it must not be treated as live proof.",
    }


def run_self_test(agent_id: str, runner: "Runner", prover: Prover | None = None) -> dict:
    """Run the self-test for ``agent_id``."""
    spec = get_agent(agent_id)  # raises ValueError for an unknown agent

    code, out, err = runner.run(["basemind", "--version"])
    basemind_available = code == 0
    static = _static_detection(spec, runner, basemind_available)

    if prover is None:
        prover = _prover_from_env(runner)

    proof: dict | None = None
    if prover is not None:
        try:
            proof = prover(agent_id)
        except Exception as exc:  # a broken prover is not proof either
            proof = {"proven": False, "error": f"prover raised {type(exc).__name__}: {exc}"}

    proven = bool(proof and proof.get("proven") is True)
    if proven:
        status = "ok"
        message = f"Live proof for {spec.label} succeeded."
    else:
        status = "unsupported"
        reason = ""
        if proof is not None:
            reason = f" Prover result: {json.dumps(proof, ensure_ascii=False)}."
        elif prover is None:
            reason = " No live prover is configured in this environment."
        message = (
            f"Live agent-specific proof for {spec.label} is not available.{reason} "
            "Static detection (binary/config presence) is explicitly NOT proof. "
            "Complete the manual steps below in an indexed Orca worktree."
        )

    return {
        "schema": SCHEMA,
        "harness_version": __version__,
        "agent": spec.id,
        "label": spec.label,
        "mode": spec.mode,
        "status": status,
        "static_detection": static,
        "static_detection_is_proof": False,
        "live_proof": proof,
        "manual_steps": list(spec.manual_steps),
        "message": message,
    }


def selftest_exit_code(report: dict) -> int:
    """Return 0 only for a real live proof; otherwise a non-zero status."""
    return 0 if report["status"] == "ok" else 3
