"""Tests for the six-agent registry, path resolution and CLI detection."""

from __future__ import annotations

import unittest

from agent_harness.agents import (
    AGENT_IDS,
    AGENTS,
    FALLBACK_MODE,
    NATIVE_MODE,
    config_paths,
    detect_cli,
    get_agent,
    resolve_config_path,
)
from tests.fakes import FakeRunner, TempEnvTestCase


class RegistryTests(unittest.TestCase):
    def test_six_agents_with_expected_modes(self):
        self.assertEqual(
            tuple(AGENT_IDS),
            ("claude", "claude-teams", "codex", "opencode", "opencode2", "pi"),
        )
        by_id = {spec.id: spec for spec in AGENTS}
        self.assertEqual(by_id["pi"].mode, FALLBACK_MODE)
        for agent_id in ("claude", "claude-teams", "codex", "opencode", "opencode2"):
            self.assertEqual(by_id[agent_id].mode, NATIVE_MODE)

    def test_get_agent_rejects_unknown(self):
        with self.assertRaises(ValueError):
            get_agent("gemini")

    def test_desired_entries_match_native_shapes(self):
        self.assertEqual(get_agent("codex").desired_entry, {"command": "basemind", "args": ["serve"]})
        self.assertEqual(
            get_agent("opencode").desired_entry,
            {"type": "local", "command": ["basemind", "serve"], "enabled": True},
        )


class PathAndDetectionTests(TempEnvTestCase):
    def test_config_paths_resolve_xdg_and_home(self):
        self.assertEqual(
            str(config_paths(get_agent("claude"))[0]), str(self.root / ".claude.json")
        )
        self.assertEqual(
            str(config_paths(get_agent("opencode2"))[0]),
            str(self.root / ".config" / "opencode2" / "opencode.json"),
        )

    def test_resolve_config_path_prefers_existing_candidate(self):
        jsonc = self.root / ".config" / "opencode" / "opencode.jsonc"
        jsonc.parent.mkdir(parents=True)
        jsonc.write_text("{}", encoding="utf-8")
        self.assertEqual(resolve_config_path(get_agent("opencode")), jsonc)

    def test_cli_detection_uses_first_binary(self):
        runner = FakeRunner(which_map={"claude": "/usr/bin/claude"})
        result = detect_cli(get_agent("claude-teams"), runner)
        self.assertEqual(result["status"], "installed")
        self.assertEqual(result["binary"], "claude")
        self.assertIn("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", result["note"])

    def test_cli_detection_not_installed(self):
        result = detect_cli(get_agent("pi"), FakeRunner())
        self.assertEqual(result["status"], "not_installed")
        self.assertIsNone(result["path"])


if __name__ == "__main__":
    unittest.main()
