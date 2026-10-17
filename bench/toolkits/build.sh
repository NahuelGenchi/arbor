#!/usr/bin/env bash
# Builds the test programs of issue #8 (README.md) into OUT, one folder per toolkit with its program at
# OUT/<toolkit>/bin, and prints `built <toolkit>` or `failed <toolkit>: <why>` for each:
#   toolkits/build.sh OUT [gtk4 qtquick slint iced]     (all four when none is named)
# It writes only in OUT. Cargo gets a home of its own there, which reads the registry's index, the crates already
# downloaded and cargo's config in ~/.cargo through links, offline (`cargo fetch` in slint/ and iced/ downloads them
# once); its objects, about a gigabyte a program, are removed once the program is built, since OUT is often in memory
# (tmpfs).
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=${1:?usage: build.sh OUT [toolkit...]}
shift
[ $# -gt 0 ] || set -- gtk4 qtquick slint iced
mkdir -p "$OUT" && OUT="$(cd "$OUT" && pwd)" || exit 1
JOBS=${CARGO_BUILD_JOBS:-8}

failed() { echo "failed $1: $2"; }
last() { grep -v '^\s*$' "$1" | tail -n 1 | cut -c1-300; }

build_gtk4() {
  pkg-config --exists gtk4 || { failed gtk4 'pkg-config finds no gtk4 (the libgtk-4-dev package)'; return; }
  # shellcheck disable=SC2046  # pkg-config's flags are words
  if "${CC:-gcc}" -O2 -o "$OUT/gtk4/gtk4-bench" "$HERE/gtk4/main.c" $(pkg-config --cflags --libs gtk4) -lm \
       >"$OUT/gtk4/build.log" 2>&1; then
    ln -sfn gtk4-bench "$OUT/gtk4/bin" && echo 'built gtk4'
  else
    failed gtk4 "the compiler failed: $(last "$OUT/gtk4/build.log")"
  fi
}

build_qtquick() {
  command -v cmake >/dev/null || { failed qtquick 'cmake not found'; return; }
  local gen=()
  command -v ninja >/dev/null && gen=(-G Ninja)
  if cmake -S "$HERE/qtquick" -B "$OUT/qtquick/build" "${gen[@]}" -DCMAKE_BUILD_TYPE=Release >"$OUT/qtquick/build.log" 2>&1 &&
     cmake --build "$OUT/qtquick/build" -j "$JOBS" >>"$OUT/qtquick/build.log" 2>&1; then
    ln -sfn build/qtquick-bench "$OUT/qtquick/bin" && echo 'built qtquick'
  else
    failed qtquick "cmake failed (Qt 6 Quick: the qt6-base-dev and qt6-declarative-dev packages): $(last "$OUT/qtquick/build.log")"
  fi
}

cargo_home() {  # a CARGO_HOME in OUT that reads ~/.cargo's index, downloaded crates and config through links
  local real=${CARGO_HOME:-$HOME/.cargo} name
  [ -d "$OUT/cargo" ] && return
  mkdir -p "$OUT/cargo/registry/src"
  for name in registry/index registry/cache config.toml config; do
    [ -e "$real/$name" ] && ln -s "$real/$name" "$OUT/cargo/$name"
  done
}

build_rust() {  # slint or iced; the toolchain is the one ../rust-toolchain.toml names, found from the program's folder
  local cargo
  cargo=$(command -v cargo || echo "${CARGO_HOME:-$HOME/.cargo}/bin/cargo")
  [ -x "$cargo" ] || { failed "$1" 'cargo not found'; return; }
  cargo_home
  if (cd "$HERE/$1" && CARGO_HOME="$OUT/cargo" CARGO_TARGET_DIR="$OUT/$1/target" RUSTUP_AUTO_INSTALL=0 \
        "$cargo" build --release --locked --offline --quiet -j "$JOBS") >"$OUT/$1/build.log" 2>&1 &&
     mv "$OUT/$1/target/release/$1-bench" "$OUT/$1/"; then
    ln -sfn "$1-bench" "$OUT/$1/bin" && echo "built $1"
  else
    failed "$1" "cargo build failed: $(last "$OUT/$1/build.log")"
  fi
  rm -rf "$OUT/$1/target"
}

for t in "$@"; do
  mkdir -p "$OUT/$t"
  case $t in
    gtk4) build_gtk4 ;;
    qtquick) build_qtquick ;;
    slint|iced) build_rust "$t" ;;
    *) failed "$t" 'no such toolkit (gtk4 qtquick slint iced)' ;;
  esac
done
rm -rf "$OUT/cargo"  # the links, and the crates cargo unpacked
