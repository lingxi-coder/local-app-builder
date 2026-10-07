#!/usr/bin/env python3
"""Rules for the Codex / Claude Code plugin in plugins/local-app. Offline, no client needed.

The two clients read different files from one tree (P1, experiments 6 and 7): Claude reads `.claude-plugin/plugin.json`
and `.mcp.json`; Codex reads the root `plugin.json` and `mcp.json`. Each is blind to the other's, so nothing stops them
drifting apart except this gate.

  C1  both manifests are JSON, kebab-case name, a semver version and a description, and agree on all three
  C2  `mcp.json` and `.mcp.json` are the same document: one server `local-app`, the bare command `local-app` with
      args `["mcp"]` (no placeholder, which Claude does not expand in the shared form), type `stdio`
  C3  both marketplace files list the plugin under the same name, from a `./plugins/local-app` that exists
  C4  every skill is `skills/<dir>/SKILL.md` with frontmatter `name` equal to the directory and a description
  C5  no skill uses a `${...}` placeholder (Codex substitutes none, Claude only its own) or names the engine's product
      (`window.lingxi` is the app bridge's real name and is allowed)
  C6  every `LocalApp…` tool a skill names is one the server offers today (the CLI's SERVED operations, through the
      service's tool table), and every offered tool is named in the `local-app` skill, so the model is told about it
"""
import json
import os
import re
import sys

ROOT = os.path.realpath(os.path.join(os.path.dirname(__file__), "..", ".."))
PLUGIN = "plugins/local-app"
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
    """Tool names the CLI offers: SERVED operations looked up in the service's (name, operation, read_only) table."""
    try:
        with open(os.path.join(root, "crates/local-app-cli/src/mcp_backend.rs"), encoding="utf-8") as f:
            backend = f.read()
        with open(os.path.join(root, "crates/local-app-service/src/tool_names.rs"), encoding="utf-8") as f:
            table = f.read()
    except FileNotFoundError as e:
        problems.append("cannot read the tool sources: %s" % e)
        return None, None
    served = re.search(r"pub const SERVED: &\[&str\] = &\[(.*?)\];", backend, re.S)
    if not served:
        problems.append("crates/local-app-cli/src/mcp_backend.rs has no `SERVED` list")
        return None, None
    operations = set(re.findall(r'"([a-z_]+)"', served.group(1)))
    rows = re.findall(r'\(\s*"(LocalApp[A-Za-z]+)"\s*,\s*"([a-z_]+)"\s*,\s*(true|false)\s*,?\s*\)', table)
    every = {name for name, _, _ in rows}
    offered = {name for name, op, read_only in rows if op in operations and read_only == "true"}
    if len(offered) != len(operations):
        problems.append("SERVED names operations with no read-only tool row: %s"
                        % ", ".join(sorted(operations - {op for _, op, ro in rows if ro == "true"})))
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
    codex_mcp = read_json(root, PLUGIN + "/mcp.json", problems)
    claude_mcp = read_json(root, PLUGIN + "/.mcp.json", problems)
    if codex_mcp is not None and claude_mcp is not None and codex_mcp != claude_mcp:
        problems.append("mcp.json and .mcp.json differ — each client reads only one of them, so they must be the same document")
    for label, doc in (("mcp.json", codex_mcp), (".mcp.json", claude_mcp)):
        if doc is None:
            continue
        servers = doc.get("mcpServers")
        if not isinstance(servers, dict) or list(servers) != ["local-app"]:
            problems.append("%s: expected exactly one server, `local-app`" % label)
            continue
        server = servers["local-app"]
        if server.get("command") != "local-app" or server.get("args") != ["mcp"] or server.get("type") != "stdio":
            problems.append("%s: the server must be {type: stdio, command: local-app, args: [mcp]}, got %s"
                            % (label, json.dumps(server, sort_keys=True)))
        if "${" in json.dumps(server):
            problems.append("%s: a placeholder is not expanded the same way by both clients; use the bare command" % label)

    # C3
    for rel, entries_of in ((".agents/plugins/marketplace.json", lambda d: [(p.get("name"), (p.get("source") or {}).get("path")) for p in d.get("plugins", [])]),
                            (".claude-plugin/marketplace.json", lambda d: [(p.get("name"), p.get("source")) for p in d.get("plugins", [])])):
        doc = read_json(root, rel, problems)
        if doc is None:
            continue
        entries = entries_of(doc)
        if entries != [("local-app", "./" + PLUGIN)]:
            problems.append("%s: expected one plugin `local-app` from ./%s, got %s" % (rel, PLUGIN, entries))
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
        # `window.lingxi` is the app bridge's real name and stays; any other mention is the engine's product.
        stripped = text.replace("window.lingxi", "")
        if re.search(r"lingxi", stripped, re.I):
            problems.append("%s names the engine's product (`%s`)" % (rel, re.search(r"\S*lingxi\S*", stripped, re.I).group(0)))
        mentioned = set(re.findall(r"\bLocalApp[A-Z][A-Za-z]*\b", text))
        named_anywhere |= mentioned
        if name == "local-app":
            main_skill = text
        if every is not None:
            for tool in sorted(mentioned):
                if tool not in every:
                    problems.append("%s names `%s`, which is not a tool of the service" % (rel, tool))
                elif tool not in (offered or set()):
                    problems.append("%s names `%s`, which exists but is not offered by `local-app mcp` yet" % (rel, tool))
    if offered:
        for tool in sorted(offered):
            if tool not in main_skill:
                problems.append("the `local-app` skill does not mention the offered tool `%s`" % tool)
    return problems


def main():
    root = os.path.realpath(sys.argv[1]) if len(sys.argv) > 1 else ROOT
    problems = check(root)
    if problems:
        sys.stderr.write("[deny] client-plugin violations (%d):\n" % len(problems))
        for p in problems:
            sys.stderr.write("  - " + p + "\n")
        return 1
    print("check-client-plugin: OK — manifests, MCP configs, marketplaces and skills agree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
