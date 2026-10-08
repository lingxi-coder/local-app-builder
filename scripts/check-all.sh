#!/usr/bin/env bash
# Single entry point for every gate under scripts/checks/: discovered from the filesystem (`check-*.sh`), never from a
# list this file would need editing to extend, and each one is run for real as a child process.
set -euo pipefail
cd "$(dirname "$0")/.."

shopt -s nullglob
gates=()
for f in scripts/checks/check-*.sh; do
    base="$(basename "$f")"
    [[ "$base" == "check-all.sh" ]] && continue
    if [[ ! -x "$f" ]]; then
        echo "check-all: matching gate is not executable: $base" >&2
        exit 1
    fi
    gates+=("$base")
done
if [[ ${#gates[@]} -eq 0 ]]; then
    echo "check-all: discovered 0 gate scripts under scripts/checks/ — discovery is broken, not the repo" >&2
    exit 1
fi

status=0
for g in $(printf '%s\n' "${gates[@]}" | sort); do
    echo "=== RUNNING: $g ==="
    if ./scripts/checks/"$g" 2>&1; then rc=0; else rc=$?; fi
    echo "=== RESULT: $g exit=$rc ==="
    [[ $rc -ne 0 ]] && status=$rc
done

# The gates' own negative-probe tests are part of the same entry point: a gate nobody proves can fail is not a gate.
py_tests=(scripts/tests/test_*.py)
if [[ ${#py_tests[@]} -eq 0 ]]; then
    echo "check-all: discovered 0 python test files under scripts/tests/ — discovery is broken, not the repo" >&2
    exit 1
fi
echo "=== RUNNING: scripts/tests (${#py_tests[@]} file(s)) ==="
if py_out="$(python3 -m unittest discover -s scripts/tests -p 'test_*.py' 2>&1)"; then rc=0; else rc=$?; fi
echo "$py_out"
ran="$(printf '%s\n' "$py_out" | sed -n 's/^Ran \([0-9][0-9]*\) test.*/\1/p')"
if [[ $rc -eq 0 && ( -z "$ran" || "$ran" -lt ${#py_tests[@]} ) ]]; then
    echo "check-all: python tests ran '${ran:-0}' case(s) for ${#py_tests[@]} file(s) — a file that adds no cases is not discovered" >&2
    rc=1
fi
echo "=== RESULT: scripts/tests exit=$rc ==="
[[ $rc -ne 0 ]] && status=$rc

echo "check-all: ran ${#gates[@]} gate(s): ${gates[*]}"
exit "$status"
