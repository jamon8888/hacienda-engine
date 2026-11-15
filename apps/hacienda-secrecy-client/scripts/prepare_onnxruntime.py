#!/usr/bin/env python3
"""Stage a checksum-pinned ONNX Runtime for a native Tauri build."""

from __future__ import annotations

import hashlib
import os
import platform
import shutil
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath


VERSION = "1.23.2"
RELEASE = f"https://github.com/microsoft/onnxruntime/releases/download/v{VERSION}"
# Digests are the official GitHub release asset digests for microsoft/onnxruntime v1.23.2.
ASSETS = {
    ("Darwin", "x86_64"): (
        "onnxruntime-osx-x86_64-1.23.2.tgz",
        "d10359e16347b57d9959f7e80a225a5b4a66ed7d7e007274a15cae86836485a6",
        "macos-x86_64",
    ),
    ("Darwin", "arm64"): (
        "onnxruntime-osx-arm64-1.23.2.tgz",
        "b4d513ab2b26f088c66891dbbc1408166708773d7cc4163de7bdca0e9bbb7856",
        "macos-aarch64",
    ),
    ("Linux", "x86_64"): (
        "onnxruntime-linux-x64-1.23.2.tgz",
        "1fa4dcaef22f6f7d5cd81b28c2800414350c10116f5fdd46a2160082551c5f9b",
        "linux-x86_64",
    ),
    ("Linux", "aarch64"): (
        "onnxruntime-linux-aarch64-1.23.2.tgz",
        "7c63c73560ed76b1fac6cff8204ffe34fe180e70d6582b5332ec094810241e5c",
        "linux-aarch64",
    ),
    ("Windows", "AMD64"): (
        "onnxruntime-win-x64-1.23.2.zip",
        "0b38df9af21834e41e73d602d90db5cb06dbd1ca618948b8f1d66d607ac9f3cd",
        "windows-x86_64",
    ),
    ("Windows", "ARM64"): (
        "onnxruntime-win-arm64-1.23.2.zip",
        "1cfe88b6435df3b5fb0e9f6bd7d6f5df1e887b6174de7f6e2a47bab956f3f168",
        "windows-aarch64",
    ),
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def download_verified(asset: str, expected: str, cache: Path) -> Path:
    archive = cache / asset
    if archive.is_file() and sha256(archive) == expected:
        print(f"Using verified ONNX Runtime archive cache: {archive}")
        return archive

    archive.unlink(missing_ok=True)
    temporary = archive.with_suffix(archive.suffix + ".part")
    request = urllib.request.Request(
        f"{RELEASE}/{asset}", headers={"User-Agent": "hacienda-secrecy-client"}
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response, temporary.open("wb") as output:
            shutil.copyfileobj(response, output, length=1024 * 1024)
        actual = sha256(temporary)
        if actual != expected:
            raise RuntimeError(f"SHA-256 mismatch for {asset}: expected {expected}, got {actual}")
        temporary.replace(archive)
    finally:
        temporary.unlink(missing_ok=True)
    return archive


def safe_relative(name: str) -> PurePosixPath | None:
    normalized = name.replace("\\", "/")
    path = PurePosixPath(normalized)
    if path.is_absolute() or ".." in path.parts:
        return None
    return path


def is_runtime_library(name: str) -> bool:
    return (
        name.endswith((".dylib", ".dll"))
        or ".so." in name
        or name.endswith(".so")
    )


def copy_archive_members(archive: Path, stage: Path, asset: str) -> None:
    copied_runtime = False
    copied_license = False

    def write_member(name: str, source, relative: PurePosixPath) -> None:
        destination = stage.joinpath(*relative.parts)
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open("wb") as output:
            shutil.copyfileobj(source, output)

    if asset.endswith(".tgz"):
        with tarfile.open(archive, "r:gz") as bundle:
            for member in bundle.getmembers():
                relative = safe_relative(member.name)
                if relative is None or not member.isfile():
                    continue
                parts = relative.parts
                if len(parts) == 3 and parts[1] == "lib" and is_runtime_library(parts[2]):
                    stream = bundle.extractfile(member)
                    if stream is None:
                        continue
                    with stream:
                        write_member(member.name, stream, PurePosixPath(*parts[1:]))
                    copied_runtime = True
                elif len(parts) == 2 and parts[1] in {"LICENSE", "ThirdPartyNotices.txt"}:
                    stream = bundle.extractfile(member)
                    if stream is None:
                        continue
                    with stream:
                        write_member(member.name, stream, PurePosixPath(parts[1]))
                    copied_license = True
    else:
        with zipfile.ZipFile(archive) as bundle:
            for member in bundle.infolist():
                relative = safe_relative(member.filename)
                if relative is None or member.is_dir():
                    continue
                parts = relative.parts
                if len(parts) == 3 and parts[1] == "lib" and is_runtime_library(parts[2]):
                    with bundle.open(member) as stream:
                        write_member(member.filename, stream, PurePosixPath(*parts[1:]))
                    copied_runtime = True
                elif len(parts) == 2 and parts[1] in {"LICENSE", "ThirdPartyNotices.txt"}:
                    with bundle.open(member) as stream:
                        write_member(member.filename, stream, PurePosixPath(parts[1]))
                    copied_license = True

    if not copied_runtime:
        raise RuntimeError(f"No runtime libraries found inside {asset}")
    if not copied_license:
        raise RuntimeError(f"No ONNX Runtime license files found inside {asset}")


def main() -> int:
    identity = (platform.system(), platform.machine())
    try:
        asset, expected, target_dir = ASSETS[identity]
    except KeyError as error:
        raise RuntimeError(f"Unsupported native build target: {identity[0]} {identity[1]}") from error

    cache_root = Path(
        os.environ.get(
            "HACIENDA_ORT_CACHE",
            Path.home() / ".cache" / "hacienda-secrecy-client" / "onnxruntime" / VERSION,
        )
    )
    cache_root.mkdir(parents=True, exist_ok=True)
    stage_root = Path(__file__).resolve().parents[1] / "src-tauri" / "resources" / "onnxruntime"
    stage = stage_root / target_dir
    archive = download_verified(asset, expected, cache_root)

    with tempfile.TemporaryDirectory(prefix="hacienda-ort-stage-") as temporary_dir:
        temporary_stage = Path(temporary_dir)
        copy_archive_members(archive, temporary_stage, asset)
        if not any(temporary_stage.glob("lib/*onnxruntime*")):
            raise RuntimeError(f"No ONNX Runtime shared library found in {asset}")
        if stage_root.exists():
            shutil.rmtree(stage_root)
        stage.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(temporary_stage, stage)

    print(f"Staged verified ONNX Runtime {VERSION} in {stage}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:  # noqa: BLE001 - actionable build-time diagnostic
        print(f"ONNX Runtime staging failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
