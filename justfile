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

# Install pinned dev tools (prefers prebuilt binaries via cargo-binstall).
setup:
    #!/usr/bin/env bash
    set -euo pipefail
    # --force: we only call install() when the binary is provably absent, but a
    # restored CI cache can contain binstall's .crates.toml (claiming "already
    # installed") without the binaries — binstall would otherwise skip.
    install() { # name version
        if command -v cargo-binstall >/dev/null; then
            cargo binstall --no-confirm --force --version "$2" "$1"
        else
            cargo install --locked --force --version "$2" "$1"
        fi
    }
    command -v cargo-nextest  >/dev/null || install cargo-nextest  "{{nextest_version}}"
    cargo llvm-cov --version >/dev/null 2>&1 || install cargo-llvm-cov "{{llvm_cov_version}}"
    command -v cargo-deny     >/dev/null || install cargo-deny     "{{deny_version}}"
    command -v zizmor         >/dev/null || install zizmor         "{{zizmor_version}}"
    command -v prek           >/dev/null || install prek           "{{prek_version}}"
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
