# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# The documentation gates and their measurement.
# Imported by the root justfile; recipe names are global (`just <recipe>`).

# The docs-drift gate alone (issues #119/#173/#174): CONFIG.md vs the
# clap/serde schemas, ENDPOINTS.md vs the axum routers, source-fingerprint
# stamp freshness, deferral pointers, cross-doc claims, inline
# headline-number markers vs the perf artifacts — mutation verification
# included. The same tests run inside `just test` (they live in swath-cli);
# this recipe is the fast docs-only loop and what CI's docs-check job runs
# (with full git history: SWATH_DOCS_CHECK_REQUIRE_GIT=1 forbids the
# shallow-checkout skip of the gate's history-dependent tests there).
docs-check:
    cargo nextest run -p swath-cli -E 'test(docs_check)'

# The word-count measurement behind the word budgets (issue #177, #342):
# `wc -w` per doc over README.md + docs/*.md + docs/design/*.md, then the
# docs/media/*.md sum the directory ceiling applies to — the same counting
# rule as crates/swath-cli/src/docs_check/budgets.rs.
docs-words:
    #!/usr/bin/env bash
    set -euo pipefail
    wc -w README.md docs/*.md docs/design/*.md | sort -n
    echo "docs/media/*.md sum: $(cat docs/media/*.md | wc -w | tr -d ' ')"
