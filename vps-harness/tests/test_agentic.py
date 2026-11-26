"""Offline unit tests for the hardened agentic pipeline.

Every assertion is deterministic; external processes are never started.
"""

from __future__ import annotations

import contextlib
import io
import json
import random
import unittest
from pathlib import Path
from unittest import mock

from agent_harness import agentic
from agent_harness.cli import build_parser, main
from tests.fakes import FakeRunner, TempEnvTestCase, healthy_programs


def spec(**overrides) -> agentic.TaskSpec:
    values = {"name": "task", "rust": True, "typescript": False}
    values.update(overrides)
    return agentic.TaskSpec(**values)


class RulesAndRolesTests(unittest.TestCase):
    def test_strict_rules_contain_all_hardening(self):
        for snippet in (
            "unwrap()",
            "expect()",
            "panic!()",
            "unsafe",
            "#[allow(...)]",
            "thiserror",
            "any",
            "@ts-ignore",
            "noUncheckedIndexedAccess",
            "proptest",
            "fast-check",
            "toBeDefined()",
            "EXIT_CODE = 0",
        ):
            self.assertIn(snippet, agentic.STRICT_RULES)

    def test_task_spec_validates_inputs(self):
        with self.assertRaises(ValueError):
            agentic.TaskSpec(name="  ")
        with self.assertRaises(ValueError):
            agentic.TaskSpec(name="x", rust=False, typescript=False)
        with self.assertRaises(ValueError):
            agentic.TaskSpec(name="x", tests_paths=())
        with self.assertRaises(ValueError):
            agentic.TaskSpec(name="x", max_iterations=0)

    def test_is_protected_covers_tests_and_test_files(self):
        s = spec()
        for path in ("tests/foo.rs", "src/tests/helper.rs", "foo.test.ts", "foo.spec.ts", "./tests/x.rs"):
            self.assertTrue(agentic.is_protected(path, s), path)
        for path in ("src/lib.rs", "testsx.rs", "src/spect.ts"):
            self.assertFalse(agentic.is_protected(path, s), path)

    def test_role_prompts_separate_test_author_from_implementer(self):
        prompts = agentic.build_prompts(spec())
        tests, impl = prompts["test_agent"], prompts["impl_agent"]
        self.assertIn("ROLE: TEST AUTHOR", tests)
        self.assertIn("You may ONLY create or modify", tests)
        self.assertNotIn("FORBIDDEN: you must not create", tests)
        self.assertIn("ROLE: IMPLEMENTER", impl)
        self.assertIn("FORBIDDEN: you must not create", impl)
        self.assertNotIn("You may ONLY create or modify", impl)
        for prompt in (tests, impl):
            self.assertIn("proptest", prompt)
            self.assertIn("fast-check", prompt)
            self.assertIn("run_gatekeeper", prompt)
            self.assertIn("EXIT_CODE = 0", prompt)


class CircuitBreakerTests(unittest.TestCase):
    def test_fingerprint_ignores_case_whitespace_and_line_numbers(self):
        first = agentic.error_fingerprint("error[E0308]: mismatch at src/a.rs:12:5")
        second = agentic.error_fingerprint("  ERROR[E0308]: mismatch  at src/a.rs:999:5\n")
        self.assertEqual(first, second)
        self.assertNotEqual(first, agentic.error_fingerprint("error[E0382]: moved value"))

    def test_distinct_errors_retry(self):
        breaker = agentic.CircuitBreaker()
        for error in ("error A", "error B", "error C"):
            action = breaker.record_failure(error)
            self.assertEqual(action["state"], "running")
            self.assertEqual(action["action"], "retry_with_fix")

    def test_repeated_error_forces_reflection_then_escalation(self):
        breaker = agentic.CircuitBreaker()
        first = breaker.record_failure("E0277: trait bound not satisfied at line 10")
        self.assertEqual(first["state"], "running")
        second = breaker.record_failure("E0277: trait bound not satisfied at line 77")
        self.assertEqual(second["state"], "reflect")
        self.assertEqual(second["action"], "reflect_without_code")
        third = breaker.record_failure("E0277: trait bound not satisfied at line 999")
        self.assertEqual(third["state"], "escalate")

    def test_budget_caps_at_five_iterations(self):
        breaker = agentic.CircuitBreaker(max_iterations=5)
        actions = [breaker.record_failure(f"distinct error {chr(65 + index)}") for index in range(6)]
        self.assertEqual(actions[4]["iteration"], 5)
        self.assertEqual(actions[4]["state"], "escalate")
        self.assertEqual(actions[5]["iteration"], 5)
        self.assertEqual(actions[5]["state"], "escalate")

    def test_success_is_final(self):
        breaker = agentic.CircuitBreaker()
        breaker.record_failure("one error")
        action = breaker.record_success()
        self.assertEqual(action["state"], "done")
        self.assertEqual(action["action"], "finish")


