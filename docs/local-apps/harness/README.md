# Execution records the plugin gates read

These three files were moved here from the engine repository's `docs/local-apps/harness/`, where the rest of
the implementation harness (task contracts for the other phases, the test baseline, and the `lap-gate` review
tooling) stays as history:

- `template-migration-manifest.json` — the 112 template files the plugin ships, with sizes and digests
  (`scripts/checks/check-phase2-plugin.py`).
- `tasks-phase-2.json`, `tasks-phase-7.json` — what those phases claimed to add
  (`check-phase2-plugin.py` verifies the `addedTests` it can see; `check-phase7-plugin.py` the task list).

Paths in them that start with `crates/runtime`, `crates/mcp` and the like name the engine repository.
