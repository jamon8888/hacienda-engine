"""Environment-aware path resolution.

Every path is derived from environment variables so tests can point the
harness at a temporary HOME/XDG tree without touching the real global
configuration.  ``--check``/doctor/setup never write to any of these paths.
"""

from __future__ import annotations

import os
from pathlib import Path


def home() -> Path:
    """Return the user home directory (honours ``HOME`` on POSIX)."""
    return Path(os.path.expanduser("~")).expanduser()


def data_home() -> Path:
    """Return the harness user data directory."""
    override = os.environ.get("AGENT_HARNESS_DATA_HOME")
    if override:
        return Path(override).expanduser()
    return home() / ".local" / "share" / "agent-harness"


def bin_home() -> Path:
    """Return the directory that holds user launchers."""
    override = os.environ.get("AGENT_HARNESS_BIN_HOME")
    if override:
        return Path(override).expanduser()
    return home() / ".local" / "bin"


def xdg_config_home() -> Path:
    """Return the XDG configuration root."""
    override = os.environ.get("XDG_CONFIG_HOME")
    if override:
        return Path(override).expanduser()
    return home() / ".config"


def expand_template(template: str) -> Path:
    """Expand a config path template built with ``{home}`` and ``{xdg}``."""
    rendered = template.replace("{home}", str(home())).replace("{xdg}", str(xdg_config_home()))
    return Path(rendered).expanduser()
