#!/usr/bin/env bash
# Source checks for the prototype: formatting, lints and tests. Measures nothing and reads no personal data.
#   ./check.sh
# Needs Rust (the toolchain pinned in rust-toolchain.toml), and for Mission Control (mission/) GTK 4.14 or later with
# its development files. Without those, only arbor is linted and tested, with a note. Tests that need bubblewrap's
# overlay, or a systemd user manager for limits, skip with a note where it is missing, so a pass there proves less.
# ARBOR_REQUIRE_BWRAP=1, ARBOR_REQUIRE_CGROUPS=1 and ARBOR_REQUIRE_GTK=1 turn those skips into failures, as in CI.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"
fail=0
check() {
  local what="$1"; shift
  if "$@"; then printf 'ok    %s\n' "$what"; else printf 'FAIL  %s\n' "$what"; fail=1; fi
}

# A busy PC has little memory to spare; fewer parallel jobs keep the build out of swap.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}"
members=()
if pkg-config --atleast-version=4.14 gtk4 2>/dev/null; then
  members=(--workspace)
elif [ "${ARBOR_REQUIRE_GTK:-}" = 1 ]; then
  printf 'FAIL  GTK 4.14 or later, with its development files, for Mission Control\n'; fail=1
else
  printf 'note  no GTK 4.14 or later with its development files: Mission Control is not built or tested\n'
fi
check "cargo fmt --check" cargo fmt --all --check
check "cargo clippy" cargo clippy --locked "${members[@]}" --all-targets -- -D warnings
check "cargo test" cargo test --locked "${members[@]}"

exit "$fail"
