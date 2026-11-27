"""Bootstrap tests: read-only check, backed-up idempotent apply, fail-soft skips."""

from __future__ import annotations

import json
import unittest
from unittest import mock

import agent_harness.bootstrap as bootstrap_mod
from agent_harness.agents import get_agent
from agent_harness.bootstrap import bootstrap, bootstrap_exit_code, verify_agent
from tests.fakes import FakeRunner, TempEnvTestCase


class BootstrapTests(TempEnvTestCase):
    def setUp(self):
        super().setUp()
        self.claude_cfg = self.root / ".claude.json"
        self.claude_cfg.write_text(
            json.dumps({"mcpServers": {"exa": {"command": "npx"}}, "theme": "dark"}),
            encoding="utf-8",
        )
        self.codex_cfg = self.root / ".codex" / "config.toml"
        self.codex_cfg.parent.mkdir(parents=True)
        self.codex_cfg.write_text('[mcp_servers.exa]\ncommand = "npx"\n', encoding="utf-8")
        self.runner = FakeRunner(which_map={"claude": "/usr/bin/claude", "codex": "/usr/bin/codex"})

    def _by_id(self, report, agent_id):
        return next(item for item in report["agents"] if item["id"] == agent_id)

    def test_check_is_read_only_and_reports_drift(self):
        before = self.claude_cfg.read_bytes()
        report = bootstrap("check", self.runner)
        self.assertTrue(report["read_only"])
        self.assertEqual(report["status"], "ok")
        self.assertEqual(self._by_id(report, "claude")["status"], "needs_change")
        self.assertEqual(self.claude_cfg.read_bytes(), before)
        self.assertEqual(bootstrap_exit_code(report), 1)

    def test_apply_merges_and_backs_up(self):
        report = bootstrap("apply", self.runner)
        self.assertEqual(report["status"], "ok")
        claude = self._by_id(report, "claude")
        self.assertEqual(claude["status"], "applied")
        self.assertTrue(claude["verified"])
        self.assertIsNotNone(claude["backup"])
        data = json.loads(self.claude_cfg.read_text())
        self.assertEqual(data["mcpServers"]["basemind"]["command"], "basemind")
        self.assertEqual(data["mcpServers"]["exa"], {"command": "npx"})
        self.assertEqual(data["theme"], "dark")
        parsed_toml = self.codex_cfg.read_text()
        self.assertIn("[mcp_servers.basemind]", parsed_toml)
        self.assertIn("[mcp_servers.exa]", parsed_toml)

    def test_apply_is_idempotent(self):
        bootstrap("apply", self.runner)
        backups_after_first = list(self.root.glob("**/*.agent-harness.bak.*"))
        second = bootstrap("apply", self.runner)
        self.assertEqual(second["summary"]["applied"], 0)
        for item in second["agents"]:
            self.assertNotIn("backup", item)
            self.assertNotIn("_new_text", item)
        self.assertEqual(list(self.root.glob("**/*.agent-harness.bak.*")), backups_after_first)
        self.assertEqual(bootstrap_exit_code(second), 0)
        self.assertTrue(verify_agent(get_agent("claude")))

    def test_missing_optional_cli_and_no_config_is_skipped(self):
        (self.root / ".config" / "opencode2").mkdir(parents=True, exist_ok=True)
        report = bootstrap("check", FakeRunner())
        opencode2 = self._by_id(report, "opencode2")
        self.assertEqual(opencode2["status"], "skipped_not_installed")
        self.assertFalse(opencode2["change"])

    def test_pi_reports_cli_fallback(self):
        report = bootstrap("check", self.runner)
        pi = self._by_id(report, "pi")
        self.assertEqual(pi["status"], "cli_fallback")
        self.assertIsNone(pi["config_path"])

    def test_check_detects_already_configured(self):
        bootstrap("apply", self.runner)
        report = bootstrap("check", self.runner)
        self.assertEqual(self._by_id(report, "claude")["status"], "already_configured")
        self.assertEqual(self._by_id(report, "codex")["status"], "already_configured")
        self.assertEqual(bootstrap_exit_code(report), 0)

    def test_config_error_does_not_write(self):
        self.claude_cfg.write_text("{broken", encoding="utf-8")
        before = self.claude_cfg.read_bytes()
        report = bootstrap("apply", self.runner)
        self.assertEqual(self._by_id(report, "claude")["status"], "config_error")
        self.assertEqual(self.claude_cfg.read_bytes(), before)

    def test_config_error_counts_as_error(self):
        self.claude_cfg.write_text("{broken", encoding="utf-8")
        report = bootstrap("check", self.runner)
        self.assertEqual(report["status"], "error")
        self.assertGreaterEqual(report["summary"]["errors"], 1)

    def test_invalid_utf8_config_is_config_error(self):
        self.claude_cfg.write_bytes(b"\xff\xfe{")
        report = bootstrap("check", self.runner)
        self.assertEqual(self._by_id(report, "claude")["status"], "config_error")

    def test_conflict_skips_write(self):
        real_plan = bootstrap_mod.plan_agent

        def stale_plan(spec, runner):
            plan = real_plan(spec, runner)
            if spec.id in ("claude", "claude-teams") and plan.get("_new_text"):
                plan["_source_hash"] = "stale-hash"
            return plan

        before = self.claude_cfg.read_bytes()
        with mock.patch.object(bootstrap_mod, "plan_agent", side_effect=stale_plan):
            report = bootstrap("apply", self.runner)
        self.assertEqual(self._by_id(report, "claude")["status"], "conflict")
        self.assertEqual(self.claude_cfg.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
