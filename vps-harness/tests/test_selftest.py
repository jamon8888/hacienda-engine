"""Self-test: live proof or actionable unsupported, never static detection as proof."""

from __future__ import annotations

import os
import unittest
from unittest import mock

from agent_harness.selftest import run_self_test, selftest_exit_code
from tests.fakes import FakeRunner, TempEnvTestCase, healthy_programs


class SelfTestTests(TempEnvTestCase):
    def setUp(self):
        super().setUp()
        self.runner = FakeRunner(
            programs=healthy_programs(),
            which_map={"claude": "/usr/bin/claude", "pi": "/usr/bin/pi"},
        )

    def test_unsupported_without_live_prover(self):
        report = run_self_test("claude", self.runner)
        self.assertEqual(report["status"], "unsupported")
        self.assertEqual(selftest_exit_code(report), 3)
        self.assertFalse(report["static_detection_is_proof"])
        self.assertFalse(report["static_detection"]["is_proof"])
        self.assertIsNone(report["live_proof"])
        self.assertIn("NOT proof", report["message"])
        self.assertGreaterEqual(len(report["manual_steps"]), 2)
        self.assertIn("basemind", " ".join(report["manual_steps"]))

    def test_unsupported_static_signals_do_not_change_status(self):
        # CLI installed and integration ok in static detection, still unsupported.
        self.runner.programs["basemind"] = (0, "basemind 0.33.0\n", "")
        report = run_self_test("pi", self.runner)
        self.assertEqual(report["static_detection"]["cli"]["status"], "installed")
        self.assertEqual(report["status"], "unsupported")

    def test_injected_prover_can_prove(self):
        report = run_self_test("codex", self.runner, prover=lambda _id: {"proven": True, "evidence": "tool call seen"})
        self.assertEqual(report["status"], "ok")
        self.assertEqual(selftest_exit_code(report), 0)
        self.assertEqual(report["live_proof"]["evidence"], "tool call seen")

    def test_prover_saying_false_is_still_unsupported(self):
        report = run_self_test("opencode", self.runner, prover=lambda _id: {"proven": False, "error": "no tool call"})
        self.assertEqual(report["status"], "unsupported")
        self.assertIn("no tool call", report["message"])

    def test_env_prover_missing_reports_unsupported(self):
        with mock.patch.dict(os.environ, {"AGENT_HARNESS_SELFTEST_PROVER": "/no/such/prover"}):
            report = run_self_test("pi", self.runner)
        self.assertEqual(report["status"], "unsupported")
        self.assertIn("prover not found", report["message"])

    def test_unknown_agent_raises(self):
        with self.assertRaises(ValueError):
            run_self_test("gemini", self.runner)


if __name__ == "__main__":
    unittest.main()
