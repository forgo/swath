#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

# Scenario driver for `just load` (issue #101). ASSUMES the compose stack
# is already up with the fixture granule live (tests/e2e/stack-up.sh —
# the single owner of stack lifecycle; teardown stays the recipe's trap).
# Orchestrates the pinned scenarios — parameters come from ONE place,
# `tests/load/load.py params` — then distills the committed baseline
# (docs/perf/load-baseline.{json,md}) and prints the table.
#
#   healthz idle  a cheap no-load reference point
#   (a)           hot-cache tile storm (oha, pre-warmed, cache_hit proven)
#   (b)           cold live-render burst (unique tiles exactly once —
#                 load.py cold; oha can't guarantee a never-repeated set)
#   (c)           mixed: oha storm over the heaviest tiles kept on the
#                 Live path by a cache-buster loop, WHILE a second oha
#                 measures /healthz and curl holds an SSE /traces
#                 subscription — the ARCHITECTURE §16.7 evidence.
set -euo pipefail

started=${1:?usage: load.sh <recipe-start-epoch>}
base=${SWATH_LOAD_BASE:-http://localhost:8080}
out=target/load
cache=target/e2e/cache

rm -rf "$out" && mkdir -p "$out"
eval "$(uv run tests/load/load.py params)"

curl -sf -o /dev/null "$base/healthz" || { echo "FAIL: stack not healthy at $base"; exit 1; }

echo "== load: /healthz idle baseline (c=$LOAD_HEALTHZ_CONNS, $LOAD_HEALTHZ_IDLE_DURATION)"
oha --no-tui -w --output-format json -c "$LOAD_HEALTHZ_CONNS" -z "$LOAD_HEALTHZ_IDLE_DURATION" \
    "$base/healthz" > "$out/healthz-idle.json"

# --- (a) hot-cache tile storm -------------------------------------------
# Pre-warm the proven tile and PROVE the storm hits the cache path.
curl -sf -o /dev/null "$base$LOAD_HOT_TILE"
trace=$(curl -sf -o /dev/null -w '%header{x-swath-trace}' "$base$LOAD_HOT_TILE")
grep -q '"decision":"cache_hit"' <<<"$trace" \
    || { echo "FAIL: hot tile not a cache_hit before the storm: $trace"; exit 1; }
echo "== load: (a) hot-cache tile storm (c=$LOAD_HOT_CONNS, $LOAD_HOT_DURATION)"
oha --no-tui -w --output-format json -c "$LOAD_HOT_CONNS" -z "$LOAD_HOT_DURATION" \
    "$base$LOAD_HOT_TILE" > "$out/hot.json"

# --- (b) cold live-render burst -----------------------------------------
echo "== load: (b) cold live-render burst ($LOAD_COLD_COUNT unique z$LOAD_COLD_ZOOM tiles, c=$LOAD_COLD_CONNS)"
uv run tests/load/load.py cold --base "$base" --out "$out/cold.json"

# --- (c) mixed: warps + /healthz + SSE ----------------------------------
for path in $LOAD_MIXED_TILES; do echo "$base$path"; done > "$out/mixed-urls.txt"

echo "== load: (c) mixed storm (c=$LOAD_MIXED_CONNS, $LOAD_MIXED_DURATION) + /healthz + SSE /traces"
# SSE subscription first, held across the whole window: exit 28
# (--max-time expired) is the SURVIVED signal, anything else = died early.
curl -sN --max-time "$LOAD_SSE_WINDOW" "$base/traces" > "$out/sse.log" &
sse_pid=$!
sleep 1

oha --no-tui -w --output-format json -c "$LOAD_MIXED_CONNS" -z "$LOAD_MIXED_DURATION" \
    --urls-from-file "$out/mixed-urls.txt" > "$out/mixed.json" &
storm_pid=$!

# Cache-buster: the write-through cache would turn the storm into cache
# hits after one lap of 6 URLs; clearing entries (files only — the
# server's dirs stay) every 250 ms keeps large warps in flight, which is
# the §16.7 condition. Probes below RECORD the actual decision mix.
(
    while kill -0 "$storm_pid" 2>/dev/null; do
        find "$cache" -type f -delete 2>/dev/null || true
        sleep 0.25
    done
) &
buster_pid=$!

# Decision probes: sample what the storm is actually being served.
(
    for _ in $(seq 1 "$LOAD_PROBE_COUNT"); do
        curl -s -o /dev/null -w '%header{x-swath-trace}\n' "$base$LOAD_PROBE_TILE" || true
        sleep "$LOAD_PROBE_INTERVAL"
    done
) > "$out/probes.txt" &
probe_pid=$!

sleep "$LOAD_HEALTHZ_DELAY"
echo "== load: (c) /healthz under warps (c=$LOAD_HEALTHZ_CONNS, $LOAD_HEALTHZ_LOAD_DURATION, ${LOAD_HEALTHZ_DELAY}s into the storm)"
oha --no-tui -w --output-format json -c "$LOAD_HEALTHZ_CONNS" -z "$LOAD_HEALTHZ_LOAD_DURATION" \
    "$base/healthz" > "$out/healthz-under-warps.json"

wait "$storm_pid"
wait "$buster_pid" 2>/dev/null || true
wait "$probe_pid" || true
sse_rc=0
wait "$sse_pid" || sse_rc=$?
printf '{"curl_exit":%d}\n' "$sse_rc" > "$out/sse-meta.json"

# --- distill ------------------------------------------------------------
uv run tests/load/load.py report --dir "$out" --started "$started" \
    --json docs/perf/load-baseline.json --md docs/perf/load-baseline.md
echo
cat docs/perf/load-baseline.md
