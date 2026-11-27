"""Hardened agentic orchestration helpers (standard library only).

This module never launches a process.  It produces the strict rules, the two
role-separated prompts, a diff-targeted gatekeeper plan, a circuit-breaker state
machine and compacted diagnostics.  Keeping all of it as pure data makes the
whole pipeline unit-testable offline with the existing ``FakeRunner`` style.
"""

from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass, field
from typing import Any

PLAN_SCHEMA = "agent-harness/agentic-plan/1"
COMPACT_SCHEMA = "agent-harness/agentic-compact/1"

STRICT_RULES = """# RÈGLES D'INGÉNIERIE STRICTES (RUST & TYPESCRIPT)

## Interdictions absolues
- RUST : interdiction stricte de `unwrap()`, `expect()` (hors tests), `panic!()`,
  `unsafe` ou des `#[allow(...)]` génériques. Toute erreur doit être typée
  (`thiserror` / `anyhow`).
- TYPESCRIPT : interdiction de `any`, `@ts-ignore`, `@ts-expect-error` et des
  assertions non-null (`!`). Activer `noUncheckedIndexedAccess`.
- TESTS : interdiction de modifier les assertions d'un test existant pour le faire
  passer, ou d'écrire des assertions triviales (`expect(res).toBeDefined()`).

## Exigences de tests durcis
1. Chaque fonction publique doit avoir des tests nominaux et des cas limites
   (valeurs nulles, vides, bornes min/max, concurrence), et au moins un test
   property-based vérifiant un invariant métier sur des entrées aléatoires
   (`proptest` en Rust, `fast-check` en TypeScript).
2. Les mocks doivent être strictement typés et limités aux frontières I/O
   (réseau, base de données).

## Boucle de vérification
L'agent n'a pas le droit de déclarer la tâche terminée tant que `run_gatekeeper`
n'a pas retourné `EXIT_CODE = 0` avec le rapport complet. En cas d'échec, lire le
rapport compacté (fichier, ligne, mutant survivant), corriger, puis relancer.
"""

TOOLS_SCHEMA: dict[str, Any] = {
    "read_file": {
        "description": "Read a file at a 1-based inclusive line span.",
        "parameters": {"path": "string", "start_line": "integer?", "end_line": "integer?"},
    },
    "apply_patch": {
        "description": "Apply a unified diff; paths matching the protected test set are rejected.",
        "parameters": {"patch": "string"},
    },
    "run_gatekeeper": {
        "description": "verify_changes(): run static checks, unit/property tests and diff-targeted mutation. Returns EXIT_CODE and a compact report.",
        "parameters": {"root": "string?"},
        "returns": {"EXIT_CODE": "integer", "stages": "array<{stage, exit_code}>", "findings": "array"},
    },
}


@dataclass(frozen=True)
class TaskSpec:
    """A single isolated task. Tests paths are read-only for the implementer."""

    name: str
    root: str = "."
    rust: bool = True
    typescript: bool = False
    tests_paths: tuple[str, ...] = ("tests/", "*.test.ts", "*.spec.ts")
    max_iterations: int = 5
    repeat_threshold: int = 2

    def __post_init__(self) -> None:
        if not self.name.strip():
            raise ValueError("task name must not be empty")
        if not self.rust and not self.typescript:
            raise ValueError("at least one of rust/typescript must be enabled")
        if not self.tests_paths:
            raise ValueError("tests_paths must not be empty")
        if self.max_iterations < 1:
            raise ValueError("max_iterations must be >= 1")
        if self.repeat_threshold < 1:
            raise ValueError("repeat_threshold must be >= 1")


def is_protected(path: str, spec: TaskSpec) -> bool:
    """Return True when *path* belongs to the test-only protected set."""
    normalized = path.replace("\\", "/").lstrip("./")
    parts = set(normalized.split("/"))
    for raw in spec.tests_paths:
        pattern = raw.replace("\\", "/").lstrip("./")
        bare = pattern.rstrip("/")
        if pattern.startswith("*") and normalized.endswith(pattern.lstrip("*")):
            return True
        if pattern.endswith("/") and normalized.startswith(pattern):
            return True
        if normalized == bare or bare in parts:
            return True
    return False


