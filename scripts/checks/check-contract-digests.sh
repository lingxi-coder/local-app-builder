#!/usr/bin/env bash
# The catalog digests are pinned by tests that live with the runtime profiles, in local-app-builder-service
# (they ran from the engine's repository's phase-2 gate until the crate moved here).
#
# A filter that matches nothing reports "0 passed" and exits 0, so the step also requires the tests to have
# actually run: this is the failure that made the step vacuous once already.
set -euo pipefail
cd "$(dirname "$0")/../.."

if ! contract_output="$(cargo test --locked -q -p local-app-builder-service contract_digest 2>&1)"; then
    printf '%s\n' "$contract_output" >&2
    echo "CONTRACT-DIGESTS FAIL: runtime profile catalog must match production and the pre-release r1 golden" >&2
    exit 1
fi
if ! grep -Eq 'test result: ok\. ([3-9]|[1-9][0-9]+) passed' <<<"$contract_output"; then
    printf '%s\n' "$contract_output" >&2
    echo "CONTRACT-DIGESTS FAIL: fewer than three contract_digest tests ran in local-app-builder-service" >&2
    exit 1
fi
echo "CONTRACT-DIGESTS OK: all five catalog digests match production and tampering each family is rejected"
