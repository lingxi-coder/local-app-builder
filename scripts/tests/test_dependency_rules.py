#!/usr/bin/env python3
"""Negative probes for scripts/checks/check_deps.py, with Cargo-shaped data."""
import json
import subprocess
import sys
import unittest
from pathlib import Path

ENGINE = Path(__file__).resolve().parents[1] / "checks/check_deps.py"
ROOT = "/repo"


class DependencyRules(unittest.TestCase):
    def run_gate(self, edges):
        """`edges`: crate -> list of dependency names or dicts. Every key is a workspace member."""
        metadata = {
            "workspace_root": ROOT,
            "packages": [
                {"name": name, "dependencies": [dep if isinstance(dep, dict) else {"name": dep} for dep in deps]}
                for name, deps in edges.items()
            ],
        }
        return subprocess.run([sys.executable, str(ENGINE)], input=json.dumps(metadata), text=True, capture_output=True)

    def members(self, **edges):
        base = {n: [] for n in ("device-api", "local-app-contracts", "mcp-wire", "rooted-fs", "local-apps",
                                "local-app-service", "local-app-plugin")}
        base.update(edges)
        return base

    def test_the_shape_the_crates_have_today_passes(self):
        result = self.run_gate(self.members(**{
            "local-apps": ["local-app-contracts", "mcp-wire", "rooted-fs", {"name": "git2", "optional": True}],
            "local-app-service": [{"name": "local-apps", "uses_default_features": False},
                                  "device-api", "local-app-contracts", "mcp-wire", "rooted-fs"],
        }))
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_primitive_cannot_depend_on_anything_in_the_workspace(self):
        result = self.run_gate(self.members(**{"mcp-wire": ["local-apps"]}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("mcp-wire depends on local-apps — shared primitives have no workspace dependencies",
                      result.stderr)

    def test_the_core_cannot_depend_on_the_service_above_it(self):
        result = self.run_gate(self.members(**{"local-apps": ["local-app-service"]}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("local-apps depends on local-app-service — the core reaches the workspace only through",
                      result.stderr)

    def test_the_service_cannot_reach_the_plugin_crate(self):
        result = self.run_gate(self.members(**{"local-app-service": ["local-app-plugin"]}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("local-app-service depends on local-app-plugin — the service reaches the workspace only",
                      result.stderr)

    def test_the_service_cannot_take_the_core_with_its_default_features(self):
        result = self.run_gate(self.members(**{
            "local-app-service": [{"name": "local-apps", "uses_default_features": True}]}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("local-app-service depends on local-apps with its default features", result.stderr)

    def test_the_plugin_crate_has_no_dependencies(self):
        result = self.run_gate(self.members(**{"local-app-plugin": ["serde"]}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("local-app-plugin must have no dependencies", result.stderr)

    def test_libgit2_stays_behind_its_feature(self):
        result = self.run_gate(self.members(**{"local-apps": [{"name": "git2", "optional": False}]}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("local-apps depends on git2 unconditionally", result.stderr)

    def test_a_git_dependency_on_another_repository_is_rejected(self):
        result = self.run_gate(self.members(**{"local-apps": [
            {"name": "core", "source": "git+https://example.invalid/harness-runtime.git?rev=0123#0123"}]}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("this repository's graph must not reach another repository", result.stderr)

    def test_a_path_dependency_that_leaves_the_repository_is_rejected(self):
        result = self.run_gate(self.members(**{"local-apps": [
            {"name": "core", "source": None, "path": "/somewhere/else/crates/core"}]}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("outside this repository", result.stderr)

    def test_a_path_dependency_inside_the_repository_is_fine(self):
        result = self.run_gate(self.members(**{"local-apps": [
            {"name": "rooted-fs", "source": None, "path": "/repo/crates/rooted-fs"}]}))
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
