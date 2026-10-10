#!/usr/bin/env python3
"""Rules for the Codex / Claude Code plugin in plugins/local-app-builder. Offline, no client needed.

The two clients read different files from one tree (P1, experiments 6 and 7): Claude reads `.claude-plugin/plugin.json`;
Codex reads the root `plugin.json`. Each is blind to the other's, so nothing stops them
drifting apart except this gate.

  C1  both manifests are JSON, kebab-case name, a semver version and a description, and agree on all three
  C2  neither plugin tree carries an MCP config (`mcp.json`, `.mcp.json`): the server is installed through the client's own MCP flow
  C3  both marketplace files list the plugin under the same name, from a `./plugins/local-app-builder` that exists
  C4  every skill is `skills/<dir>/SKILL.md` with frontmatter `name` equal to the directory and a description
  C5  no skill uses a `${...}` placeholder (Codex substitutes none, Claude only its own) or names the engine's product
      (`window.lingxi` and `LINGXI.md` are real names and are allowed)
  C6  every `LocalApp…` tool a skill names is one the server offers today (the CLI's SERVED_READ and SERVED_WRITE
      operations, through the service's tool table), and every offered tool is named in the `local-app-builder` skill, so the model is told about it
  C7  the LingXi plugin tree (`crates/plugins/lingxi-local-app`) is installable on its own: `.lingxi-plugin/marketplace.json` lists it
      from a path that exists, and its manifest is MIT like the repository
"""
import json
import os
import re
import sys

ROOT = os.path.realpath(os.path.join(os.path.dirname(__file__), "..", ".."))
PLUGIN = "plugins/local-app-builder"
LINGXI_PLUGIN = "crates/plugins/lingxi-local-app"
KEBAB = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")


def read_json(root, rel, problems):
    path = os.path.join(root, rel)
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        problems.append("%s is missing" % rel)
    except ValueError as e:
        problems.append("%s is not valid JSON: %s" % (rel, e))
    return None


def frontmatter(text):
    m = re.match(r"^---\n(.*?)\n---\n", text, re.S)
    if not m:
        return None
    fields = {}
    for line in m.group(1).splitlines():
        if ":" in line and not line.startswith(" "):
            key, value = line.split(":", 1)
            fields[key.strip()] = value.strip()
    return fields


def served_tool_names(root, problems):
    """Tool names the CLI offers: SERVED_READ operations that are read-only in the service's (name, operation,
    read_only) table, plus every SERVED_WRITE operation."""
    try:
        with open(os.path.join(root, "crates/local-app-builder-cli/src/mcp_backend.rs"), encoding="utf-8") as f:
            backend = f.read()
        with open(os.path.join(root, "crates/local-app-builder-service/src/tool_names.rs"), encoding="utf-8") as f:
            table = f.read()
    except FileNotFoundError as e:
        problems.append("cannot read the tool sources: %s" % e)
        return None, None
    lists = {}
    for kind in ("SERVED_READ", "SERVED_WRITE"):
        found = re.search(r"pub const %s: &\[&str\] = &\[(.*?)\];" % kind, backend, re.S)
        if not found:
            problems.append("crates/local-app-builder-cli/src/mcp_backend.rs has no `%s` list" % kind)
            return None, None
        lists[kind] = set(re.findall(r'"([a-z_]+)"', found.group(1)))
    rows = re.findall(r'\(\s*"(LocalApp[A-Za-z]+)"\s*,\s*"([a-z_]+)"\s*,\s*(true|false)\s*,?\s*\)', table)
    every = {name for name, _, _ in rows}
    offered = {name for name, op, read_only in rows
               if op in lists["SERVED_WRITE"] or (op in lists["SERVED_READ"] and read_only == "true")}
    known_ops = {op for _, op, _ in rows}
    missing = (lists["SERVED_READ"] | lists["SERVED_WRITE"]) - known_ops
    if missing:
        problems.append("SERVED names operations with no tool row: %s" % ", ".join(sorted(missing)))
    unreadable = {op for _, op, ro in rows if op in lists["SERVED_READ"] and ro != "true"}
    if unreadable:
        problems.append("SERVED_READ names operations that are not read-only: %s" % ", ".join(sorted(unreadable)))
    return offered, every


