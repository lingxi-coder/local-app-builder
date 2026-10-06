#!/usr/bin/env bash
# The Local App plugin's checked-in contract (skills, agents, workflows, schemas, template assets, inventory).
# The engine-side workflow tests that run these scripts live in the engine repository's own gate.
set -euo pipefail
cd "$(dirname "$0")/../.."
python3 scripts/checks/check-phase2-plugin.py
