# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# Fixtures, goldens and oracles: regenerate or verify the committed truth the tests compare against.
# Imported by the root justfile; recipe names are global (`just <recipe>`).

# Rebuild the committed Rust UDF fixture modules from examples/udf (their
# own wasm32-unknown-unknown workspace). Deterministic by construction —
# pinned toolchain (rust-toolchain.toml), symbols stripped, panic=abort,
# no host paths in the bytes — so a rebuild anywhere reproduces the
# committed fixtures exactly; udf-fixtures-verify is that claim, gated.
# (The AssemblyScript/TinyGo fixtures are deliberately prebuilt from the
# pinned toolchains in examples/udf/README.md, not rebuilt here; the
# swath-udf-wasmtime conformance suite keeps their committed bytes honest.)
udf-fixtures:
    #!/usr/bin/env bash
    set -euo pipefail
    rustup target add wasm32-unknown-unknown
    (cd examples/udf && cargo build --release --target wasm32-unknown-unknown)
    out=examples/udf/target/wasm32-unknown-unknown/release
    dst=crates/adapters/swath-udf-wasmtime/tests/fixtures
    cp "$out/swath_udf_example_ndvi.wasm"      "$dst/ndvi.wasm"
    cp "$out/swath_udf_example_hillshade.wasm" "$dst/hillshade.wasm"
    cp "$out/swath_udf_example_qamask.wasm"    "$dst/qamask.wasm"
    shasum -a 256 "$dst"/ndvi.wasm "$dst"/hillshade.wasm "$dst"/qamask.wasm

# Recapture the golden request/response buffers (the #209 verification-
# lattice set: pinned OUTPUT planes over committed input tiles) after a
# deliberate change to a reference UDF or its input tile. The ignored
# capture test writes tests/fixtures/golden/*.bin; the un-ignored tests
# in the same file are the gate that the committed goldens replay
# byte-identically (they run in `just test`).
udf-goldens: udf-fixtures
    cargo test -p swath-udf-wasmtime --test golden_outputs -- --ignored golden_capture

# The reproducibility gate (CI: the udf-guest job): rebuild the Rust
# examples and byte-compare against the committed fixtures — committed
# .wasm can never drift from its source.
udf-fixtures-verify:
    #!/usr/bin/env bash
    set -euo pipefail
    rustup target add wasm32-unknown-unknown
    (cd examples/udf && cargo build --release --target wasm32-unknown-unknown)
    out=examples/udf/target/wasm32-unknown-unknown/release
    dst=crates/adapters/swath-udf-wasmtime/tests/fixtures
    cmp "$out/swath_udf_example_ndvi.wasm"      "$dst/ndvi.wasm"
    cmp "$out/swath_udf_example_hillshade.wasm" "$dst/hillshade.wasm"
    cmp "$out/swath_udf_example_qamask.wasm"    "$dst/qamask.wasm"
    echo "udf-fixtures-verify PASS"

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
    # Time-dimension goldens (ADR 0015 / issue #180): the Park Fire series'
    # proven z13 tile (fully inside the t10tfk fixture window) at a pre-fire
    # and a fresh-burn-scar date — grayscale NDVI, matched by the swath-api
    # tiles_datetime suite and, as level 1 of the #94 scheme, by
    # swath-render's golden_ir fire test, which blesses the RdYlGn
    # self-goldens (fire-ndvi-rdylgn-*, SWATH_BLESS=1) the e2e datetime=
    # frame checks pin the compose stack's colormapped frames against.
    FIRE=tests/fixtures/hlss30-t10tfk
    for day in 2024204 2024229; do
        compose 13 1326 3100 "$D/fire-ndvi-13-1326-3100-$day.png" \
            --input "$FIRE-$day-b8a.tif" --input "$FIRE-$day-b04.tif" \
            --expression "(b1 - b2) / (b1 + b2)" --rescale=-1,1 --resampling bilinear
    done
    # Per-timestamp goldens (issue #181, ADR 0015 frame selection): NDVI over
    # the Park Fire series (tests/fixtures/hlss30-t10tfk-*, issue #179) at
    # the same z13 tile. One golden per acquisition pins that a graph's
    # temporal window serves the granule of *that* date: pre-fire green
    # (159), fresh burn scar (229), early post-fire (289).
    for day in 2024159 2024229 2024289; do
        compose 13 1326 3100 "$D/ndvi-$day-13-1326-3100.png" \
            --input "$FIRE-$day-b8a.tif" --input "$FIRE-$day-b04.tif" \
            --expression "(b1 - b2) / (b1 + b2)" --rescale=-1,1 --resampling bilinear
    done
    # The two-date change golden (ADR 0022, issue #296): NDVI(2024229) −
    # NDVI(2024204) over the same z13 tile — what a published merge_cubes
    # change layer must serve from its two resolved granules (matched by the
    # swath-api openeo_change_detection suite). Inputs in plan order: the
    # `after` branch's bands first.
    compose 13 1326 3100 "$D/fire-change-13-1326-3100-2024204-2024229.png" \
        --input "$FIRE-2024229-b8a.tif" --input "$FIRE-2024229-b04.tif" \
        --input "$FIRE-2024204-b8a.tif" --input "$FIRE-2024204-b04.tif" \
        --expression "(b1 - b2) / (b1 + b2) - (b3 - b4) / (b3 + b4)" \
        --rescale=-1,1 --resampling bilinear