class CompactionTests(unittest.TestCase):
    def test_compact_cargo_json_lines(self):
        raw = "\n".join(
            [
                '{"reason":"compiler-artifact","package_id":"x"}',
                json.dumps(
                    {
                        "reason": "compiler-message",
                        "message": {
                            "message": "mismatched types",
                            "spans": [{"file_name": "src/a.rs", "line_start": 12, "is_primary": True}],
                        },
                    }
                ),
            ]
        )
        report = agentic.compact_output(raw, "cargo")
        self.assertEqual(report["count"], 1)
        self.assertEqual(report["findings"][0]["file"], "src/a.rs")
        self.assertEqual(report["findings"][0]["line"], 12)
        self.assertIn("src/a.rs:12: mismatched types", report["text"])

    def test_compact_vitest_json(self):
        raw = json.dumps(
            {
                "testResults": [
                    {
                        "name": "tests/a.test.ts",
                        "assertionResults": [
                            {"fullName": "adds", "failureMessages": ["expected 3, got 4"]}
                        ],
                    }
                ]
            }
        )
        report = agentic.compact_output(raw, "vitest")
        self.assertEqual(report["findings"][0]["file"], "tests/a.test.ts")
        self.assertEqual(report["findings"][0]["message"], "expected 3, got 4")

    def test_compact_stryker_json(self):
        raw = json.dumps(
            {
                "mutants": [
                    {
                        "fileName": "src/a.ts",
                        "status": "Survived",
                        "location": {"start": {"line": 7}},
                    }
                ]
            }
        )
        report = agentic.compact_output(raw, "stryker")
        self.assertEqual(report["findings"][0]["line"], 7)
        self.assertIn("Survived", report["findings"][0]["message"])

    def test_compact_fallback_and_limit(self):
        report = agentic.compact_output("plain clippy output\nsecond line", "cargo")
        self.assertEqual(report["count"], 2)
        self.assertEqual(report["findings"][0]["message"], "plain clippy output")
        limited = agentic.compact_output("\n".join(f"line {i}" for i in range(50)), "cargo", limit=3)
        self.assertEqual(limited["count"], 3)
        with self.assertRaises(ValueError):
            agentic.compact_output("", "unknown")


class PlanTests(unittest.TestCase):
    def test_gatekeeper_steps_are_diff_targeted(self):
        steps = agentic.gatekeeper_steps(spec(typescript=True))
        commands = {step["name"]: step for step in steps}
        self.assertIn("--in-diff git-diff.patch", commands["mutation-rust"]["command"])
        self.assertEqual(commands["mutation-rust"]["scope"], "diff")
        self.assertIn("--mutate", commands["mutation-ts"]["command"])
        self.assertEqual(commands["mutation-ts"]["scope"], "diff")
        for name in ("static-rust", "lint-rust", "unit-rust", "static-ts", "lint-ts", "unit-ts"):
            self.assertIn(name, commands)

    def test_rust_only_plan_has_no_typescript_steps(self):
        steps = agentic.gatekeeper_steps(spec())
        self.assertEqual(len(steps), 4)
        self.assertFalse(any("ts" in step["name"] for step in steps))

    def test_plan_shape_and_tools(self):
        result = agentic.plan(spec(typescript=True))
        self.assertEqual(result["schema"], "agent-harness/agentic-plan/1")
        self.assertFalse(result["isolation"]["same_agent_writes_code_and_tests"])
        self.assertEqual(result["gatekeeper"]["exit_code_required"], 0)
        self.assertEqual(len(result["rules_sha256"]), 64)
        self.assertEqual(set(result["tools"]), {"read_file", "apply_patch", "run_gatekeeper"})
        self.assertIn("EXIT_CODE", result["tools"]["run_gatekeeper"]["returns"])
        self.assertEqual(result["log_compaction"]["max_findings"], 20)


