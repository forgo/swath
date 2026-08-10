# Swath task contract (ENGINEERING.md §1, ADR 0007).
# CI runs exactly these recipes; anything CI checks, a developer runs identically
# with `just <recipe>`. One entrypoint, no drift.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Pinned dev-tool versions (Renovate bumps these alongside CI).
nextest_version := "0.9.143"
llvm_cov_version := "0.8.7"
deny_version := "0.20.2"
zizmor_version := "1.29.0"
prek_version := "0.4.12"

# List available recipes.
default:
    @just --list

# Install pinned CI tools (prebuilt via cargo-binstall; --locked so a source-
# build fallback can't be broken by upstream semver drift). Pass a subset to
# install only what a job runs ("none" for toolchain-only jobs); default: all.
setup-ci *tools="nextest llvm-cov deny":
    #!/usr/bin/env bash
    set -euo pipefail
    # --force: we only call install() when the binary is provably absent, but a
    # restored CI cache can contain binstall's .crates.toml (claiming "already
    # installed") without the binaries — binstall would otherwise skip.
    install() { # name version
        if command -v cargo-binstall >/dev/null; then
            cargo binstall --no-confirm --force --locked --version "$2" "$1"
        else
            cargo install --locked --force --version "$2" "$1"
        fi
    }
    for tool in {{tools}}; do
        case "$tool" in
            nextest)  command -v cargo-nextest  >/dev/null || install cargo-nextest  "{{nextest_version}}" ;;
            llvm-cov) cargo llvm-cov --version >/dev/null 2>&1 || install cargo-llvm-cov "{{llvm_cov_version}}" ;;
            deny)     command -v cargo-deny     >/dev/null || install cargo-deny     "{{deny_version}}" ;;
            none)     ;;
            *)        echo "unknown tool: $tool" >&2; exit 1 ;;
        esac
    done
    echo "setup-ci complete ({{tools}})"

# Full developer setup: CI tools + local-only tools (zizmor, prek) + git hook.
setup: setup-ci
    #!/usr/bin/env bash
    set -euo pipefail
    install() { # name version
        if command -v cargo-binstall >/dev/null; then
            cargo binstall --no-confirm --force --locked --version "$2" "$1"
        else
            cargo install --locked --force --version "$2" "$1"
        fi
    }
    command -v zizmor >/dev/null || install zizmor "{{zizmor_version}}"
    command -v prek   >/dev/null || install prek   "{{prek_version}}"
    # Optional, never mandatory: install the git pre-commit hook.
    prek install 2>/dev/null || true
    echo "setup complete"

# Format all Rust code.
fmt:
    cargo fmt --all

# Verify formatting without modifying (CI).
fmt-check:
    cargo fmt --all --check

# Lint: clippy over the whole workspace, warnings are errors.
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Run all tests: nextest (unit/integration) + doctests (nextest skips them).
test:
    cargo nextest run --workspace
    cargo test --workspace --doc

# Supply-chain gate: advisories, licenses, bans, sources (config: deny.toml).
deny:
    cargo deny check

# Coverage (region) over nextest; writes lcov.info for upload/inspection.
# (Doctest coverage needs nightly rustdoc; doctests still RUN in `just test`.)
cov:
    cargo llvm-cov --workspace --lcov --output-path lcov.info nextest
    cargo llvm-cov report

# Workflow static analysis (all findings are errors in CI).
zizmor:
    zizmor .github

# License/SPDX compliance (REUSE 3.3; needs uv). Lints exactly the files git
# tracks — reuse's own walker trips over pnpm's node_modules/.bin shims even
# though they're gitignored (CI's reuse job lints a clean checkout, same set).
reuse:
    git ls-files -z | xargs -0 uvx --from 'reuse[charset-normalizer]' reuse lint-file

# --- tests/oracle (GDAL/rio-tiler correctness oracle, ADR 0002 / issue #19) ---

# Passthrough to the reference renderer (PEP 723 script; uv resolves the pins).
oracle-render *ARGS:
    uv run tests/oracle/render_reference.py {{ARGS}}

