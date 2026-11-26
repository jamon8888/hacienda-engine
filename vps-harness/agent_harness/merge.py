"""Narrow, idempotent config merging (pure functions + write helpers).

JSON and JSONC user configs are parsed, one key is replaced, and the rest of
the document is preserved.  TOML is merged at the text level: only the single
managed table is touched, and a timestamped backup is written before any
atomic replacement.  Nothing in this module performs discovery or validation
of agent CLIs; it only transforms text.
"""

from __future__ import annotations

import datetime as _dt
import hashlib
import json
import os
import re
import shutil
import tempfile
import tomllib
from pathlib import Path


class ConfigError(ValueError):
    """Raised when a config file cannot be parsed or has an unexpected shape."""


_IGNORED_MATCH_KEYS = frozenset({"env"})


def entry_satisfied(current: object, desired: dict) -> bool:
    """Return True when ``current`` already provides the desired entry.

    Extra keys (for example a user-added ``env``) are preserved and ignored,
    so bootstrap never clobbers unrelated customisation.  The comparison is
    intentionally core-fields only: ``command``/``args``/``type``/``enabled``.
    """
    if not isinstance(current, dict):
        return False
    return all(
        current.get(key) == value
        for key, value in desired.items()
        if key not in _IGNORED_MATCH_KEYS
    )


# ---------------------------------------------------------------------------
# JSON / JSONC
# ---------------------------------------------------------------------------

def strip_jsonc_comments(text: str) -> str:
    """Remove ``//`` and ``/* */`` comments without touching string literals."""
    out: list[str] = []
    i, n = 0, len(text)
    in_string = False
    escaped = False
    while i < n:
        char = text[i]
        if in_string:
            out.append(char)
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            i += 1
        elif char == '"':
            in_string = True
            out.append(char)
            i += 1
        elif char == "/" and i + 1 < n and text[i + 1] == "/":
            i += 2
            while i < n and text[i] not in "\r\n":
                i += 1
        elif char == "/" and i + 1 < n and text[i + 1] == "*":
            i += 2
            while i + 1 < n and not (text[i] == "*" and text[i + 1] == "/"):
                i += 1
            i += 2
        else:
            out.append(char)
            i += 1
    return "".join(out)


def strip_trailing_commas(text: str) -> str:
    """Remove trailing commas before ``}``/``]`` outside of string literals."""
    out: list[str] = []
    i, n = 0, len(text)
    in_string = False
    escaped = False
    while i < n:
        char = text[i]
        if in_string:
            out.append(char)
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            i += 1
        elif char == '"':
            in_string = True
            out.append(char)
            i += 1
        elif char == ",":
            j = i + 1
            while j < n and text[j] in " \t\r\n":
                j += 1
            if j < n and text[j] in "}]":
                i += 1
            else:
                out.append(char)
                i += 1
        else:
            out.append(char)
            i += 1
    return "".join(out)


def loads_json_config(text: str, *, jsonc: bool = False) -> dict:
    """Parse a JSON/JSONC config into a dictionary."""
    if jsonc:
        text = strip_trailing_commas(strip_jsonc_comments(text))
    try:
        data = json.loads(text)
    except json.JSONDecodeError as exc:
        raise ConfigError(f"invalid JSON/JSONC config: {exc}") from exc
    if not isinstance(data, dict):
        raise ConfigError("config root is not a JSON object")
    return data


def dumps_json_config(data: dict) -> str:
    """Serialize a config deterministically with two-space indentation."""
    return json.dumps(data, indent=2, ensure_ascii=False) + "\n"


def merge_json_config(
    text: str,
    key: str,
    desired: dict,
    *,
    jsonc: bool = False,
    name: str = "basemind",
) -> str | None:
    """Return merged config text, or ``None`` when no change is required."""
    data = loads_json_config(text, jsonc=jsonc)
    container = data.get(key)
    if not isinstance(container, dict):
        container = {}
        data[key] = container
    existing = container.get(name)
    if entry_satisfied(existing, desired):
        return None
    if isinstance(existing, dict):
        # Repair only the managed fields; keep user-added keys such as `env`.
        for key, value in desired.items():
            if key not in _IGNORED_MATCH_KEYS:
                existing[key] = value
    else:
        container[name] = dict(desired)
    return dumps_json_config(data)


