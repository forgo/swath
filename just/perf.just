# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# Measurement: load and bench baselines, the perf artifacts, and the docs they fill.
# Imported by the root justfile; recipe names are global (`just <recipe>`).

# `just load`: the concurrency measurement `just e2e` never takes. Brings
# up the SAME compose stack (tests/e2e/stack-up.sh — the single owner of
# lifecycle; its full path also drops the fixture granule and polls the
# proven tile live), then tests/load/load.sh runs the pinned scenarios:
# (a) hot-cache tile storm, (b) cold live-render burst (unique tiles,
# each exactly once), (c) a mixed storm of the heaviest live warps WHILE
# a second oha measures /healthz and curl holds an SSE /traces
# subscription — the §16.7 (async-vs-blocking render boundary, #102)
# evidence. Distills the committed baseline docs/perf/load-baseline.
# {json,md} and prints the table. Under 5 minutes on a laptop.
load: (setup-ci "oha")
    #!/usr/bin/env bash
    set -euo pipefail
    started=$(date +%s)
    trap 'docker compose down -v' EXIT
    tests/e2e/stack-up.sh
    tests/load/load.sh "$started"

# `just load-h2h` (issue #121): Swath vs a digest-pinned TiTiler on the
# ONE overlapping capability — serving a static COG as tiles. Same
# machine, same committed fixture COGs, `just load`'s own scenario
# parameters, both containers pinned to the same CPU quota (H2H_CPUS,
# default 4), run one at a time. TiTiler is configured per its OWN
# documented production guidance (citations in tests/load/h2h.py — no
# strawman). Rewrites docs/perf/load-h2h-titiler.{json,md}: the committed,
# honestly-framed publication (maintainer pre-commitment: published
# regardless of which server wins). Laptop evidence, not capacity
# planning; internal baselines stay the regression reference.
load-h2h: (setup-ci "oha")
    #!/usr/bin/env bash
    set -euo pipefail
    started=$(date +%s)
    trap 'docker compose down -v; docker rm -f swath-h2h-titiler >/dev/null 2>&1 || true' EXIT
    tests/load/h2h.sh "$started"

# `just load-temporal` (issue #184): the M7 measurement — temporal
# frame-serving over the six-date Park Fire series (ADR 0015 / #223) and
# overview-backed tile serving through the materialized pyramid path
# (#183/#218). Brings up the SAME compose stack (tests/e2e/stack-up.sh —
# the single owner of lifecycle; its full path drops the single-date
# granule and polls the proven tile live) and builds the release binary
# (materialize runs host-side — the container's /data mount is read-only,
# writers live outside), then tests/load/temporal.sh runs the pinned
# scenarios: (d) the fire drop + the `datetime=` frame loop, cold (every
# dated frame a Live render) then hot (every frame a granule-scoped cache
# hit); (e) an overview zoom ladder around a timed `swath materialize`,
# every rung decision-probed (x-swath-trace header + SSE envelope levels).
# Distills the committed baseline docs/perf/temporal-baseline.{json,md}.
load-temporal:
    #!/usr/bin/env bash
    set -euo pipefail
    # Same port discipline as `just demo`: refuse to collide with a live
    # sibling stack (a running e2e/demo shares :8080 and the compose project).
    if curl -sf http://localhost:8080/healthz >/dev/null 2>&1; then
        echo "FAIL: a swath stack is already up on :8080 — 'docker compose down -v' first"; exit 1
    fi
    started=$(date +%s)
    cargo build --release -q -p swath-cli
    trap 'docker compose down -v' EXIT
    tests/e2e/stack-up.sh
    tests/load/temporal.sh "$started"

# `just load-udf` (issue #207): the `run_udf` live-latency evidence under
# the ADR 0012 reopen guard. Brings up the SAME compose stack — whose
# catalog now wires a `udf-store` (tests/e2e/swath-catalog.toml) so
# run_udf is offered — then tests/load/load_udf.sh publishes two services
# and runs the pinned scenarios: (u) the reference NDVI UDF stormed on its
# heaviest Live tiles WHILE /healthz + SSE /traces are probed (the ADR
# 0012 signals, recorded and verdicted against the 50 ms trigger); (f) a
# runaway-loop UDF refused on the tile path (500 RFC 7807 fuel) and the
# preview (400 ProcessGraphComplexity) with the same probes proving zero
# collateral. Distills docs/perf/load-udf-baseline.{json,md}. The report
# fails on scenario-integrity violations (storm not Live-through-UDF, or
# the bomb not refused) but never on the ADR 0012 signals themselves — a
# trip is a maintainer lane-decision on committed evidence, not a silent
# fix (ADR 0018 rollback / ADR 0012 reopen).
load-udf: (setup-ci "oha")
    #!/usr/bin/env bash
    set -euo pipefail
    # Port discipline (as `just load-temporal`/`just demo`): refuse to
    # collide with a live sibling stack sharing :8080.
    if curl -sf http://localhost:8080/healthz >/dev/null 2>&1; then
        echo "FAIL: a swath stack is already up on :8080 — 'docker compose down -v' first"; exit 1
    fi
    started=$(date +%s)
    trap 'docker compose down -v' EXIT
    tests/e2e/stack-up.sh
    tests/load/load_udf.sh "$started"

