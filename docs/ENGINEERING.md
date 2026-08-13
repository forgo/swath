# Swath — Engineering Standards & CI/CD Foundation

_Draft v1.0 — 2026-08-08. The repo-wide contract for how we build: toolchains, linting, testing,
coverage, security posture, CI architecture, and release automation. Recorded as
[ADR 0007](decisions/0007-engineering-standards-ci-foundation.md), which carries the full
context: the lineage (conventions from the maintainer's forgo-rust and forgo-auth repos, a
verified survey of flagship 2026 Rust/polyglot OSS, post-incident supply-chain guidance) and the
contested alternatives weighed. This file is the operating summary plus the as-shipped status of
every practice._

---

## 1. Repository layout (polyglot monorepo)

Cargo workspace (`crates/`) + `web/` (pnpm) + `python/` (uv workspace) + `prototypes/` (dated,
immutable) + `docs/`; SHA-pinned workflows and a composite `setup-rust` action. No Bazel-class
tooling — structure comes from the three workspaces, orchestration from `just`. **Tasks are the
contract**: CI never invokes raw tool commands a developer can't run identically via
`just <recipe>`.

## 2. Rust standards

- **Edition 2024, resolver 3, pinned stable**; MSRV = stable minus ~2 releases, checked by a
  CI job. Workspace inheritance everywhere (`[workspace.package/dependencies/lints]`).
- **Lints**: `unsafe_code = "warn"`, `unreachable_pub = "warn"`; clippy `pedantic` at warn with
  a curated allow-list plus targeted restriction lints; CI runs clippy with `-D warnings`.
  rustfmt: stable defaults, no config.
- **Supply chain**: cargo-deny (advisories + licenses + bans + sources) on every PR and nightly
  (`security.yml`, files issues). `cargo auditable` is **deferred to graduation tier** (§7,
  RELEASING.md checklist); cargo-vet/crev skipped (org-scale); `cargo-semver-checks` only when
  library crates are published.
