#!/usr/bin/env bash
# Source checks for the baseline suite: every script parses. Measures nothing and reads no personal data.
#   ./check.sh
# Needs bash, Python 3.14+ (the suite imports compression.zstd), node and gcc; g++ and pkg-config for the
# toolkit programs, which are skipped where their toolkit's development files are missing.
set -euo pipefail

BENCH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$BENCH_DIR"
fail=0
check() {
  local what="$1"; shift
  if "$@"; then printf 'ok    %s\n' "$what"; else printf 'FAIL  %s\n' "$what"; fail=1; fi
}

for f in run.sh config.sh check.sh toolkits/build.sh; do check "bash -n $f" bash -n "$f"; done

# Compiled files go to a temporary folder, so no __pycache__ appears in bench/.
PYTHONPYCACHEPREFIX="$(mktemp -d)"
export PYTHONPYCACHEPREFIX
trap 'rm -rf "$PYTHONPYCACHEPREFIX"' EXIT
for f in lib/*.py sections/*.py; do check "py_compile $f" python3 -m py_compile "$f"; done

for f in lib/*.js sections/*.js; do check "node --check $f" node --check "$f"; done

for f in src/*.c; do check "gcc -fsyntax-only $f" gcc -fsyntax-only "$f"; done

# The toolkit programs that need a toolkit's development files are checked where they are installed (not on CI).
# shellcheck disable=SC2046  # pkg-config's flags are words
if pkg-config --exists gtk4 2>/dev/null; then
  check "gcc -fsyntax-only toolkits/gtk4/main.c" gcc -fsyntax-only $(pkg-config --cflags gtk4) toolkits/gtk4/main.c
else
  printf 'skip  toolkits/gtk4/main.c (pkg-config finds no gtk4)\n'
fi
# shellcheck disable=SC2046
if pkg-config --exists Qt6Quick 2>/dev/null; then
  check "g++ -fsyntax-only toolkits/qtquick/main.cpp" g++ -fsyntax-only -std=c++17 -fPIC \
    $(pkg-config --cflags Qt6Quick) toolkits/qtquick/main.cpp
else
  printf 'skip  toolkits/qtquick/main.cpp (pkg-config finds no Qt6Quick)\n'
fi

exit "$fail"
