# Swath task contract (ENGINEERING.md §1, ADR 0007).
# CI runs exactly these recipes; anything CI checks, a developer runs identically
# with `just <recipe>`. One entrypoint, no drift.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Pinned dev-tool versions (Renovate bumps these alongside CI).
nextest_version := "0.9.143"
llvm_cov_version := "0.8.7"
deny_version := "0.20.2"
machete_version := "0.9.2"
zizmor_version := "1.29.0"
prek_version := "0.4.12"
oha_version := "1.15.0"
# Release-pipeline tools (docs/RELEASING.md, issue #116). cargo-dist is
# pinned separately in dist-workspace.toml (github-releases datasource).
release_plz_version := "0.3.160"
git_cliff_version := "2.13.1"
cargo_edit_version := "0.13.13"

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
            machete)  command -v cargo-machete  >/dev/null || install cargo-machete  "{{machete_version}}" ;;
            oha)      command -v oha            >/dev/null || install oha            "{{oha_version}}" ;;
            release-plz) command -v release-plz >/dev/null || install release-plz "{{release_plz_version}}" ;;
            git-cliff)   command -v git-cliff   >/dev/null || install git-cliff   "{{git_cliff_version}}" ;;
            cargo-edit)  command -v cargo-set-version >/dev/null || install cargo-edit "{{cargo_edit_version}}" ;;
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

# Unused-dependency gate (issue #105): cargo-machete (pinned; install via
# `just setup-ci machete`) over the workspace manifests. Zero findings is
# the bar; false positives get a documented `[package.metadata.cargo-machete]
# ignored` entry, never a shrug.
machete:
    cargo machete

# Run all tests: nextest (unit/integration) + doctests (nextest skips them).
test:
    cargo nextest run --workspace
    cargo test --workspace --doc

# --- fast dev-loop profile (issue #99: `legacy-hdf5` OFF) ---
#
# The bundled-libhdf5 C build (hdf5-metno `static`, behind the default-ON
# `legacy-hdf5` feature) costs minutes of wall clock and gigabytes of
# target/ that loops never touching HDF5 referencing don't need. These
# recipes are the documented opt-out: no C toolchain, no cmake, GRIB2 and
# everything else exactly as default. `just test`/`just check` remain the
# full gate — nothing below replaces them before a push.

# Type-check the whole workspace without the libhdf5 build (embedded-ui
# stays on — the profile differs from default in `legacy-hdf5` only).
check-fast:
    cargo check --workspace --no-default-features --features swath-cli/embedded-ui

# Fast tests: the same feature-off profile. HDF5-dependent tests are
# feature-gated (compile out; `just test` always runs them), and the two
# crates whose test suites drive real HDF5 generation through a
# dev-dependency on swath-referencer (default features — they'd rebuild
# libhdf5) are excluded. No doctests (nextest); the full gate runs them.
test-fast:
    cargo nextest run --workspace --no-default-features --features swath-cli/embedded-ui \
        --exclude swath-events-filedrop --exclude swath-source-virtual

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

# The full production build (issue #103): the web bundle (vite, into
# web/dist), then the release binary embedding it (swath-cli's build
# script stages web/dist; feature `embedded-ui` is default-on). The result
# is one self-contained `swath` — `swath serve --fixtures` serves the UI
# at / and the demo layers beside it, zero external files.
build-full:
    cd web && pnpm run build
    cargo build --release --locked -p swath-cli

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

# The viewer e2e (issue #33): the same stack bring-up + granule drop as
# `just e2e` (shared, tests/e2e/stack-up.sh — no duplicated drop logic),
# then the Playwright suites drive the <swath-map> demo page against it
# TWICE (issue #103) — first through the vite dev server (which proxies
# the OGC routes to :8080; CORS stays opt-in and off, ADR 0011), then in
# against-binary mode: the same specs on the production bundle the swath
# container embeds and serves itself at :8080/ (feature `embedded-ui`).
# Asserted: map renders, real tile requests answer 200 image/png, the
# canvas shows actual pixels, the layer switcher re-points requests, and
# the x-ray overlay agrees with the SSE trace stream — in both modes.
# Needs `just setup-web` first (deps + chromium).
e2e-web:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'docker compose down -v' EXIT
    tests/e2e/stack-up.sh
    cd web
    pnpm exec playwright test
    SWATH_E2E_MODE=binary pnpm exec playwright test