class PropertyInvariantTests(unittest.TestCase):
    def test_fingerprint_stable_under_random_line_numbers(self):
        rng = random.Random(20261126)
        for _ in range(100):
            line = rng.randint(1, 10_000)
            first = agentic.error_fingerprint(f"error[E0277]: trait bound at src/a.rs:{line}:3")
            shifted = agentic.error_fingerprint(
                f"  ERROR[E0277]: trait bound at src/a.rs:{line * 7}:99  "
            )
            self.assertEqual(first, shifted)

    def test_breaker_never_exceeds_budget(self):
        rng = random.Random(42)
        for _ in range(100):
            breaker = agentic.CircuitBreaker(max_iterations=5)
            for index in range(rng.randint(1, 15)):
                action = breaker.record_failure(f"error kind {rng.randint(0, 3)} #{index}")
                self.assertLessEqual(action["iteration"], 5)
                self.assertIn(action["state"], {"running", "reflect", "escalate"})


class CLITests(TempEnvTestCase):
    def _run(self, argv, fake=None):
        with mock.patch("agent_harness.cli.SubprocessRunner", return_value=fake or FakeRunner()):
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                code = main(argv)
        return code, buffer.getvalue()

    def test_parser_accepts_agentic_subcommands(self):
        parser = build_parser()
        self.assertEqual(parser.parse_args(["agentic", "rules", "--json"]).agentic_command, "rules")
        plan_args = parser.parse_args(["agentic", "plan", "--spec-file", "s.json", "--json"])
        self.assertEqual(plan_args.spec_file, "s.json")
        self.assertEqual(
            parser.parse_args(["agentic", "compact", "--kind", "cargo", "--input", "f"]).kind,
            "cargo",
        )

    def test_rules_dispatch(self):
        code, output = self._run(["agentic", "rules", "--json"])
        payload = json.loads(output)
        self.assertEqual(code, 0)
        self.assertIn("unwrap()", payload["rules"])

    def test_plan_dispatch(self):
        spec_file = self.root / "spec.json"
        spec_file.write_text(json.dumps({"name": "unit", "rust": True}), encoding="utf-8")
        code, output = self._run(["agentic", "plan", "--spec-file", str(spec_file), "--json"])
        payload = json.loads(output)
        self.assertEqual(code, 0)
        self.assertEqual(payload["task"]["name"], "unit")

    def test_compact_dispatch(self):
        raw = self.root / "raw.txt"
        raw.write_text('{"message":{"message":"oops","spans":[{"file_name":"a.rs","line_start":1}]}}', encoding="utf-8")
        code, output = self._run(["agentic", "compact", "--kind", "cargo", "--input", str(raw), "--json"])
        payload = json.loads(output)
        self.assertEqual(code, 0)
        self.assertEqual(payload["findings"][0]["file"], "a.rs")

    def test_gatekeeper_print_plan_dispatch(self):
        fake = FakeRunner(programs=healthy_programs())
        code, output = self._run(["agentic", "gatekeeper", "--root", str(self.root), "--print-plan", "--json"], fake)
        payload = json.loads(output)
        self.assertEqual(code, 0)
        self.assertEqual(payload["EXIT_CODE"], 0)
        self.assertIn("--root", payload["argv"])


if __name__ == "__main__":
    unittest.main()
