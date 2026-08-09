#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

# Shared bring-up for the compose e2e suites (`just e2e`, `just e2e-web`)
# and the human-facing stopwatch demo (`just demo`):
# build the swath image, start the full stack, verify infra health, assert
# the honest pre-drop 404, drop the fixture granule (the filedrop
# convention: band COGs first, manifest renamed into place last), and poll
# until the tile is live. Leaves target/e2e/tile.png + tile-headers.txt
# for callers' further assertions. Teardown is the CALLER's job
# (trap 'docker compose down -v' EXIT) — this script only brings up.
set -euo pipefail

dir=target/e2e
granule=hlss30-t13sdd-2024158
# The mounted data plane must exist (and be empty) before `up`.
rm -rf "$dir" && mkdir -p "$dir/store/drop"
docker compose build swath
start=$(date +%s)
docker compose up -d --wait
echo "stack healthy in $(( $(date +%s) - start ))s (pull/start -> all healthchecks green)"
docker compose exec -T pgstac psql -qtA -c "select pgstac.get_version();" | grep -E '^[0-9.]+' \
    && echo "pgstac: migrations present"
curl -sf http://localhost:9000/minio/health/live && echo "minio: live"
base=http://localhost:8080
# Landing page (OGC API root) answers with the Swath document.
curl -sf "$base/" | grep -q '"title":"Swath"' && echo "swath: landing page OK"
# The catalog-backed dataset registered at startup: visible to a plain
# STAC client (R5) before any granule exists.
docker compose exec -T pgstac psql -qtA -c \
    "select pgstac.get_collection('hls-s30') is not null;" | grep -qx t \
    && echo "pgstac: hls-s30 dataset registered (plain STAC visibility)"
# R1 pre-condition: the layer exists, its pixels don't — a tile of the
# empty catalog is an honest 404.
tile="$base/tilesets/truecolor/tiles/12/1561/848"
code=$(curl -s -o /dev/null -w '%{http_code}' "$tile")
[ "$code" = "404" ] || { echo "FAIL: expected 404 before any granule, got $code"; exit 1; }
echo "swath: tile is 404 before ingest (catalog empty)"
# `just demo` sets SWATH_DROP_COUNTDOWN so a human watching the map sees
# the before-state (the honest 404 gray) turn into imagery: hold the drop
# for a visible countdown. Default 0 — the e2e suites drop immediately.
countdown=${SWATH_DROP_COUNTDOWN:-0}
if [ "$countdown" -gt 0 ]; then
    for i in $(seq "$countdown" -1 1); do
        printf '\r  granule drops in %2ds — watch the map ' "$i"
        sleep 1
    done
    printf '\r%45s\r' ''
fi
# THE DROP (the manual step count for everything below: zero). Per the
# filedrop convention: band COGs land first, the manifest is staged
# under an ignored dotfile name and renamed into place last.
cp tests/fixtures/$granule-*.tif "$dir/store/"
printf '%s\n' \
  '{' \
  '  "dataset": "hls-s30",' \
  "  \"granule\": \"$granule\"," \
  '  "bbox": [-106.1, 39.2, -105.9, 39.4],' \
  '  "datetime": "2024-06-06T17:54:00Z",' \
  '  "assets": {' \
  "    \"b02\": \"$granule-b02.tif\"," \
  "    \"b03\": \"$granule-b03.tif\"," \
  "    \"b04\": \"$granule-b04.tif\"," \
  "    \"b8a\": \"$granule-b8a.tif\"," \
  "    \"fmask\": \"$granule-fmask.tif\"" \
  '  }' \
  '}' > "$dir/store/drop/.$granule.json"
mv "$dir/store/drop/.$granule.json" "$dir/store/drop/$granule.json"
echo "swath: granule dropped at $(date -u '+%H:%M:%S') UTC"
# Arrive -> catalog -> serve, automatically: poll until the tile is live.
code=000
for _ in $(seq 1 120); do
    code=$(curl -s -D "$dir/tile-headers.txt" -o "$dir/tile.png" -w '%{http_code}' "$tile")
    [ "$code" = "200" ] && break
    sleep 0.5
done
[ "$code" = "200" ] || { echo "FAIL: tile not servable within 60s of the drop (last: $code)"; exit 1; }
echo "swath: tile went live with zero manual steps (R1)"
