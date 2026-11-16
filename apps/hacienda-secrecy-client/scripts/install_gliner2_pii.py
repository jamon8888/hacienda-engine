#!/usr/bin/env python3
"""Install the pinned GLiNER2 multilingual PII checkpoint.

The installer is deliberately dependency-free and has no import-time or
runtime-model auto-download behavior. Invoke it explicitly with either an
offline source directory or ``--download``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import sys
import tempfile
from typing import BinaryIO, Iterable, Mapping
from urllib.parse import quote
from urllib.request import Request, urlopen


DEFAULT_MANIFEST = (
    Path(__file__).resolve().parents[1] / "models" / "gliner2-pii" / "manifest.json"
)
PINNED_MODEL_ID = "fastino/gliner2-privacy-filter-PII-multi"
PINNED_REVISION = "1cb4166094dc58fa8d836429f060d6c95f62b495"
CHUNK_SIZE = 1024 * 1024
SHA256_HEX_LENGTH = 64


class InstallError(RuntimeError):
    """Raised for a refused or failed installation."""


def load_manifest(path: Path) -> dict:
    try:
        with path.open("r", encoding="utf-8") as stream:
            manifest = json.load(stream)
    except (OSError, json.JSONDecodeError) as exc:
        raise InstallError(f"cannot read manifest {path}: {exc}") from exc
    validate_manifest(manifest)
    return manifest


def validate_manifest(manifest: Mapping[str, object]) -> None:
    if not isinstance(manifest, Mapping):
        raise InstallError("manifest root must be an object")
    required = {"schema_version", "model_id", "revision", "files"}
    missing = required - manifest.keys()
    if missing:
        raise InstallError(f"manifest is missing required fields: {sorted(missing)}")
    if manifest["schema_version"] != 1:
        raise InstallError("unsupported manifest schema_version")
    model_id = manifest["model_id"]
    revision = manifest["revision"]
    if not isinstance(model_id, str) or not model_id or "/" not in model_id:
        raise InstallError("manifest model_id must be an owner/name")
    if model_id != PINNED_MODEL_ID:
        raise InstallError(f"manifest model_id must be {PINNED_MODEL_ID}")
    if (
        not isinstance(revision, str)
        or len(revision) != 40
        or any(character not in "0123456789abcdef" for character in revision)
    ):
        raise InstallError("manifest revision must be a lowercase 40-character SHA-1")
    if revision != PINNED_REVISION:
        raise InstallError(f"manifest revision must be {PINNED_REVISION}")
    files = manifest["files"]
    if not isinstance(files, list) or not files:
        raise InstallError("manifest files must be a non-empty list")

    seen: set[str] = set()
    for record in files:
        if not isinstance(record, Mapping):
            raise InstallError("each manifest file entry must be an object")
        for key in ("path", "size", "sha256"):
            if key not in record:
                raise InstallError(f"manifest file entry is missing {key!r}")
        relative_path = record["path"]
        if not isinstance(relative_path, str) or not _is_safe_relative_path(relative_path):
            raise InstallError(f"unsafe manifest path: {relative_path!r}")
        if relative_path in seen:
            raise InstallError(f"duplicate manifest path: {relative_path}")
        seen.add(relative_path)
        size = record["size"]
        if not isinstance(size, int) or isinstance(size, bool) or size < 0:
            raise InstallError(f"invalid expected size for {relative_path}")
        digest = record["sha256"]
        if (
            not isinstance(digest, str)
            or len(digest) != SHA256_HEX_LENGTH
            or any(character not in "0123456789abcdef" for character in digest)
        ):
            raise InstallError(f"invalid SHA-256 for {relative_path}")


def _is_safe_relative_path(value: str) -> bool:
    if not value or "\\" in value:
        return False
    path = PurePosixPath(value)
    return not path.is_absolute() and all(part not in ("", ".", "..") for part in path.parts)


def manifest_files(manifest: Mapping[str, object]) -> Iterable[Mapping[str, object]]:
    files = manifest["files"]
    assert isinstance(files, list)
    return files  # type: ignore[return-value]


def expected_path(root: Path, relative_path: str) -> Path:
    """Resolve a manifest path while ensuring it stays below ``root``."""
    root = root.resolve()
    candidate = (root / PurePosixPath(relative_path)).resolve()
    try:
        candidate.relative_to(root)
    except ValueError as exc:
        raise InstallError(f"path escapes source directory: {relative_path}") from exc
    return candidate


def _hash_stream(
    stream: BinaryIO, output: BinaryIO | None = None, max_size: int | None = None
) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    while True:
        chunk = stream.read(CHUNK_SIZE)
        if not chunk:
            break
        next_size = size + len(chunk)
        if max_size is not None and next_size > max_size:
            raise InstallError(f"file exceeds expected size ({max_size} bytes)")
        size = next_size
        digest.update(chunk)
        if output is not None:
            output.write(chunk)
    return size, digest.hexdigest()


def verify_file(path: Path, record: Mapping[str, object]) -> None:
    if not path.is_file() or path.is_symlink():
        raise InstallError(f"missing or non-regular model file: {path}")
    try:
        with path.open("rb") as stream:
            actual_size, actual_hash = _hash_stream(stream, max_size=record["size"])
    except OSError as exc:
        raise InstallError(f"cannot read model file {path}: {exc}") from exc
    expected_size = record["size"]
    expected_hash = record["sha256"]
    if actual_size != expected_size:
        raise InstallError(
            f"size mismatch for {record['path']}: expected {expected_size}, got {actual_size}"
        )
    if actual_hash != expected_hash:
        raise InstallError(
            f"SHA-256 mismatch for {record['path']}: expected {expected_hash}, got {actual_hash}"
        )


def copy_verified_source(source: Path, destination: Path, manifest: Mapping[str, object]) -> None:
    source = source.resolve()
    if not source.is_dir():
        raise InstallError(f"offline source is not a directory: {source}")
    for record in manifest_files(manifest):
        relative_path = record["path"]
        assert isinstance(relative_path, str)
        source_path = expected_path(source, relative_path)
        verify_file(source_path, record)
        target_path = expected_path(destination, relative_path)
        target_path.parent.mkdir(parents=True, exist_ok=True)
        try:
            shutil.copyfile(source_path, target_path)
            os.chmod(target_path, 0o644)
        except OSError as exc:
            raise InstallError(f"cannot stage {relative_path}: {exc}") from exc
        verify_file(target_path, record)


def download_url(manifest: Mapping[str, object], relative_path: str) -> str:
    model_id = manifest["model_id"]
    revision = manifest["revision"]
    assert isinstance(model_id, str) and isinstance(revision, str)
    encoded_path = "/".join(quote(part, safe="") for part in relative_path.split("/"))
    return f"https://huggingface.co/{model_id}/resolve/{revision}/{encoded_path}"


def download_verified_file(
    url: str, destination: Path, record: Mapping[str, object]
) -> None:
    request = Request(url, headers={"User-Agent": "hacienda-gliner2-installer/1"})
    try:
        with urlopen(request, timeout=120) as response, destination.open("wb") as output:
            actual_size, actual_hash = _hash_stream(
                response, output, max_size=record["size"]
            )
    except OSError as exc:
        raise InstallError(f"cannot download {record['path']}: {exc}") from exc
    expected_size = record["size"]
    expected_hash = record["sha256"]
    if actual_size != expected_size:
        raise InstallError(
            f"size mismatch for downloaded {record['path']}: expected {expected_size}, got {actual_size}"
        )
    if actual_hash != expected_hash:
        raise InstallError(
            f"SHA-256 mismatch for downloaded {record['path']}: expected {expected_hash}, got {actual_hash}"
        )
    os.chmod(destination, 0o644)


def download_verified(manifest: Mapping[str, object], destination: Path) -> None:
    for record in manifest_files(manifest):
        relative_path = record["path"]
        assert isinstance(relative_path, str)
        target_path = expected_path(destination, relative_path)
        target_path.parent.mkdir(parents=True, exist_ok=True)
        download_verified_file(download_url(manifest, relative_path), target_path, record)


def install(
    *,
    destination: Path,
    manifest_path: Path = DEFAULT_MANIFEST,
    source: Path | None = None,
    download: bool = False,
) -> None:
    if (source is None) == (not download):
        raise InstallError("choose exactly one of an offline source directory or --download")
    manifest = load_manifest(manifest_path)
    destination = destination.expanduser().resolve()
    if os.path.lexists(destination):
        raise InstallError(f"destination already exists; refusing to replace it: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    stage: Path | None = None
    try:
        stage = Path(tempfile.mkdtemp(prefix=f".{destination.name}.staging-", dir=destination.parent))
        os.chmod(stage, 0o700)
        if source is not None:
            copy_verified_source(source, stage, manifest)
        else:
            download_verified(manifest, stage)
        os.replace(stage, destination)
        stage = None
    except (OSError, InstallError) as exc:
        if isinstance(exc, InstallError):
            raise
        raise InstallError(f"cannot publish checkpoint: {exc}") from exc
    finally:
        if stage is not None:
            shutil.rmtree(stage, ignore_errors=True)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", type=Path, required=True, help="directory to publish")
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    source_group = parser.add_mutually_exclusive_group(required=True)
    source_group.add_argument("--source", type=Path, help="offline model directory")
    source_group.add_argument(
        "--download",
        action="store_true",
        help="explicitly fetch the pinned allowlisted files from Hugging Face",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv if argv is not None else sys.argv[1:])
    try:
        install(
            destination=args.destination,
            manifest_path=args.manifest,
            source=args.source,
            download=args.download,
        )
    except InstallError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    print(f"installed pinned GLiNER2 PII checkpoint to {args.destination}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