# ---------------------------------------------------------------------------
# TOML
# ---------------------------------------------------------------------------

def parse_toml(text: str) -> dict:
    """Parse TOML using the stdlib ``tomllib``."""
    try:
        return tomllib.loads(text)
    except tomllib.TOMLDecodeError as exc:
        raise ConfigError(f"invalid TOML config: {exc}") from exc


def toml_lookup(data: dict, dotted_key: str) -> object:
    """Walk a dotted TOML key, returning ``None`` when absent."""
    current: object = data
    for part in dotted_key.split("."):
        if not isinstance(current, dict):
            return None
        current = current.get(part)
    return current


def _toml_value(value: object) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, (list, tuple)):
        return "[" + ", ".join(_toml_value(item) for item in value) + "]"
    raise ConfigError(f"unsupported TOML value type: {type(value).__name__}")


def render_toml_table(section: str, entry: dict) -> str:
    """Render one TOML table for ``section`` from ``entry``."""
    lines = [f"[{section}]"]
    lines.extend(f"{key} = {_toml_value(value)}" for key, value in entry.items())
    return "\n".join(lines)


def remove_toml_table(text: str, section: str) -> str:
    """Delete ``section`` and its sub-tables without disturbing other tables."""
    prefix = section + "."
    leaf = section.rsplit(".", 1)[-1]
    inline_pattern = re.compile(rf"^{re.escape(leaf)}\s*=")
    # A TOML table header may be followed by whitespace and a trailing comment.
    header_pattern = re.compile(r"^\[([^\]]+)\]\s*(?:#.*)?$")
    kept: list[str] = []
    current_table = ""
    skipping = False
    for line in text.splitlines():
        stripped = line.strip()
        header_match = header_pattern.match(stripped)
        is_header = header_match is not None and not stripped.startswith("[[")
        if is_header:
            current_table = header_match.group(1).strip()  # type: ignore[union-attr]
            if current_table == section or current_table.startswith(prefix):
                skipping = True
                continue
            skipping = False
        if skipping:
            continue
        if current_table == section.rsplit(".", 1)[0] and inline_pattern.match(stripped):
            continue
        kept.append(line)
    return "\n".join(kept)


def merge_toml_config(text: str, section: str, desired: dict) -> str | None:
    """Return merged TOML text, or ``None`` when the desired table is present."""
    current = toml_lookup(parse_toml(text), section)
    if entry_satisfied(current, desired):
        return None
    cleaned = remove_toml_table(text, section)
    if cleaned and not cleaned.endswith("\n"):
        cleaned += "\n"
    if cleaned and not cleaned.endswith("\n\n"):
        cleaned += "\n"
    return cleaned + render_toml_table(section, desired) + "\n"


# ---------------------------------------------------------------------------
# Safe writes
# ---------------------------------------------------------------------------

def atomic_write(path: Path, text: str) -> None:
    """Atomically replace ``path`` with ``text`` via a temp file in its dir."""
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(prefix=".agent-harness-", dir=str(path.parent))
    tmp = Path(tmp_name)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(text)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp, path)
    finally:
        if tmp.exists():
            tmp.unlink()


def backup_and_write(path: Path, text: str) -> str | None:
    """Back up ``path`` (if present) then atomically write ``text``.

    Returns the backup path, or ``None`` when there was nothing to back up.
    """
    backup: str | None = None
    if path.exists():
        stamp = _dt.datetime.now().strftime("%Y%m%d%H%M%S")
        stem = f"{path.name}.agent-harness.bak.{stamp}"
        for sequence in range(1000):
            candidate = path.with_name(stem if sequence == 0 else f"{stem}.{sequence:03d}")
            try:
                # Reserve the name exclusively before copying into it.
                fd = os.open(candidate, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
                os.close(fd)
            except FileExistsError:
                continue
            shutil.copy2(path, candidate)
            backup = str(candidate)
            break
        else:  # pragma: no cover - exhausted only under pathological collisions
            raise RuntimeError(f"could not allocate a backup path for {path}")
    atomic_write(path, text)
    return backup


def sha256_file(path: Path) -> str:
    """Return the SHA-256 hex digest of ``path``."""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()
