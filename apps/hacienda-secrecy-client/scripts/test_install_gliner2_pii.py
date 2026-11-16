#!/usr/bin/env python3
"""Focused tests for the dependency-free GLiNER2 checkpoint installer."""

from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("install_gliner2_pii.py")
SPEC = importlib.util.spec_from_file_location("install_gliner2_pii", SCRIPT)
assert SPEC and SPEC.loader
installer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(installer)


class InstallerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.root = Path(self.temp_dir.name)
        self.source = self.root / "offline-source"
        self.source.mkdir()
        self.manifest_path = self.root / "fixture-manifest.json"
        self.files = {
            "config.json": b'{"span": true}\n',
            "encoder_config/config.json": b'{"hidden_size": 8}\n',
        }
        records = []
        for relative_path, content in self.files.items():
            source_path = self.source / relative_path
            source_path.parent.mkdir(parents=True, exist_ok=True)
            source_path.write_bytes(content)
            records.append(
                {
                    "path": relative_path,
                    "size": len(content),
                    "sha256": hashlib.sha256(content).hexdigest(),
                }
            )
        self.manifest_path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "model_id": installer.PINNED_MODEL_ID,
                    "revision": installer.PINNED_REVISION,
                    "files": records,
                }
            ),
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def test_offline_install_verifies_and_atomically_publishes(self) -> None:
        destination = self.root / "installed"
        (self.source / "ignored-extra.bin").write_bytes(b"not allowlisted")

        installer.install(
            source=self.source,
            destination=destination,
            manifest_path=self.manifest_path,
        )

        self.assertEqual(
            {path: (destination / path).read_bytes() for path in self.files}, self.files
        )
        self.assertFalse((destination / "ignored-extra.bin").exists())
        self.assertEqual(list(self.root.glob(".installed.staging-*")), [])

    def test_offline_install_rejects_hash_mismatch_without_publishing(self) -> None:
        destination = self.root / "installed"
        (self.source / "config.json").write_bytes(b'{"span": true}!')

        with self.assertRaisesRegex(installer.InstallError, "SHA-256 mismatch"):
            installer.install(
                source=self.source,
                destination=destination,
                manifest_path=self.manifest_path,
            )

        self.assertFalse(destination.exists())
        self.assertEqual(list(self.root.glob(".installed.staging-*")), [])

    def test_offline_install_rejects_size_mismatch_without_publishing(self) -> None:
        destination = self.root / "installed"
        manifest = json.loads(self.manifest_path.read_text(encoding="utf-8"))
        manifest["files"][0]["size"] += 1
        self.manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

        with self.assertRaisesRegex(installer.InstallError, "size mismatch"):
            installer.install(
                source=self.source,
                destination=destination,
                manifest_path=self.manifest_path,
            )

        self.assertFalse(destination.exists())

    def test_existing_destination_is_never_replaced(self) -> None:
        destination = self.root / "installed"
        destination.mkdir()
        sentinel = destination / "sentinel"
        sentinel.write_text("keep", encoding="utf-8")

        with self.assertRaisesRegex(installer.InstallError, "already exists"):
            installer.install(
                source=self.source,
                destination=destination,
                manifest_path=self.manifest_path,
            )

        self.assertEqual(sentinel.read_text(encoding="utf-8"), "keep")

    def test_manifest_rejects_path_traversal(self) -> None:
        manifest = json.loads(self.manifest_path.read_text(encoding="utf-8"))
        manifest["files"][0]["path"] = "../outside"
        self.manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

        with self.assertRaisesRegex(installer.InstallError, "unsafe manifest path"):
            installer.load_manifest(self.manifest_path)


if __name__ == "__main__":
    unittest.main()
