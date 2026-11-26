"""Test doubles: an in-memory Runner and a temporary HOME/XDG environment."""

from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock


class FakeRunner:
    """Deterministic Runner: lookup tables for ``which`` and ``run``."""

    def __init__(
        self,
        *,
        which_map: dict[str, str] | None = None,
        programs: dict[str, object] | None = None,
    ) -> None:
        self.which_map = dict(which_map or {})
        self.programs = dict(programs or {})
        self.calls: list[list[str]] = []

    def which(self, name: str) -> str | None:
        return self.which_map.get(name)

    def run(self, argv: list[str], *, timeout: int = 15) -> tuple[int, str, str]:
        self.calls.append(list(argv))
        handler = self.programs.get(argv[0])
        if handler is None:
            return 127, "", f"command not found: {argv[0]}"
        if callable(handler):
            return handler(argv, timeout=timeout)
        return handler  # type: ignore[return-value]


def healthy_programs(basemind_version: str = "basemind 0.34.0") -> dict[str, object]:
    """Return the standard program map for a healthy VPS."""
    help_text = "\n".join(
        [
            "Commands:",
            "  admin  code  docs  git  memory  overview  status",
        ]
    )
    return {
        "basemind": lambda argv, **_: (
            (0, basemind_version + "\n", "")
            if argv[1:] == ["--version"]
            else (0, help_text, "")
            if argv[1:] == ["--help"]
            else (0, '{"file_count": 42}', "")
        ),
        "orca": (0, "1.4.263\n", ""),
        "git": (0, "true\n", ""),
    }


class TempEnvTestCase(unittest.TestCase):
    """Point all path resolution at a fresh temporary tree."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        overrides = {
            "HOME": self.tmp.name,
            "XDG_CONFIG_HOME": str(self.root / ".config"),
            "AGENT_HARNESS_DATA_HOME": str(self.root / "data"),
            "AGENT_HARNESS_BIN_HOME": str(self.root / "bin"),
            "AGENT_HARNESS_SOURCE_ROOT": "",
            "AGENT_HARNESS_SELFTEST_PROVER": "",
        }
        patcher = mock.patch.dict(os.environ, overrides, clear=False)
        patcher.start()
        self.addCleanup(patcher.stop)
