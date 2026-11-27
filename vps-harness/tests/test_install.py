"""Installer tests: versioned copy, symlinks, idempotency, collision backup."""

from __future__ import annotations

import json
import os
import unittest
from pathlib import Path

from agent_harness.install import install, source_manifest
from agent_harness.version import __version__
from tests.fakes import TempEnvTestCase


class InstallTests(TempEnvTestCase):
    def setUp(self):
        super().setUp()
        self.source = self.root / "src"
        (self.source / "agent_harness").mkdir(parents=True)
        (self.source / "bin").mkdir()
        (self.source / "agent_harness" / "__init__.py").write_text("VALUE = 1\n", encoding="utf-8")
        (self.source / "bin" / "agent-harness").write_text("#!/usr/bin/env python3\n", encoding="utf-8")
        (self.source / "bin" / "gatekeeper").write_text("#!/usr/bin/env bash\n", encoding="utf-8")
        self.data_home = self.root / "data"
        self.bin_home = self.root / "bin"

    def run_install(self):
        return install(
            self.source,
            data_home=self.data_home,
            bin_home=self.bin_home,
        )

    def test_install_creates_version_dir_and_symlinks(self):
        report = self.run_install()
        self.assertEqual(report["status"], "installed")
        self.assertEqual(report["version"], __version__)
        version_dir = self.data_home / "versions" / __version__
        self.assertTrue((version_dir / "agent_harness" / "__init__.py").is_file())
        self.assertTrue((version_dir / "MANIFEST.json").is_file())
        manifest = json.loads((version_dir / "MANIFEST.json").read_text())
        self.assertIn("agent_harness/__init__.py", manifest)
        self.assertIn("bin/agent-harness", manifest)
        current = self.data_home / "current"
        self.assertTrue(current.is_symlink())
        self.assertEqual(os.readlink(current), f"versions/{__version__}")
        launcher = self.bin_home / "agent-harness"
        self.assertTrue(launcher.is_symlink())
        self.assertEqual(Path(os.readlink(launcher)).name, "agent-harness")
        self.assertTrue(Path(os.readlink(launcher)).is_absolute())
        self.assertTrue(os.access(launcher, os.X_OK))

    def test_default_data_home_is_not_appended_twice(self):
        report = install(self.source)
        self.assertEqual(report["data_dir"], str(self.root / "data"))
        self.assertTrue((self.root / "data" / "versions" / __version__).is_dir())
        self.assertFalse((self.root / "data" / "agent-harness").exists())

    def test_missing_launcher_preserves_working_version(self):
        self.run_install()
        version_dir = self.data_home / "versions" / __version__
        before = (version_dir / "agent_harness" / "__init__.py").read_text()
        (self.source / "bin" / "agent-harness").unlink()
        with self.assertRaises(RuntimeError):
            self.run_install()
        self.assertEqual((version_dir / "agent_harness" / "__init__.py").read_text(), before)

    def test_second_install_is_noop(self):
        self.run_install()
        second = self.run_install()
        self.assertEqual(second["status"], "already_installed")
        self.assertFalse(any("installed version" in item for item in second["actions"]))
        self.assertTrue(any("already correct" in item for item in second["actions"]))

    def test_changed_source_reinstalls_same_version(self):
        self.run_install()
        (self.source / "agent_harness" / "__init__.py").write_text("VALUE = 2\n", encoding="utf-8")
        report = self.run_install()
        self.assertEqual(report["status"], "installed")
        self.assertIn("VALUE = 2", (self.data_home / "current" / "agent_harness" / "__init__.py").read_text())

    def test_non_symlink_launcher_is_backed_up(self):
        self.bin_home.mkdir(parents=True, exist_ok=True)
        (self.bin_home / "agent-harness").write_text("custom\n", encoding="utf-8")
        report = self.run_install()
        self.assertTrue(any("backed up existing launcher" in item for item in report["actions"]))
        self.assertTrue((self.bin_home / "agent-harness").is_symlink())
        backups = list(self.bin_home.glob("agent-harness.agent-harness.bak.*"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_text(), "custom\n")

    def test_source_manifest_ignores_pycache(self):
        cache = self.source / "agent_harness" / "__pycache__"
        cache.mkdir()
        (cache / "junk.pyc").write_text("junk", encoding="utf-8")
        manifest = source_manifest(self.source)
        self.assertNotIn("agent_harness/__pycache__/junk.pyc", manifest)
        self.assertEqual(len(manifest), 3)

    def test_empty_source_raises(self):
        empty = self.root / "empty"
        empty.mkdir()
        with self.assertRaises(RuntimeError):
            install(empty, data_home=self.data_home, bin_home=self.bin_home)


if __name__ == "__main__":
    unittest.main()