# Regenerate the web TMS truth table (web/src/tms_truth.json) from the pinned
# morecantile oracle (issue #106; the TS twin of the swath-core table). The
# committed JSON is what CI asserts against (web/src/tms.test.ts) — review the
# diff before committing a regeneration.
tms-truth-web:
    uv run tests/oracle/tms_truth_web.py > web/src/tms_truth.json

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
    # Icechunk interop round trip (#191, ADR 0017): commit the refs to a
    # local Icechunk repo, then icechunk-python + xarray read every array
    # back byte-equal to the HDF5 source (no Swath Rust in the read loop).
    rm -rf "$dir/icechunk-repo"
    cargo run -q -p swath-cli -- ingest reference "$granule"         --output "$dir/rs2.vmanifest.json" --icechunk "$dir/icechunk-repo"
    uv run tests/referencer/icechunk_check.py "$dir/icechunk-repo" "$granule"
    echo "test-referencer PASS"

# The credential-free Icechunk round trip (#191, ADR 0017) over the tiny
# committed HDF-EOS fixture: the Rust committer writes a local Icechunk
# repo; icechunk-python/zarr/xarray read every array back byte-equal to
# the HDF5 source. The same check `just test-referencer` runs against a
# real VNP09GA granule; this one needs only uv and runs anywhere.
test-icechunk:
    #!/usr/bin/env bash
    set -euo pipefail
    dir=target/icechunk-tiny && rm -rf "$dir" && mkdir -p "$dir"
    cargo run -q -p swath-cli -- ingest reference         crates/swath-referencer/tests/data/tiny.h5         --output "$dir/tiny.vmanifest.json" --icechunk "$dir/repo"
    uv run tests/referencer/icechunk_check.py "$dir/repo"         crates/swath-referencer/tests/data/tiny.h5

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
    # Icechunk read-back demo (#193, ADR 0017): the SAME tile served from
    # an Icechunk commit, byte-identical to the manifest path, provenance
    # into the original .h5 asserted inside the test.
    SWATH_VNP09GA="$granule"     SWATH_VNP09GA_MANIFEST="$dir/real.vmanifest.json"         cargo test -q -p swath-icechunk --test vnp09ga_serve -- --ignored
    echo "test-virtual PASS"

# --- python/ (uv workspace; ingest sidecars only, ADR 0006) ---

# The pgstac catalog integration suite (issue #30): the adapter's live tests
# are #[ignore] by default (they need a real pgstac); this recipe brings up
# the compose pgstac service (reusing one that is already running), exports
# the gating SWATH_PGSTAC_URL (the tests skip cleanly without it — issue
# #105 unified all gated suites on swath_testsupport::gated_var), runs them
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
    SWATH_PGSTAC_URL="${SWATH_PGSTAC_URL:-postgres://swath:swath-local-dev@localhost:5432/swath}" \
        cargo nextest run -p swath-catalog-pgstac --run-ignored all
