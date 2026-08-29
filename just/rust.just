# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# Rust: toolchain setup, the per-crate gates, coverage, supply-chain and publishing checks.
# Imported by the root justfile; recipe names are global (`just <recipe>`).

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

# --- examples/udf (UDF guest kit, issue #202 / ADR 0018) ---

# Packaging dry-run for the five publishable crates (ADR 0016's four, #192,
# plus swath-udf-guest — ADR 0020, the #209 SDK): cargo
# resolves the in-workspace publish order and builds each packaged crate in
# isolation (path deps resolved via the multi-package local overlay, so the
# not-yet-on-crates.io `swath-manifest` still verifies under
# `swath-referencer`). This is the "can these actually publish?" gate; the
# actual `cargo publish` stays maintainer-executed (docs/RELEASING.md).
publish-dry:
    cargo publish --dry-run --allow-dirty         -p swath-manifest -p swath-planner -p swath-referencer -p swath-warp -p swath-udf-guest
