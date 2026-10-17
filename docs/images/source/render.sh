#!/usr/bin/env bash
# Renders the README images from the pages in this folder: each page in a light and a dark version, at twice its
# size, as lossless WebP (the social preview as PNG, light only). Needs Google Chrome, Python 3 with Pillow, and a
# network connection for the Newsreader and Inter fonts.
#   ./render.sh            every image
#   ./render.sh hero       only hero-light.webp and hero-dark.webp
set -euo pipefail

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$(dirname "$SRC")"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# page width height
PAGES=(
  "hero 880 420"
  "decision 880 404"
  "bill 880 264"
  "measured 880 468"
  "social 1280 640"
)

shoot() { # page theme width height out.png
  google-chrome --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=2 \
    --default-background-color=00000000 --virtual-time-budget=8000 --window-size="$3,$4" \
    --screenshot="$5" "file://$SRC/$1.html?theme=$2" >/dev/null 2>&1
}

for entry in "${PAGES[@]}"; do
  read -r page width height <<<"$entry"
  if [[ $# -gt 0 && " $* " != *" $page "* ]]; then continue; fi
  if [[ $page == social ]]; then
    # GitHub's social preview: 1280×640 PNG, opaque, rendered at its own size.
    google-chrome --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
      --virtual-time-budget=8000 --window-size="$width,$height" \
      --screenshot="$OUT/social-preview.png" "file://$SRC/social.html" >/dev/null 2>&1
    echo "social-preview.png"
    continue
  fi
  for theme in light dark; do
    shoot "$page" "$theme" "$width" "$height" "$TMP/$page-$theme.png"
    python3 -c 'import sys; from PIL import Image; Image.open(sys.argv[1]).save(sys.argv[2], "WEBP", lossless=True, method=6)' \
      "$TMP/$page-$theme.png" "$OUT/$page-$theme.webp"
    echo "$page-$theme.webp"
  done
done