# All criterion benches across the workspace: the planner microbench
# (swath-planner) plus the render-stage suites (swath-render: warp, IR eval,
# PNG encode, source window, full-tile composite). Inputs are the committed
# HLS fixtures — no network, no downloads. Compare runs against the
# committed baseline in docs/perf/bench-baseline.json.
bench:
    cargo bench --workspace

# Re-capture the committed perf baseline: run every bench, then distill
# criterion's estimates (per-bench median + MAD, ns) plus machine/toolchain
# metadata into docs/perf/bench-baseline.json. Commit the result; the PR
# that changes it should say why the numbers moved.
bench-baseline: bench
    #!/usr/bin/env bash
    set -euo pipefail
    python3 tests/perf/bench_baseline.py

# --- docs/PERFORMANCE.md instruments (issue #115) ---

# Refresh the committed ingest-to-pixel baseline: the full `just e2e`
# measures ingest_to_pixel_ms (and enforces the band budget); its
# machine-readable artifact is then committed as the PERFORMANCE.md input.
perf-i2p: e2e
    cp target/e2e/metrics.json docs/perf/i2p-baseline.json
    cat docs/perf/i2p-baseline.json

# Re-measure the referencer bake-off claims (prototype 0001 §7) on the
# CURRENT tree: the production Rust generator (`swath ingest reference`,
# release profile) vs the VirtualiZarr sidecar (python/sidecars/referencer)
# on the pinned VNP09GA conformance granule (ADR 0008 — the exact granule
# the prototype ran on). Full-process wall clock, one cold run + median of
# the remaining warm runs, distilled to docs/perf/referencer-baseline.json
# (commit it; PERFORMANCE.md quotes it). Granule sourcing mirrors
# test-referencer, but no credentials is a FAIL here, not a skip — this
# recipe exists to produce numbers.
perf-referencer runs="10":
    #!/usr/bin/env bash
    set -euo pipefail
    dir=target/referencer && mkdir -p "$dir"
    granule="${SWATH_VNP09GA:-}"
    if [ -z "$granule" ]; then
        for c in "$dir"/VNP09GA*.h5 prototypes/0001-*/data/VNP09GA*.h5; do
            if [ -f "$c" ]; then granule="$c"; break; fi
        done
    fi
    if [ -z "$granule" ]; then
        if [ -f "$HOME/.netrc" ] && grep -q "urs.earthdata.nasa.gov" "$HOME/.netrc"; then
            granule=$(uv run tests/referencer/fetch_vnp09ga.py "$dir")
        else
            echo "FAIL perf-referencer: no VNP09GA granule and no Earthdata credentials." >&2
            echo "  Provide SWATH_VNP09GA=<path>, or add a ~/.netrc entry for" >&2
            echo "  urs.earthdata.nasa.gov to fetch the pinned granule (~8 MB)." >&2
            exit 1
        fi
    fi
    granule=$(cd "$(dirname "$granule")" && pwd)/$(basename "$granule")
    echo "perf granule: $granule"
    cargo build --release -q -p swath-cli
    (cd python && uv sync -q)
    SWATH_PERF_GRANULE="$granule" SWATH_PERF_RUNS={{runs}} python3 tests/perf/perf_referencer.py

# Regenerate the results tables and measurement stamp in docs/PERFORMANCE.md
# AND every inline headline figure across the docs from the committed
# artifacts (docs/perf/*.json + docs/perf/load-2cpu-16.7-evidence.md).
# Everything between the `table:*` HTML-comment markers is owned by this
# recipe, and so is the content of every inline
# `<!-- number:<key> -->…<!-- /number:<key> -->` pair (README.md, the
# swath-referencer crate README, DEMO.md, CHARTER.md, REQUIREMENTS.md,
# PERFORMANCE.md, ARCHITECTURE.md,
# COMPARISON.md, media/wedge.notes.md); the prose around the markers is
# hand-written. Idempotent — rerunning without artifact changes is a no-op.
# The docs-check gate (crates/swath-cli/src/docs_check/numbers.rs) verifies
# marker content against the same artifacts and rejects naked headline
# literals. Run after any baseline artifact changes.
perf-doc:
    #!/usr/bin/env bash
    set -euo pipefail
    python3 tests/perf/perf_doc.py
