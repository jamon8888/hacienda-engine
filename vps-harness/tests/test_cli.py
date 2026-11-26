"""CLI parsing and dispatch tests (no real subprocesses)."""

from __future__ import annotations

import contextlib
import io
import json
import unittest
from unittest import mock

from agent_harness.cli import build_parser, main
from tests.fakes import FakeRunner, TempEnvTestCase, healthy_programs


class ParserTests(unittest.TestCase):
    def test_each_command_parses(self):
        parser = build_parser()
        self.assertEqual(parser.parse_args(["doctor", "--json"]).command, "doctor")
        apply_args = parser.parse_args(["bootstrap", "--apply", "--json"])
        self.assertTrue(apply_args.apply)
        self.assertEqual(parser.parse_args(["setup", "--root", "/repo"]).root, "/repo")
        self.assertEqual(parser.parse_args(["self-test", "--agent", "pi"]).agent, "pi")
        self.assertEqual(parser.parse_args(["install"]).command, "install")

    def test_bootstrap_requires_exactly_one_mode(self):
        parser = build_parser()
        with self.assertRaises(SystemExit):
            parser.parse_args(["bootstrap"])
        with self.assertRaises(SystemExit):
            parser.parse_args(["bootstrap", "--check", "--apply"])


class DispatchTests(TempEnvTestCase):
    def _run(self, argv, fake):
        with mock.patch("agent_harness.cli.SubprocessRunner", return_value=fake):
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                code = main(argv)
        return code, buffer.getvalue()

    def test_doctor_json_dispatch(self):
        fake = FakeRunner(programs=healthy_programs(), which_map={"pi": "/usr/bin/pi"})
        code, output = self._run(["doctor", "--json"], fake)
        payload = json.loads(output)
        self.assertEqual(code, 0)
        self.assertEqual(payload["schema"], "agent-harness/doctor/1")
        self.assertEqual(payload["status"], "ok")

    def test_selftest_unsupported_exit_code(self):
        fake = FakeRunner(programs=healthy_programs())
        code, output = self._run(["self-test", "--agent", "pi", "--json"], fake)
        payload = json.loads(output)
        self.assertEqual(code, 3)
        self.assertEqual(payload["status"], "unsupported")


if __name__ == "__main__":
    unittest.main()
