#!/usr/bin/env bash
# Client-plugin gate (rules in scripts/checks/check_client_plugin.py): the plugin for Codex and Claude Code, in
# plugins/local-app-builder, keeps its two manifests, its two MCP configs and its two marketplace files in agreement, and its
# skills name only tools the `local-app-builder` server really offers.
set -euo pipefail
cd "$(dirname "$0")/../.."
exec python3 scripts/checks/check_client_plugin.py "$@"
