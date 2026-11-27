"""Shared user-level VPS agent harness (Python standard library only).

This package is installed under the user data directory (by default
``~/.local/share/agent-harness``) and exposes the ``agent-harness`` commands:

* ``doctor --json`` -- read-only health report (never fails on an absent
  optional agent CLI).
* ``bootstrap --check|--apply`` -- narrowly merge the basemind MCP entry into
  each agent's user-global config; ``--check`` is read-only, ``--apply``
  backs up before writing and is idempotent.
* ``setup --root <repo>`` -- validate that a repository is indexed by
  basemind and exposes the full tool set; fail closed.
* ``self-test --agent <id>`` -- attempt agent-specific *live* proof, and
  return an actionable ``unsupported`` status otherwise.  Static detection
  is explicitly never presented as proof.
"""

from .version import __version__

__all__ = ["__version__"]
