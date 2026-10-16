#!/bin/sh
# Record the site / README clips: rgrip plays a scripted demo (debug build,
# RGRIP_DEMO_STEPS) while the window region is recorded, then ffmpeg makes
# an MP4 (site) and a GIF (README). Same setup as scripts/screenshots.sh.
# The window must stay on top while recording: don't touch anything.
# Usage: scripts/record.sh [clip…]   (default: all clips)
set -e
cd "$(dirname "$0")/.."
out=${RGRIP_CLIPS_OUT:-docs/clips}
mkdir -p "$out" target
cargo build -q -p rgrip
swiftc -O scripts/winid.swift -o target/winid 2>/dev/null
only=" $* "

clip() { # name, seconds, then env assignments
  name=$1; secs=$2; shift 2
  [ "$only" = "  " ] || case "$only" in *" $name "*) ;; *) return 0 ;; esac
  ready=$(mktemp -u); done_file=$(mktemp -u); mov=$(mktemp -u).mov
  env RGRIP_DEMO_CONN="rgrip dev (PostGIS)" RGRIP_DEMO_ONLY=1 RGRIP_DEMO_THEME=dark RGRIP_DEMO_PALETTE="rgrip Dark" RGRIP_DEMO_SIZE=1280x760 \
      RGRIP_DEMO_READY="$ready" RGRIP_DEMO_DONE="$done_file" "$@" target/debug/rgrip >/dev/null 2>&1 &
  pid=$!
  i=0
  while [ ! -f "$ready" ] && [ $i -lt 300 ]; do sleep 0.1; i=$((i + 1)); done
  set -- $(target/winid $pid)
  # Keep the pointer out of the clip.
  target/winid --park $(($2 + $4 + 30)) $(($3 + $5 / 2))
  screencapture -x -v -V "$secs" -R"$2,$3,$4,$5" "$mov"
  kill $pid 2>/dev/null || true
  wait $pid 2>/dev/null || true
  [ -f "$done_file" ] || echo "clip $name: script didn't finish in ${secs}s" >&2
  ffmpeg -loglevel error -y -i "$mov" -vf "fps=30,scale=1600:-2:flags=lanczos" -c:v libx264 -preset slow -crf 27 -pix_fmt yuv420p -movflags +faststart -an "$out/$name.mp4"
  ffmpeg -loglevel error -y -i "$mov" -vf "fps=12,scale=900:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" "$out/$name.gif"
  rm -f "$mov" "$ready" "$done_file"
  ls -la "$out/$name.mp4" "$out/$name.gif"
}

clip map 13 RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_SQL=" " \
  RGRIP_DEMO_STEPS="wait:500;type:select name, geom from demo.districts\\nunion all select parcel_code, geom from demo.parcels\\nunion all select name, geom from demo.places;wait:300;run;wait:1200;view:map;wait:3500;done"
clip relations 9 RGRIP_DEMO_EXPAND=legacy RGRIP_DEMO_TABLE=legacy.customers \
  RGRIP_DEMO_STEPS="wait:700;select:1;menu:1;wait:1800;key:Escape;related:orders;wait:3000;done"
clip edit 8 RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_TABLE=demo.owners \
  RGRIP_DEMO_STEPS="wait:700;editor:2:registered_on;wait:2000;edit:2:registered_on=2024-02-14;wait:800;edit:0:phone=+855 12 888 999;wait:2200;done"
clip wide 10 RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_TABLE=demo.parcels \
  RGRIP_DEMO_STEPS="wait:600;sort:price_usd:desc;wait:700;freeze:parcel_code;wait:600;goto;wait:300;keys:surv;wait:600;key:Enter;wait:2600;done"
clip commands 7 RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_TABLE=demo.parcels \
  RGRIP_DEMO_STEPS="wait:500;commands:parcels 3857;wait:2600;done"
clip linked 9 RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_TABLE=demo.parcels RGRIP_DEMO_TABLE_VIEW=split \
  RGRIP_DEMO_STEPS="wait:900;select:2;wait:1100;select:2,7;wait:1100;select:2,7,9,15;wait:1400;select:21;wait:1800;done"
