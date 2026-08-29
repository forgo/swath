# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# The web workspace: pnpm setup, the web gate, the embedded build, Playwright and screenshots.
# Imported by the root justfile; recipe names are global (`just <recipe>`).

# Install web deps + the Playwright chromium the browser tests run in.
setup-web:
    cd web && pnpm install --frozen-lockfile && pnpm exec playwright install chromium

# Biome lint/format check + TypeScript typecheck.
lint-web:
    cd web && pnpm run lint && pnpm run typecheck

# Vitest Browser Mode (real chromium — Custom Elements + WebGL are untestable in jsdom).
test-web:
    cd web && pnpm run test

# The web half of `just check` (issue #271): exactly what CI's `web` job
# runs — biome lint, tsc typecheck, vitest unit tests — so a broken `.ts`
# can't pass the local gate and redden CI. Needs `just setup-web` (deps +
# chromium). The web toolchain is optional for Rust-only work, so no pnpm
# on PATH is a loud SKIP (exit 0, the test-referencer pattern — never a
# silent pass); pnpm present but deps not installed is a FAIL that names
# the fix, because CI will run these regardless.
check-web:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v pnpm >/dev/null; then
        echo "SKIP check-web: pnpm not on PATH — CI's web job (lint, typecheck,"
        echo "  vitest) will still run. Install node 24 + pnpm, then \`just setup-web\`."
        exit 0
    fi
    if [ ! -d web/node_modules ]; then
        echo "FAIL check-web: web/node_modules missing — run \`just setup-web\`." >&2
        exit 1
    fi
    just lint-web test-web

# The full production build (issue #103): the web bundle (vite, into
# web/dist), then the release binary embedding it (swath-cli's build
# script stages web/dist; feature `embedded-ui` is default-on). The result
# is one self-contained `swath` — `swath serve --fixtures` serves the UI
# at / and the demo layers beside it, zero external files.
build-full:
    cd web && pnpm run build
    cargo build --release --locked -p swath-cli

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
