#!/usr/bin/env python3
"""Dependency-direction gate for the Local App repository (driver: scripts/checks/check-deps.sh).

Reads `cargo metadata --format-version=1 --no-deps` on stdin. Only *declared* dependencies are looked at, so it runs
offline. The rules are the ones the crates were arranged to satisfy before they left the engine's repository; they are
the reason this repository can be built, tested and released without it.

  R1  shared primitives (device-api, local-app-contracts, mcp-wire, rooted-fs) depend on no workspace crate
  R2  local-apps (the core) depends on the workspace only through the primitives
  R3  local-app-service depends on the workspace only through the primitives and local-apps, and takes local-apps
      with its default features off (the service's own `git-checkpoints` feature forwards the one that matters)
  R4  local-app-plugin depends on nothing
  R5  libgit2 is reached through a feature of local-apps only: the graph that links it chooses where it comes from
  R6  the project's graph is self-contained: no dependency may come from a git source or from a path outside this
      repository. A path or git dependency on the engine (or on a product crate) is exactly what this rule exists for.
"""
import json
import os
import sys

PRIMITIVES = {"device-api", "local-app-contracts", "mcp-wire", "rooted-fs"}
CORE = "local-apps"
SERVICE = "local-app-service"
PLUGIN = "local-app-plugin"
GATED = {CORE: {"git2"}}


def main():
    m = json.load(sys.stdin)
    root = os.path.realpath(m["workspace_root"])
    pkgs = {p["name"]: p for p in m["packages"]}
    members = set(pkgs)
    violations = []

    def ws(p):
        return sorted({d["name"] for d in p["dependencies"] if d["name"] in members and d["name"] != p["name"]
                       and d.get("kind") != "dev"})

    for n, p in sorted(pkgs.items()):
        deps = ws(p)
        if n in PRIMITIVES and deps:
            violations.append("%s depends on %s — shared primitives have no workspace dependencies" % (n, ", ".join(deps)))
        if n == CORE:
            bad = [d for d in deps if d not in PRIMITIVES]
            if bad:
                violations.append("%s depends on %s — the core reaches the workspace only through %s"
                                  % (n, ", ".join(bad), ", ".join(sorted(PRIMITIVES))))
        if n == SERVICE:
            bad = [d for d in deps if d not in PRIMITIVES | {CORE}]
            if bad:
                violations.append("%s depends on %s — the service reaches the workspace only through the primitives and %s"
                                  % (n, ", ".join(bad), CORE))
            for d in p["dependencies"]:
                if d["name"] == CORE and d.get("kind") != "dev" and d.get("uses_default_features", True):
                    violations.append("%s depends on %s with its default features — a consumer could not turn them off"
                                      % (n, CORE))
        if n == PLUGIN and [d for d in p["dependencies"] if d.get("kind") != "dev"]:
            violations.append("%s must have no dependencies" % n)
        for d in p["dependencies"]:
            if d["name"] in GATED.get(n, set()) and d.get("kind") != "dev" and not d.get("optional", False):
                violations.append("%s depends on %s unconditionally — it must stay behind its feature" % (n, d["name"]))
            source = d.get("source")
            if source and source.startswith("git+"):
                violations.append("%s depends on %s from %s — this repository's graph must not reach another repository"
                                  % (n, d["name"], source))
            path = d.get("path")
            if path and not source and not os.path.realpath(path).startswith(root + os.sep):
                violations.append("%s depends on %s at %s — outside this repository" % (n, d["name"], path))

    if violations:
        sys.stderr.write("[deny] dependency-graph violations (%d):\n" % len(violations))
        for v in violations:
            sys.stderr.write("  - " + v + "\n")
        return 1
    print("check-deps: OK — %d workspace crates, no dependency-direction violations" % len(pkgs))
    return 0


if __name__ == "__main__":
    sys.exit(main())
