#!/usr/bin/env bash
# Runs the debug build headless under Xvfb with Mesa's software Vulkan
# (lavapipe) and saves a screenshot. Extra environment (XDG dirs, BB_*)
# passes through.
#
# Usage: scripts/screenshot.sh out.png [seconds] [width x height]
#
# BB_KEYS='xdotool type "hello"; xdotool key Return' runs as a shell
# script halfway through the wait, to drive the app before the capture.
set -euo pipefail
out="$(realpath -m "$1")"
mkdir -p "$(dirname "$out")"
wait="${2:-6}"
geometry="${3:-1200x800}"
here="$(cd "$(dirname "$0")/.." && pwd)"
mesa=$(nix build --no-link --print-out-paths nixpkgs#mesa)
xvfb=$(nix build --no-link --print-out-paths nixpkgs#xvfb-run)
xwd=$(nix build --no-link --print-out-paths nixpkgs#xorg.xwd)
magick=$(nix build --no-link --print-out-paths nixpkgs#imagemagick)
xdotool=$(nix build --no-link --print-out-paths nixpkgs#xdotool)
keys=$(mktemp)
trap 'rm -f "$keys"' EXIT
printf 'PATH=%s/bin:$PATH\n%s\n' "$xdotool" "${BB_KEYS:-}" > "$keys"
half=$((wait / 2))
cd "$here"
nix develop --command bash -c "VK_ICD_FILENAMES=$mesa/share/vulkan/icd.d/lvp_icd.x86_64.json WAYLAND_DISPLAY= \
  $xvfb/bin/xvfb-run -a -s '-screen 0 ${geometry}x24' bash -c '\
    ./target/debug/bluebubbles-gpui & pid=\$!; sleep $half; \
    bash $keys; sleep \$(($wait - $half)); \
    $xwd/bin/xwd -root -silent | $magick/bin/magick xwd:- \"$out\"; kill \$pid'"
echo "$out"
