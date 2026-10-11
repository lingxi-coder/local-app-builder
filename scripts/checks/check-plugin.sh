#!/usr/bin/env bash
# The local-app-builder plugin's checked-in contract (rules in scripts/checks/check-plugin.py): skills and templates
# only, catalog and template packages consistent, and the file:// build settings intact.
set -euo pipefail
cd "$(dirname "$0")/../.."
exec python3 scripts/checks/check-plugin.py "$@"
