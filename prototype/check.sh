#!/usr/bin/env bash
# Source checks for the prototype: formatting, lints and tests. Measures nothing and reads no personal data.
#   ./check.sh
# Needs Rust (the toolchain pinned in rust-toolchain.toml). Tests that need bubblewrap's overlay, or a systemd user
# manager for limits, skip with a note where it is missing, so a pass there proves less. ARBOR_REQUIRE_BWRAP=1 and
# ARBOR_REQUIRE_CGROUPS=1 turn those skips into failures, as in CI.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"
fail=0
check() {
  local what="$1"; shift
  if "$@"; then printf 'ok    %s\n' "$what"; else printf 'FAIL  %s\n' "$what"; fail=1; fi
}

# A busy PC has little memory to spare; fewer parallel jobs keep the build out of swap.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}"
check "cargo fmt --check" cargo fmt --all --check
check "cargo clippy" cargo clippy --locked --all-targets -- -D warnings
check "cargo test" cargo test --locked

exit "$fail"
