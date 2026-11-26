"""Installer: copy the versioned package into the user data dir and link the launcher.

Runs without sudo.  Re-running with the same version and identical content is
a no-op; a different version is installed alongside the old one and ``current``
is repointed.  The launcher symlink in ``~/.local/bin`` is only replaced when
it points elsewhere, and a non-symlink collision is backed up first.
"""

from __future__ import annotations

import datetime as _dt
import json
import os
import shutil
import tempfile
from pathlib import Path

from . import paths
from .merge import sha256_file
from .version import __version__

SCHEMA = "agent-harness/install/1"


def _iter_package_files(root: Path):
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        if "__pycache__" in path.parts or path.suffix == ".pyc":
            continue
        if path.name == "MANIFEST.json":
            continue
        yield path


def source_manifest(source_root: Path) -> dict[str, str]:
    """Return the SHA-256 manifest of the installable source files."""
    source_root = Path(source_root).resolve()
    manifest: dict[str, str] = {}
    for path in _iter_package_files(source_root / "agent_harness"):
        manifest[path.relative_to(source_root).as_posix()] = sha256_file(path)
    launcher = source_root / "bin" / "agent-harness"
    if not launcher.is_file():
        raise RuntimeError(f"missing launcher in source: {launcher}")
    manifest["bin/agent-harness"] = sha256_file(launcher)
    gatekeeper = source_root / "bin" / "gatekeeper"
    if not gatekeeper.is_file():
        raise RuntimeError(f"missing gatekeeper in source: {gatekeeper}")
    manifest["bin/gatekeeper"] = sha256_file(gatekeeper)
    if not (source_root / "agent_harness").is_dir():
        raise RuntimeError(f"no installable package found under {source_root}")
    return manifest


def _installed_manifest(version_dir: Path) -> dict[str, str]:
    if not version_dir.is_dir():
        return {}
    manifest: dict[str, str] = {}
    for path in _iter_package_files(version_dir):
        manifest[path.relative_to(version_dir).as_posix()] = sha256_file(path)
    return manifest


def _ensure_symlink(link: Path, target: str) -> bool:
    """Create/replace ``link`` -> ``target``; return True if changed."""
    if link.is_symlink() and os.readlink(link) == target:
        return False
    link.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(prefix=".agent-harness-link-", dir=str(link.parent))
    os.close(fd)
    tmp = Path(tmp_name)
    try:
        tmp.unlink(missing_ok=True)
        tmp.symlink_to(target)
        os.replace(tmp, link)
    finally:
        tmp.unlink(missing_ok=True)
    return True


def install(
    source_root: Path,
    *,
    data_home: Path | None = None,
    bin_home: Path | None = None,
) -> dict:
    """Install ``source_root`` into the user data dir; return a JSON-ready report."""
    source_root = Path(source_root).resolve()
    # The data dir is the application directory itself; an override replaces it.
    data = (Path(data_home) if data_home is not None else paths.data_home()).resolve()
    bin_dir = (Path(bin_home) if bin_home is not None else paths.bin_home()).resolve()
    versions_dir = data / "versions"
    version_dir = versions_dir / __version__
    desired = source_manifest(source_root)
    actions: list[str] = []

    if _installed_manifest(version_dir) == desired:
        actions.append(f"version {__version__} already installed with identical content")
    else:
        staging = versions_dir / f".staging-{__version__}-{os.getpid()}"
        backup_dir: Path | None = None
        if staging.exists():
            shutil.rmtree(staging)
        try:
            shutil.copytree(
                source_root / "agent_harness",
                staging / "agent_harness",
                ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
            )
            (staging / "bin").mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_root / "bin" / "agent-harness", staging / "bin" / "agent-harness")
            os.chmod(staging / "bin" / "agent-harness", 0o755)
            shutil.copy2(source_root / "bin" / "gatekeeper", staging / "bin" / "gatekeeper")
            os.chmod(staging / "bin" / "gatekeeper", 0o755)
            (staging / "MANIFEST.json").write_text(
                json.dumps(desired, indent=2, sort_keys=True) + "\n", encoding="utf-8"
            )
            if _installed_manifest(staging) != desired:
                raise RuntimeError("staged manifest verification failed")
            # Only now replace the working version; a failure above leaves it intact.
            if version_dir.exists():
                stamp = _dt.datetime.now().strftime("%Y%m%d%H%M%S")
                backup_dir = versions_dir / f".backup-{__version__}-{stamp}"
                if backup_dir.exists():
                    shutil.rmtree(backup_dir)
                shutil.move(str(version_dir), str(backup_dir))
            shutil.move(str(staging), str(version_dir))
            if backup_dir is not None:
                shutil.rmtree(backup_dir)
                backup_dir = None
            actions.append(f"installed version {__version__}")
        finally:
            if staging.exists():
                shutil.rmtree(staging)

    current = data / "current"
    if _ensure_symlink(current, f"versions/{__version__}"):
        actions.append(f"pointed current -> versions/{__version__}")
    else:
        actions.append("current symlink already correct")

    launcher = bin_dir / "agent-harness"
    launcher_target = str(version_dir / "bin" / "agent-harness")
    if launcher.exists() and not launcher.is_symlink():
        stamp = _dt.datetime.now().strftime("%Y%m%d%H%M%S")
        backup = launcher.with_name(f"{launcher.name}.agent-harness.bak.{stamp}")
        shutil.move(str(launcher), str(backup))
        actions.append(f"backed up existing launcher to {backup.name}")
    if _ensure_symlink(launcher, launcher_target):
        actions.append(f"linked launcher {launcher} -> {launcher_target}")
    else:
        actions.append("launcher symlink already correct")

    return {
        "schema": SCHEMA,
        "harness_version": __version__,
        "status": "installed" if any("installed version" in item for item in actions) else "already_installed",
        "version": __version__,
        "data_dir": str(data),
        "version_dir": str(version_dir),
        "current": str(current),
        "launcher": str(launcher),
        "actions": actions,
        "note": "No sudo required; re-running is idempotent.",
    }
