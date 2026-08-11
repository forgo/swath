#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

# Scenario driver for `just load-h2h` (issue #121): Swath vs a pinned
# TiTiler on the ONE overlapping capability — serving a static COG as
# tiles. Same machine, same committed fixture COGs, same scenario
# parameters (tests/load/load.py via h2h.py — one source of truth), both
# containers pinned to the same CPU quota, run ONE AT A TIME. Teardown of
# both stacks stays the recipe's trap; this script also tears each side
# down as it finishes so the sides never share the machine.
#
# Scenarios per side (rationale in h2h.py):
#   healthz idle    each server's own liveness route, no load
#   repeated-tile   c=32/20s one truecolor z12 tile — architecture
#                   contrast (swath cache_hit vs titiler re-render)
#   cold burst      `just load` (b) verbatim: 128 unique z15 tiles,
#                   each exactly once — the honest render-vs-render row
#   heavy storm     c=16/40s over the 6 heaviest products; swath's cache
#                   cleared every 250 ms so BOTH sides render every hit
set -euo pipefail

started=${1:?usage: h2h.sh <recipe-start-epoch>}
out=target/h2h
cache=target/e2e/cache
titiler_name=swath-h2h-titiler
titiler_base=http://localhost:8000
swath_base=http://localhost:8080

eval "$(uv run tests/load/h2h.py params)"
cpus=$H2H_CPUS

# --- port discipline: refuse to start over anything already listening ----
if docker ps --format '{{.Names}} {{.Ports}}' | grep -E ':(8080|8000)->' ; then
    echo "FAIL: a container already publishes :8080 or :8000 (docker ps above) — take it down first"
    exit 1
fi
for port in 8080 8000; do
    if curl -sf -o /dev/null --max-time 2 "http://localhost:$port/" 2>/dev/null; then
        echo "FAIL: something already answers on :$port — free it first"
        exit 1
    fi
done

rm -rf "$out" && mkdir -p "$out/swath" "$out/titiler"

# Shared scenario runner. Args: side, base URL.
run_suite() {
    side=$1 base=$2
    dir=$out/$side

    uv run tests/load/h2h.py verify --side "$side" --base "$base"

    echo "== h2h[$side]: healthz idle (c=$H2H_HEALTHZ_CONNS, $H2H_HEALTHZ_DURATION)"
    oha --no-tui -w --output-format json -c "$H2H_HEALTHZ_CONNS" -z "$H2H_HEALTHZ_DURATION" \
        "$base/healthz" > "$dir/healthz-idle.json"

    hot_url=$(uv run tests/load/h2h.py urls --side "$side" --scenario hot)
    # Warm once on both sides (swath: populates its cache; titiler: warms
    # GDAL block/VSI caches) — then swath's cache path is ASSERTED, so the
    # architecture contrast in the report is proven, not presumed.
    curl -sf -o /dev/null "$base$hot_url"
    if [ "$side" = swath ]; then
        trace=$(curl -sf -o /dev/null -w '%header{x-swath-trace}' "$base$hot_url")
        grep -q '"decision":"cache_hit"' <<<"$trace" \
            || { echo "FAIL: hot tile not a cache_hit before the storm: $trace"; exit 1; }
    fi
    echo "== h2h[$side]: repeated-tile storm (c=$H2H_HOT_CONNS, $H2H_HOT_DURATION)"
    oha --no-tui -w --output-format json -c "$H2H_HOT_CONNS" -z "$H2H_HOT_DURATION" \
        "$base$hot_url" > "$dir/hot.json"

    echo "== h2h[$side]: cold burst (unique tiles, each exactly once)"
    uv run tests/load/h2h.py cold --side "$side" --base "$base" --out "$dir/cold.json"

    uv run tests/load/h2h.py urls --side "$side" --scenario heavy \
        | sed "s|^|$base|" > "$dir/heavy-urls.txt"
    echo "== h2h[$side]: heavy-tile storm (c=$H2H_HEAVY_CONNS, $H2H_HEAVY_DURATION)"
    oha --no-tui -w --output-format json -c "$H2H_HEAVY_CONNS" -z "$H2H_HEAVY_DURATION" \
        --urls-from-file "$dir/heavy-urls.txt" > "$dir/heavy.json" &
    storm_pid=$!

    if [ "$side" = swath ]; then
        # Cache-buster (as in `just load` (c)): clears tile-cache entries
        # every 250 ms so the storm stays on the Live path — the fair
        # condition against a server that renders every request. Probes
        # RECORD the actual decision mix rather than asserting it.
        (
            while kill -0 "$storm_pid" 2>/dev/null; do
                find "$cache" -type f -delete 2>/dev/null || true
                sleep 0.25
            done
        ) &
        buster_pid=$!
        (
            for _ in $(seq 1 "$H2H_PROBE_COUNT"); do
                curl -s -o /dev/null -w '%header{x-swath-trace}\n' "$base$H2H_PROBE_TILE" || true
                sleep "$H2H_PROBE_INTERVAL"
            done
        ) > "$dir/probes.txt" &
        probe_pid=$!
        wait "$storm_pid"
        wait "$buster_pid" 2>/dev/null || true
        wait "$probe_pid" || true
    else
        wait "$storm_pid"
    fi
}

