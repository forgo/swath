# ADR 0007 — Engineering standards & CI/CD foundation

**Status:** Accepted · **Date:** 2026-08-08

## Context

Before writing production code we want a bullet-proof foundation: repo-wide standards for the
polyglot monorepo (Rust core, TypeScript Web Components, thin Python sidecars) and GitHub workflows
that hold the line on linting, testing, coverage, security scanning, build/e2e, publish, and
release. Inputs: an audit of conventions already learned in the maintainer's prior repos
(forgo-rust, forgo-auth), a verified survey of mid-2026 practice in flagship Rust/polyglot OSS
(uv, ruff, Biome, Meilisearch, Zed), and post-incident supply-chain guidance (tj-actions
CVE-2025-30066, GhostAction, Shai-Hulud).

## Decision

Adopt the standards codified in **`docs/ENGINEERING.md` v1.0**. Headline commitments:

- **Layout:** Cargo workspace (`crates/`) + `web/` (pnpm) + `python/` (uv workspace); `just` as
  the single task entrypoint shared by CI and local dev (tasks-as-contract, from forgo-rust); no
  Bazel/Nx-class tooling.
- **Rust:** edition 2024, pinned stable toolchain, MSRV = stable−2 checked in CI; workspace-
  inherited lints (clippy pedantic-with-allows + targeted restriction lints, `-D warnings`);
  rustfmt stable defaults; cargo-deny (PR + nightly); nextest + proptest + insta + criterion;
  scheduled Miri on unsafe-bearing crates; cargo-llvm-cov → Codecov.
- **Web:** pnpm 11, Biome, strict TS 7, Vitest 4 Browser Mode (WebGL/MapLibre needs a real
  browser), Playwright e2e, ESM-only.
- **Python:** uv + ruff (lint+format), pyright now → ty at 1.0, pytest + Hypothesis, pip-audit.
- **Workflow security:** SHA-pinned actions, least-privilege permissions, OIDC trusted publishing
  everywhere (no long-lived tokens), zizmor + Scorecard + CodeQL(Rust) + dependency-review,
  Renovate (updates, cooldown) + Dependabot (security alerts).
- **CI shape:** one always-run workflow; `changes` path-filter job; a single `ci-ok` aggregator as
  the only required status check; formatting checked once; composite `setup-rust` action with
  tiered caching (from forgo-rust); smoke-test images before push (from forgo-auth).
- **Release:** release-plz (versioning/changelog/crates.io via trusted publishing) + pinned
  cargo-dist (single-binary artifacts, installers, cargo-auditable) + build-provenance
  attestations; squash-only merges with PR-title conventional-commit lint.
- **Hygiene:** SECURITY.md + Private Vulnerability Reporting, DCO via cncf/dco2 (per ADR 0003),
  CODEOWNERS owning `/.github/workflows/`, issue forms, SPDX/REUSE headers from day one, light
  prek pre-commit hooks, OpenSSF badge.

## Consequences

- The quality bar is enforced by machinery, not vigilance (R10); the supply-chain posture reflects
  the 2025-26 incident lessons rather than pre-incident habits.
- Known risks are named and pinned: cargo-dist stewardship history (pin + vendor), uv pre-1.0
  breaking minors (pin), ty pre-release (pyright until 1.0), Biome-vs-oxc contest (revisit on
  evidence). Deliberate non-adoptions are recorded in ENGINEERING.md §9.
- Supersedes nothing; complements ADRs 0001-0006. ENGINEERING.md amendments append to its §10 log;
  reversals of headline commitments get a superseding ADR.
