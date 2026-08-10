#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

# Shared bring-up for the compose e2e suites (`just e2e`, `just e2e-web`)
# and the human-facing stopwatch demo (`just demo`) — process lifecycle
# ONLY (issue #98): build the swath image, start the full stack, verify
# infra health. Teardown is the CALLER's job (trap 'docker compose down
# -v' EXIT) — this script only brings up.
#
# `just e2e` sets SWATH_STACK_UP_ONLY=1 and stops here: the swath-e2e
# harness owns every API assertion from the honest pre-drop 404 onward
# (it triggers the drop itself, via the shared drop-granule.sh).
#
# `just e2e-web` and `just demo` have no Rust toolchain in their path, so
# for them this script continues: drop the fixture granule and poll until
# the tile is live (readiness, not assertion), leaving target/e2e/tile.png
# + tile-headers.txt for the demo's ingest-to-pixel readout.
set -euo pipefail

dir=target/e2e
# The mounted data plane must exist (and be empty) before `up`. The tile
# cache (#36) gets its own writable mount — world-writable because the
# container runs as uid 65534 (local-dev-only bind mount, never real infra).
rm -rf "$dir" && mkdir -p "$dir/store/drop" "$dir/cache"
chmod 777 "$dir/cache"
docker compose build swath
start=$(date +%s)
docker compose up -d --wait
echo "stack healthy in $(( $(date +%s) - start ))s (pull/start -> all healthchecks green)"
docker compose exec -T pgstac psql -qtA -c "select pgstac.get_version();" | grep -E '^[0-9.]+' \
    && echo "pgstac: migrations present"
curl -sf http://localhost:9000/minio/health/live && echo "minio: live"

if [ "${SWATH_STACK_UP_ONLY:-0}" = "1" ]; then
    exit 0
fi

# `just demo` sets SWATH_DROP_COUNTDOWN so a human watching the map sees
# the before-state (the honest 404 gray) turn into imagery: hold the drop
# for a visible countdown. Default 0 — e2e-web drops immediately.
countdown=${SWATH_DROP_COUNTDOWN:-0}
if [ "$countdown" -gt 0 ]; then
    for i in $(seq "$countdown" -1 1); do
        printf '\r  granule drops in %2ds — watch the map ' "$i"
        sleep 1
    done
    printf '\r%45s\r' ''
fi
tests/e2e/drop-granule.sh
# Arrive -> catalog -> serve, automatically: poll until the tile is live
# (readiness for the Playwright suite / the demo — the e2e ASSERTION of
# this path lives in swath-e2e's tile_live_within_60s_of_drop).
base=http://localhost:8080
tile="$base/tilesets/truecolor/tiles/12/1561/848"
code=000
for _ in $(seq 1 120); do
    code=$(curl -s -D "$dir/tile-headers.txt" -o "$dir/tile.png" -w '%{http_code}' "$tile")
    [ "$code" = "200" ] && break
    sleep 0.5
done
[ "$code" = "200" ] || { echo "FAIL: tile not servable within 60s of the drop (last: $code)"; exit 1; }
echo "swath: tile went live with zero manual steps (R1)"