def build_prompts(spec: TaskSpec) -> dict[str, str]:
    """Return the two role-separated prompts. Neither role writes the other's files."""
    protected = ", ".join(spec.tests_paths)
    shared = (
        f"{STRICT_RULES}\n"
        f"Task: {spec.name}\n"
        f"Repository root: {spec.root}\n"
        "You must call run_gatekeeper and obtain EXIT_CODE = 0 before reporting completion.\n"
    )
    test_agent = shared + (
        "ROLE: TEST AUTHOR (agent A).\n"
        f"You may ONLY create or modify files matching: {protected}.\n"
        "Write nominal, edge-case and property-based tests for every public function "
        "using proptest (Rust) / fast-check (TypeScript). Never weaken an existing "
        "assertion and never write trivial existence assertions.\n"
        "Do not modify implementation files. Run the gatekeeper to confirm the tests fail "
        "against the unimplemented code, then stop.\n"
    )
    impl_agent = shared + (
        "ROLE: IMPLEMENTER (agent B).\n"
        f"FORBIDDEN: you must not create, edit or delete any file matching: {protected}.\n"
        "Implement only the production code. Typed errors, no unwrap/expect/panic/unsafe, "
        "no any/@ts-ignore/non-null assertions.\n"
        "If a gatekeeper failure points at a test, report it instead of editing the test.\n"
    )
    return {"test_agent": test_agent, "impl_agent": impl_agent}


ANSI_RE = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
NUMBER_RE = re.compile(r"\b\d+\b")


def error_fingerprint(error: str) -> str:
    """Stable identity of an error: ignores case, whitespace and line numbers."""
    text = ANSI_RE.sub("", error)
    text = " ".join(text.lower().split())
    text = NUMBER_RE.sub("#", text)
    return hashlib.sha256(text.encode("utf-8")).hexdigest()[:16]


@dataclass
class CircuitBreaker:
    """Budget + repeated-error breaker for the correction loop."""

    max_iterations: int = 5
    repeat_threshold: int = 2
    failures: list[str] = field(default_factory=list)
    reflected: bool = False
    state: str = "running"

    def __post_init__(self) -> None:
        if self.max_iterations < 1 or self.repeat_threshold < 1:
            raise ValueError("budgets must be >= 1")

    def record_failure(self, error: str) -> dict[str, Any]:
        """Record a failure and return the next allowed action."""
        if self.state in ("escalate", "done"):
            return self.next_action()
        fingerprint = error_fingerprint(error)
        self.failures.append(fingerprint)
        repeats = 0
        for previous in reversed(self.failures[:-1]):
            if previous != fingerprint:
                break
            repeats += 1
        if len(self.failures) >= self.max_iterations:
            self.state = "escalate"
        elif repeats + 1 >= self.repeat_threshold:
            if self.reflected:
                self.state = "escalate"
            else:
                self.state = "reflect"
                self.reflected = True
        else:
            self.state = "running"
        return self.next_action()

    def record_success(self) -> dict[str, Any]:
        """Mark the task settled; no further corrections are allowed."""
        self.state = "done"
        return self.next_action()

    def next_action(self) -> dict[str, Any]:
        actions = {
            "running": "retry_with_fix",
            "reflect": "reflect_without_code",
            "escalate": "escalate_to_stronger_model_or_human",
            "done": "finish",
        }
        return {
            "state": self.state,
            "iteration": len(self.failures),
            "max_iterations": self.max_iterations,
            "action": actions[self.state],
        }


def _findings_from_json_lines(raw: str) -> list[dict[str, Any]]:
    findings: list[dict[str, Any]] = []
    for line in raw.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            obj = json.loads(line)
        except (ValueError, TypeError):
            continue
        message = obj.get("message")
        if isinstance(message, dict):
            spans = message.get("spans") or []
            first = next((s for s in spans if s.get("is_primary")), spans[0] if spans else {})
            findings.append(
                {
                    "file": first.get("file_name"),
                    "line": first.get("line_start"),
                    "message": str(message.get("message") or message.get("rendered") or "")[:400],
                }
            )
        elif "mutant" in obj or "outcome" in obj:
            mutant = obj.get("mutant") or {}
            findings.append(
                {
                    "file": mutant.get("file") if isinstance(mutant, dict) else None,
                    "line": mutant.get("line") if isinstance(mutant, dict) else None,
                    "message": str(obj.get("outcome") or obj.get("mutant"))[:400],
                }
            )
    return findings