# Issue #19 validation gate: reference renders are byte-stable and the
# perceptual diff catches a seeded single-pixel error at tolerance 0.
oracle-verify:
    #!/usr/bin/env bash
    set -euo pipefail
    dir=target/oracle
    rm -rf "$dir" && mkdir -p "$dir"
    uv run tests/oracle/render_reference.py synth-cog "$dir/synth.tif" --nodata-corner
    uv run tests/oracle/render_reference.py render "$dir/synth.tif" 6 10 24 "$dir/a.png"
    uv run tests/oracle/render_reference.py render "$dir/synth.tif" 6 10 24 "$dir/b.png"
    sha_a=$(openssl dgst -sha256 -r "$dir/a.png" | cut -d' ' -f1)
    sha_b=$(openssl dgst -sha256 -r "$dir/b.png" | cut -d' ' -f1)
    echo "render 1 sha256: $sha_a"
    echo "render 2 sha256: $sha_b"
    [ "$sha_a" = "$sha_b" ] || { echo "FAIL: renders are not byte-stable"; exit 1; }
    cargo run --quiet -p swath-testkit --bin pdiff -- "$dir/a.png" "$dir/b.png"
    cargo run --quiet -p swath-testkit --bin pdiff -- --corrupt "$dir/a.png" "$dir/corrupt.png"
    if cargo run --quiet -p swath-testkit --bin pdiff -- --tolerance 0 --max-bad-frac 0 "$dir/a.png" "$dir/corrupt.png"; then
        echo "FAIL: pdiff missed the seeded single-pixel error"; exit 1
    fi
    echo "seeded error caught at tolerance 0"
    echo "oracle-verify PASS"

# Regenerate swath-render's committed golden tiles (crates/swath-render/tests/data)
# from the HLS fixtures via the pinned oracle. z12/z13 goldens use rio-tiler's
# serving path; the decimating z11 goldens use --exact-grid (single-stage warp on
# the tile grid, exact transformer — see render_reference.py for why). The oracle
# is deterministic, so a rerun must reproduce the committed bytes exactly.
render-goldens:
    #!/usr/bin/env bash
    set -euo pipefail
    F=tests/fixtures/hlss30-t13sdd-2024158
    D=crates/swath-render/tests/data
    render() { uv run tests/oracle/render_reference.py render "$@"; }
    while read -r z x y; do
        render "$F-b04.tif"   "$z" "$x" "$y" "$D/b04-$z-$x-$y.png"   --bands 1 --rescale 0,3000 --resampling bilinear
        render "$F-fmask.tif" "$z" "$x" "$y" "$D/fmask-$z-$x-$y.png" --bands 1 --resampling nearest
    done <<< $'12 848 1561\n12 848 1562\n13 1697 3122'
    render "$F-b04.tif"   11 424 780 "$D/b04-11-424-780.png"   --bands 1 --rescale 0,3000 --resampling bilinear --no-overviews --exact-grid
    render "$F-fmask.tif" 11 424 780 "$D/fmask-11-424-780.png" --bands 1 --resampling nearest --no-overviews --exact-grid
    # Render-IR goldens (issue #25): multi-file composites via the compose
    # subcommand — true-color BGR->RGB and NDVI band math, both bilinear.
    compose() { uv run tests/oracle/render_reference.py compose "$@"; }
    # Overview-path goldens (issue #38): the same z11 tile rendered from the
    # fixtures' x2 overview IFD (--overview-level 0 — GDAL's own selection
    # serves exactly this overview on rio-tiler's default path at z11,
    # verified byte-identical; the explicit open pins WHICH bytes while
    # --exact-grid removes the two-stage read pipeline's point-sampling
    # artifact, as for the kernel z11 goldens above, which stay untouched:
    # they pin the decimating warp KERNEL against full-res pixels, a
    # different question). Rendered via `compose` because its masked-pixel
    # encoding (transparent black) is the Render IR's documented output —
    # these goldens are matched by the tiler's Overview strategy end to end.
    compose 11 424 780 "$D/b04-ov-11-424-780.png" \
        --input "$F-b04.tif" --expression "b1" --rescale 0,3000 \
        --resampling bilinear --overview-level 0 --exact-grid
    compose 11 424 780 "$D/fmask-ov-11-424-780.png" \
        --input "$F-fmask.tif" --expression "b1" \
        --resampling nearest --overview-level 0 --exact-grid
    for yy in 1561 1562; do
        compose 12 848 "$yy" "$D/truecolor-12-848-$yy.png" \
            --input "$F-b04.tif" --input "$F-b03.tif" --input "$F-b02.tif" \
            --rescale 0,3000 --resampling bilinear
        compose 12 848 "$yy" "$D/ndvi-12-848-$yy.png" \
            --input "$F-b8a.tif" --input "$F-b04.tif" \
            --expression "(b1 - b2) / (b1 + b2)" --rescale=-1,1 --resampling bilinear
    done

