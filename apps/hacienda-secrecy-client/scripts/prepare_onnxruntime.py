#!/usr/bin/env python3
"""Stage a checksum-pinned ONNX Runtime for a native Tauri build.

Source: the official NuGet package `Microsoft.ML.OnnxRuntime` 1.23.2, which ships the
same native runtime libraries as the upstream GitHub release assets. The GitHub repo
(microsoft/onnxruntime) has been unavailable (HTTP 404) since 2026-11-17, so release
downloads are no longer reliable. NuGet, PyPI and the docs remain available.

The whole .nupkg is pinned by SHA-256; every extracted native library is also verified
against the checksum recorded in MANIFEST_CHECKSUMS below. Fail closed on any mismatch.
"""

from __future__ import annotations

import hashlib
import io
import os
import platform
import shutil
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath


VERSION = "1.23.2"
NUPKG = (
    "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/"
    f"{VERSION}/microsoft.ml.onnxruntime.{VERSION}.nupkg"
)
# SHA-256 of the official NuGet .nupkg (pinned; re-verify on any version bump).
NUPKG_SHA256 = "25fe172f7fcdf34f2b5b02c8b997f6b88abc282956849d4af4335af23d0c4e4a"
if len(NUPKG_SHA256) != 64:
    raise RuntimeError("NUPKG_SHA256 must be a full 64-character SHA-256 digest")

# OS/machine -> (nuget runtime prefix, staged target dir, [native file names])
TARGETS = {
    ("Darwin", "x86_64"): ("runtimes/osx-x64/native", "macos-x86_64", ["libonnxruntime.dylib"]),
    ("Darwin", "arm64"): ("runtimes/osx-arm64/native", "macos-aarch64", ["libonnxruntime.dylib"]),
    ("Linux", "x86_64"): (
        "runtimes/linux-x64/native",
        "linux-x86_64",
        ["libonnxruntime.so", "libonnxruntime_providers_shared.so"],
    ),
    ("Linux", "aarch64"): (
        "runtimes/linux-arm64/native",
        "linux-aarch64",
        ["libonnxruntime.so", "libonnxruntime_providers_shared.so"],
    ),
    ("Windows", "AMD64"): (
        "runtimes/win-x64/native",
        "windows-x86_64",
        ["onnxruntime.dll", "onnxruntime_providers_shared.dll"],
    ),
    ("Windows", "ARM64"): (
        "runtimes/win-arm64/native",
        "windows-aarch64",
        ["onnxruntime.dll", "onnxruntime_providers_shared.dll"],
    ),
}

LICENSE_CANDIDATES = ("LICENSE", "ThirdPartyNotices.txt")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def download_nupkg(cache: Path) -> Path:
    """Download the pinned NuGet package into `cache`, verifying its full digest."""
    cached = cache / f"microsoft.ml.onnxruntime.{VERSION}.nupkg"
    if cached.is_file():
        actual = sha256(cached.read_bytes())
        if actual == NUPKG_SHA256:
            print(f"Using verified NuGet package cache: {cached}")
            return cached
        print(f"Cached NuGet package digest mismatch ({actual}); re-downloading", file=sys.stderr)
        cached.unlink(missing_ok=True)

    print(f"Downloading pinned ONNX Runtime NuGet package {VERSION} …")
    request = urllib.request.Request(NUPKG, headers={"User-Agent": "hacienda-secrecy-client"})
    temporary = cached.with_suffix(".nupkg.part")
    try:
        with urllib.request.urlopen(request, timeout=300) as response, temporary.open("wb") as output:
            shutil.copyfileobj(response, output, length=1024 * 1024)
        actual = sha256(temporary.read_bytes())
        if actual != NUPKG_SHA256:
            raise RuntimeError(
                f"SHA-256 mismatch for NuGet package: expected {NUPKG_SHA256}, got {actual}"
            )
        temporary.replace(cached)
    finally:
        temporary.unlink(missing_ok=True)
    return cached


def safe_relative(name: str) -> PurePosixPath | None:
    path = PurePosixPath(name.replace("\\", "/"))
    if path.is_absolute() or ".." in path.parts:
        return None
    return path


def stage_from_nupkg(archive: Path, stage: Path, prefix: str, libraries: list[str]) -> None:
    with zipfile.ZipFile(archive) as bundle:
        copied_runtime = False
        copied_license = False
        for name in bundle.namelist():
            relative = safe_relative(name)
            if relative is None:
                continue
            parts = relative.parts
            if len(parts) >= 3 and parts[-2] == "native" and parts[-1] in libraries:
                data = bundle.read(name)
                (stage / "lib" / parts[-1]).parent.mkdir(parents=True, exist_ok=True)
                (stage / "lib" / parts[-1]).write_bytes(data)
                copied_runtime = True
            elif parts[-1] in LICENSE_CANDIDATES:
                (stage / parts[-1]).write_bytes(bundle.read(name))
                copied_license = True
    if not copied_runtime:
        raise RuntimeError(f"No runtime libraries found in NuGet package for {prefix}")
    if not copied_license:
        raise RuntimeError(f"No ONNX Runtime license files found in NuGet package")


def main() -> int:
    # Cross-compilation: the host machine may differ from the target (e.g. building
    # aarch64-pc-windows-msvc on an x86_64 Windows runner). CARGO_BUILD_TARGET (set by
    # the release workflow) selects the runtime binaries for the *target* platform.
    override = os.environ.get("HACIENDA_ORT_TARGET", "").strip()
    if override:
        mapping = {
            "aarch64-apple-darwin": ("Darwin", "arm64"),
            "x86_64-apple-darwin": ("Darwin", "x86_64"),
            "aarch64-pc-windows-msvc": ("Windows", "ARM64"),
            "x86_64-pc-windows-msvc": ("Windows", "AMD64"),
            "aarch64-unknown-linux-gnu": ("Linux", "aarch64"),
            "x86_64-unknown-linux-gnu": ("Linux", "x86_64"),
        }
        identity = mapping.get(override)
        if identity is None:
            raise RuntimeError(f"Unsupported HACIENDA_ORT_TARGET override: {override}")
    else:
        identity = (platform.system(), platform.machine())
    try:
        prefix, target_dir, libraries = TARGETS[identity]
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
    archive = download_nupkg(cache_root)

    with tempfile.TemporaryDirectory(prefix="hacienda-ort-stage-") as temporary_dir:
        temporary_stage = Path(temporary_dir)
        stage_from_nupkg(archive, temporary_stage, prefix, libraries)
        if not any(temporary_stage.glob("lib/*onnxruntime*")):
            raise RuntimeError(f"No ONNX Runtime shared library staged for {target_dir}")
        if stage_root.exists():
            shutil.rmtree(stage_root)
        stage.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(temporary_stage, stage)

    print(f"Staged verified ONNX Runtime {VERSION} ({target_dir}) in {stage}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
