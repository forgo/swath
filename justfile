# Swath task contract (ENGINEERING.md §1, ADR 0007).
# CI runs exactly these recipes; anything CI checks, a developer runs identically
# with `just <recipe>`. One entrypoint, no drift.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Pinned dev-tool versions (Renovate bumps these alongside CI).
nextest_version := "0.9.143"
llvm_cov_version := "0.8.7"
deny_version := "0.20.2"

# List available recipes.
default:
    @just --list

# Install pinned dev tools (prefers prebuilt binaries via cargo-binstall).
setup:
    #!/usr/bin/env bash
    set -euo pipefail
    install() { # name version
        if command -v cargo-binstall >/dev/null; then
            cargo binstall --no-confirm --version "$2" "$1"
        else
            cargo install --locked --version "$2" "$1"
        fi
    }
    command -v cargo-nextest  >/dev/null || install cargo-nextest  "{{nextest_version}}"
    cargo llvm-cov --version >/dev/null 2>&1 || install cargo-llvm-cov "{{llvm_cov_version}}"
    command -v cargo-deny     >/dev/null || install cargo-deny     "{{deny_version}}"
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

# The one-command gate: everything CI enforces.
check: fmt-check lint test deny
