# Swath — Engineering Standards & CI/CD Foundation

_Draft v1.0 — 2026-08-08. The repo-wide contract for how we build. Recorded as
[ADR 0007](decisions/0007-engineering-standards-ci-foundation.md), which carries the full
context — the lineage (forgo-rust, forgo-auth, a verified survey of flagship 2026 polyglot OSS,
post-incident supply-chain guidance) and the contested alternatives weighed. This file is the
operating summary plus the as-shipped status of every practice._

---

## 1. Repository layout (polyglot monorepo)

Cargo workspace (`crates/`) + `web/` (pnpm) + `python/` (uv workspace) + `prototypes/` (dated,
immutable) + `docs/`; SHA-pinned workflows and a composite `setup-rust` action. No Bazel-class
tooling. **Tasks are the contract**: CI never invokes raw tool commands a developer can't run
identically via `just <recipe>`; `just check` is everything CI enforces. The workflow around it
— gate before push, merge on green by exit code — is `CONTRIBUTING.md`; the docs gates it names
live in `tools/docs-check/src/check/`.

## 2. Rust standards

- **Edition 2024, resolver 3, pinned stable**; MSRV = stable minus ~2 releases, checked by a
  CI job. Workspace inheritance everywhere (`[workspace.package/dependencies/lints]`).
- **Lints**: `unsafe_code = "warn"`, `unreachable_pub = "warn"`; clippy `pedantic` at warn with
  a curated allow-list plus targeted restriction lints; CI runs clippy with `-D warnings`.
  rustfmt: stable defaults, no config.
- **Supply chain**: cargo-deny on every PR and nightly; the reasoning per dependency is
  `SUPPLY-CHAIN.md`.
  `cargo auditable` is **deferred to graduation tier** (§7, RELEASING.md); cargo-vet/crev
  skipped; `cargo-semver-checks` only when library crates are published.
