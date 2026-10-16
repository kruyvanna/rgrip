#!/bin/sh
# Capture the README / site screenshots from scripted scenes (debug build).
# Needs the dev PostGIS with demo data and a connection named "rgrip dev (PostGIS)":
#   docker compose -f dev/docker-compose.yml up -d
#   psql "host=localhost port=55432 user=rgrip dbname=rgrip" -f dev/demo.sql
# Only rgrip's own window is captured (by PID), at 2x on Retina. The demo
# windows take focus: don't type while it runs.
# Usage: scripts/screenshots.sh [scene…]   (default: all scenes)
set -e
only=" $* "
cd "$(dirname "$0")/.."
out=${RGRIP_SHOTS_OUT:-docs/screenshots}
mkdir -p "$out" target
cargo build -q -p rgrip
swiftc -O scripts/winid.swift -o target/winid 2>/dev/null

scene() { # name, then env assignments
  name=$1; shift
  [ "$only" = "  " ] || case "$only" in *" $name "*) ;; *) return 0 ;; esac
  ready=$(mktemp -u)
  env RGRIP_DEMO_CONN="rgrip dev (PostGIS)" RGRIP_DEMO_ONLY=1 RGRIP_DEMO_THEME=dark RGRIP_DEMO_PALETTE="rgrip Dark" RGRIP_DEMO_SIZE=1440x900 \
      RGRIP_DEMO_READY="$ready" "$@" target/debug/rgrip >/dev/null 2>&1 &
  pid=$!
  i=0
  while [ ! -f "$ready" ] && [ $i -lt 300 ]; do sleep 0.1; i=$((i + 1)); done
  if [ -f "$ready" ] && wid=$(target/winid $pid | cut -d" " -f1); then
    screencapture -x -o -l "$wid" "$out/$name.png"
    echo "$out/$name.png"
  else
    echo "scene $name: not ready" >&2
  fi
  kill $pid 2>/dev/null || true
  wait $pid 2>/dev/null || true
  rm -f "$ready"
}

scene hero RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_SQL="select p.parcel_code, o.full_name as owner, d.name as district, p.land_use, p.area_m2, p.price_usd, p.surveyed_at, p.geom\nfrom demo.parcels p\njoin demo.owners o on o.id = p.owner_id\njoin demo.districts d on d.id = p.district_id\norder by p.price_usd desc;"
scene map RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_VIEW=map RGRIP_DEMO_SQL="select 'district' as kind, name, geom from demo.districts\nunion all select land_use, parcel_code, geom from demo.parcels\nunion all select category, name, geom from demo.places;"
scene edit RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_TABLE=demo.owners RGRIP_DEMO_EDITS="0:phone=+855 12 888 999;3:verified=t;5:full_name=Mao Sokha" RGRIP_DEMO_EDITOR=2:registered_on
scene related RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_TABLE=demo.owners RGRIP_DEMO_SELECT=1 RGRIP_DEMO_RELATED=parcels
scene freeze RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_TABLE=demo.parcels RGRIP_DEMO_FREEZE=parcel_code,land_use RGRIP_DEMO_SELECT=2 RGRIP_DEMO_SCROLL_X=420
scene complete RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_COMPLETE="select p.| from demo.parcels p"
scene tabs RGRIP_DEMO_EXPAND=demo,legacy RGRIP_DEMO_COLOR=red RGRIP_DEMO_SCHEMA=demo RGRIP_DEMO_MORE=demo.owners,legacy.orders RGRIP_DEMO_SQL="select parcel_code, land_use, area_m2, price_usd, surveyed_at\nfrom parcels\norder by price_usd desc;"
scene database RGRIP_DEMO_DATABASE=shop RGRIP_DEMO_SCHEMA=sales RGRIP_DEMO_SQL="select * from invoices\norder by total_usd desc;"
scene search RGRIP_DEMO_FILTER=order RGRIP_DEMO_SCHEMA=demo,legacy RGRIP_DEMO_SQL="select o.id, c.name as customer, o.total_usd, p.parcel_code\nfrom orders o\njoin customers c on c.id = o.customer_id\njoin parcels p on p.id = o.id\norder by o.total_usd desc;"
scene tabmenu RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_SQL="select 1;" RGRIP_DEMO_MORE=demo.owners,legacy.orders RGRIP_DEMO_TAB_MENU=1
scene connmenu RGRIP_DEMO_SQL="select 1;" RGRIP_DEMO_CONN_MENU=0
scene sticky RGRIP_DEMO_EXPAND=public,tiger RGRIP_DEMO_EXPLORER_SCROLL=420 RGRIP_DEMO_SQL="select 1;"
scene split RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_TABLE=demo.parcels RGRIP_DEMO_TABLE_VIEW=split RGRIP_DEMO_SELECT=2,7,9,15
scene import RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_IMPORT="$PWD/crates/core/tests/fixtures/places.csv"
scene import-markets RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_IMPORT="$PWD/dev/sample/markets.csv"
scene import-geojson RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_IMPORT="$PWD/crates/core/tests/fixtures/places.geojson"
scene edit-table RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_EDIT_TABLE=demo.owners
scene confirm RGRIP_DEMO_COLOR=red RGRIP_DEMO_SQL="update demo.owners set verified = verified;"
scene goto RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_TABLE=demo.parcels RGRIP_DEMO_SORT=price_usd:desc RGRIP_DEMO_GOTO=sur
scene rowmenu RGRIP_DEMO_EXPAND=legacy RGRIP_DEMO_TABLE=legacy.customers RGRIP_DEMO_ROW_MENU=1
scene inferred RGRIP_DEMO_EXPAND=legacy RGRIP_DEMO_TABLE=legacy.customers RGRIP_DEMO_SELECT=1 RGRIP_DEMO_RELATED=orders
scene commands RGRIP_DEMO_EXPAND=demo RGRIP_DEMO_TABLE=demo.parcels RGRIP_DEMO_COMMANDS="parcels 3857"
[ "$only" = "  " ] || case "$only" in *" copy-image "*) only="  " ;; *) exit 0 ;; esac
rm -f "$out/copy-image.png"
scene copy-image-window RGRIP_DEMO_TABLE=demo.owners RGRIP_DEMO_SELECT=0,1,2,3,4,5 RGRIP_DEMO_COPY_IMAGE="$PWD/$out/copy-image.png"
rm -f "$out/copy-image-window.png"
ls -la "$out"
