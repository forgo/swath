#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

# Scenario driver for `just load-temporal` (issue #184). ASSUMES the
# compose stack is already up with the single-date fixture granule live
# (tests/e2e/stack-up.sh — the single owner of stack lifecycle; teardown
# stays the recipe's trap) and a release `swath` binary built. Parameters
# come from ONE place, `tests/load/temporal.py params`; that script also
# owns the HTTP loops and the distilled baseline
# (docs/perf/temporal-baseline.{json,md}).
#
#   (d) frames    the Park Fire series is dropped (shared
#                 drop-fire-granules.sh) and the slider's request loop is
#                 replayed twice: cold (fresh tile cache — every dated
#                 frame is a Live render) then hot (same loop — every
#                 frame a granule-scoped cache hit).
#   (e) overview  a zoom ladder of single-tile render rungs, tile cache
#                 cleared before every request and an SSE /traces capture
#                 held per rung (the envelope carries the overview LEVEL
#                 the header decision cannot): z10 before materialization
#                 (embedded x2 overview), then `swath materialize` over
#                 the store (timed), then z11 (pyramid x2), z10 (pyramid
#                 x4), and the z12 Live comparator.
set -euo pipefail

started=${1:?usage: temporal.sh <recipe-start-epoch>}
base=${SWATH_LOAD_BASE:-http://localhost:8080}
out=target/load-temporal
cache=target/e2e/cache
store=target/e2e/store

rm -rf "$out" && mkdir -p "$out"
eval "$(uv run tests/load/temporal.py params)"

curl -sf -o /dev/null "$base/healthz" || { echo "FAIL: stack not healthy at $base"; exit 1; }
[ -x target/release/swath ] || { echo "FAIL: target/release/swath missing (the recipe builds it)"; exit 1; }

# --- (d) the fire drop + the animation frame loop, cold then hot --------
tests/e2e/drop-fire-granules.sh
matched=0
for _ in $(seq 1 120); do
    matched=$(curl -sf "$base/datasets/hls-s30-fire/granules?limit=1" \
        | sed -n 's/.*"numberMatched":\([0-9][0-9]*\).*/\1/p')
    [ "${matched:-0}" -ge 6 ] && break
    sleep 0.5
done
[ "${matched:-0}" -ge 6 ] || { echo "FAIL: fire series not cataloged within 60s (matched: ${matched:-0})"; exit 1; }
echo "== load-temporal: all $matched fire granules cataloged"

find "$cache" -type f -delete 2>/dev/null || true
echo "== load-temporal: (d) frame loop, cold (fresh cache — all Live)"
uv run tests/load/temporal.py frames --base "$base" --out "$out/frames-cold.json"
echo "== load-temporal: (d) frame loop, hot (same loop — all cache hits)"
uv run tests/load/temporal.py frames --base "$base" --out "$out/frames-hot.json"

# --- (e) the overview zoom ladder, around `swath materialize` -----------
# Each rung runs under its own SSE /traces capture; the subscription only
# sees renders from connection time on, so start it first and give it a
# beat to attach.
rung() { # name tile
    curl -sN "$base/traces" > "$out/sse-$1.log" &
    local sse_pid=$!
    sleep 1
    uv run tests/load/temporal.py overview --base "$base" --tile "$2" \
        --cache "$cache" --out "$out/$1.json"
    sleep 0.5
    kill "$sse_pid" 2>/dev/null || true
    wait "$sse_pid" 2>/dev/null || true
}

echo "== load-temporal: (e) z10 rung BEFORE materialize (embedded overview only)"
rung overview_embedded_z10 "$TEMPORAL_OV_Z10_TILE"

# Materialize the pyramid ladder for the whole store, host-side (the
# container mounts /data read-only — writers live outside, ARCHITECTURE
# §8). The config is the compose stack's own file with its container
# paths swapped for the host's, so layer definitions stay single-sourced.
sed -e 's|^store-root = .*|store-root = "'"$store"'"|' \
    -e 's|^cache = .*|cache = "'"$cache"'"|' \
    -e 's|@pgstac:|@localhost:|' \
    -e 's|^watch-dir = .*|watch-dir = "'"$store"'/drop"|' \
    tests/e2e/swath-catalog.toml > "$out/materialize.toml"
echo "== load-temporal: swath materialize --min-dim $TEMPORAL_MATERIALIZE_MIN_DIM (timed)"
mat_start=$(python3 -c 'import time; print(time.time_ns())')
target/release/swath materialize --config "$out/materialize.toml" \
    --min-dim "$TEMPORAL_MATERIALIZE_MIN_DIM"
mat_ms=$(python3 -c "import time; print(round((time.time_ns() - $mat_start) / 1e6))")
printf '{"wall_ms":%d,"command":"swath materialize --min-dim %d","min_dim":%d}\n' \
    "$mat_ms" "$TEMPORAL_MATERIALIZE_MIN_DIM" "$TEMPORAL_MATERIALIZE_MIN_DIM" \
    > "$out/materialize.json"
echo "== load-temporal: materialized in ${mat_ms} ms"

echo "== load-temporal: (e) z11 rung (pyramid overview x2)"
rung overview_pyramid_z11 "$TEMPORAL_OV_Z11_TILE"
echo "== load-temporal: (e) z10 rung (pyramid overview x4)"
rung overview_pyramid_z10 "$TEMPORAL_OV_Z10_TILE"
echo "== load-temporal: (e) z12 rung (the Live comparator)"
rung overview_live_z12 "$TEMPORAL_OV_LIVE_TILE"

# --- distill ------------------------------------------------------------
uv run tests/load/temporal.py report --dir "$out" --started "$started" \
    --json docs/perf/temporal-baseline.json --md docs/perf/temporal-baseline.md
echo
cat docs/perf/temporal-baseline.md
