#!/usr/bin/env bash
# Opens Mission Control on made-up worlds, so it can be tried, and pictured, without showing a real computer's folders:
#   mission/demo.sh [shot OUT.png [light|dark] [WIDTHxHEIGHT] [SCENE]]
# In a temp folder it makes a home with four small projects, an arbor state folder with a try of each and four more
# tries of one of them, a room working in each of those, two decisions waiting and a kept try, then opens the window there (or, with `shot`, draws it
# into OUT.png). Closing the window stops the rooms and removes the temp folder. It needs bwrap 0.10+ and, for the
# rooms' memory and CPU, a systemd user manager: it starts `arbor serve` if it is not running, and stops it again.
# It runs target/release/arbor and arbor-mission (`cargo build --release -p arbor -p mission` in prototype/).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="${CARGO_TARGET_DIR:-$HERE/../target}/release"
for b in arbor arbor-mission; do
  [ -x "$BIN/$b" ] || { echo "demo.sh: no $BIN/$b: run cargo build --release -p arbor -p mission in prototype/" >&2; exit 1; }
done
if [ "${1:-}" = shot ]; then
  [ -x "$BIN/examples/shot" ] || { echo "demo.sh: no $BIN/examples/shot: run cargo build --release -p mission --example shot" >&2; exit 1; }
fi

T="$(mktemp -d "${TMPDIR:-/tmp}/arbor-demo.XXXXXX")"
was="$(systemctl --user is-active arbor 2>/dev/null || true)"
cleanup() {
  # Stopping a room makes arbor record it, so the rooms are stopped before the folder goes.
  for c in $(ARBOR_HOME="$T/state" "$BIN/arbor" ps 2>/dev/null | awk 'NR > 1 { print $1 }'); do
    HOME="$T/home" ARBOR_HOME="$T/state" "$BIN/arbor" stop "$c" >/dev/null 2>&1 || true
  done
  if [ "$was" != active ] && [ "$(systemctl --user is-active arbor 2>/dev/null)" = active ] &&
     [ -z "$(find "/sys/fs/cgroup$(systemctl --user show -p ControlGroup --value arbor)/cells" -mindepth 1 -maxdepth 1 -type d 2>/dev/null)" ]; then
    systemctl --user stop arbor
  fi
  chmod -R u+rwX "$T" 2>/dev/null || true # overlay work folders are made without permissions
  rm -rf "$T"
}
trap cleanup EXIT

export HOME="$T/home" ARBOR_HOME="$T/state"
a() { "$BIN/arbor" "$@"; }
mkdir -p "$HOME/projects"/{help-pages,shop,site,notes}
cd "$HOME/projects"
printf '# Help pages\n' >help-pages/README.md
printf '#!/bin/sh\n# translates the help pages into one language (here: holds some memory and waits)\nexec python3 -c "import time; b = bytearray($1 * 1000000); time.sleep(3600)"\n' >help-pages/translate
chmod +x help-pages/translate
printf 'fn total() {}\n' >shop/cart.rs
printf '<h1>Shop</h1>\n' >site/index.html
# What the site's decision runs once released: it takes a moment, then says what it did (made up; nothing leaves).
printf '#!/bin/sh\nsleep 2\necho "deployed the gallery to the test site: 3 pages, 12 images"\n' >site/deploy
chmod +x site/deploy
printf 'call the bookkeeper\n' >notes/todo.txt

a new help-pages --from ~/projects/help-pages >/dev/null
for lang in es fr de it; do a new "help-$lang" --from ~/projects/help-pages >/dev/null; done
a new shop --from ~/projects/shop >/dev/null
a new site --from ~/projects/site >/dev/null
a new notes --from ~/projects/notes >/dev/null
a key new translate --for 6h >/dev/null
a run shop -- sh -c 'printf "fn total() -> u32 { 0 }\n" > cart.rs; mkdir -p tests; echo "#[test] fn t() {}" > tests/cart.rs' >/dev/null
a run shop -- arbor effect --cost 0.40 --why 'publish the fix for rounding the total' -- git push origin fix >/dev/null
a run site -- sh -c 'echo "<img loading=lazy>" >> index.html' >/dev/null
a run site -- arbor effect --why 'deploy the faster gallery to the test site' -- ./deploy --to test-site >/dev/null
a run notes -- sh -c 'echo "call the bookkeeper on Monday" > todo.txt' >/dev/null
a keep notes >/dev/null
# A room in each translation's try (a world runs one command at a time). A limit gives each a cgroup of its own,
# so its memory and CPU can be read; the first one starts `arbor serve` if it is not running.
a run help-pages --pids 64 -- true >/dev/null
for t in es:42 fr:57 de:38 it:61; do
  (cd help-pages && a run "help-${t%:*}" --key translate --mem 1G -- ./translate "${t#*:}" >/dev/null 2>&1 &)
done
for _ in $(seq 50); do
  [ "$(a ps 2>/dev/null | awk 'NR > 1' | wc -l)" -ge 4 ] && break
  sleep 0.1
done

if [ "${1:-}" = shot ]; then
  shift
  "$BIN/examples/shot" "$@"
else
  "$BIN/arbor-mission"
fi
