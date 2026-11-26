"""Doctor tests: required tools gate status; optional CLIs never fail it."""

from __future__ import annotations

import unittest
from pathlib import Path

from agent_harness.health import doctor, doctor_exit_code, parse_version
from tests.fakes import FakeRunner, TempEnvTestCase, healthy_programs


class VersionParseTests(unittest.TestCase):
    def test_parse_version(self):
        self.assertEqual(parse_version("basemind 0.33.0"), (0, 33, 0))
        self.assertEqual(parse_version("orca 1.4.263"), (1, 4, 263))
        self.assertIsNone(parse_version("no version here"))


class DoctorTests(TempEnvTestCase):
    def test_missing_optional_clis_do_not_fail_doctor(self):
        runner = FakeRunner(programs=healthy_programs(), which_map={"claude": "/usr/bin/claude"})
        report = doctor(runner)
        self.assertEqual(report["status"], "ok")
        self.assertEqual(doctor_exit_code(report), 0)
        statuses = {a["id"]: a["cli"]["status"] for a in report["agents"]}
        self.assertEqual(statuses["claude"], "installed")
        self.assertEqual(statuses["pi"], "not_installed")
        self.assertGreaterEqual(report["summary"]["not_installed"], 4)
        self.assertIn("do not fail", report["note"])

    def test_missing_basemind_fails_doctor(self):
        programs = healthy_programs()
        del programs["basemind"]
        report = doctor(FakeRunner(programs=programs, which_map={"pi": "/usr/bin/pi"}))
        self.assertEqual(report["status"], "fail")
        self.assertEqual(doctor_exit_code(report), 1)
        self.assertEqual(report["required"]["basemind"]["status"], "missing")

    def test_old_orca_degrades_but_does_not_fail(self):
        programs = healthy_programs()
        programs["orca"] = (0, "1.4.100\n", "")
        report = doctor(FakeRunner(programs=programs))
        self.assertEqual(report["status"], "degraded")
        self.assertEqual(report["required"]["orca"]["status"], "version_mismatch")

    def test_wrong_basemind_version_is_reported(self):
        programs = healthy_programs("basemind 0.32.0")
        report = doctor(FakeRunner(programs=programs))
        self.assertEqual(report["required"]["basemind"]["status"], "version_mismatch")

    def test_nonzero_version_probe_exit_is_error(self):
        programs = healthy_programs()
        programs["basemind"] = (1, "basemind 0.33.0\n", "probe failed")
        report = doctor(FakeRunner(programs=programs))
        self.assertEqual(report["required"]["basemind"]["status"], "error")
        self.assertEqual(report["status"], "fail")

    def test_invalid_utf8_config_is_config_error(self):
        (self.root / ".claude.json").write_bytes(b"\xff\xfe{")
        report = doctor(FakeRunner(programs=healthy_programs()))
        claude = next(agent for agent in report["agents"] if agent["id"] == "claude")
        self.assertEqual(claude["integration"]["status"], "config_error")

    def test_pi_integration_is_cli_fallback_status(self):
        report = doctor(FakeRunner(programs=healthy_programs(), which_map={"pi": "/usr/bin/pi"}))
        pi = next(agent for agent in report["agents"] if agent["id"] == "pi")
        self.assertEqual(pi["integration"]["mode"], "cli_fallback")
        self.assertEqual(pi["integration"]["status"], "ok")


if __name__ == "__main__":
    unittest.main()
