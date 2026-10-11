"""Negative probes for the plugin gates: every rule must fail on the defect it names.

Each case copies the plugin and the marketplace into a scratch repository, plants one defect, and runs the gate
against it with `--repo`. A gate that passes the planted defect is not a gate.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[2]
CHECK_PLUGIN = REPO / "scripts" / "checks" / "check-plugin.py"
CHECK_FRONTMATTER = REPO / "scripts" / "checks" / "check_skill_frontmatter.py"
PLUGIN = Path("plugins") / "local-app-builder"
TEMPLATES = PLUGIN / "assets" / "templates"
FAMILIES = ["react-dom", "canvas-2d", "three-3d", "phaser-2d", "babylon-3d"]


class ScratchRepo(unittest.TestCase):
    def setUp(self) -> None:
        self.scratch = Path(tempfile.mkdtemp(prefix="plugin-gate-"))
        self.addCleanup(shutil.rmtree, self.scratch)
        shutil.copytree(REPO / ".lingxi-plugin", self.scratch / ".lingxi-plugin")
        shutil.copytree(REPO / PLUGIN, self.scratch / PLUGIN)

    def path(self, relative: str | Path) -> Path:
        return self.scratch / relative

    def edit(self, relative: str | Path, old: str, new: str) -> None:
        path = self.path(relative)
        text = path.read_text(encoding="utf-8")
        self.assertIn(old, text, f"probe precondition: {old!r} not in {relative}")
        path.write_text(text.replace(old, new), encoding="utf-8")

    def edit_json(self, relative: str | Path, change) -> None:
        path = self.path(relative)
        value = json.loads(path.read_text(encoding="utf-8"))
        change(value)
        path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")

    def run_gate(self, gate: Path) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(gate), "--repo", str(self.scratch)], capture_output=True, text=True, check=False
        )

    def assert_fails(self, gate: Path, expected: str) -> None:
        result = self.run_gate(gate)
        self.assertEqual(result.returncode, 1, f"gate passed a planted defect:\n{result.stdout}{result.stderr}")
        self.assertIn(expected, result.stderr)


class CheckPluginTest(ScratchRepo):
    def test_clean_copy_passes(self) -> None:
        result = self.run_gate(CHECK_PLUGIN)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PLUGIN OK", result.stdout)

    def test_manifest_component_key_fails(self) -> None:
        self.edit_json(PLUGIN / ".lingxi-plugin" / "plugin.json", lambda value: value.update(agents="./agents/"))
        self.assert_fails(CHECK_PLUGIN, "plugin manifest declares ['agents']")

    def test_marketplace_old_source_fails(self) -> None:
        def move(value):
            value["plugins"][0]["source"] = "./crates/plugins/local-app-builder"
        self.edit_json(".lingxi-plugin/marketplace.json", move)
        self.assert_fails(CHECK_PLUGIN, "marketplace must list exactly")

    def test_extra_plugin_component_directory_fails(self) -> None:
        (self.path(PLUGIN / "workflows")).mkdir()
        self.assert_fails(CHECK_PLUGIN, "extra=['workflows']")

    def test_extra_skill_fails(self) -> None:
        shutil.copytree(self.path(PLUGIN / "skills" / "accessibility"), self.path(PLUGIN / "skills" / "local-app-run"))
        self.assert_fails(CHECK_PLUGIN, "extra=['local-app-run']")

    def test_build_output_inside_plugin_fails(self) -> None:
        (self.path(TEMPLATES / "react-dom" / "r4" / "node_modules" / "react")).mkdir(parents=True)
        self.assert_fails(CHECK_PLUGIN, "build output or local junk inside the plugin")

    def test_shared_file_drift_fails(self) -> None:
        self.edit(TEMPLATES / "three-3d" / "r4" / "lib" / "device-context.js", '"desktop"', '"windows"')
        self.assert_fails(CHECK_PLUGIN, "shared file lib/device-context.js differs from react-dom in ['three-3d']")

    def test_lost_file_url_setting_fails(self) -> None:
        for family in FAMILIES:
            self.edit(
                TEMPLATES / family / "r4" / "vite.config.mjs",
                "assetsInlineLimit: () => true,",
                'assetsInlineLimit: (filePath) => (filePath.endsWith(".wasm") ? true : undefined),',
            )
        self.assert_fails(CHECK_PLUGIN, "vite.config.mjs lost 'assetsInlineLimit: () => true'")

    def test_host_bridge_reference_fails(self) -> None:
        path = self.path(PLUGIN / "skills" / "react-best-practices" / "SKILL.md")
        path.write_text(path.read_text(encoding="utf-8") + "\nCall `window.lingxi.v2.data.mutate`.\n", encoding="utf-8")
        self.assert_fails(CHECK_PLUGIN, "refers to the removed Local App host ('window.lingxi')")

    def test_host_tool_reference_fails(self) -> None:
        path = self.path(PLUGIN / "skills" / "create-local-app" / "SKILL.md")
        path.write_text(path.read_text(encoding="utf-8") + "\nThen call `LocalAppBuild`.\n", encoding="utf-8")
        self.assert_fails(CHECK_PLUGIN, "('LocalAppBuild')")

    def test_plugin_root_token_in_skill_fails(self) -> None:
        self.edit(
            PLUGIN / "skills" / "create-local-app" / "SKILL.md",
            "${LINGXI_SKILL_DIR}/../../assets/templates/catalog.json",
            "${LINGXI_PLUGIN_ROOT}/assets/templates/catalog.json",
        )
        self.assert_fails(CHECK_PLUGIN, "which is not substituted in a skill body")

    def test_skill_dir_reference_must_exist(self) -> None:
        self.edit(
            PLUGIN / "skills" / "create-local-app" / "SKILL.md",
            "${LINGXI_SKILL_DIR}/../../assets/templates/catalog.json",
            "${LINGXI_SKILL_DIR}/../assets/templates/catalog.json",
        )
        self.assert_fails(CHECK_PLUGIN, "points at ${LINGXI_SKILL_DIR}/../assets/templates/catalog.json")

    def test_cited_template_file_must_exist(self) -> None:
        self.edit(
            PLUGIN / "skills" / "phaser-2d-local-app" / "references" / "router.md",
            "`assets/templates/phaser-2d/r4/lib/phaser-runtime.js`",
            "`assets/templates/phaser-2d/r4/lib/phaser-adapter.js`",
        )
        self.assert_fails(CHECK_PLUGIN, "cites assets/templates/phaser-2d/r4/lib/phaser-adapter.js")

    def test_skill_version_drift_fails(self) -> None:
        self.edit(PLUGIN / "skills" / "threejs-local-app" / "SKILL.md", "three@0.186.0", "three@0.185.1")
        self.assert_fails(CHECK_PLUGIN, "names three@0.185.1; the templates pin ['0.186.0']")

    def test_catalog_path_fails(self) -> None:
        def break_path(value):
            value["templates"][0]["path"] = "react-dom/r3"
        self.edit_json(TEMPLATES / "catalog.json", break_path)
        self.assert_fails(CHECK_PLUGIN, "catalog react-dom path must be 'react-dom/r4'")

    def test_loose_dependency_range_fails(self) -> None:
        self.edit_json(
            TEMPLATES / "canvas-2d" / "r4" / "package.json",
            lambda value: value["dependencies"].update(react="^19.3.0"),
        )
        self.assert_fails(CHECK_PLUGIN, "templates/canvas-2d pins react to '^19.3.0'")


class CheckSkillFrontmatterTest(ScratchRepo):
    def test_clean_copy_passes(self) -> None:
        result = self.run_gate(CHECK_FRONTMATTER)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_wide_description_fails_by_columns_not_characters(self) -> None:
        # 100 CJK characters: under 180 by len(), 200 display columns.
        description = "描" * 100
        self.edit(
            PLUGIN / "skills" / "accessibility" / "SKILL.md",
            "description: Enforce local-app accessibility",
            f"description: {description} Enforce local-app accessibility",
        )
        self.assert_fails(CHECK_FRONTMATTER, "accessibility: description is")

    def test_name_must_match_directory(self) -> None:
        self.edit(PLUGIN / "skills" / "accessibility" / "SKILL.md", "name: accessibility", "name: a11y")
        self.assert_fails(CHECK_FRONTMATTER, "frontmatter name is 'a11y' but the directory is 'accessibility'")


if __name__ == "__main__":
    unittest.main()
