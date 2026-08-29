# Swath task contract (ENGINEERING.md §1, ADR 0007; the workflow around it is
# CONTRIBUTING.md). CI runs exactly these recipes; anything CI checks, a
# developer runs identically with `just <recipe>`. One entrypoint, no drift.
# Recipes live in just/*.just by area (imported below, one flat namespace);
# the tool pins stay here because Renovate reads them from this file.

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

import 'just/rust.just'
import 'just/fixtures.just'
import 'just/python.just'
import 'just/web.just'
import 'just/stack.just'
import 'just/perf.just'
import 'just/docs.just'

# The one-command gate: everything CI enforces, web included (check-web
# mirrors CI's `web` job; issue #271). check-fast/test-fast are the libhdf5
# opt-out for the Rust dev loop (just/rust.just), not a gate.
check: fmt-check lint machete test check-web deny zizmor reuse udf-fixtures-verify publish-dry
