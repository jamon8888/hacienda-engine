"""Pure-function tests for JSON/JSONC/TOML merging and safe writes."""

from __future__ import annotations

import datetime
import json
import unittest
from pathlib import Path
from unittest import mock

from agent_harness.merge import (
    ConfigError,
    backup_and_write,
    entry_satisfied,
    loads_json_config,
    merge_json_config,
    merge_toml_config,
    parse_toml,
    remove_toml_table,
    strip_jsonc_comments,
    strip_trailing_commas,
)
from tests.fakes import TempEnvTestCase

CLAUDE_ENTRY = {"type": "stdio", "command": "basemind", "args": ["serve"], "env": {}}
OPENCODE_ENTRY = {"type": "local", "command": ["basemind", "serve"], "enabled": True}
CODEX_ENTRY = {"command": "basemind", "args": ["serve"]}


class JsonHelpersTests(unittest.TestCase):
    def test_strip_jsonc_comments_preserves_string_literals(self):
        text = '{ "url": "https://x/y//z", /* block */ "n": 1 // line\n }'
        stripped = strip_jsonc_comments(text)
        self.assertIn("https://x/y//z", stripped)
        self.assertNotIn("block", stripped)
        self.assertEqual(loads_json_config(text, jsonc=True)["n"], 1)

    def test_strip_trailing_commas(self):
        self.assertEqual(strip_trailing_commas('{"a":1,}'), '{"a":1}')
        self.assertEqual(strip_trailing_commas('{"a":"x,}"}'), '{"a":"x,}"}')

    def test_merge_json_adds_only_desired_key(self):
        original = json.dumps({"mcpServers": {"exa": {"command": "npx"}}, "other": 1}, indent=2)
        merged = merge_json_config(original, "mcpServers", CLAUDE_ENTRY)
        assert merged is not None
        data = json.loads(merged)
        self.assertEqual(data["mcpServers"]["basemind"], CLAUDE_ENTRY)
        self.assertEqual(data["mcpServers"]["exa"], {"command": "npx"})
        self.assertEqual(data["other"], 1)

    def test_merge_json_idempotent(self):
        data = {"mcp": {"basemind": OPENCODE_ENTRY, "exa": {"type": "local"}}}
        text = json.dumps(data)
        self.assertIsNone(merge_json_config(text, "mcp", OPENCODE_ENTRY, jsonc=True))

    def test_entry_satisfied_ignores_user_env(self):
        current = dict(CLAUDE_ENTRY, env={"SECRET": "x"})
        self.assertTrue(entry_satisfied(current, CLAUDE_ENTRY))
        self.assertFalse(entry_satisfied({"command": "other", "args": ["serve"]}, CODEX_ENTRY))
        self.assertFalse(entry_satisfied(None, CODEX_ENTRY))

    def test_merge_json_creates_missing_container(self):
        merged = merge_json_config('{"x": 1}', "mcpServers", CLAUDE_ENTRY)
        assert merged is not None
        self.assertEqual(json.loads(merged)["mcpServers"]["basemind"], CLAUDE_ENTRY)

    def test_merge_json_preserves_custom_fields_when_repairing(self):
        current = {"type": "stdio", "command": "old", "args": ["x"], "env": {"KEY": "v"}, "note": "keep"}
        merged = merge_json_config(json.dumps({"mcpServers": {"basemind": current}}), "mcpServers", CLAUDE_ENTRY)
        assert merged is not None
        entry = json.loads(merged)["mcpServers"]["basemind"]
        self.assertEqual(entry["command"], "basemind")
        self.assertEqual(entry["args"], ["serve"])
        self.assertEqual(entry["env"], {"KEY": "v"})
        self.assertEqual(entry["note"], "keep")

    def test_merge_json_bad_config_raises(self):
        with self.assertRaises(ConfigError):
            merge_json_config("{not json", "mcpServers", CLAUDE_ENTRY)


