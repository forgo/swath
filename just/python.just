# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# The Python workspace (oracles, the legacy referencer sidecar).
# Imported by the root justfile; recipe names are global (`just <recipe>`).

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