def compact_output(raw: str, kind: str, *, limit: int = 20) -> dict[str, Any]:
    """Reduce noisy tool output to file/line/message findings for the agent."""
    if kind not in {"cargo", "vitest", "stryker", "mutants"}:
        raise ValueError(f"unknown output kind: {kind}")
    findings: list[dict[str, Any]] = []
    try:
        parsed = json.loads(raw)
    except (ValueError, TypeError):
        parsed = None

    if parsed is not None:
        if kind == "vitest" and isinstance(parsed, dict):
            for result in parsed.get("testResults", []):
                for failure in result.get("assertionResults", []):
                    messages = failure.get("failureMessages") or []
                    findings.append(
                        {
                            "file": result.get("name"),
                            "line": None,
                            "message": str(messages[0] if messages else failure.get("fullName", ""))[:400],
                        }
                    )
        elif kind == "stryker" and isinstance(parsed, dict):
            for mutant in parsed.get("mutants", []):
                location = mutant.get("location") or {}
                start = location.get("start") or {}
                findings.append(
                    {
                        "file": mutant.get("fileName"),
                        "line": start.get("line"),
                        "message": f"mutant {mutant.get('status', 'unknown')}",
                    }
                )
    if not findings:
        findings = _findings_from_json_lines(raw)
    if not findings:
        findings = [
            {"file": None, "line": None, "message": line.strip()[:400]}
            for line in raw.splitlines()
            if line.strip()
        ]
    findings = findings[:limit]
    text = "\n".join(
        f"{item['file'] or '<unknown>'}:{item['line'] or '-'}: {item['message']}" for item in findings
    )
    return {
        "schema": COMPACT_SCHEMA,
        "kind": kind,
        "count": len(findings),
        "findings": findings,
        "text": text,
    }


def gatekeeper_steps(spec: TaskSpec) -> list[dict[str, str]]:
    """The four verification stages; mutation is always restricted to the diff."""
    steps: list[dict[str, str]] = []
    if spec.rust:
        steps.extend(
            [
                {"name": "static-rust", "scope": "all", "command": "cargo fmt --check"},
                {
                    "name": "lint-rust",
                    "scope": "all",
                    "command": "cargo clippy --workspace --all-targets --all-features -- -D warnings",
                },
                {"name": "unit-rust", "scope": "all", "command": "cargo nextest run --workspace"},
                {
                    "name": "mutation-rust",
                    "scope": "diff",
                    "command": "cargo mutants --in-diff git-diff.patch --minimum-test-suite-success-rate 85",
                },
            ]
        )
    if spec.typescript:
        steps.extend(
            [
                {"name": "static-ts", "scope": "all", "command": "pnpm exec tsc --noEmit"},
                {"name": "lint-ts", "scope": "all", "command": "pnpm exec eslint . --max-warnings 0"},
                {"name": "unit-ts", "scope": "all", "command": "pnpm exec vitest run --bail 1"},
                {
                    "name": "mutation-ts",
                    "scope": "diff",
                    "command": (
                        "pnpm exec stryker run --mutate "
                        "\"$(git diff --name-only HEAD | grep -E '\\.tsx?$' | paste -sd, -)\""
                    ),
                },
            ]
        )
    return steps


def plan(spec: TaskSpec) -> dict[str, Any]:
    """Build the full orchestration plan for one isolated task."""
    breaker = CircuitBreaker(
        max_iterations=spec.max_iterations, repeat_threshold=spec.repeat_threshold
    )
    return {
        "schema": PLAN_SCHEMA,
        "task": {
            "name": spec.name,
            "root": spec.root,
            "rust": spec.rust,
            "typescript": spec.typescript,
            "protected_tests_paths": list(spec.tests_paths),
        },
        "isolation": {
            "worktree_per_task": True,
            "shared_build_cache": ["sccache", "pnpm-store"],
            "roles": ["test_agent", "impl_agent"],
            "same_agent_writes_code_and_tests": False,
        },
        "prompts": build_prompts(spec),
        "gatekeeper": {
            "command": "agent-harness agentic gatekeeper --root <worktree>",
            "exit_code_required": 0,
            "steps": gatekeeper_steps(spec),
        },
        "circuit_breaker": breaker.next_action(),
        "tools": TOOLS_SCHEMA,
        "rules_sha256": hashlib.sha256(STRICT_RULES.encode("utf-8")).hexdigest(),
        "log_compaction": {
            "parsers": sorted({"cargo", "vitest", "stryker", "mutants"}),
            "max_findings": 20,
            "note": "Only file, line and exact failing message/mutant are returned to the agent.",
        },
    }
