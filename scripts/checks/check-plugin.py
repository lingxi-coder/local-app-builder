#!/usr/bin/env python3
"""The local-app-builder plugin's checked-in contract.

The plugin is skills plus template assets and nothing else: no agents,
workflows, MCP servers or hooks, and no host tool its skills depend on. The
model plans in plan mode, writes source with Write/Edit and builds with Bash.
The generated app is a page the user opens from disk, so the templates must
keep the build settings that make `file://` work.

Every rule names the regression it stops. `--repo` points the gate at another
checkout; `scripts/tests/test_plugin_gates.py` uses it to prove each rule can
fail.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


PLUGIN_NAME = "local-app-builder"
PLUGIN_DIR = Path("plugins") / PLUGIN_NAME
MARKETPLACE = Path(".lingxi-plugin") / "marketplace.json"

# The manifest's whole key set. Any other key is a component (`agents`,
# `workflows`, `mcpServers`, `hooks`, `lspServers`, ...) that this plugin no
# longer carries, so an extra key is a regression, not an addition.
MANIFEST_KEYS = {
    "name", "displayName", "version", "description", "author", "license", "defaultEnabled", "skills",
}
MANIFEST_VALUES = {
    "name": PLUGIN_NAME,
    "displayName": "Local App Builder",
    "license": "MIT",
    "defaultEnabled": True,
    "skills": "./skills/",
}
PLUGIN_ENTRIES = {".lingxi-plugin", "assets", "skills"}

EXPECTED_SKILLS = {
    "accessibility", "apple-design", "babylon-3d-local-app", "canvas-2d-local-app", "create-local-app",
    "frontend-design", "ionic-react-local-app", "phaser-2d-local-app", "react-best-practices",
    "threejs-local-app",
}

# Template directory -> catalog family.
FAMILIES = {
    "react-dom": "react_dom",
    "canvas-2d": "canvas_2d",
    "three-3d": "three_3d",
    "phaser-2d": "phaser_2d",
    "babylon-3d": "babylon_3d",
}
REVISION = 4
CATALOG_KEYS = {"templateId", "path", "family", "revision", "surface", "summary", "recommendedFor", "notFor"}

# Files every template carries byte for byte. They are edited once and copied
# to the other families; a family that drifts silently keeps the old bug.
SHARED_FILES = [
    ".gitignore",
    "index.html",
    "jsconfig.json",
    "pnpm-workspace.yaml",
    "vite.config.mjs",
    "app/error-boundary.jsx",
    "app/globals.css",
    "app/main.jsx",
    "lib/device-context.js",
    "lib/platform-adapter.js",
    "lib/platform-provider.jsx",
    "styles/foundation.css",
]
# Shared by the four canvas families only (react-dom's providers add a router).
CANVAS_SHARED_FILES = ["app/providers.jsx", "lib/frame-loop.js"]

# The engine each canvas family is built around.
ENGINE_PACKAGES = {
    "three-3d": ["three"],
    "phaser-2d": ["phaser"],
    "babylon-3d": ["@babylonjs/core", "@babylonjs/loaders", "@babylonjs/havok"],
}

PACKAGE_MANAGER = "pnpm@12.5.1"
NODE_ENGINES = "^20.19.0 || >=22.12.0"
EXACT_VERSION = re.compile(r"\d+\.\d+\.\d+")

# What makes `dist/index.html` work when it is opened from disk (the comments in
# the template's `vite.config.mjs` say why). Losing any one of them breaks the
# page in the browser while the build still succeeds.
FILE_URL_BUILD = {
    'base: "./"': "relative asset URLs",
    'format: "iife"': "a classic-script bundle (a file:// page refuses ES modules)",
    "assetsInlineLimit: () => true": "every imported asset inlined (a file:// page cannot fetch a sibling file)",
    '"<script defer src="': "the entry tag rewritten to a classic script",
}

# Strings that only exist when the plugin talks to a host that is gone: the
# Local App host tools, the page bridge, the MCP widget, the host build state.
HOST_COUPLING = re.compile(
    r"window\.lingxi|lingxi-bridge|LingXiBridge|\bLocalApp[A-Z][A-Za-z]+|mcp-widget|"
    r"\.lingxi-build-state|authoring-spec"
)
# The engine substitutes only `${LINGXI_SKILL_DIR}` and `${LINGXI_SESSION_ID}`
# in a skill body; `${LINGXI_PLUGIN_ROOT}` reaches the model verbatim there.
UNSUBSTITUTED_IN_SKILLS = re.compile(r"\$\{LINGXI_PLUGIN_(?:ROOT|DATA)\}")
SKILL_DIR_PATH = re.compile(r"\$\{LINGXI_SKILL_DIR\}(/[^\s`'\")]*)")
# A template file a skill cites as a source, relative to the plugin root.
TEMPLATE_CITATION = re.compile(r"`(assets/templates/[^`\s]+)`")
# `name@1.2.3` or `@scope/name@1.2.3`, not part of a longer token.
PINNED_MENTION = re.compile(r"(?<![\w@/.-])((?:@[a-z0-9-]+/)?[a-z0-9][a-z0-9.-]*)@(\d+\.\d+\.\d+)\b")

TEXT_SUFFIXES = {".md", ".yaml", ".yml", ".json", ".js", ".jsx", ".mjs", ".css", ".html"}
STRAY_PARTS = {"node_modules", "dist", ".vite"}
STRAY_NAMES = {".DS_Store"}


class Gate:
    def __init__(self, repo: Path):
        self.repo = repo
        self.plugin = repo / PLUGIN_DIR
        self.templates = self.plugin / "assets" / "templates"
        self.problems: list[str] = []

    def problem(self, message: str) -> None:
        self.problems.append(message)

    def read_json(self, path: Path, label: str):
        if not path.is_file():
            self.problem(f"{label} missing: {path.relative_to(self.repo)}")
            return None
        try:
            return json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as error:
            self.problem(f"{label} is not valid JSON: {error}")
            return None

    def check_manifest(self) -> None:
        manifest = self.read_json(self.plugin / ".lingxi-plugin" / "plugin.json", "plugin manifest")
        if not isinstance(manifest, dict):
            return
        extra = sorted(set(manifest) - MANIFEST_KEYS)
        if extra:
            self.problem(f"plugin manifest declares {extra}; the plugin carries skills and templates only")
        for key, expected in MANIFEST_VALUES.items():
            if manifest.get(key) != expected:
                self.problem(f"plugin manifest {key!r} must be {expected!r}, got {manifest.get(key)!r}")
        for key in ("version", "description"):
            if not isinstance(manifest.get(key), str) or not manifest[key].strip():
                self.problem(f"plugin manifest {key!r} must be a non-empty string")
        if not isinstance(manifest.get("author"), dict) or manifest["author"].get("name") != "LingXi":
            self.problem("plugin manifest author.name must be 'LingXi'")

        marketplace = self.read_json(self.repo / MARKETPLACE, "marketplace")
        if isinstance(marketplace, dict):
            listed = [(entry.get("name"), entry.get("source")) for entry in marketplace.get("plugins", [])]
            expected = [(PLUGIN_NAME, "./" + PLUGIN_DIR.as_posix())]
            if listed != expected:
                self.problem(f"marketplace must list exactly {expected}, got {listed}")

        if self.plugin.is_dir():
            entries = {path.name for path in self.plugin.iterdir()}
            if entries != PLUGIN_ENTRIES:
                self.problem(
                    f"plugin root must hold exactly {sorted(PLUGIN_ENTRIES)}: "
                    f"missing={sorted(PLUGIN_ENTRIES - entries)}, extra={sorted(entries - PLUGIN_ENTRIES)}"
                )
            manifest_dir = {path.name for path in (self.plugin / ".lingxi-plugin").iterdir()}
            if manifest_dir != {"plugin.json"}:
                self.problem(f".lingxi-plugin must hold only plugin.json, got {sorted(manifest_dir)}")
        else:
            self.problem(f"plugin directory missing: {PLUGIN_DIR}")

    def check_skills(self) -> None:
        skill_root = self.plugin / "skills"
        if not skill_root.is_dir():
            self.problem("skills/ missing")
            return
        names = {path.name for path in skill_root.iterdir() if path.is_dir()}
        if names != EXPECTED_SKILLS:
            self.problem(
                f"skill roster differs: missing={sorted(EXPECTED_SKILLS - names)}, "
                f"extra={sorted(names - EXPECTED_SKILLS)}"
            )
        for name in sorted(names):
            for required in ("SKILL.md", "agents/openai.yaml"):
                if not (skill_root / name / required).is_file():
                    self.problem(f"skills/{name} is missing {required}")

        for path in sorted(skill_root.rglob("*")):
            if not path.is_file() or path.suffix not in {".md", ".yaml"}:
                continue
            relative = path.relative_to(self.plugin).as_posix()
            text = path.read_text(encoding="utf-8")
            if UNSUBSTITUTED_IN_SKILLS.search(text):
                self.problem(
                    f"{relative} uses ${{LINGXI_PLUGIN_ROOT}}/${{LINGXI_PLUGIN_DATA}}, which is not "
                    "substituted in a skill body; use ${LINGXI_SKILL_DIR}"
                )
            skill_dir = skill_root / path.relative_to(skill_root).parts[0]
            for match in SKILL_DIR_PATH.finditer(text):
                reference = match.group(1)
                if "<" in reference:
                    continue
                if not (skill_dir / reference.lstrip("/")).exists():
                    self.problem(f"{relative} points at ${{LINGXI_SKILL_DIR}}{reference}, which does not exist")
            for match in TEMPLATE_CITATION.finditer(text):
                if "<" not in match.group(1) and not (self.plugin / match.group(1)).exists():
                    self.problem(f"{relative} cites {match.group(1)}, which does not exist")
            self.check_pinned_mentions(relative, text)

    def check_pinned_mentions(self, relative: str, text: str) -> None:
        """A version a skill names must be the version the templates pin.

        The Three.js skill named `three@0.185.1` for months while every
        template pinned 0.186.0; nothing compared the two."""
        pins: dict[str, set[str]] = {}
        for family in FAMILIES:
            package = self.package_json(family)
            for section in ("dependencies", "devDependencies", "optionalDependencies"):
                for name, version in (package.get(section) or {}).items():
                    pins.setdefault(name, set()).add(version)
        for match in PINNED_MENTION.finditer(text):
            name, version = match.group(1), match.group(2)
            if name == "pnpm":
                if f"pnpm@{version}" != PACKAGE_MANAGER:
                    self.problem(f"{relative} names pnpm@{version}; the templates pin {PACKAGE_MANAGER}")
            elif name in pins and pins[name] != {version}:
                self.problem(f"{relative} names {name}@{version}; the templates pin {sorted(pins[name])}")

    def package_json(self, family: str) -> dict:
        path = self.templates / family / f"r{REVISION}" / "package.json"
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            return {}
        return value if isinstance(value, dict) else {}

    def check_templates(self) -> None:
        if not self.templates.is_dir():
            self.problem("assets/templates missing")
            return
        entries = {path.name for path in self.templates.iterdir()}
        expected_entries = {"catalog.json", *FAMILIES}
        if entries != expected_entries:
            self.problem(
                f"assets/templates must hold exactly {sorted(expected_entries)}: "
                f"missing={sorted(expected_entries - entries)}, extra={sorted(entries - expected_entries)}"
            )
        for family in FAMILIES:
            family_dir = self.templates / family
            if family_dir.is_dir() and {path.name for path in family_dir.iterdir()} != {f"r{REVISION}"}:
                self.problem(f"templates/{family} must hold only r{REVISION}")

        self.check_catalog()
        for family in FAMILIES:
            self.check_package(family)
        self.check_shared_files()

        vite_config = self.templates / "react-dom" / f"r{REVISION}" / "vite.config.mjs"
        if vite_config.is_file():
            text = vite_config.read_text(encoding="utf-8")
            for needle, purpose in FILE_URL_BUILD.items():
                if needle not in text:
                    self.problem(f"vite.config.mjs lost {needle!r} ({purpose})")

    def check_catalog(self) -> None:
        catalog = self.read_json(self.templates / "catalog.json", "template catalog")
        if not isinstance(catalog, dict):
            return
        if catalog.get("schemaVersion") != 3:
            self.problem(f"catalog schemaVersion must be 3, got {catalog.get('schemaVersion')!r}")
        templates = catalog.get("templates")
        if not isinstance(templates, list):
            self.problem("catalog templates must be a list")
            return
        seen = []
        for template in templates:
            if not isinstance(template, dict):
                self.problem(f"catalog entry is not an object: {template!r}")
                continue
            family = next((key for key, value in FAMILIES.items() if value == template.get("family")), None)
            if family is None:
                self.problem(f"catalog entry has an unknown family: {template.get('family')!r}")
                continue
            seen.append(family)
            if set(template) != CATALOG_KEYS:
                self.problem(
                    f"catalog {family} keys differ: missing={sorted(CATALOG_KEYS - set(template))}, "
                    f"extra={sorted(set(template) - CATALOG_KEYS)}"
                )
            expected = {
                "templateId": f"{family}-r{REVISION}",
                "path": f"{family}/r{REVISION}",
                "revision": REVISION,
                "surface": "dom" if family == "react-dom" else "canvas",
            }
            for key, value in expected.items():
                if template.get(key) != value:
                    self.problem(f"catalog {family} {key} must be {value!r}, got {template.get(key)!r}")
            if not isinstance(template.get("summary"), str) or not template["summary"].strip():
                self.problem(f"catalog {family} summary must be a non-empty string")
            for key in ("recommendedFor", "notFor"):
                value = template.get(key)
                if not isinstance(value, list) or not value or not all(isinstance(item, str) and item for item in value):
                    self.problem(f"catalog {family} {key} must be a non-empty list of strings")
            if not (self.templates / str(template.get("path"))).is_dir():
                self.problem(f"catalog {family} path does not exist: {template.get('path')!r}")
        if sorted(seen) != sorted(FAMILIES):
            self.problem(f"catalog must list each family once, got {seen}")

    def check_package(self, family: str) -> None:
        root = self.templates / family / f"r{REVISION}"
        if not (root / "pnpm-lock.yaml").is_file():
            self.problem(f"templates/{family} has no pnpm-lock.yaml; `pnpm install --frozen-lockfile` needs it")
        package = self.package_json(family)
        if not package:
            self.problem(f"templates/{family}/package.json missing or invalid")
            return
        if package.get("packageManager") != PACKAGE_MANAGER:
            self.problem(f"templates/{family} packageManager must be {PACKAGE_MANAGER!r}")
        if (package.get("engines") or {}).get("node") != NODE_ENGINES:
            self.problem(f"templates/{family} engines.node must be {NODE_ENGINES!r}")
        if (package.get("scripts") or {}).get("build") != "vite build":
            self.problem(f"templates/{family} scripts.build must be 'vite build'")
        dependencies = {}
        for section in ("dependencies", "devDependencies", "optionalDependencies"):
            dependencies.update(package.get(section) or {})
        for name, version in sorted(dependencies.items()):
            if not isinstance(version, str) or not EXACT_VERSION.fullmatch(version):
                self.problem(f"templates/{family} pins {name} to {version!r}; every dependency must be exact")
        for name in ENGINE_PACKAGES.get(family, []):
            if name not in dependencies:
                self.problem(f"templates/{family} lost its engine package {name}")

    def check_shared_files(self) -> None:
        def compare(files: list[str], families: list[str]) -> None:
            for relative in files:
                contents = {}
                for family in families:
                    path = self.templates / family / f"r{REVISION}" / relative
                    if not path.is_file():
                        self.problem(f"templates/{family} is missing shared file {relative}")
                        continue
                    contents[family] = path.read_bytes()
                if len(set(contents.values())) > 1:
                    reference = contents.get(families[0])
                    drifted = sorted(family for family, data in contents.items() if data != reference)
                    self.problem(f"shared file {relative} differs from {families[0]} in {drifted}")

        compare(SHARED_FILES, list(FAMILIES))
        compare(CANVAS_SHARED_FILES, [family for family in FAMILIES if family != "react-dom"])

    def check_files(self) -> None:
        for path in sorted(self.plugin.rglob("*")):
            relative = path.relative_to(self.plugin)
            if STRAY_PARTS.intersection(relative.parts) or path.name in STRAY_NAMES or path.suffix == ".log":
                self.problem(f"build output or local junk inside the plugin: {relative.as_posix()}")
                continue
            if not path.is_file() or path.suffix not in TEXT_SUFFIXES or path.name == "pnpm-lock.yaml":
                continue
            for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                match = HOST_COUPLING.search(line)
                if match:
                    self.problem(
                        f"{relative.as_posix()}:{number} refers to the removed Local App host "
                        f"({match.group(0)!r})"
                    )

    def run(self) -> int:
        self.check_manifest()
        self.check_skills()
        self.check_templates()
        self.check_files()
        if self.problems:
            print("PLUGIN FAIL:\n  - " + "\n  - ".join(self.problems), file=sys.stderr)
            return 1
        print(
            f"PLUGIN OK: {len(EXPECTED_SKILLS)} skills, {len(FAMILIES)} templates; the manifest declares "
            "skills only, shared template files agree, and the file:// build settings are intact"
        )
        return 0


def main() -> int:
    parser = argparse.ArgumentParser(prog="check-plugin")
    parser.add_argument("--repo", default=str(Path(__file__).resolve().parents[2]))
    args = parser.parse_args()
    return Gate(Path(args.repo).resolve()).run()


if __name__ == "__main__":
    sys.exit(main())
