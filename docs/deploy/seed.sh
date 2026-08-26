#!/bin/sh
# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

# Seed phase of the read-only demo (docs/deploy/compose.yml, run as the
# `seed` service): hand the fresh volumes to the swath uid, drop the
# fire-season fixtures the image carries through the filedrop convention
# (tests/e2e/drop-granule.sh: band COGs first, manifest staged under an
# ignored dotfile name, renamed into place last), wait until the series
# serves, then publish the reference NDVI UDF service over it via the
# WRITABLE seed instance — the one `POST /services` this deployment ever
# sees. Everything persists (pgstac + the module store volume); the
# read-only instance restores the service at startup. POSIX sh: the
# runtime image is debian-slim without bash.
set -eu

base=${SWATH_SEED_BASE:-http://swath-seed:8080}
fixtures=/app/tests/fixtures
module=/seed/ndvi.wasm

# The image runs swath as 65534; the volumes were created root-owned.
chown 65534:65534 /data /cache /udf
mkdir -p /data/drop
chown 65534:65534 /data/drop

drop() { # dataset granule bbox datetime assets-json
  cp "$fixtures/$2"-*.tif /data/
  chown 65534:65534 /data/"$2"-*.tif
  printf '{"dataset":"%s","granule":"%s","bbox":%s,"datetime":"%s","assets":%s}\n' \
    "$1" "$2" "$3" "$4" "$5" > "/data/drop/.$2.json"
  mv "/data/drop/.$2.json" "/data/drop/$2.json"
}

hls() { # granule
  printf '{"b02":"%s-b02.tif","b03":"%s-b03.tif","b04":"%s-b04.tif","b8a":"%s-b8a.tif","fmask":"%s-fmask.tif"}' \
    "$1" "$1" "$1" "$1" "$1"
}
fire() { # granule
  printf '{"b04":"%s-b04.tif","b12":"%s-b12.tif","b8a":"%s-b8a.tif","fmask":"%s-fmask.tif"}' \
    "$1" "$1" "$1" "$1"
}

echo "seed: dropping the single-date HLS granule"
g=hlss30-t13sdd-2024158
drop hls-s30 "$g" '[-105.5370,39.1954,-105.3581,39.3345]' 2024-06-06T17:54:00Z "$(hls "$g")"

echo "seed: dropping the six-date Park Fire series"
bbox='[-121.7388,39.9856,-121.6475,40.0559]'
for pair in 2024159:2024-06-07T19:03:00Z 2024204:2024-07-22T19:03:00Z \
            2024229:2024-08-16T19:03:00Z 2024249:2024-09-05T19:03:00Z \
            2024274:2024-09-30T19:03:00Z 2024289:2024-10-15T19:03:00Z; do
  day=${pair%%:*}; when=${pair#*:}
  g="hlss30-t10tfk-$day"
  drop hls-s30-fire "$g" "$bbox" "$when" "$(fire "$g")"
done

echo "seed: waiting for the series to catalog and serve"
i=0
until curl -sf "$base/datasets/hls-s30-fire/granules" | grep -q '"numberMatched":6' \
   && [ "$(curl -s -o /dev/null -w '%{http_code}' "$base/tilesets/park-fire-ndvi/tiles/13/3100/1326")" = 200 ]; do
  i=$((i + 1))
  [ "$i" -lt 240 ] || { echo "FAIL: fire series not servable within 120s"; exit 1; }
  sleep 0.5
done
echo "seed: park fire series live"

curl -sf "$base/processes" | grep -q '"run_udf"' \
  || { echo "FAIL: run_udf not offered — is udf-store wired?"; exit 1; }

# The NDVI UDF product over the fire collection: load(b8a,b04) -> run_udf
# -> linear_scale_range(-1..1 -> 0..255) -> save_result — the graph the
# API tests and `just load-udf` publish (tests/load/load_udf.py), pointed
# at the time series so the service is playable.
echo "seed: publishing the reference NDVI UDF service over hls-s30-fire"
b64=$(base64 -w0 "$module")
cat > /tmp/service.json <<EOF
{"type":"xyz","title":"Park Fire NDVI (run_udf)","process":{"process_graph":{
 "load":{"process_id":"load_collection","arguments":{"id":"hls-s30-fire","spatial_extent":null,"temporal_extent":null,"bands":["b8a","b04"]}},
 "udf":{"process_id":"run_udf","arguments":{"data":{"from_node":"load"},"udf":"data:application/wasm;base64,$b64","runtime":"wasm","version":"1"}},
 "scale":{"process_id":"linear_scale_range","arguments":{"x":{"from_node":"udf"},"inputMin":-1,"inputMax":1,"outputMin":0,"outputMax":255}},
 "save":{"process_id":"save_result","arguments":{"data":{"from_node":"scale"},"format":"png"},"result":true}}}}
EOF
status=$(curl -s -o /tmp/publish.out -D /tmp/publish.hdr -w '%{http_code}' \
  -H 'content-type: application/json' --data-binary @/tmp/service.json "$base/services")
[ "$status" = 201 ] || { echo "FAIL: publish answered $status"; cat /tmp/publish.out; exit 1; }
id=$(tr -d '\r' < /tmp/publish.hdr | awk 'tolower($1)=="openeo-identifier:"{print $2}')
[ -n "$id" ] || { echo "FAIL: no openeo-identifier header"; exit 1; }

# Prove it renders through the module before declaring the seed done.
trace=$(curl -sf -o /dev/null -w '%header{x-swath-trace}' "$base/tilesets/$id/tiles/13/3100/1326")
echo "$trace" | grep -q '"udf_fuel_used"' \
  || { echo "FAIL: the UDF service tile served no udf_fuel_used: $trace"; exit 1; }

echo "seed: done — UDF service id: $id"
echo "seed: demo deep link: /?layer=$id&xray"