- **Testing stack**: cargo-nextest (plus a `cargo test --doc` step), proptest, insta, criterion.
  **Miri**: dormant by design — no crate contains `unsafe`, so no job exists; it lands with the
  first unsafe-bearing crate. **ASan/UBSan**: **explicitly deferred** — FFI has entered (the
  bundled libhdf5 build behind `legacy-hdf5`) but no sanitizer mode exists yet; deferral
  inventory candidate for ROADMAP.md (#126). Until then the honest mitigations are the
  referencer conformance gate and the known-answer tests.
- **Coverage**: cargo-llvm-cov → Codecov (informational; no hard gate until a baseline exists).
- **Fast dev-loop profile** (issue #99): HDF5/NetCDF4 support — and with it the bundled libhdf5
  C build — sits behind the default-ON `legacy-hdf5` feature. Defaults are untouched (R8);
  `just check-fast` / `just test-fast` are the documented opt-out; a feature-off binary declines
  `.h5`/`.nc` with a loud error; CI's `rust-check-fast` job keeps the feature-off state
  compiling.

## 3. TypeScript / Web Components standards (`web/`)

pnpm 11 (pinned via `packageManager`; its minimum-release-age default is a supply-chain guard we
keep). Biome for lint + format. TypeScript 7, strict baseline (`strict`,
`noUncheckedIndexedAccess`, `verbatimModuleSyntax`, `erasableSyntaxOnly`). Vitest 4 Browser Mode
(Playwright provider) for component tests — MapLibre is untestable outside a real browser —
plus Playwright for e2e flows. ESM-only. Vanilla-vs-Lit stays per-ADR-0005 vanilla.
`custom-elements.json` manifest: **explicitly deferred** until the components are published as a
library (ADR 0007 records the call); the component showcase exists as the embedded demo viewer
(`web/demo/`) rather than a per-component gallery.

## 4. Python sidecar standards (`python/`)

uv workspace (single committed `uv.lock`, pinned uv version in CI); ruff for lint **and**
format; pyright now, migrate to Astral's `ty` at its 1.0 (contested — recorded in ADR 0007);
pytest + Hypothesis (property tests mirroring proptest across the referencer port); pip-audit.

## 5. Workflow security posture (non-negotiable)

The 2025-26 incidents (tj-actions CVE-2025-30066, GhostAction, Shai-Hulud) each have a codified
lesson; these are standing rules, enforced by tooling: **every action SHA-pinned** (Renovate
keeps pins fresh); **top-level `permissions: contents: read`** with per-job elevation only, and
`id-token: write` only on publish jobs inside a reviewed environment; **no long-lived registry
tokens** — OIDC trusted publishing everywhere; **never checkout PR code under
`pull_request_target`** (dorny/paths-filter, never tj-actions/*); **zizmor** in CI and
pre-commit; **Scorecard** on a schedule + `dependency-review-action` on PRs; caches treated as
untrusted near PR triggers; **Renovate for updates** (grouped, `minimumReleaseAge` cooldown)
**+ Dependabot for security alerts only** (a repo setting; no `dependabot.yml`); harden-runner
**not deployed** (telemetry-only, not a boundary — a judgment call left open, not a gap); CodeQL
with the Rust pack as supplementary signal.

## 6. CI architecture

One always-triggered `ci.yml` (workflow-level `paths:` breaks required checks): a `changes`
path-filter job fans out to rust, rust-msrv, an OS matrix, web, python, zizmor/deny, and e2e,
all aggregated by **`ci-ok`** — the ONLY required status check (`if: always()`, fails on any
failure or unexpected skip), decoupling branch protection from matrix/filter shape. Formatting
is checked once. The composite `setup-rust` action does toolchain + tiered caching + pinned
tools. Scheduled security surfaces live in `security.yml`/`scorecard.yml`/`codeql.yml`. Images
are **smoke-tested before pushing** to GHCR. Merge queue: adopt with a second regular committer
(the `ci-ok` shape is already `merge_group`-compatible).

## 7. Release & publish

**Implemented (pre-release tier), graduation documented** — issue #116; the operating manual is
`docs/RELEASING.md`, which includes the maintainer-signed graduation checklist gating the first
official release.

Two-tier discipline: `v0.1.0-alpha.N` tags ship now as GitHub prereleases with full build rigor
and zero stability commitment; any plain-semver release is forbidden until graduation.
release-plz turns merged release PRs into tags (pre-release *bumps* computed by `cut-alpha.yml`,
since release-plz's detection only matches plain-semver tags); cargo-dist builds the artifacts
on tag push (mac-arm64 + linux-x64 with the embedded viewer, checksums, automatic prerelease
marking; the workflow is vendored and hand-hardened); `release-image.yml` publishes versioned
GHCR images, smoke-tested before push. All crates stay `publish = false`; installers,
`cargo auditable`, and artifact attestation (SLSA L2) are graduation-tier. Conventional commits
via squash-only merges + PR-title lint — squashed PR titles are exactly what release-plz and
git-cliff read.

## 8. Project hygiene

SECURITY.md + Private Vulnerability Reporting. **DCO** enforced via cncf/dco2 — a required
status alongside `ci-ok` and `lint-pr-title` (ADR 0003; DCO grants no relicensing rights).
CODEOWNERS owning `/.github/workflows/`. Issue forms; PR template. SPDX headers + REUSE lint
(the open-core license boundary, machine-checkable). Light prek pre-commit hooks via
`just setup`, never mandatory. **OpenSSF Best Practices badge**: **explicitly deferred** — not
applied for (Scorecard runs and is badged); deferral inventory candidate for ROADMAP.md (#126).
Contributor Covenant; CONTRIBUTING.md points at `just` recipes and this doc.

## 9. What we deliberately did NOT adopt

Recorded so future us knows these were considered, not missed: cargo-vet/crev, tarpaulin,
Bazel/Nx/moon, husky, Lit (per ADR 0005), per-commit commitlint, harden-runner-as-boundary,
workflow-level `paths:` filtering, `--allow-dirty` publishing, cargo-make (replaced by `just`).

## 10. Amendments

- 2026-08-08 — v1.0 established (with ADR 0007).
- 2026-08-10 — §7 updated from planned to implemented (pre-release tier; issue #116).
- 2026-08-11 — full aspiration-vs-reality pass (issue #123; walked checklist in the PR). Every
  practice now either exists in the tree/CI or carries an explicit status marker
  (deferred/dormant markers in §§2-3, 5, 8).
