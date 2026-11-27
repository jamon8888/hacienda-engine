"""The real runner validates argv and never invokes a shell."""

from __future__ import annotations

import unittest
from types import SimpleNamespace
from unittest import mock

from agent_harness.runners import SubprocessRunner


class SubprocessRunnerTests(unittest.TestCase):
    def test_rejects_non_vector_argv(self):
        runner = SubprocessRunner()
        for bad in ("echo hi", [], None, ["echo", 1]):
            with self.subTest(argv=bad), self.assertRaises(ValueError):
                runner.run(bad)  # type: ignore[arg-type]

    def test_passes_argv_vector_without_shell(self):
        with mock.patch("agent_harness.runners.subprocess.run") as run:
            run.return_value = SimpleNamespace(returncode=0, stdout="ok\n", stderr="")
            code, out, err = SubprocessRunner().run(["basemind", "--version"])

        self.assertEqual((code, out, err), (0, "ok\n", ""))
        args, kwargs = run.call_args
        self.assertEqual(args[0], ["basemind", "--version"])
        self.assertIs(kwargs["shell"], False)

    def test_missing_command_is_reported_not_raised(self):
        with mock.patch("agent_harness.runners.subprocess.run", side_effect=FileNotFoundError):
            code, out, err = SubprocessRunner().run(["absent-tool"])
        self.assertEqual((code, out), (127, ""))
        self.assertIn("absent-tool", err)


if __name__ == "__main__":
    unittest.main()