class TomlHelpersTests(unittest.TestCase):
    BASE = """\
# keep this comment
[mcp_servers.exa]
command = "npx"

[mcp_servers.basemind]
command = "basemind"
args = ["serve"]

[mcp_servers.playwright]
command = "npx"
"""

    def test_merge_toml_adds_table_preserving_others(self):
        text = "[mcp_servers.exa]\ncommand = \"npx\"\n"
        merged = merge_toml_config(text, "mcp_servers.basemind", CODEX_ENTRY)
        assert merged is not None
        parsed = parse_toml(merged)
        self.assertEqual(parsed["mcp_servers"]["basemind"], CODEX_ENTRY)
        self.assertEqual(parsed["mcp_servers"]["exa"]["command"], "npx")

    def test_merge_toml_updates_wrong_entry_and_drops_subtables(self):
        text = (
            "[mcp_servers.basemind]\ncommand = \"old\"\n"
            "[mcp_servers.basemind.env]\nTOKEN = \"x\"\n"
            "[mcp_servers.exa]\ncommand = \"npx\"\n"
        )
        merged = merge_toml_config(text, "mcp_servers.basemind", CODEX_ENTRY)
        assert merged is not None
        parsed = parse_toml(merged)
        self.assertEqual(parsed["mcp_servers"]["basemind"], CODEX_ENTRY)
        self.assertNotIn("env", parsed["mcp_servers"]["basemind"])
        self.assertIn("exa", parsed["mcp_servers"])

    def test_merge_toml_idempotent(self):
        self.assertIsNone(merge_toml_config(self.BASE, "mcp_servers.basemind", CODEX_ENTRY))

    def test_remove_toml_table_handles_inline_definition(self):
        text = "[mcp_servers]\nbasemind = { command = \"old\" }\nexa = { command = \"npx\" }\n"
        cleaned = remove_toml_table(text, "mcp_servers.basemind")
        self.assertNotIn("basemind", cleaned)
        self.assertIn("exa", cleaned)

    def test_merge_toml_keeps_table_with_trailing_comment_header(self):
        text = (
            "[mcp_servers.basemind]\ncommand = \"old\"\n"
            "[mcp_servers.exa] # keep this table\ncommand = \"npx\"\n"
        )
        merged = merge_toml_config(text, "mcp_servers.basemind", CODEX_ENTRY)
        assert merged is not None
        parsed = parse_toml(merged)
        self.assertEqual(parsed["mcp_servers"]["basemind"], CODEX_ENTRY)
        self.assertEqual(parsed["mcp_servers"]["exa"]["command"], "npx")

    def test_merge_toml_bad_config_raises(self):
        with self.assertRaises(ConfigError):
            merge_toml_config("not = = toml", "mcp_servers.basemind", CODEX_ENTRY)


class SafeWriteTests(TempEnvTestCase):
    def test_backup_and_write_creates_backup_once(self):
        path = self.root / "config.json"
        path.write_text("original\n", encoding="utf-8")
        backup = backup_and_write(path, "updated\n")
        self.assertIsNotNone(backup)
        self.assertEqual(path.read_text(), "updated\n")
        self.assertEqual(Path(backup).read_text(), "original\n")
        self.assertFalse(Path(backup).read_text().endswith("updated"))

    def test_backup_paths_are_unique_within_same_second(self):
        path = self.root / "config.json"
        path.write_text("v0\n", encoding="utf-8")
        with mock.patch("agent_harness.merge._dt.datetime") as dt:
            dt.now.return_value = datetime.datetime(2026, 1, 1, 12, 0, 0)
            first = backup_and_write(path, "v1\n")
            second = backup_and_write(path, "v2\n")
        self.assertIsNotNone(first)
        self.assertIsNotNone(second)
        self.assertNotEqual(first, second)
        self.assertEqual(Path(first).read_text(), "v0\n")
        self.assertEqual(Path(second).read_text(), "v1\n")


if __name__ == "__main__":
    unittest.main()
