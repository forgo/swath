#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

# Scenario driver for `just load-udf` (issue #207) — `run_udf` live-latency
# evidence under the ADR 0012 guard. ASSUMES the compose stack is up with
# the fixture granule live AND a udf-store wired (tests/e2e/stack-up.sh;
# tests/e2e/swath-catalog.toml sets `udf-store = "/udf"`). Parameters come
# from ONE place, `tests/load/load_udf.py params`; that script also owns
# the publish motion, the HTTP probes, and the distilled baseline
# (docs/perf/load-udf-baseline.{json,md}).
#
#   (u) UDF storm   the reference NDVI UDF published as an xyz service and
#                   stormed on its heaviest Live tiles (cache-buster keeps
#                   them on the Live+UDF path) WHILE /healthz + SSE /traces
#                   are probed — the ADR 0012 signals, recorded/verdicted.
#   (f) fuel bomb   a runaway-loop UDF published just as cleanly, then
#                   refused on the tile path (500 RFC 7807 fuel) and the
#                   preview (400 ProcessGraphComplexity) with the SAME
#                   /healthz + SSE probes proving ZERO collateral.
set -euo pipefail

started=${1:?usage: load_udf.sh <recipe-start-epoch>}
base=${SWATH_LOAD_BASE:-http://localhost:8080}
out=target/load-udf
cache=target/e2e/cache

rm -rf "$out" && mkdir -p "$out"
eval "$(uv run tests/load/load_udf.py params)"

curl -sf -o /dev/null "$base/healthz" || { echo "FAIL: stack not healthy at $base"; exit 1; }
# run_udf must be offered (the udf-store is wired); otherwise publish 4xxs.
curl -sf "$base/processes" | grep -q '"run_udf"' \
    || { echo "FAIL: run_udf not offered — is udf-store wired in the catalog?"; exit 1; }

# --- publish both services ----------------------------------------------
echo "== load-udf: publishing the reference NDVI UDF service"
ndvi_id=$(uv run tests/load/load_udf.py publish --base "$base" ndvi)
echo "== load-udf: publishing the runaway fuel-bomb UDF service"
bomb_id=$(uv run tests/load/load_udf.py publish --base "$base" fuelbomb)
echo "== load-udf: ndvi=$ndvi_id bomb=$bomb_id"

# Tile URL files (the pinned suffixes prefixed with each published id).
: > "$out/udf-urls.txt"; : > "$out/bomb-urls.txt"
for suffix in $UDF_TILES; do
    echo "$base/tilesets/$ndvi_id/tiles/$suffix" >> "$out/udf-urls.txt"
    echo "$base/tilesets/$bomb_id/tiles/$suffix" >> "$out/bomb-urls.txt"
done
probe_tile="/tilesets/$ndvi_id/tiles/$UDF_PROBE_TILE"
bomb_tile="/tilesets/$bomb_id/tiles/$UDF_PROBE_TILE"

# --- pre-flight: the NDVI UDF tile serves Live with fuel; the bomb 500s ---
trace=$(curl -sf -o /dev/null -w '%header{x-swath-trace}' "$base$probe_tile")
grep -q '"udf_fuel_used"' <<<"$trace" \
    || { echo "FAIL: NDVI UDF tile served no udf_fuel_used: $trace"; exit 1; }
echo "== load-udf: NDVI UDF tile is Live through the module ($trace)"

# --- refusal evidence (recorded before the storms) ----------------------
uv run tests/load/load_udf.py preview --base "$base" ndvi --out "$out/ndvi-preview.json"
uv run tests/load/load_udf.py preview --base "$base" fuelbomb --out "$out/fuelbomb-preview.json"
uv run tests/load/load_udf.py tileprobe --base "$base" --tile "$bomb_tile" \
    --out "$out/fuelbomb-tileprobe.json"

# --- a reusable storm: SSE + oha storm + healthz probe (+ optional buster/probes) ---
# args: name urls-file healthz-out sse-name [buster] [probes-tile]
storm() {
    local name=$1 urls=$2 healthz_out=$3 sse_name=$4 buster=${5:-} probes_tile=${6:-}

    curl -sN --max-time "$UDF_SSE_WINDOW" "$base/traces" > "$out/sse-$sse_name.log" &
    local sse_pid=$!
    sleep 1

    oha --no-tui -w --output-format json -c "$UDF_STORM_CONNS" -z "$UDF_STORM_DURATION" \
        --urls-from-file "$urls" > "$out/$name.json" &
    local storm_pid=$!

    local buster_pid=""
    if [ "$buster" = "buster" ]; then
        # Keep the heavy tiles on the Live+UDF path (a UDF stage only runs
        # on a Live render); the write-through cache would otherwise turn
        # the storm into cache reads after one lap.
        ( while kill -0 "$storm_pid" 2>/dev/null; do
              find "$cache" -type f -delete 2>/dev/null || true
              sleep 0.25
          done ) &
        buster_pid=$!
    fi

    local probe_pid=""
    if [ -n "$probes_tile" ]; then
        ( uv run tests/load/load_udf.py probe --base "$base" --tile "$probes_tile" \
              --out "$out/storm-probes.json" ) &
        probe_pid=$!
    fi

    sleep "$UDF_HEALTHZ_DELAY"
    echo "== load-udf: ($name) /healthz under load (c=$UDF_HEALTHZ_CONNS, $UDF_HEALTHZ_LOAD_DURATION, ${UDF_HEALTHZ_DELAY}s in)"
    oha --no-tui -w --output-format json -c "$UDF_HEALTHZ_CONNS" -z "$UDF_HEALTHZ_LOAD_DURATION" \
        "$base/healthz" > "$healthz_out"

    wait "$storm_pid"
    [ -n "$buster_pid" ] && { wait "$buster_pid" 2>/dev/null || true; }
    [ -n "$probe_pid" ] && { wait "$probe_pid" || true; }
    local sse_rc=0
    wait "$sse_pid" || sse_rc=$?
    printf '{"curl_exit":%d}\n' "$sse_rc" > "$out/sse-$sse_name-meta.json"
}

echo "== load-udf: (u) UDF storm (c=$UDF_STORM_CONNS, $UDF_STORM_DURATION) + healthz + SSE + probes"
storm udf-storm "$out/udf-urls.txt" "$out/healthz-under-udf-storm.json" udf-storm buster "$probe_tile"

echo "== load-udf: (f) fuel-bomb storm (c=$UDF_STORM_CONNS, $UDF_STORM_DURATION) + healthz + SSE"
storm fuelbomb-storm "$out/bomb-urls.txt" "$out/healthz-under-fuelbomb.json" fuelbomb

# --- distill ------------------------------------------------------------
uv run tests/load/load_udf.py report --dir "$out" --started "$started" \
    --json docs/perf/load-udf-baseline.json --md docs/perf/load-udf-baseline.md
echo
cat docs/perf/load-udf-baseline.md
