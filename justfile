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

# Bring up the local stack (pgstac + MinIO), smoke-check it, tear it down.
# Grows into the real e2e harness as the binary and viewer land (issues #29+).
e2e:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'docker compose down -v' EXIT
    docker compose up -d --wait
    docker compose exec -T pgstac psql -qtA -c "select pgstac.get_version();" | grep -E '^[0-9.]+' \
        && echo "pgstac: migrations present"
    curl -sf http://localhost:9000/minio/health/live && echo "minio: live"
    echo "e2e smoke OK"

# The one-command gate: everything CI enforces.
check: fmt-check lint test deny zizmor reuse
