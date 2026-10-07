#!/usr/bin/env python3
"""Negative probes for scripts/checks/check_client_plugin.py: each rule is shown to reject a broken copy."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

REPO = os.path.realpath(os.path.join(os.path.dirname(__file__), "..", ".."))
GATE = os.path.join(REPO, "scripts/checks/check_client_plugin.py")
COPIED = ["plugins/local-app", ".agents", ".claude-plugin", "crates/local-app-cli/src/mcp_backend.rs",
          "crates/local-app-service/src/tool_names.rs"]


class ClientPlugin(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.root, True)
        for rel in COPIED:
            src, dst = os.path.join(REPO, rel), os.path.join(self.root, rel)
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            (shutil.copytree if os.path.isdir(src) else shutil.copy)(src, dst)

    def path(self, rel):
        return os.path.join(self.root, rel)

    def edit(self, rel, change):
        with open(self.path(rel), encoding="utf-8") as f:
            text = f.read()
        new = change(text)
        self.assertNotEqual(new, text, "the probe changed nothing in " + rel)
        with open(self.path(rel), "w", encoding="utf-8") as f:
            f.write(new)

    def edit_json(self, rel, change):
        with open(self.path(rel), encoding="utf-8") as f:
            doc = json.load(f)
        change(doc)
        with open(self.path(rel), "w", encoding="utf-8") as f:
            f.write(json.dumps(doc, indent=2) + "\n")

    def gate(self):
        return subprocess.run([sys.executable, GATE, self.root], text=True, capture_output=True)

    def rejects(self, *fragments):
        result = self.gate()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        for fragment in fragments:
            self.assertIn(fragment, result.stderr)

    def test_the_tree_as_committed_passes(self):
        result = self.gate()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_manifests_that_disagree_are_rejected(self):
        self.edit_json("plugins/local-app/.claude-plugin/plugin.json", lambda d: d.update(version="0.2.0"))
        self.rejects("the manifests disagree on version")

    def test_a_manifest_without_a_semver_version_or_with_a_bad_name_is_rejected(self):
        self.edit_json("plugins/local-app/plugin.json", lambda d: d.update(version="1.0", name="Local App"))
        self.rejects("not kebab-case", "not major.minor.patch")

    def test_the_two_mcp_files_must_be_one_document(self):
        self.edit_json("plugins/local-app/.mcp.json", lambda d: d["mcpServers"]["local-app"].update(args=["mcp", "--x"]))
        self.rejects("mcp.json and .mcp.json differ")

    def test_the_server_must_be_the_bare_command(self):
        for rel in ("plugins/local-app/mcp.json", "plugins/local-app/.mcp.json"):
            self.edit_json(rel, lambda d: d["mcpServers"]["local-app"].update(command="${CLAUDE_PLUGIN_ROOT}/bin/local-app"))
        self.rejects("must be {type: stdio, command: local-app, args: [mcp]}", "placeholder")

    def test_a_marketplace_that_points_elsewhere_is_rejected(self):
        self.edit_json(".claude-plugin/marketplace.json", lambda d: d["plugins"][0].update(source="./plugins/other"))
        self.rejects(".claude-plugin/marketplace.json: expected one plugin")
        self.edit_json(".agents/plugins/marketplace.json", lambda d: d["plugins"][0]["source"].update(path="./nope"))
        self.rejects(".agents/plugins/marketplace.json: expected one plugin")

    def test_a_skill_whose_name_is_not_its_directory_is_rejected(self):
        self.edit("plugins/local-app/skills/local-app-setup/SKILL.md", lambda t: t.replace("name: local-app-setup", "name: setup", 1))
        self.rejects("must equal the directory")

    def test_a_skill_directory_without_a_skill_file_is_rejected(self):
        os.remove(self.path("plugins/local-app/skills/local-app-setup/SKILL.md"))
        self.rejects("is missing")

    def test_a_tool_that_does_not_exist_is_rejected(self):
        self.edit("plugins/local-app/skills/local-app/SKILL.md", lambda t: t + "\nCall `LocalAppFrobnicate` next.\n")
        self.rejects("`LocalAppFrobnicate`, which is not a tool of the service")

    def test_a_tool_the_server_does_not_offer_yet_is_rejected(self):
        self.edit("plugins/local-app/skills/local-app/SKILL.md", lambda t: t + "\nThen run `LocalAppRuntime`.\n")
        self.rejects("`LocalAppRuntime`, which exists but is not offered")

    def test_an_offered_tool_the_skill_does_not_mention_is_rejected(self):
        self.edit("plugins/local-app/skills/local-app/SKILL.md", lambda t: t.replace("LocalAppCheckpointList", "the checkpoint tool"))
        self.rejects("does not mention the offered tool `LocalAppCheckpointList`")

    def test_a_placeholder_in_a_skill_is_rejected(self):
        self.edit("plugins/local-app/skills/local-app-setup/SKILL.md", lambda t: t + "\nSee ${CLAUDE_PLUGIN_ROOT}/x.\n")
        self.rejects("uses a ${...} placeholder")

    def test_the_engines_product_name_is_rejected_but_the_real_names_are_not(self):
        self.edit("plugins/local-app/skills/local-app-setup/SKILL.md", lambda t: t + "\nThe app talks to window.lingxi.v2.\nRead LINGXI.md first.\n")
        self.assertEqual(self.gate().returncode, 0, "window.lingxi and LINGXI.md are real names")
        self.edit("plugins/local-app/skills/local-app-setup/SKILL.md", lambda t: t + "\nAsk the LingXi app to do it.\n")
        self.rejects("names the engine's product")

    def test_a_server_list_with_an_operation_that_has_no_tool_row_is_rejected(self):
        self.edit("crates/local-app-cli/src/mcp_backend.rs", lambda t: t.replace('    "background_status",', '    "background_status",\n    "no_such_operation",', 1))
        self.rejects("SERVED names operations with no tool row")

    def test_a_read_list_naming_a_writing_operation_is_rejected(self):
        self.edit("crates/local-app-cli/src/mcp_backend.rs", lambda t: t.replace('    "background_status",', '    "background_status",\n    "build",', 1))
        self.rejects("SERVED_READ names operations that are not read-only: build")


if __name__ == "__main__":
    unittest.main()
