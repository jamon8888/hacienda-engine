#!/usr/bin/env python3
"""Propagate sdks/VERSION into every SDK package manifest.

The single source of truth is sdks/VERSION. Run this after bumping it, before
tagging a release — mirrors xberg-sdks' scripts/sync-versions.py, adapted to the
two packages hacienda-engine actually ships (Python, TypeScript; no Go/Dart).
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

SDKS_ROOT = Path(__file__).resolve().parent.parent

PYPROJECT = SDKS_ROOT / "python" / "pyproject.toml"
PY_INIT = SDKS_ROOT / "python" / "src" / "hacienda_sdk" / "__init__.py"
PACKAGE_JSON = SDKS_ROOT / "typescript" / "package.json"


def read_version() -> str:
    return (SDKS_ROOT / "VERSION").read_text().strip()


def sync_pyproject(version: str) -> None:
    text = PYPROJECT.read_text()
    # re.subn's match count, not a before/after text comparison: if the
    # target version already matches what's on disk, substitution still
    # happens but produces byte-identical text, and a text-equality check
    # would misreport that as "no match found."
    new_text, count = re.subn(
        r'(?m)^version = "[^"]*"', f'version = "{version}"', text, count=1
    )
    if count == 0:
        raise SystemExit(f"no `version = \"...\"` line found in {PYPROJECT}")
    PYPROJECT.write_text(new_text)


def sync_py_init(version: str) -> None:
    text = PY_INIT.read_text()
    new_text, count = re.subn(
        r'(?m)^__version__ = "[^"]*"',
        f'__version__ = "{version}"',
        text,
        count=1,
    )
    if count == 0:
        raise SystemExit(f'no `__version__ = "..."` line found in {PY_INIT}')
    PY_INIT.write_text(new_text)


def sync_package_json(version: str) -> None:
    data = json.loads(PACKAGE_JSON.read_text())
    data["version"] = version
    PACKAGE_JSON.write_text(json.dumps(data, indent=2) + "\n")


def main() -> int:
    version = read_version()
    sync_pyproject(version)
    sync_py_init(version)
    sync_package_json(version)
    print(f"synced sdks/VERSION ({version}) to python and typescript manifests")
    return 0


if __name__ == "__main__":
    sys.exit(main())