- **Testing stack**: cargo-nextest (plus `cargo test --doc`), proptest, insta, criterion.
  **Miri**: dormant by design — no crate contains `unsafe`; it lands with the first
  unsafe-bearing crate. **ASan/UBSan**: **explicitly deferred** — FFI has entered (bundled
  libhdf5 behind `legacy-hdf5`) but no sanitizer mode exists; deferral inventory candidate for
  ROADMAP.md (#126); the honest mitigations meanwhile are the referencer conformance gate and
  the known-answer tests.
- **Coverage**: cargo-llvm-cov → Codecov (informational; no hard gate until a baseline exists).
- **Fast dev-loop profile** (#99): the libhdf5 C build sits behind the default-ON `legacy-hdf5`
  feature; `just check-fast`/`just test-fast` are the opt-out, and CI's `rust-check-fast` keeps
  the feature-off state compiling.

## 3. TypeScript / Web Components standards (`web/`)

pnpm 11 (pinned; its minimum-release-age default is a supply-chain guard). Biome for lint +
format. TypeScript 7, strict baseline. Vitest 4 Browser Mode for component tests — MapLibre is
untestable outside a real browser — plus Playwright e2e. ESM-only; vanilla-vs-Lit stays
per-ADR-0005 vanilla. `custom-elements.json` manifest: **explicitly deferred** until the
components are published as a library (ADR 0007); the showcase exists as the embedded demo
viewer (`web/demo/`). UI structure per ADR 0021 / `docs/design/ui-system.md`: shadow-DOM
primitives in `web/src/ui/` (never importing upward) on `tokens.css`; the DRY gate
(`check-ui-dry`) runs in `pnpm run lint`.

## 4. Python sidecar standards (`python/`)

uv workspace (single committed `uv.lock`, pinned uv in CI); ruff for lint **and** format;
pyright now, migrating to Astral's `ty` at its 1.0 (contested — ADR 0007); pytest + Hypothesis
(mirroring proptest across the referencer port); pip-audit.

## 5. Workflow security posture (non-negotiable)

Standing rules, enforced by tooling (each a codified lesson from the 2025-26 incidents —
tj-actions CVE-2025-30066, GhostAction, Shai-Hulud): **every action SHA-pinned** (Renovate
keeps pins fresh); **top-level `permissions: contents: read`**, per-job elevation only,
`id-token: write` only on publish jobs inside a reviewed environment; **no long-lived registry
tokens** — OIDC trusted publishing everywhere; **never checkout PR code under
`pull_request_target`** (dorny/paths-filter, never tj-actions/*); **zizmor** in CI and
pre-commit; **Scorecard** + `dependency-review-action`; caches treated as untrusted near PR
triggers; **Renovate for updates** (grouped, `minimumReleaseAge` cooldown) **+ Dependabot for
security alerts only**; harden-runner **not deployed** (telemetry-only, not a boundary — a
judgment call left open, not a gap); CodeQL with the Rust pack as supplementary signal.

## 6. CI architecture

One always-triggered `ci.yml` (workflow-level `paths:` breaks required checks): a `changes`
path-filter job fans out to rust, rust-msrv, an OS matrix, web, python, zizmor/deny, and e2e,
aggregated by **`ci-ok`** — the ONLY required status check (`if: always()`, failing on any
failure or unexpected skip), decoupling branch protection from matrix shape. Formatting is
checked once; the composite `setup-rust` action does toolchain + tiered caching + pinned tools;
scheduled security surfaces live in `security.yml`/`scorecard.yml`/`codeql.yml`; images are
**smoke-tested before pushing** to GHCR. Merge queue: adopt with a second regular committer.

## 7. Release & publish

**Implemented (pre-release tier)** — issue #116; the operating manual is `docs/RELEASING.md`,
whose maintainer-signed graduation checklist gates the first official release.

Two-tier discipline: `v0.1.0-alpha.N` tags ship now as GitHub prereleases with full build
rigor and zero stability commitment; plain-semver releases are forbidden until graduation.
release-plz turns merged release PRs into tags (pre-release *bumps* computed by
`cut-alpha.yml`); cargo-dist builds the artifacts on tag push (mac-arm64 + linux-x64 with the
embedded viewer, checksums, automatic prerelease marking; the workflow is vendored and
hand-hardened); `release-image.yml` publishes versioned GHCR images, smoke-tested before push.
Exactly five crates publish (swath-manifest/-planner/-referencer/-warp, ADR 0016, plus
swath-udf-guest, the UDF SDK, ADR 0020; maintainer-executed, RELEASING.md's crates tier); every
other crate stays `publish = false`.
Installers, `cargo auditable`, and attestation (SLSA L2) are graduation-tier. Conventional commits via squash-only merges + PR-title lint — squashed titles
are exactly what release-plz and git-cliff read.

## 8. Project hygiene

SECURITY.md + Private Vulnerability Reporting. **DCO** via cncf/dco2 — a required status
alongside `ci-ok` and `lint-pr-title` (ADR 0003; DCO grants no relicensing rights). CODEOWNERS
owning `/.github/workflows/`. Issue forms; PR template. SPDX + REUSE lint (the open-core
boundary, machine-checkable). Light prek pre-commit hooks via `just setup`, never mandatory.
**OpenSSF Best Practices badge**: **explicitly deferred** — not applied for; deferral inventory
candidate for ROADMAP.md (#126). Contributor Covenant.

## 9. What we deliberately did NOT adopt

Considered, not missed: cargo-vet/crev, tarpaulin, Bazel/Nx/moon, husky, Lit (ADR 0005),
per-commit commitlint, harden-runner-as-boundary, workflow-level `paths:` filtering,
`--allow-dirty` publishing, cargo-make.

## 10. Amendments

- 2026-08-08 — v1.0 established (with ADR 0007).
- 2026-08-10 — §7 planned → implemented (pre-release tier; #116).
- 2026-08-11 — full aspiration-vs-reality pass (#123): every practice now either exists in the
  tree/CI or carries an explicit status marker (§§2-3, 5, 8).
- 2026-08-26 — §3: the UI system (ADR 0021) and its DRY gate (#277).
- 2026-08-29 — §§1, 8: workflow rules moved to `CONTRIBUTING.md`, their one home (#341).
