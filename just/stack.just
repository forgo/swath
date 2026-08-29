# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# The compose stack: the CI-asserted stopwatch demo and its interactive twin.
# Imported by the root justfile; recipe names are global (`just <recipe>`).

# The compose-stack e2e — now THE north-star demo path (issues #15/#29/#31,
# REQUIREMENTS.md R1/R8 + §3): build the swath image, bring up the full local
# stack (swath in catalog mode + pgstac + MinIO, via tests/e2e/stack-up.sh —
# lifecycle only), then hand over to the typed Rust harness (issue #98,
# crates/swath-e2e), which drives granule-to-live-tile end to end with zero
# manual steps: honest 404 while the catalog is empty, the granule drop,
# poll-to-live, ingest_to_pixel_ms under the north-star budget (emitted as a
# machine-readable JSON line + target/e2e/metrics.json), cache-hit byte
# stability, pdiff golden matches, typed SSE trace assertions, the openEO
# authoring round trip, and the declared-bounds check. Every check is named;
# failures name endpoint/expected/actual. Teardown is trap-based: the stack
# never outlives the recipe.
e2e:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'docker compose down -v' EXIT
    SWATH_STACK_UP_ONLY=1 tests/e2e/stack-up.sh
    cargo run --quiet -p swath-e2e
    # The standard openEO Python client against the same live stack
    # (#195, docs/RECIPES.md): connect -> discover -> build -> download.
    # No bespoke SDK — the pinned stock client is the compatibility bar.
    uv run tests/openeo/client_check.py

# THE stopwatch demo (issue #35, CHARTER.md §10 Phase 1): the same
# north-star path the e2e asserts forever, run for human eyes. Brings up
# the full stack (shared tests/e2e/stack-up.sh), serves the viewer, then
# holds the granule drop behind a countdown so you can watch the honest
# 404-gray turn into imagery on the map — with the x-ray overlay narrating
# every tile. Prints the measured ingest-to-pixel number at the end and
# stays up until ctrl-c (trap-based teardown). Needs `just setup-web`.
demo countdown="15":
    #!/usr/bin/env bash
    set -euo pipefail
    # Refuse to start over a live sibling: a previous `just demo` (waiting on
    # ctrl-c) or a running e2e shares the compose project, drop dir, and port
    # 5173 — colliding silently made runs look flaky. Fail loudly instead.
    if lsof -ti :5173 >/dev/null 2>&1; then
        echo "FAIL: port 5173 is busy — a previous demo/vite is still running (ctrl-c it first)"; exit 1
    fi
    if curl -sf http://localhost:8080/healthz >/dev/null 2>&1; then
        echo "FAIL: a swath stack is already up on :8080 — 'docker compose down -v' first"; exit 1
    fi
    vite=""
    teardown() {
        [ -n "$vite" ] && kill "$vite" 2>/dev/null || true
        docker compose down -v
    }
    trap teardown EXIT
    mkdir -p target/demo
    (cd web && exec pnpm exec vite dev --port 5173 --strictPort) \
        > target/demo/vite.log 2>&1 &
    vite=$!
    url="http://localhost:5173/demo/?xray&basemap=demo&layer=truecolor&center=-105.4475,39.2650&zoom=12"
    echo ""
    echo "  Building and starting the stack (the first run takes a while)."
    echo "  Open NOW and keep it visible:"
    echo ""
    echo "      $url"
    echo ""
    echo "  The x-ray overlay is already on: every tile is annotated with"
    echo "  its render decision, and the top-left readout shows ingest->pixel."
    echo "  The map is gray on purpose — the layer exists, its pixels don't"
    echo "  (an honest 404). When the countdown ends, the granule drops."
    echo ""
    SWATH_DROP_COUNTDOWN={{countdown}} tests/e2e/stack-up.sh
    i2p=$(sed -n 's/.*"ingest_to_pixel_ms":\([0-9][0-9]*\).*/\1/p' target/e2e/tile-headers.txt | head -1)
    echo ""
    echo "  =============================================="
    echo ""
    echo "     INGEST-TO-PIXEL: $i2p ms"
    echo ""
    echo "     (CI asserts this same path under a 10000 ms"
    echo "      budget on every commit — forever.)"
    echo ""
    echo "  =============================================="
    echo ""
    echo "  If the map is still gray, nudge it (drag or zoom) — MapLibre"
    echo "  won't refetch tiles it already saw 404. Switch the layer control"
    echo "  to 'HLS NDVI' to watch NDVI computed on the fly (decision: live"
    echo "  in the x-ray badges — nothing is pre-baked)."
    echo ""
    echo "  Ctrl-C to tear everything down."
    wait "$vite" || true
