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
# prominently and asserted under a deliberately GENEROUS budget (30s —
# tightening is #35's job). The served tile is perceptually matched against
# the committed rio-tiler/GDAL golden (cross-encoder comparison is pdiff at
# the default policy, exactly like the golden suites; byte-stability is
# asserted by fetching twice), and a `trace` SSE event is captured. Teardown
# is trap-based: the stack never outlives the recipe.
e2e:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'docker compose down -v' EXIT
    dir=target/e2e
    granule=hlss30-t13sdd-2024158
    # The mounted data plane must exist (and be empty) before `up`.
    rm -rf "$dir" && mkdir -p "$dir/store/drop"
    docker compose build swath
    start=$(date +%s)
    docker compose up -d --wait
    echo "stack healthy in $(( $(date +%s) - start ))s (pull/start -> all healthchecks green)"
    docker compose exec -T pgstac psql -qtA -c "select pgstac.get_version();" | grep -E '^[0-9.]+' \
        && echo "pgstac: migrations present"
    curl -sf http://localhost:9000/minio/health/live && echo "minio: live"
    base=http://localhost:8080
    # Landing page (OGC API root) answers with the Swath document.
    curl -sf "$base/" | grep -q '"title":"Swath"' && echo "swath: landing page OK"
    # The catalog-backed dataset registered at startup: visible to a plain
    # STAC client (R5) before any granule exists.
    docker compose exec -T pgstac psql -qtA -c \
        "select pgstac.get_collection('hls-s30') is not null;" | grep -qx t \
        && echo "pgstac: hls-s30 dataset registered (plain STAC visibility)"
    # R1 pre-condition: the layer exists, its pixels don't — a tile of the
    # empty catalog is an honest 404.
    tile="$base/tilesets/truecolor/tiles/12/1561/848"
    code=$(curl -s -o /dev/null -w '%{http_code}' "$tile")
    [ "$code" = "404" ] || { echo "FAIL: expected 404 before any granule, got $code"; exit 1; }
    echo "swath: tile is 404 before ingest (catalog empty)"
    # THE DROP (the manual step count for everything below: zero). Per the
    # filedrop convention: band COGs land first, the manifest is staged
    # under an ignored dotfile name and renamed into place last.
    cp tests/fixtures/$granule-*.tif "$dir/store/"
    printf '%s\n' \
      '{' \
      '  "dataset": "hls-s30",' \
      "  \"granule\": \"$granule\"," \
      '  "bbox": [-106.1, 39.2, -105.9, 39.4],' \
      '  "datetime": "2024-06-06T17:54:00Z",' \
      '  "assets": {' \
      "    \"b02\": \"$granule-b02.tif\"," \
      "    \"b03\": \"$granule-b03.tif\"," \
      "    \"b04\": \"$granule-b04.tif\"," \
      "    \"b8a\": \"$granule-b8a.tif\"," \
      "    \"fmask\": \"$granule-fmask.tif\"" \
      '  }' \
      '}' > "$dir/store/drop/.$granule.json"
    mv "$dir/store/drop/.$granule.json" "$dir/store/drop/$granule.json"
    echo "swath: granule dropped at $(date -u '+%H:%M:%S') UTC"
    # Arrive -> catalog -> serve, automatically: poll until the tile is live.
    code=000
    for _ in $(seq 1 120); do
        code=$(curl -s -D "$dir/tile-headers.txt" -o "$dir/tile.png" -w '%{http_code}' "$tile")
        [ "$code" = "200" ] && break
        sleep 0.5
    done
    [ "$code" = "200" ] || { echo "FAIL: tile not servable within 60s of the drop (last: $code)"; exit 1; }
    echo "swath: tile went live with zero manual steps (R1)"
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
    echo ""
    echo "=========================================="
    echo "   INGEST-TO-PIXEL: $i2p ms"
    echo "=========================================="
    echo ""
    [ "$i2p" -lt 30000 ] || { echo "FAIL: ingest-to-pixel $i2p ms exceeds the 30000 ms budget"; exit 1; }
    echo "swath: ingest-to-pixel is under the generous 30s budget (#35 tightens)"
    # Same request, same bytes: the container render is deterministic.
    curl -sf -o "$dir/tile-again.png" "$tile"
    cmp "$dir/tile.png" "$dir/tile-again.png" && echo "swath: tile bytes are stable across requests"
    # Correctness oracle: the catalog-served tile perceptually matches the
    # committed golden — "a CORRECT tile is visible" (§3), not just any tile.
    cargo run --quiet -p swath-testkit --bin pdiff -- \
        "$dir/tile.png" crates/swath-render/tests/data/truecolor-12-848-1561.png
    echo "swath: tile matches the rio-tiler/GDAL golden (default pdiff policy)"
    # The x-ray stream: subscribe, trigger a render, expect a `trace` event
    # that also carries the ingest-to-pixel number.
    curl -sN --max-time 15 "$base/traces" > "$dir/traces.txt" &
    sse=$!
    sleep 1
    curl -sf -o /dev/null "$base/tilesets/ndvi/tiles/12/1561/848"
    for _ in $(seq 1 20); do
        grep -q '^event: trace' "$dir/traces.txt" && break
        sleep 0.5
    done
    kill "$sse" 2>/dev/null || true
    wait "$sse" 2>/dev/null || true
    grep -q '^event: trace' "$dir/traces.txt" && echo "swath: trace SSE event captured"
    grep -q '"ingest_to_pixel_ms":[0-9]' "$dir/traces.txt" \
        && echo "swath: SSE trace carries ingest_to_pixel_ms"
    echo "e2e OK"

# The one-command gate: everything CI enforces.
check: fmt-check lint test deny zizmor reuse
