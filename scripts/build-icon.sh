#!/bin/sh
# Render assets/logo.svg into the PNG and .icns files used by the app.
# Uses Quick Look (WebKit) for SVG rendering, sips and iconutil — all built into macOS.
set -e
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

qlmanage -t -s 1024 -o "$tmp" assets/logo.svg >/dev/null
master="$tmp/logo.svg.png"

iconset="$tmp/rgrip.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z $size $size "$master" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z $double $double "$master" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o assets/rgrip.icns
# Window/Dock icon embedded in the binary.
sips -z 512 512 "$master" --out assets/icon-512.png >/dev/null
echo "assets/rgrip.icns and assets/icon-512.png updated"