# --- tests/fixtures (committed HLS COG subsets, issue #20 / ADR 0004) ---

# Fixture integrity gate: checksums + offline rasterio sanity load against
# manifest.json. Fixtures are immutable once committed (tests/fixtures/README.md).
fixtures-verify:
    cd tests/fixtures && shasum -a 256 -c SHA256SUMS
    uv run tests/fixtures/verify_fixtures.py

# --- tests/referencer (conformance harness, ADR 0006 / issue #40) ---

# The gated generator-equivalence run: production Rust referencer vs the
# VirtualiZarr sidecar on a REAL VNP09GA granule, byte-range equivalence
# asserted (promoted from prototype 0001). Needs a granule: $SWATH_VNP09GA,
# a cached copy under target/referencer or the prototype's data dir, or a
# NASA Earthdata netrc entry to fetch one (~8 MB) — otherwise it skips with
# a message. PR CI covers the same code paths credential-free via the tiny
# committed fixture (Rust known-answer + sidecar test_cli.py against the
# same h5py-derived truth); this recipe is the real-data gate, run locally
# or via the manual referencer-conformance workflow.
test-referencer:
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
            echo "SKIP test-referencer: no VNP09GA granule and no Earthdata credentials."
            echo "  Provide SWATH_VNP09GA=<path>, or add a ~/.netrc entry for"
            echo "  urs.earthdata.nasa.gov to fetch the pinned granule (~8 MB)."
            exit 0
        fi
    fi
    granule=$(cd "$(dirname "$granule")" && pwd)/$(basename "$granule")
    echo "conformance granule: $granule"
    # Production generator (the exact binary path operators use)...
    cargo run -q -p swath-cli -- ingest reference "$granule" --output "$dir/rs.vmanifest.json"
    # ...vs the VirtualiZarr sidecar (the independent reference)...
    (cd python && uv run swath-referencer "$granule") > "$dir/vz.vmanifest.json"
    # ...byte-range equivalence, or die.
    cargo run -q -p swath-referencer --bin vmanifest-compare --         "$dir/rs.vmanifest.json" "$dir/vz.vmanifest.json"
    # And the gated structural/georef assertions on the real granule.
    SWATH_VNP09GA="$granule" cargo test -q -p swath-referencer --test vnp09ga_real -- --ignored
    echo "test-referencer PASS"