def check(root):
    problems = []
    plugin_dir = os.path.join(root, PLUGIN)

    # C1
    codex = read_json(root, PLUGIN + "/plugin.json", problems)
    claude = read_json(root, PLUGIN + "/.claude-plugin/plugin.json", problems)
    for label, manifest in (("plugin.json", codex), (".claude-plugin/plugin.json", claude)):
        if manifest is None:
            continue
        if not KEBAB.match(str(manifest.get("name", ""))):
            problems.append("%s: name %r is not kebab-case" % (label, manifest.get("name")))
        if not SEMVER.match(str(manifest.get("version", ""))):
            problems.append("%s: version %r is not major.minor.patch" % (label, manifest.get("version")))
        if not str(manifest.get("description", "")).strip():
            problems.append("%s: description is empty" % label)
    if codex and claude:
        for field in ("name", "version"):
            if codex.get(field) != claude.get(field):
                problems.append("the manifests disagree on %s: plugin.json %r, .claude-plugin/plugin.json %r"
                                % (field, codex.get(field), claude.get(field)))

    # C2
    for rel in (PLUGIN + "/mcp.json", PLUGIN + "/.mcp.json", LINGXI_PLUGIN + "/.mcp.json"):
        if os.path.exists(os.path.join(root, rel)):
            problems.append("%s: the plugin does not carry an MCP config; the server is installed through the client's MCP flow" % rel)

    # C3
    for rel, entries_of in ((".agents/plugins/marketplace.json", lambda d: [(p.get("name"), (p.get("source") or {}).get("path")) for p in d.get("plugins", [])]),
                            (".claude-plugin/marketplace.json", lambda d: [(p.get("name"), p.get("source")) for p in d.get("plugins", [])])):
        doc = read_json(root, rel, problems)
        if doc is None:
            continue
        entries = entries_of(doc)
        if entries != [("local-app-builder", "./" + PLUGIN)]:
            problems.append("%s: expected one plugin `local-app-builder` from ./%s, got %s" % (rel, PLUGIN, entries))

    # C7
    lingxi_market = read_json(root, ".lingxi-plugin/marketplace.json", problems)
    if lingxi_market is not None:
        entries = [(p.get("name"), p.get("source")) for p in lingxi_market.get("plugins", [])]
        if entries != [("lingxi-local-app", "./" + LINGXI_PLUGIN)]:
            problems.append(".lingxi-plugin/marketplace.json: expected one plugin `lingxi-local-app` from ./%s, got %s" % (LINGXI_PLUGIN, entries))
    lingxi_manifest = read_json(root, LINGXI_PLUGIN + "/.lingxi-plugin/plugin.json", problems)
    if lingxi_manifest is not None:
        if lingxi_manifest.get("name") != "lingxi-local-app":
            problems.append("%s/.lingxi-plugin/plugin.json: name must be lingxi-local-app (the workflow ids are `lingxi-local-app:<script>`)" % LINGXI_PLUGIN)
        if lingxi_manifest.get("license") != "MIT":
            problems.append("%s/.lingxi-plugin/plugin.json: license must be MIT like the repository, got %r" % (LINGXI_PLUGIN, lingxi_manifest.get("license")))
    if not os.path.isdir(plugin_dir):
        problems.append("%s does not exist" % PLUGIN)
        return problems

    # C4, C5, C6
    offered, every = served_tool_names(root, problems)
    skills_dir = os.path.join(plugin_dir, "skills")
    skills = sorted(d for d in os.listdir(skills_dir)) if os.path.isdir(skills_dir) else []
    if not skills:
        problems.append("the plugin has no skills")
    named_anywhere = set()
    main_skill = ""
    for name in skills:
        rel = "%s/skills/%s/SKILL.md" % (PLUGIN, name)
        path = os.path.join(root, rel)
        if not os.path.isfile(path):
            problems.append("%s is missing (every skills/<dir> holds a SKILL.md)" % rel)
            continue
        with open(path, encoding="utf-8") as f:
            text = f.read()
        fm = frontmatter(text)
        if fm is None:
            problems.append("%s has no frontmatter" % rel)
        else:
            if fm.get("name") != name:
                problems.append("%s: frontmatter name %r must equal the directory %r" % (rel, fm.get("name"), name))
            if not fm.get("description"):
                problems.append("%s: frontmatter description is empty" % rel)
        if "${" in text:
            problems.append("%s uses a ${...} placeholder; neither client expands the same ones" % rel)
        # `window.lingxi` (the app bridge) and `LINGXI.md` (the file the service writes into every workspace) are real
        # names and stay; any other mention is the engine's product.
        stripped = text.replace("window.lingxi", "").replace("LINGXI.md", "")
        if re.search(r"lingxi", stripped, re.I):
            problems.append("%s names the engine's product (`%s`)" % (rel, re.search(r"\S*lingxi\S*", stripped, re.I).group(0)))
        mentioned = set(re.findall(r"\bLocalApp[A-Z][A-Za-z]*\b", text))
        named_anywhere |= mentioned
        if name == "local-app-builder":
            main_skill = text
        if every is not None:
            for tool in sorted(mentioned):
                if tool not in every:
                    problems.append("%s names `%s`, which is not a tool of the service" % (rel, tool))
                elif tool not in (offered or set()):
                    problems.append("%s names `%s`, which exists but is not offered by `local-app-builder mcp` yet" % (rel, tool))
    if offered:
        for tool in sorted(offered):
            if tool not in main_skill:
                problems.append("the `local-app-builder` skill does not mention the offered tool `%s`" % tool)
    return problems


def main():
    root = os.path.realpath(sys.argv[1]) if len(sys.argv) > 1 else ROOT
    problems = check(root)
    if problems:
        sys.stderr.write("[deny] client-plugin violations (%d):\n" % len(problems))
        for p in problems:
            sys.stderr.write("  - " + p + "\n")
        return 1
    print("check-client-plugin: OK — manifests, marketplaces and skills agree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