# --- tests/load (HTTP load harness, issue #101 / ARCHITECTURE §16.7) ---

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

# `just screenshots` (issue #112): reproducible docs screenshots, fixture
# data only. Brings up the SAME compose stack (tests/e2e/stack-up.sh — the
# single owner of lifecycle; full path: granule dropped, tile polled live),
# then the capture suite (web/screenshots/capture.ts, its own Playwright
# config) drives the entry page through the vite dev server at a pinned
# viewport/DPR into docs/media/screenshots/: deterministic filenames, an
# index.md with one-line captions, and shots.json (per-shot sha256 + pdiff
# policy), both stamped with the capture git sha. The tile cache is cleared
# before EACH capture run so every run replays the identical request history
# from the same cold state; a SECOND capture into target/screenshots is then
# perceptually diffed shot-by-shot against the first (swath-testkit pdiff,
# per-shot policy) — a shot that cannot be reproduced fails the recipe
# instead of going stale. Needs `just setup-web`.
screenshots:
    #!/usr/bin/env bash
    set -euo pipefail
    # Same port discipline as `just demo`: refuse to collide with a live
    # sibling (a waiting demo holds 5173 and the compose project).
    if lsof -ti :5173 >/dev/null 2>&1; then
        echo "FAIL: port 5173 is busy — a previous demo/vite is still running (ctrl-c it first)"; exit 1
    fi
    if curl -sf http://localhost:8080/healthz >/dev/null 2>&1; then
        echo "FAIL: a swath stack is already up on :8080 — 'docker compose down -v' first"; exit 1
    fi
    trap 'docker compose down -v' EXIT
    tests/e2e/stack-up.sh
    out="$PWD/docs/media/screenshots"
    second="$PWD/target/screenshots/second"
    sha=$(git rev-parse HEAD)
    capture() { # dest-dir
        # Cold cache per run: decisions/plan mix reproduce exactly.
        rm -rf target/e2e/cache/* 2>/dev/null || true
        (cd web && SWATH_SHOTS_DIR="$1" SWATH_CAPTURE_SHA="$sha" \
            pnpm exec playwright test --config playwright.screenshots.config.ts)
    }
    mkdir -p "$out" && rm -f "$out"/*.png "$out"/index.md "$out"/shots.json
    rm -rf "$second" && mkdir -p "$second"
    capture "$out"
    capture "$second"
    python3 tests/screenshots/verify_stable.py "$out" "$second"
    echo "screenshots: $(ls "$out"/*.png | wc -l | tr -d ' ') shots in docs/media/screenshots (capture sha $sha)"

# --- benchmarks (issue #100; ENGINEERING.md §2 criterion mandate) ---

# All criterion benches across the workspace: the planner microbench
# (swath-core) plus the render-stage suites (swath-render: warp, IR eval,
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
    python3 - <<'EOF'
    import datetime
    import json
    import pathlib
    import platform
    import subprocess

    def sh(*args: str) -> str:
        return subprocess.run(args, capture_output=True, text=True, check=True).stdout.strip()

    system = platform.system()
    if system == "Darwin":
        model = sh("sysctl", "-n", "machdep.cpu.brand_string")
    else:
        model = next(
            (
                line.split(":", 1)[1].strip()
                for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines()
                if line.startswith("model name")
            ),
            platform.machine(),
        )

    benches = []
    for est_path in sorted(pathlib.Path("target/criterion").glob("**/new/estimates.json")):
        meta = json.loads((est_path.parent / "benchmark.json").read_text())
        est = json.loads(est_path.read_text())
        benches.append(
            {
                "id": meta["full_id"],
                "median_ns": round(est["median"]["point_estimate"], 1),
                "mad_ns": round(est["median_abs_dev"]["point_estimate"], 1),
            }
        )
    benches.sort(key=lambda b: b["id"])
    assert benches, "no criterion estimates found under target/criterion"

    out = {
        "schema": "swath-bench-baseline/1",
        "captured": datetime.date.today().isoformat(),
        "git_sha": sh("git", "rev-parse", "HEAD"),
        "rustc": sh("rustc", "--version"),
        "machine": {"model": model, "arch": platform.machine(), "os": system},
        "benches": benches,
    }
    path = pathlib.Path("docs/perf/bench-baseline.json")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(out, indent=2) + "\n")
    print(f"wrote {path} ({len(benches)} benches)")
    EOF

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
    SWATH_PERF_GRANULE="$granule" SWATH_PERF_RUNS={{runs}} python3 - <<'EOF'
    import datetime
    import json
    import os
    import pathlib
    import platform
    import statistics
    import subprocess
    import time

    def sh(*args: str) -> str:
        return subprocess.run(args, capture_output=True, text=True, check=True).stdout.strip()

    granule = os.environ["SWATH_PERF_GRANULE"]
    runs = int(os.environ["SWATH_PERF_RUNS"])
    assert runs >= 2, "need at least a cold run and one warm run"
    out_manifest = "target/referencer/perf-rs.vmanifest.json"
    generators = {
        "referencer-rs": [
            "target/release/swath", "ingest", "reference", granule,
            "--output", out_manifest,
        ],
        "virtualizarr-sidecar": ["python/.venv/bin/swath-referencer", granule],
    }

    def measure(cmd: list[str]) -> list[float]:
        times = []
        for _ in range(runs):
            t0 = time.perf_counter()
            subprocess.run(
                cmd, check=True,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
            times.append(round((time.perf_counter() - t0) * 1000, 1))
        return times

    results = {}
    for name, cmd in generators.items():
        samples = measure(cmd)
        results[name] = {
            "command": " ".join(cmd),
            "cold_ms": samples[0],
            "warm_median_ms": round(statistics.median(samples[1:]), 1),
            "runs_ms": samples,
        }
        print(f"{name}: cold {samples[0]} ms, warm median {results[name]['warm_median_ms']} ms")

    system = platform.system()
    if system == "Darwin":
        model = sh("sysctl", "-n", "machdep.cpu.brand_string")
    else:
        model = next(
            (
                line.split(":", 1)[1].strip()
                for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines()
                if line.startswith("model name")
            ),
            platform.machine(),
        )

    ratio = (
        results["virtualizarr-sidecar"]["warm_median_ms"]
        / results["referencer-rs"]["warm_median_ms"]
    )
    out = {
        "schema": "swath-referencer-baseline/1",
        "captured": datetime.date.today().isoformat(),
        "git_sha": sh("git", "rev-parse", "HEAD"),
        "rustc": sh("rustc", "--version"),
        "python": platform.python_version(),
        "machine": {"model": model, "arch": platform.machine(), "os": system},
        "granule": {
            "name": pathlib.Path(granule).name,
            "bytes": pathlib.Path(granule).stat().st_size,
        },
        "timing": "full-process wall clock; run 1 = cold, warm = median of the rest",
        "runs": runs,
        "generators": results,
        "warm_ratio_rust_advantage": round(ratio, 1),
    }
    path = pathlib.Path("docs/perf/referencer-baseline.json")
    path.write_text(json.dumps(out, indent=2) + "\n")
    print(f"wrote {path} (warm ratio {out['warm_ratio_rust_advantage']}x)")
    EOF

# Regenerate the results tables and measurement stamp in docs/PERFORMANCE.md
# from the committed artifacts (docs/perf/*.json). Everything between the
# `table:*` HTML-comment markers is owned by this recipe; the prose around
# the markers is hand-written. Run after any baseline artifact changes.
perf-doc:
    #!/usr/bin/env bash
    set -euo pipefail
    python3 - <<'EOF'
    import json
    import pathlib

    perf = pathlib.Path("docs/perf")
    bench = json.loads((perf / "bench-baseline.json").read_text())
    load = json.loads((perf / "load-baseline.json").read_text())
    i2p = json.loads((perf / "i2p-baseline.json").read_text())
    ref = json.loads((perf / "referencer-baseline.json").read_text())

    def human_ns(ns: float) -> str:
        if ns < 1_000:
            return f"{ns:.1f} ns"
        if ns < 1_000_000:
            return f"{ns / 1_000:.2f} us"
        return f"{ns / 1_000_000:.2f} ms"

    blocks = {}

    blocks["stamp"] = "\n".join(
        [
            "| instrument | artifact | measured at (git sha) | date |",
            "|---|---|---|---|",
            f"| ingest-to-pixel (`just perf-i2p`) | `docs/perf/i2p-baseline.json` | `{i2p['git_sha'][:7]}` | {i2p['timestamp']} |",
            f"| stage benches (`just bench-baseline`) | `docs/perf/bench-baseline.json` | `{bench['git_sha'][:7]}` | {bench['captured']} |",
            f"| load scenarios (`just load`) | `docs/perf/load-baseline.json` | `{load['git_sha']}` | {load['generated']} |",
            f"| referencer (`just perf-referencer`) | `docs/perf/referencer-baseline.json` | `{ref['git_sha'][:7]}` | {ref['captured']} |",
        ]
    )

    blocks["i2p"] = "\n".join(
        [
            "| metric | measured | enforced budget |",
            "|---|---:|---:|",
            f"| ingest_to_pixel_ms | {i2p['value']} ms | {i2p['budget_ms']} ms |",
        ]
    )

    rows = ["| bench | median | MAD |", "|---|---:|---:|"]
    for b in bench["benches"]:
        rows.append(f"| {b['id']} | {human_ns(b['median_ns'])} | {human_ns(b['mad_ns'])} |")
    blocks["bench"] = "\n".join(rows)

    rows = [
        "| scenario | requests | errors | rps | p50 ms | p95 ms | p99 ms | max ms |",
        "|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    labels = {
        "healthz_idle": "healthz — idle baseline",
        "hot_cache_storm": "(a) hot-cache tile storm",
        "cold_live_burst": "(b) cold live-render burst",
        "mixed_tile_storm": "(c) mixed tile storm",
        "healthz_under_warps": "(c) healthz UNDER WARPS",
    }
    for key, label in labels.items():
        s = load["scenarios"][key]
        rows.append(
            f"| {label} | {s['requests']} | {s['errors']} | {s['rps']} "
            f"| {s['p50_ms']} | {s['p95_ms']} | {s['p99_ms']} | {s['max_ms']} |"
        )
    sse = load["scenarios"]["sse_under_warps"]
    rows.append(
        f"\nSSE `/traces` under the mixed storm: survived the {sse['window_seconds']}s window: "
        f"**{'yes' if sse['survived_window'] else 'NO'}** "
        f"({sse['trace_events_received']} trace events received)."
    )
    blocks["load"] = "\n".join(rows)

    rows = [
        "| generator | command | cold | warm (median) |",
        "|---|---|---:|---:|",
    ]
    granule_name = ref["granule"]["name"]
    for name, g in ref["generators"].items():
        cmd = " ".join(
            f"<path-to>/{granule_name}" if arg.endswith(granule_name) else arg
            for arg in g["command"].split()
        )
        rows.append(f"| {name} | `{cmd}` | {g['cold_ms']} ms | {g['warm_median_ms']} ms |")
    rows.append(
        f"\nWarm-generation ratio (sidecar / Rust): "
        f"**~{ref['warm_ratio_rust_advantage']}x** in Rust's favor "
        f"({ref['runs']} runs; {ref['timing']})."
    )
    blocks["referencer"] = "\n".join(rows)

    doc = pathlib.Path("docs/PERFORMANCE.md")
    text = doc.read_text()
    for name, body in blocks.items():
        begin = f"<!-- table:{name} (generated by `just perf-doc` — edit the artifact, not this block) -->"
        end = f"<!-- /table:{name} -->"
        head, _, rest = text.partition(begin)
        assert rest, f"marker not found in PERFORMANCE.md: {begin}"
        _, _, tail = rest.partition(end)
        assert tail or rest.endswith(end), f"end marker not found: {end}"
        text = head + begin + "\n" + body + "\n" + end + tail
    doc.write_text(text)
    print(f"regenerated {len(blocks)} generated blocks in {doc}")
    EOF

# The one-command gate: everything CI enforces.
check: fmt-check lint machete test deny zizmor reuse