# The gated virtual-serving run (issue #39, joins the test-referencer
# pattern): reference a REAL VNP09GA granule, render a VIIRS NDVI Web
# Mercator tile through the virtual-reference RasterSource (chunk-range
# reads into the original .h5, sinusoidal warp), and perceptually diff it
# against a GDAL oracle render of the SAME tile from the SAME original
# file (rasterio opens the HDF5 subdatasets with their sinusoidal SRS —
# verified). Granule sourcing and skip behavior are identical to
# test-referencer. PR CI covers the same code paths credential-free via
# the tiny HDF-EOS fixture (swath-source-virtual's window/describe/render
# truth tests); this recipe is the real-data gate.
test-virtual:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="$PWD/target/virtual" && mkdir -p "$dir"
    granule="${SWATH_VNP09GA:-}"
    if [ -z "$granule" ]; then
        for c in target/referencer/VNP09GA*.h5 prototypes/0001-*/data/VNP09GA*.h5; do
            if [ -f "$c" ]; then granule="$c"; break; fi
        done
    fi
    if [ -z "$granule" ]; then
        if [ -f "$HOME/.netrc" ] && grep -q "urs.earthdata.nasa.gov" "$HOME/.netrc"; then
            granule=$(uv run tests/referencer/fetch_vnp09ga.py target/referencer)
        else
            echo "SKIP test-virtual: no VNP09GA granule and no Earthdata credentials."
            echo "  Provide SWATH_VNP09GA=<path>, or add a ~/.netrc entry for"
            echo "  urs.earthdata.nasa.gov to fetch the pinned granule (~8 MB)."
            exit 0
        fi
    fi
    granule=$(cd "$(dirname "$granule")" && pwd)/$(basename "$granule")
    echo "virtual-serving granule: $granule"
    # Reference it with the production generator (the operator path)...
    cargo run -q -p swath-cli -- ingest reference "$granule" --output "$dir/real.vmanifest.json"
    # ...render the NDVI tile through the virtual source (provenance into
    # the original file asserted inside the test)...
    SWATH_VNP09GA="$granule" \
    SWATH_VNP09GA_MANIFEST="$dir/real.vmanifest.json" \
    SWATH_VIRTUAL_OUT="$dir/swath-ndvi.png" \
        cargo test -q -p swath-source-virtual --test vnp09ga_real -- --ignored
    # ...and render the SAME tile from the ORIGINAL file with the GDAL
    # oracle (z9 x=509 y=302 — the granule's valid-data region; the tile
    # constants live in tests/vnp09ga_real.rs).
    m7='HDF5:"'"$granule"'"://HDFEOS/GRIDS/VIIRS_Grid_1km_2D/Data_Fields/SurfReflect_M7_1'
    m5='HDF5:"'"$granule"'"://HDFEOS/GRIDS/VIIRS_Grid_1km_2D/Data_Fields/SurfReflect_M5_1'
    uv run tests/oracle/render_reference.py compose 9 509 302 "$dir/oracle-ndvi.png" \
        --input "$m7" --input "$m5" \
        --expression "(b1 - b2) / (b1 + b2)" --rescale=-1,1 \
        --resampling bilinear --no-overviews --exact-grid
    # Perceptual diff, default policy — the same bar the render goldens meet.
    cargo run -q -p swath-testkit --bin pdiff -- "$dir/swath-ndvi.png" "$dir/oracle-ndvi.png"
    echo "test-virtual PASS"

# --- python/ (uv workspace; ingest sidecars only, ADR 0006) ---

# Sync the python workspace (all packages + dev groups).
setup-py:
    cd python && uv sync --all-packages

# ruff lint + format check + pyright (strict).
lint-py:
    cd python && uv run ruff check . && uv run ruff format --check . && uv run pyright

# pytest (hypothesis property tests included).
test-py:
    cd python && uv run pytest -q

# Dependency vulnerability scan of the locked python graph.
audit-py:
    cd python && uv export --frozen --no-emit-workspace --format requirements-txt \
        | uvx pip-audit -r /dev/stdin --disable-pip

# --- web/ (pnpm; see web/package.json scripts) ---

# Install web deps + the Playwright chromium the browser tests run in.
setup-web:
    cd web && pnpm install --frozen-lockfile && pnpm exec playwright install chromium

# Biome lint/format check + TypeScript typecheck.
lint-web:
    cd web && pnpm run lint && pnpm run typecheck

# Vitest Browser Mode (real chromium — Custom Elements + WebGL are untestable in jsdom).
test-web:
    cd web && pnpm run test

