#!/usr/bin/env bash
# Source checks for the baseline suite: every script parses. Measures nothing and reads no personal data.
#   ./check.sh
# Needs bash, Python 3.14+ (the suite imports compression.zstd), node and gcc.
set -euo pipefail

BENCH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$BENCH_DIR"
fail=0
check() {
  local what="$1"; shift
  if "$@"; then printf 'ok    %s\n' "$what"; else printf 'FAIL  %s\n' "$what"; fail=1; fi
}

for f in run.sh config.sh check.sh; do check "bash -n $f" bash -n "$f"; done

# Compiled files go to a temporary folder, so no __pycache__ appears in bench/.
PYTHONPYCACHEPREFIX="$(mktemp -d)"
export PYTHONPYCACHEPREFIX
trap 'rm -rf "$PYTHONPYCACHEPREFIX"' EXIT
for f in lib/*.py sections/*.py; do check "py_compile $f" python3 -m py_compile "$f"; done

for f in lib/*.js sections/*.js; do check "node --check $f" node --check "$f"; done

for f in src/*.c; do check "gcc -fsyntax-only $f" gcc -fsyntax-only "$f"; done

exit "$fail"
