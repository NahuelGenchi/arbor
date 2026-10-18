#!/usr/bin/env bash
# Builds the test programs of issue #8 (README.md) into OUT, one folder per toolkit with its program at
# OUT/<toolkit>/bin, and prints `built <toolkit>` or `failed <toolkit>: <why>` for each:
#   toolkits/build.sh OUT [gtk4 gtk4rs qtquick slint iced flutter]     (all six when none is named)
# It writes only in OUT. Cargo gets a home of its own there, which reads the registry's index, the crates already
# downloaded and cargo's config in ~/.cargo through links, offline (`cargo fetch` in gtk4rs/, slint/ and iced/
# downloads them once); its objects, about a gigabyte a program, are removed once the program is built, since OUT is
# often in memory (tmpfs). Flutter gets a copy of the program, a HOME and a pub cache there, which reads the packages
# Flutter pins from $FLUTTER_PUB_CACHE (else $PUB_CACHE, else ~/.pub-cache) through a link; the flutter command is
# $FLUTTER, else flutter on PATH. It runs without a network (bwrap --unshare-net), so it can download nothing.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=${1:?usage: build.sh OUT [toolkit...]}
shift
[ $# -gt 0 ] || set -- gtk4 gtk4rs qtquick slint iced flutter
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

build_rust() {  # gtk4rs, slint or iced; the toolchain is the one ../rust-toolchain.toml names, found from the program's folder
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

build_flutter() {
  local fl=${FLUTTER:-$(command -v flutter)} cache=${FLUTTER_PUB_CACHE:-${PUB_CACHE:-$HOME/.pub-cache}}
  [ -n "$fl" ] && [ -x "$fl" ] || { failed flutter 'no flutter command: set FLUTTER, or put flutter on PATH'; return; }
  [ -d "$cache/hosted" ] || { failed flutter "no packages in $cache: run flutter pub get once in toolkits/flutter, or set FLUTTER_PUB_CACHE"; return; }
  local src="$OUT/flutter/src" home="$OUT/flutter/home" nonet=()
  command -v bwrap >/dev/null && nonet=(bwrap --dev-bind / / --unshare-net --die-with-parent --)
  rm -rf "$src" && mkdir -p "$src" "$home/.pub-cache" || return
  cp -r "$HERE/flutter/pubspec.yaml" "$HERE/flutter/pubspec.lock" "$HERE/flutter/lib" "$HERE/flutter/linux" "$src/"
  ln -s "$cache/hosted" "$home/.pub-cache/hosted"
  # A HOME of its own keeps flutter's and dart's settings out of the real one; analytics off, no check for a newer
  # flutter, and no network.
  if (cd "$src" && export HOME="$home" XDG_CONFIG_HOME="$home/.config" PUB_CACHE="$home/.pub-cache" \
        FLUTTER_SUPPRESS_ANALYTICS=true CI=true &&
        "${nonet[@]}" "$fl" --no-version-check config --no-analytics &&
        "${nonet[@]}" "$fl" --no-version-check pub get --offline &&
        "${nonet[@]}" "$fl" --no-version-check build linux --release --no-pub) >"$OUT/flutter/build.log" 2>&1; then
    ln -sfn src/build/linux/x64/release/bundle/flutter-bench "$OUT/flutter/bin" && echo 'built flutter'
  else
    failed flutter "flutter build linux failed: $(last "$OUT/flutter/build.log")"
  fi
}

for t in "$@"; do
  mkdir -p "$OUT/$t"
  case $t in
    gtk4) build_gtk4 ;;
    qtquick) build_qtquick ;;
    gtk4rs|slint|iced) build_rust "$t" ;;
    flutter) build_flutter ;;
    *) failed "$t" 'no such toolkit (gtk4 gtk4rs qtquick slint iced flutter)' ;;
  esac
done
rm -rf "$OUT/cargo"  # the links, and the crates cargo unpacked
