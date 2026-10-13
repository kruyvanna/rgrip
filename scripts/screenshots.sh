#!/bin/sh
# Capture the README / site screenshots from scripted scenes (debug build).
# Needs the dev PostGIS with demo data and a connection named "rgrip dev (PostGIS)":
#   docker compose -f dev/docker-compose.yml up -d
#   psql "host=localhost port=55432 user=rgrip dbname=rgrip" -f dev/demo.sql
# Only rgrip's own window is captured (by PID), at 2x on Retina.
set -e
cd "$(dirname "$0")/.."
out=docs/screenshots
mkdir -p "$out" target
cargo build -q -p rgrip
swiftc -O scripts/winid.swift -o target/winid 2>/dev/null

scene() { # name, then env assignments
  name=$1; shift
  ready=$(mktemp -u)
  env RGRIP_DEMO_CONN="rgrip dev (PostGIS)" RGRIP_DEMO_ONLY=1 RGRIP_DEMO_THEME=dark RGRIP_DEMO_SIZE=1440x900 \
      RGRIP_DEMO_READY="$ready" "$@" target/debug/rgrip >/dev/null 2>&1 &
  pid=$!
  i=0
  while [ ! -f "$ready" ] && [ $i -lt 300 ]; do sleep 0.1; i=$((i + 1)); done
  if [ -f "$ready" ] && wid=$(target/winid $pid); then
    screencapture -x -o -l "$wid" "$out/$name.png"
    echo "$out/$name.png"
  else
    echo "scene $name: not ready" >&2
  fi
  kill $pid 2>/dev/null || true
  wait $pid 2>/dev/null || true
  rm -f "$ready"
}

scene hero RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_SQL="select p.parcel_code, o.full_name, o.name_km as owner_km, d.name_km as district, p.land_use, p.area_m2, p.price_usd, p.surveyed_at, p.geom\nfrom demo.parcels p\njoin demo.owners o on o.id = p.owner_id\njoin demo.districts d on d.id = p.district_id\norder by p.price_usd desc;"
scene map RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_VIEW=map RGRIP_DEMO_SQL="select 'district' as kind, name_km as name, geom from demo.districts\nunion all select land_use, parcel_code, geom from demo.parcels\nunion all select category, name_km, geom from demo.places;"
scene edit RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_TABLE=demo.owners RGRIP_DEMO_EDITS="0:phone=+855 12 888 999;3:verified=t;5:name_km=ម៉ៅ សុខា" RGRIP_DEMO_EDITOR=2:registered_on
scene related RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_TABLE=demo.owners RGRIP_DEMO_SELECT=1 RGRIP_DEMO_RELATED=parcels
scene freeze RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_TABLE=demo.parcels RGRIP_DEMO_FREEZE=parcel_code,land_use RGRIP_DEMO_SELECT=2 RGRIP_DEMO_SCROLL_X=420
scene khmer RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_SQL="select name, name_km, category, opened_on, geom from demo.places order by opened_on;"
scene complete RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_COMPLETE="select p.| from demo.parcels p"
rm -f "$out/copy-image.png"
scene copy-image-window RGRIP_DEMO_TABLE=demo.owners RGRIP_DEMO_SELECT=0,1,2,3,4,5 RGRIP_DEMO_COPY_IMAGE="$PWD/$out/copy-image.png"
rm -f "$out/copy-image-window.png"
ls -la "$out"
