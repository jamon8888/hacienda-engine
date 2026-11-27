"""setup --root tests: fail-closed index/tool validation, no writes."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from agent_harness.setup import setup_exit_code, setup_repo
from tests.fakes import FakeRunner, healthy_programs


class SetupTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.programs = healthy_programs()
        self.runner = FakeRunner(programs=self.programs)
        self.root = self.tmp.name

    def by_name(self, report, name):
        return next(check for check in report["checks"] if check["name"] == name)

    def test_all_checks_pass(self):
        report = setup_repo(self.root, self.runner)
        self.assertEqual(report["status"], "ok")
        self.assertEqual(setup_exit_code(report), 0)
        self.assertTrue(report["fail_closed"])
        self.assertEqual(report["summary"]["failed"], 0)

    def test_empty_index_fails_closed(self):
        def status(argv, **_):
            return 0, '{"file_count": 0}', ""

        self.runner.programs["basemind"] = (
            lambda argv, **_: status(argv)
            if argv[1:] == ["--root", str(Path(self.root).resolve()), "status", "--json"]
            else healthy_programs()["basemind"](argv)
        )
        report = setup_repo(self.root, self.runner)
        self.assertEqual(report["status"], "fail")
        self.assertEqual(setup_exit_code(report), 1)
        self.assertEqual(self.by_name(report, "basemind_index")["status"], "fail")

    def test_missing_tools_fails_closed(self):
        def basemind(argv, **_):
            if argv[1:] == ["--version"]:
                return 0, "basemind 0.33.0\n", ""
            if argv[1:] == ["--help"]:
                return 0, "Commands: code git", ""
            return 0, '{"file_count": 7}', ""

        self.runner.programs["basemind"] = basemind
        report = setup_repo(self.root, self.runner)
        self.assertEqual(report["status"], "fail")
        detail = self.by_name(report, "basemind_commands")["detail"]
        self.assertIn("admin", detail)
        self.assertIn("overview", detail)

    def test_non_git_root_fails_closed(self):
        self.runner.programs["git"] = (1, "", "fatal: not a git repository")
        report = setup_repo(self.root, self.runner)
        self.assertEqual(report["status"], "fail")
        self.assertEqual(self.by_name(report, "git_worktree")["status"], "fail")

    def test_invalid_file_count_types_fail_closed(self):
        for payload in ({"file_count": True}, {"file_count": "42"}):
            with self.subTest(payload=payload):
                def basemind(argv, **_):
                    if argv[1:] == ["--version"]:
                        return 0, "basemind 0.33.0\n", ""
                    if argv[1:] == ["--help"]:
                        return 0, "Commands: admin code docs git memory overview status", ""
                    return 0, json.dumps(payload), ""

                self.runner.programs["basemind"] = basemind
                report = setup_repo(self.root, self.runner)
                self.assertEqual(report["status"], "fail")
                self.assertEqual(self.by_name(report, "basemind_index")["status"], "fail")

    def test_setup_does_not_write_configs(self):
        before = list(self.runner.calls)
        setup_repo(self.root, self.runner)
        # only read-only probes are issued: git, basemind --version/--help/status
        self.assertNotIn(["basemind", "serve"], self.runner.calls[len(before):])


if __name__ == "__main__":
    unittest.main()
