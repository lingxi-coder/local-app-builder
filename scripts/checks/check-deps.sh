#!/usr/bin/env bash
# Dependency-direction gate (rules in scripts/checks/check_deps.py).
#
# Offline first, from the declared dependencies: `cargo metadata --no-deps` needs no resolve. A networked call only if
# the offline one cannot be answered.
set -euo pipefail
cd "$(dirname "$0")/../.."

if META="$(cargo metadata --locked --format-version=1 --no-deps --offline 2>/dev/null)"; then
    :
elif META="$(cargo metadata --locked --format-version=1 --no-deps 2>/dev/null)"; then
    :
else
    echo "check-deps: cargo metadata failed" >&2
    exit 1
fi
printf '%s' "$META" | python3 scripts/checks/check_deps.py "$@"
