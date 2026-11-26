"""Injectable process runners.

All external process calls go through a Runner so unit tests stay fully
offline and deterministic.  The production runner is a thin wrapper around
``subprocess.run`` that converts failures into a ``(returncode, out, err)``
tuple instead of raising.
"""

from __future__ import annotations

import shutil
import subprocess
from typing import Protocol


class Runner(Protocol):
    """Minimal interface used by the health, setup and self-test modules."""

    def which(self, name: str) -> str | None: ...

    def run(self, argv: list[str], *, timeout: int = 15) -> tuple[int, str, str]: ...


class SubprocessRunner:
    """Real runner backed by ``shutil.which`` and ``subprocess.run``."""

    def which(self, name: str) -> str | None:
        return shutil.which(name)

    def run(self, argv: list[str], *, timeout: int = 15) -> tuple[int, str, str]:
        if (
            not isinstance(argv, (list, tuple))
            or not argv
            or not all(isinstance(item, str) for item in argv)
        ):
            raise ValueError("argv must be a non-empty sequence of strings")
        # SAFETY: argv comes from literal tool names assembled by this package
        # (basemind, orca, git, or an explicitly configured prover).  It is
        # passed as an argument vector with shell=False, so no shell is involved
        # and no argument can be interpreted as a command.  The security scanner
        # cannot prove that statically, hence the targeted suppression below.
        # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit
        try:
            proc = subprocess.run(
                [str(item) for item in argv],
                shell=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=timeout,
                check=False,
            )
            return proc.returncode, proc.stdout, proc.stderr
        except FileNotFoundError:
            return 127, "", f"command not found: {argv[0]}"
        except subprocess.TimeoutExpired:
            return 124, "", f"timeout after {timeout}s: {' '.join(argv)}"
        except OSError as exc:  # pragma: no cover - defensive
            return 125, "", str(exc)