# --- side 1: swath (the compose stack `just e2e`/`just load` use) --------
tests/e2e/stack-up.sh
swath_container=$(docker compose ps -q swath)
docker update --cpus "$cpus" "$swath_container" >/dev/null
echo "h2h: swath container pinned to --cpus $cpus"
run_suite swath "$swath_base"
docker compose down -v

# --- side 2: titiler, pinned by digest, its own documented tuning --------
# GDAL env: the recommended values from TiTiler's performance-tuning guide
# (https://developmentseed.org/titiler/advanced/performance_tuning/).
# Command: the docs' uvicorn invocation (https://developmentseed.org/titiler/)
# with one worker PER PINNED CPU (more generous than its `--workers 1`
# example). Same --cpus quota as swath; same fixture COGs, read-only.
uv run tests/load/h2h.py item --out "$out/item.json"
docker run -d --name "$titiler_name" --cpus "$cpus" -p 8000:8000 \
    -v "$PWD/tests/fixtures":/data/fixtures:ro \
    -v "$PWD/$out/item.json":/data/item.json:ro \
    -e GDAL_CACHEMAX=200 \
    -e VSI_CACHE=TRUE \
    -e VSI_CACHE_SIZE=5000000 \
    -e GDAL_BAND_BLOCK_CACHE=HASHSET \
    -e GDAL_DISABLE_READDIR_ON_OPEN=EMPTY_DIR \
    -e GDAL_HTTP_MERGE_CONSECUTIVE_RANGES=YES \
    "$H2H_TITILER_IMAGE" \
    uvicorn titiler.application.main:app --host 0.0.0.0 --port 8000 --workers "$cpus" >/dev/null
echo "h2h: titiler up ($H2H_TITILER_IMAGE, tag $H2H_TITILER_TAG, --cpus $cpus, --workers $cpus)"
for _ in $(seq 1 60); do
    curl -sf -o /dev/null "$titiler_base/healthz" && break
    sleep 1
done
curl -sf -o /dev/null "$titiler_base/healthz" \
    || { echo "FAIL: titiler not healthy within 60s"; docker logs "$titiler_name" | tail -20; exit 1; }
run_suite titiler "$titiler_base"
docker rm -f "$titiler_name" >/dev/null

# --- distill -------------------------------------------------------------
uv run tests/load/h2h.py report --dir "$out" --started "$started" --cpus "$cpus" \
    --json docs/perf/load-h2h-titiler.json --md docs/perf/load-h2h-titiler.md
echo
cat docs/perf/load-h2h-titiler.md