# The pgstac catalog integration suite (issue #30): the adapter's live tests
# are #[ignore] by default (they need a real pgstac); this recipe brings up
# the compose pgstac service (reusing one that is already running), runs them
# (serially — see .config/nextest.toml), and tears down only what it started.
test-catalog:
    #!/usr/bin/env bash
    set -euo pipefail
    started=0
    if [ -z "$(docker compose ps -q --status running pgstac)" ]; then
        docker compose up -d --wait pgstac
        started=1
    fi
    teardown() { if [ "$started" = 1 ]; then docker compose rm -sfv pgstac; fi; }
    trap teardown EXIT
    cargo nextest run -p swath-catalog-pgstac --run-ignored all

# The compose-stack e2e — now THE north-star demo path (issues #15/#29/#31,
# REQUIREMENTS.md R1/R8 + §3): build the swath image, bring up the full local
# stack (swath in catalog mode + pgstac + MinIO), verify infra health, then
# drive granule-to-live-tile end to end with zero manual steps: the layer 404s
# while the catalog is empty; dropping the fixture granule (5 HLS band COGs +
# a manifest, bands first, manifest renamed into place last) makes the tile
# servable automatically; the Trace carries ingest_to_pixel_ms, printed
# prominently and asserted under the north-star budget (issue #35). The
# served tile is perceptually matched against the committed rio-tiler/GDAL
# golden (cross-encoder comparison is pdiff at the default policy, exactly
# like the golden suites; byte-stability is asserted by fetching twice).
# The NDVI layer — CHARTER.md §10 Phase 1's "computed on the fly, not
# pre-baked" — is exercised on the same path: tile fetched, provenance
# asserted, matched against ITS committed golden, and the SSE trace's
# decision:"live" captured as the on-the-fly proof. Teardown is trap-based:
# the stack never outlives the recipe.
e2e:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'docker compose down -v' EXIT
    # Stack bring-up + granule drop + poll-to-live: shared with `just
    # e2e-web` (issue #33) via tests/e2e/stack-up.sh, which leaves
    # $dir/tile.png and $dir/tile-headers.txt for the assertions below.
    tests/e2e/stack-up.sh
    dir=target/e2e
    base=http://localhost:8080
    tile="$base/tilesets/truecolor/tiles/12/1561/848"
    # The Trace explains the served tile: header present, provenance
    # non-empty (real bytes were read for this granule's pixels).
    grep -qi '^x-swath-trace:' "$dir/tile-headers.txt" && echo "swath: X-Swath-Trace header present"
    bytes=$(sed -n 's/.*"bytes_read":\([0-9][0-9]*\).*/\1/p' "$dir/tile-headers.txt" | head -1)
    [ -n "$bytes" ] && [ "$bytes" -gt 0 ] || { echo "FAIL: trace reports no bytes read"; exit 1; }
    echo "swath: trace provenance is non-empty (bytes_read=$bytes)"
    # THE NORTH-STAR NUMBER (REQUIREMENTS.md §3): the first tile reflecting
    # the just-ingested granule carries ingest-to-pixel latency.
    i2p=$(sed -n 's/.*"ingest_to_pixel_ms":\([0-9][0-9]*\).*/\1/p' "$dir/tile-headers.txt" | head -1)
    [ -n "$i2p" ] || { echo "FAIL: trace carries no numeric ingest_to_pixel_ms"; exit 1; }
    # The budget (issue #35): measured 297 ms and 801 ms locally, 535 ms in
    # CI, so 10000 ms is ~20x headroom over the CI number — tight enough to
    # catch a real regression (a sleep, a poll interval, an accidental batch
    # step), loose enough to shrug off runner noise. Tightening it further
    # is a deliberate, visible act: record new measurements here when you do.
    budget=10000
    echo ""
    echo "=========================================="
    echo "   INGEST-TO-PIXEL: $i2p ms (budget $budget ms)"
    echo "=========================================="
    echo ""
    [ "$i2p" -lt "$budget" ] || { echo "FAIL: ingest-to-pixel $i2p ms exceeds the $budget ms budget"; exit 1; }
    echo "swath: ingest-to-pixel is under the $budget ms north-star budget"
    # The north-star assertion above is honest by construction: the first
    # 200 (tile-headers.txt) is a fresh, uncached render — the cache is
    # empty until that render writes through — so ingest-to-pixel is never
    # measured on a hit. Its decision must say so.
    grep -q '"decision":"live"' "$dir/tile-headers.txt" \
        || { echo "FAIL: first tile fetch was not a live render"; exit 1; }
    echo "swath: north-star tile was a fresh live render (decision: live)"
    # Same request, same bytes — and now served from the write-through
    # cache (#36): the repeat's trace must say cache_hit, byte-identical.
    curl -sf -D "$dir/tile-again-headers.txt" -o "$dir/tile-again.png" "$tile"
    cmp "$dir/tile.png" "$dir/tile-again.png" && echo "swath: tile bytes are stable across requests"
    grep -q '"decision":"cache_hit"' "$dir/tile-again-headers.txt" \
        || { echo "FAIL: second fetch of the same tile was not a cache hit"; exit 1; }
    echo "swath: second fetch served from the tile cache (decision: cache_hit, identical bytes)"
    # Correctness oracle: the catalog-served tile perceptually matches the
    # committed golden — "a CORRECT tile is visible" (§3), not just any tile.
    cargo run --quiet -p swath-testkit --bin pdiff -- \
        "$dir/tile.png" crates/swath-render/tests/data/truecolor-12-848-1561.png
    echo "swath: tile matches the rio-tiler/GDAL golden (default pdiff policy)"
    # NDVI on the fly (CHARTER.md §10 Phase 1: "NDVI computed on the fly,
    # not pre-baked"): subscribe to the x-ray stream first, then fetch the
    # never-yet-rendered NDVI tile — one request proves the whole claim.
    # HTTP side: 200, trace header, non-empty provenance, and a perceptual
    # match against the committed NDVI oracle golden (the "correct tile"
    # half of §3). SSE side: a `trace` event for the ndvi layer whose
    # decision is "live" — the on-the-fly proof — carrying the same
    # ingest-to-pixel number.
    curl -sN --max-time 15 "$base/traces" > "$dir/traces.txt" &
    sse=$!
    sleep 1
    ndvi="$base/tilesets/ndvi/tiles/12/1561/848"
    code=$(curl -s -D "$dir/ndvi-headers.txt" -o "$dir/ndvi.png" -w '%{http_code}' "$ndvi")
    [ "$code" = "200" ] || { echo "FAIL: ndvi tile answered $code"; exit 1; }
    grep -qi '^x-swath-trace:' "$dir/ndvi-headers.txt" && echo "swath: ndvi X-Swath-Trace header present"
    nbytes=$(sed -n 's/.*"bytes_read":\([0-9][0-9]*\).*/\1/p' "$dir/ndvi-headers.txt" | head -1)
    [ -n "$nbytes" ] && [ "$nbytes" -gt 0 ] || { echo "FAIL: ndvi trace reports no bytes read"; exit 1; }
    echo "swath: ndvi trace provenance is non-empty (bytes_read=$nbytes)"
    # Two-level NDVI golden scheme (issue #94): the GDAL/rio-tiler oracle
    # renders grayscale, and the RdYlGn colormap is Swath's own
    # post-processing, so the colormapped bytes have no external oracle.
    # Level 2 here: the served (colormapped) tile is byte-identical to the
    # committed self-golden produced by our own pipeline (double-render
    # byte-stable + lut[q(gray)] relation proven in golden_ir.rs; bless
    # with SWATH_BLESS=1). Level 1 — the VALUES against the grayscale GDAL
    # oracle golden — is asserted below via an authored grayscale service
    # over the exact same serve path.
    cmp "$dir/ndvi.png" crates/swath-render/tests/data/ndvi-rdylgn-12-848-1561.png \
        || { echo "FAIL: colormapped ndvi tile differs from the committed golden"; exit 1; }
    echo "swath: ndvi tile is byte-identical to the committed colormapped golden (#94, level 2)"
    # A cached repeat inside the SSE window: the truecolor tile is cached
    # by now, so this fetch must surface a keyed cache_hit trace (#36).
    curl -sf -o /dev/null "$tile"
    for _ in $(seq 1 20); do
        grep -q '^event: trace' "$dir/traces.txt" && break
        sleep 0.5
    done
    kill "$sse" 2>/dev/null || true
    wait "$sse" 2>/dev/null || true
    grep -q '^event: trace' "$dir/traces.txt" && echo "swath: trace SSE event captured"
    grep '"layer":"ndvi"' "$dir/traces.txt" | grep -q '"decision":"live"' \
        || { echo "FAIL: no live-decision ndvi trace on the SSE stream"; exit 1; }
    echo "swath: SSE trace proves ndvi was computed on the fly (decision: live)"
    grep '"layer":"truecolor"' "$dir/traces.txt" | grep -q '"decision":{"cache_hit":{"key":' \
        || { echo "FAIL: no cache_hit trace for the repeated truecolor tile on the SSE stream"; exit 1; }
    echo "swath: SSE trace reports the repeated tile as a keyed cache_hit (#36)"
    grep -q '"ingest_to_pixel_ms":[0-9]' "$dir/traces.txt" \
        && echo "swath: SSE trace carries ingest_to_pixel_ms"
    # The openEO authoring loop (issue #41, ADR 0010, R3): publish an NDVI
    # process graph as an XYZ secondary service against the live stack,
    # fetch a tile from the returned service URL, and require it
    # byte-identical to the built-in NDVI tile just pinned against the
    # colormapped golden — same compiler, same serve path, zero manual
    # steps. The graph names the colormap through the openEO
    # representation (save_result options, issue #94), so this also proves
    # colormap selection survives the graph round trip end to end. (The
    # north-star assertion above is untouched; this step runs after it.)
    svc=$(curl -sf -X POST "$base/services" -H 'content-type: application/json' \
        -d '{"type":"xyz","title":"NDVI (authored)","process":{"process_graph":{
              "load":{"process_id":"load_collection","arguments":{"id":"hls-s30","spatial_extent":null,"temporal_extent":null,"bands":["b8a","b04"]}},
              "ndvi":{"process_id":"ndvi","arguments":{"data":{"from_node":"load"},"nir":"b8a","red":"b04"}},
              "scale":{"process_id":"linear_scale_range","arguments":{"x":{"from_node":"ndvi"},"inputMin":-1,"inputMax":1,"outputMin":0,"outputMax":255}},
              "save":{"process_id":"save_result","arguments":{"data":{"from_node":"scale"},"format":"png","options":{"colormap":"rdylgn"}},"result":true}}}}' \
        -D "$dir/service-headers.txt" -o /dev/null -w '%{http_code}')
    [ "$svc" = "201" ] || { echo "FAIL: POST /services answered $svc"; exit 1; }
    sid=$(tr -d '\r' < "$dir/service-headers.txt" | sed -n 's/^[Oo]pen[Ee][Oo]-[Ii]dentifier: //p' | head -1)
    [ -n "$sid" ] || { echo "FAIL: 201 carried no OpenEO-Identifier header"; exit 1; }
    echo "swath: openEO service published ($sid)"
    code=$(curl -s -o "$dir/service.png" -w '%{http_code}' "$base/tilesets/$sid/tiles/12/1561/848")
    [ "$code" = "200" ] || { echo "FAIL: authored service tile answered $code"; exit 1; }
    cmp "$dir/service.png" "$dir/ndvi.png" \
        || { echo "FAIL: authored NDVI tile differs from the built-in NDVI tile"; exit 1; }
    echo "swath: authored service tile is byte-identical to the built-in NDVI (graph in, live XYZ out)"
    # Level 1 of the #94 two-level scheme: the same graph with a grayscale
    # colormap, published over the same serve path, must still match the
    # grayscale GDAL/rio-tiler oracle golden — the colormapped tile above
    # is therefore a palette over oracle-validated VALUES, not unchecked
    # pixels. The GDAL comparison is weakened nowhere: it moved one layer
    # down, to the values the colormap indexes by.
    gsvc=$(curl -sf -X POST "$base/services" -H 'content-type: application/json' \
        -d '{"type":"xyz","title":"NDVI (authored, grayscale)","process":{"process_graph":{
              "load":{"process_id":"load_collection","arguments":{"id":"hls-s30","spatial_extent":null,"temporal_extent":null,"bands":["b8a","b04"]}},
              "ndvi":{"process_id":"ndvi","arguments":{"data":{"from_node":"load"},"nir":"b8a","red":"b04"}},
              "scale":{"process_id":"linear_scale_range","arguments":{"x":{"from_node":"ndvi"},"inputMin":-1,"inputMax":1,"outputMin":0,"outputMax":255}},
              "save":{"process_id":"save_result","arguments":{"data":{"from_node":"scale"},"format":"png","options":{"colormap":"grayscale"}},"result":true}}}}' \
        -D "$dir/gray-service-headers.txt" -o /dev/null -w '%{http_code}')
    [ "$gsvc" = "201" ] || { echo "FAIL: POST /services (grayscale) answered $gsvc"; exit 1; }
    gsid=$(tr -d '\r' < "$dir/gray-service-headers.txt" | sed -n 's/^[Oo]pen[Ee][Oo]-[Ii]dentifier: //p' | head -1)
    [ -n "$gsid" ] || { echo "FAIL: grayscale 201 carried no OpenEO-Identifier header"; exit 1; }
    code=$(curl -s -o "$dir/gray-service.png" -w '%{http_code}' "$base/tilesets/$gsid/tiles/12/1561/848")
    [ "$code" = "200" ] || { echo "FAIL: grayscale service tile answered $code"; exit 1; }
    cargo run --quiet -p swath-testkit --bin pdiff -- \
        "$dir/gray-service.png" crates/swath-render/tests/data/ndvi-12-848-1561.png
    echo "swath: ndvi values match the rio-tiler/GDAL golden (#94, level 1: grayscale service, default pdiff policy)"
    # Metadata-vs-pixels: the tileset's DECLARED bounds must contain the tile
    # we just proved correct (12/848/1561, center -105.4248, 39.27). A wrong
    # granule bbox once put the demo viewport 48 km from the imagery while
    # every pixel test stayed green — declared geography is now asserted too.
    curl -sf "$base/tilesets/ndvi" \
        | python3 -c 'import json,sys; bb=json.load(sys.stdin)["boundingBox"]; (w,s),(e,n)=bb["lowerLeft"],bb["upperRight"]; ok=w<=-105.4248<=e and s<=39.27<=n; print(f"swath: declared tileset bounds contain the proven tile ({[w,s,e,n]})" if ok else f"FAIL: tileset bbox {[w,s,e,n]} does not contain the proven tile"); sys.exit(0 if ok else 1)'
    echo "e2e OK"

# The viewer e2e (issue #33): the same stack bring-up + granule drop as
# `just e2e` (shared, tests/e2e/stack-up.sh — no duplicated drop logic),
# then the Playwright suite drives the <swath-map> demo page against it:
# map renders, real tile requests answer 200 image/png, the canvas shows
# actual pixels, and the layer switcher re-points requests at the ndvi
# tileset. Playwright itself manages the vite dev server (which proxies
# the OGC routes to :8080 — the API serves no CORS headers yet). Needs
# `just setup-web` first (deps + chromium).
e2e-web:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'docker compose down -v' EXIT
    tests/e2e/stack-up.sh
    cd web && pnpm exec playwright test

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

# The one-command gate: everything CI enforces.
check: fmt-check lint test deny zizmor reuse
