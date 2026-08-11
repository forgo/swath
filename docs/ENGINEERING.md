# Swath — Engineering Standards & CI/CD Foundation

_Draft v1.0 — 2026-08-08. The repo-wide contract for how we build: toolchains, linting, testing,
coverage, security posture, CI architecture, and release automation. Recorded as ADR 0007. Every
claim here was verified against current (mid-2026) tooling status, not folklore; contested calls are
marked and the alternative named, so future us can re-litigate with context._

Lineage: this synthesizes (a) conventions already learned in the maintainer's prior repos —
[forgo-rust](https://github.com/forgo/forgo-rust) (composite setup action, tiered caching,
tasks-as-the-CI/local-contract, per-crate release model) and
[forgo-auth](https://github.com/forgo/forgo-auth) (least-privilege workflow permissions, pinned
formatter shared by CI/local/pre-commit, sanitizer test modes, smoke-test-before-push, layered test
taxonomy, embedded-but-hot-reloadable assets, exemplary SECURITY.md) — with (b) the practices of
flagship 2026 Rust/polyglot repos (uv, ruff, Biome, Meilisearch, Zed) and post-incident
(tj-actions, GhostAction, Shai-Hulud) supply-chain guidance.

---

## 1. Repository layout (polyglot monorepo)

The dominant convention among flagship polyglot Rust projects (uv, ruff, Biome, Tauri):

```
swath/
  Cargo.toml            # workspace root: [workspace.package/dependencies/lints]
  Cargo.lock            # committed (binary project)
  rust-toolchain.toml   # pinned stable channel
  deny.toml             # cargo-deny: advisories, licenses, bans, sources
  justfile              # THE task entrypoint — CI and local dev run the same recipes
  crates/               # Rust workspace members (see ARCHITECTURE.md §12)
  web/                  # TypeScript Web Components + MapLibre (pnpm)
  python/               # thin ingest sidecars (uv workspace, single uv.lock)
  prototypes/           # dated, immutable trade-studies (own convention; excluded from workspace)
  docs/                 # charter, requirements, architecture, this doc, decisions/
  renovate.json         # update automation (repo root; Dependabot alerts are a repo setting — §5.8)
  .github/
    workflows/          # ci.yml, security.yml, release*.yml, scorecard.yml, codeql.yml … (SHA-pinned)
    actions/setup-rust/ # composite action: toolchain + tiered caching + pinned tools
    CODEOWNERS  ISSUE_TEMPLATE/  pull_request_template.md
```

Heavy monorepo tooling (Bazel, Nx, moon, Pants) is **not warranted** — none of the surveyed
flagship projects at our scale use it. Structure comes from Cargo + pnpm + uv workspaces; task
orchestration from `just`; CI path-filtering from a `changes` job (§6).

**Tasks are the contract** (carried from forgo-rust, with `just` replacing cargo-make): CI never
invokes raw tool commands that a developer can't run identically via `just <recipe>`. One
entrypoint, no drift. *(Contested alternative: `mise` tasks; `just` has the larger OSS footprint.
Per-language version pinning stays in the idiomatic files — `rust-toolchain.toml`,
`packageManager`, `.python-version` — which mise reads natively if we adopt it later.)*

## 2. Rust standards

- **Edition 2024, resolver 3, pinned stable** in `rust-toolchain.toml` (with `clippy`, `rustfmt`
  components); Renovate bumps the pin. Declare `rust-version` (MSRV) in `[workspace.package]`;
  policy: **stable minus ~2 releases** (binary-first project — aggressive is fine); CI has an MSRV
  `cargo check` job. The MSRV-aware resolver is default under edition 2024.
- **Workspace inheritance everywhere**: `[workspace.package]` (version, edition, license,
  repository), `[workspace.dependencies]`, `[workspace.lints]` + per-crate `[lints] workspace =
  true`. No per-crate duplication (the forgo-rust gap, fixed).
- **Lints** (modeled on uv's config, the strongest public template):
  - `[workspace.lints.rust]`: `unsafe_code = "warn"` (visible, justified, and it scopes Miri),
    `unreachable_pub = "warn"`.
  - `[workspace.lints.clippy]`: `pedantic = { level = "warn", priority = -1 }` with a curated
    allow-list (`missing_errors_doc`, `module_name_repetitions`, `too_many_lines`, …, grown
    honestly as false positives appear) plus targeted restriction lints: `print_stdout`,
    `print_stderr`, `dbg_macro`, `exit`, `get_unwrap`, `rc_buffer`, `rc_mutex`. Never enable
    `restriction` or `nursery` wholesale.
  - CI runs `cargo clippy --workspace --all-targets -- -D warnings`.
- **rustfmt: stable defaults, no config.** The two settings worth wanting (import grouping /
  granularity) are still nightly-only; we don't take a nightly dependency for formatting.
  *(Contested; revisit if/when they stabilize.)*
- **Supply chain**: `cargo-deny` (advisories + licenses + bans + sources) on every PR **and**
  nightly scheduled (RustSec updates daily; scheduled run files issues — forgo-rust's audit
  pattern, upgraded from cargo-audit to deny). `cargo auditable` embedded in release binaries.
  cargo-vet/crev: skipped for now (org-scale tooling); `cargo-semver-checks` only when we publish
  library crates (release-plz integrates it). *Status: deny runs in CI (`rust-deny`) and nightly
  (`security.yml`); `cargo auditable` is **deferred to graduation tier** (§7, RELEASING.md
  checklist) — release binaries do not embed it yet.*
- **Testing stack**: `cargo-nextest` as the runner (plus a `cargo test --doc` step — nextest skips
  doctests); `proptest` for the planner's property tests; `insta` for snapshot tests; `criterion`
  for the planner/render benchmarks that gate the north-star latency budget *(divan acceptable for
  cheap always-on microbenches)*; **Miri** as a scheduled job on crates containing `unsafe` —
  *status: dormant by design — no crate in the workspace contains `unsafe` today
  (`unsafe_code = "warn"`, zero hits), so no Miri job exists; it lands with the first
  unsafe-bearing crate*; ASan/UBSan test mode carried from forgo-auth where FFI enters —
  *status: **explicitly deferred.** FFI has entered (the bundled libhdf5 C build behind
  `legacy-hdf5`) but no sanitizer test mode exists yet; deferral inventory candidate for
  ROADMAP.md (#126). Until then the honest mitigations are the referencer conformance gate
  (byte-equivalence vs the Python sidecar) and the known-answer tests.*
- **Coverage**: `cargo-llvm-cov` (region coverage) → **Codecov** (OSS default; informational PR
  comment + patch-coverage signal; no hard gate initially — gates come after the baseline exists).
- **Fast dev-loop profile** (issue #99): the referencer's HDF5/NetCDF4 support — and with it the
  bundled libhdf5 C build (hdf5-metno `static`) — sits behind the default-ON `legacy-hdf5`
  feature (`swath-referencer`, forwarded by `swath-cli`). Defaults are untouched (R8: `just
  test`, the release binary, and the Docker image all carry HDF5); `just check-fast` /
  `just test-fast` are the documented opt-out for loops that never touch legacy referencing —
  no C toolchain, minutes and gigabytes of target/ saved. A feature-off binary declines
  `.h5`/`.nc` with a loud "built without the `legacy-hdf5` feature" error; CI's
  `rust-check-fast` job keeps the feature-off state compiling.

## 3. TypeScript / Web Components standards (`web/`)

- **pnpm 11** (pinned via `packageManager` + `devEngines`; Corepack is gone from Node 25+). Its
  default 24h minimum-release-age is a supply-chain guard we keep.
- **Biome** for lint + format — single fast tool, type-aware linting without tsc, right fit for a
  vanilla-TS no-framework codebase. *(Contested: oxlint+oxfmt is the momentum stack, ESLint the
  ecosystem stack; Biome wins on cohesion for greenfield. Note typescript-eslint currently can't
  run on TS 7 at all.)*
- **TypeScript 7** (Go-native, GA July 2026), strict baseline: `strict`,
  `noUncheckedIndexedAccess`, `verbatimModuleSyntax`, `erasableSyntaxOnly`, `target: ES2022+`;
  `isolatedDeclarations` + `declaration(Map)` if/when we publish the component library.
- **Vitest 4 with Browser Mode (Playwright provider)** for component tests — jsdom/happy-dom have
  no real Shadow DOM or WebGL, so **MapLibre is untestable outside a real browser**; Browser Mode
  is the 2026 consensus for Web Components. **Playwright** for e2e flows.
- **ESM-only** *(real)*; MapLibre GL as a `peerDependency` if the components are published *(not
  published — conditional, dormant)*; `custom-elements.json` manifest via
  `@custom-elements-manifest/analyzer` — *status: **explicitly deferred** until the components are
  published as a library; not generated today*; a served component **showcase page as living
  documentation** (carried from forgo-auth) — *status: exists as the embedded demo viewer
  (`web/demo/`, served by `just demo` and the fixtures container) rather than a per-component
  gallery*. Light-DOM vs Shadow-DOM and vanilla-vs-Lit stay per-ADR-0005 vanilla; revisit only if
  template/state complexity grows.

## 4. Python sidecar standards (`python/`)

- **uv workspace** (single committed `uv.lock`, `.python-version` pin; target Python ≥3.13);
  `uv_build` backend (stable, default, pure-Python — exactly our case); PEP 723 inline metadata +
  `uv run` for one-file scripts. uv is pre-1.0 — pin its version in CI.
- **ruff** for lint **and** format (Black is legacy for greenfield; ruff 0.16 defaults are now
  broad — add `I`, `S` (security), `DTZ` (naive-datetime bans — apt for ingest timestamps), `PT`,
  `SIM`, `PTH`).
- **Type checking: pyright now; migrate to Astral's `ty` at its 1.0.** *(Contested — ty is still
  pre-release (0.0.x) despite roadmap talk; Meta's pyrefly 1.0 is the other stable Rust-based
  option. For thin sidecars pyright is the safe pick with a planned migration.)*
- **pytest 9 + Hypothesis** (property tests on manifest generation — mirrors proptest on the Rust
  side of the same port, per prototype 0001's equivalence harness). **pip-audit** for dependency
  scanning (`uv audit` exists but is preview — not yet a gate).

## 5. Workflow security posture (non-negotiable)

The 2025-26 incidents (tj-actions/changed-files CVE-2025-30066, GhostAction, Shai-Hulud) each have
a codified lesson; these are standing rules, enforced by tooling, not memory:

1. **Every action SHA-pinned** with a `# vX.Y.Z` comment. Renovate keeps pins fresh (it converts
   tags→SHAs natively). Adopt GitHub's workflow lockfile when it ships (preview late 2026).
2. **Top-level `permissions: contents: read`**; per-job elevation only (forgo-auth already did
   this). `id-token: write` exists only on publish jobs, inside a GitHub **environment** with
   required reviewers.
3. **No long-lived registry tokens anywhere.** OIDC **trusted publishing** for crates.io (enforce
   trusted-publishing-only per crate — supported since Jan 2026), npm, and PyPI.
4. **Never checkout PR code under `pull_request_target`.** Avoid tj-actions/* entirely; use
   dorny/paths-filter (or the step-security fork) for change detection.
5. **zizmor** (workflow static analysis) in CI (SARIF) and pre-commit — it's the "clippy for
   workflows," Trail-of-Bits-audited, and catches template injection, cache poisoning, unpinned
   uses.
6. **OpenSSF Scorecard** action on a schedule; `dependency-review-action` on PRs.
7. Treat caches as untrusted near PR triggers (cache poisoning); `Swatinem/rust-cache` for Rust.
8. **Renovate for updates** (one config across Cargo/npm/pip/Actions, grouped PRs, a
   `minimumReleaseAge` cooldown — the direct Shai-Hulud countermeasure) **+ Dependabot for
   security alerts only**. *(Contested; this split is the common compromise.)*
9. harden-runner: `egress-policy: audit` for telemetry only — it is **not** a security boundary
   (bypass research + CVE-2026-32946). *Status: **not deployed** — no workflow installs it today;
   given it is telemetry-only, adopting it is a judgment call left open rather than a gap.*
10. CodeQL with the Rust pack (GA Oct 2025, codeql-action v4) — supplementary signal, not primary.

## 6. CI architecture

One `ci.yml`, always triggered (no workflow-level `paths:` — it breaks required checks), shaped as:

```
changes (dorny/paths-filter) ─┬─ rust: fmt → clippy → nextest (+ --doc) → llvm-cov
                              ├─ rust-msrv: cargo check on MSRV toolchain
                              ├─ rust-matrix: nextest on {ubuntu, macos, windows} (lint ubuntu-only)
                              ├─ web:  biome → tsc → vitest (browser mode) → build
                              ├─ py:   ruff → pyright → pytest
                              ├─ zizmor / deny (fast, always run)
                              └─ e2e:  compose up → playwright (needs rust+web builds)
                                        ↓
                              ci-ok  (needs: all, if: always(), fails on any failure
                                      or unexpected skip) ← the ONLY required status check
```

- The **`ci-ok` aggregator as sole required check** decouples branch protection from matrix/filter
  shape — jobs skipped by path-filter report success, and adding a matrix leg never strands a PR.
- `concurrency: group: ${{ github.workflow }}-${{ github.ref }}`, `cancel-in-progress` off-main only.
- **Composite `setup-rust` action** (carried from forgo-rust): toolchain install + two-tier cache
  (registry/git/target keyed on `Cargo.lock`; `~/.cargo/bin` keyed on pinned tool versions) +
  `cargo-binstall`-based pinned tool installs.
- Formatting is checked **once**, not per-matrix-leg (forgo-auth pattern).
- Scheduled security surfaces, as shipped: nightly cargo-deny advisories in `security.yml` (files
  issues); Scorecard and CodeQL in their own workflows (`scorecard.yml`, `codeql.yml`) rather than
  one file; Miri has no job because no crate carries `unsafe` (§2).
- Docker images (when they exist): buildx with GHA layer cache, **smoke-test the image before
  pushing** (forgo-auth pattern), GHCR via `GITHUB_TOKEN`.
- Merge queue: adopt when there's more than one regular committer; the `ci-ok` shape is already
  `merge_group`-compatible.

## 7. Release & publish

**Implemented (pre-release tier), graduation documented** — issue #116; the operating manual
is `docs/RELEASING.md`, which includes the graduation checklist (maintainer-signed) gating the
first official release.

- **Two-tier discipline**: `v0.1.0-alpha.N` tags ship now as GitHub **prereleases** with full
  build rigor (built, tested, checksummed, changelogged) and zero semver/stability commitment;
  any plain-semver release is forbidden until the graduation checklist in RELEASING.md is
  signed off.
- **release-plz** (pinned in the justfile, Renovate-tracked) turns merged release PRs into
  `v{version}` tags (`release-plz.yml`, config `release-plz.toml`). Known limitation, verified:
  its release detection only matches plain-semver tags, so pre-release *bumps* are computed by
  `cut-alpha.yml` (`cargo set-version --workspace` + git-cliff changelog, both pinned) and the
  stock release-PR flow takes over at graduation. All crates stay `publish = false` for now;
  crates.io via **trusted publishing (OIDC)** remains the plan for any genuinely reusable
  library.
- **cargo-dist (dist)** builds the release artifacts on tag push (`release.yml`):
  mac-arm64 + linux-x64 binaries with the embedded viewer, per-artifact `.sha256` + aggregate
  checksums, prerelease marking automatic from the version suffix. Version **pinned** in
  `dist-workspace.toml` and the workflow is **vendored** and hand-hardened (SHA-pinned actions,
  zizmor-clean) — `allow-dirty = ["ci"]` records that divergence. Installers and
  `cargo auditable` are graduation-tier decisions.
- **Versioned GHCR images**: `release-image.yml` publishes `ghcr.io/forgo/swath:v{version}`,
  smoke-tested before push with the same script as main-branch publishing; `latest` keeps
  tracking main until graduation.
- **Artifact attestation**: `actions/attest-build-provenance` on release artifacts (SLSA Build
  L2 ~free) at graduation; SLSA L3 via reusable workflow when demand appears.
- Web components (if published): npm **trusted publishing** (classic tokens are dead as of Dec
  2025); provenance automatic.
- **Conventional commits via squash-only merges + PR-title lint** (amannn/action-semantic-pull-request,
  step-security fork) — per-commit enforcement is noise; squashed PR titles are exactly what
  release-plz and git-cliff read. Tag scheme: the workspace releases in lockstep under
  `v<semver>`; any future independently-released component gets `<name>@<semver>`
  (forgo-rust convention).

## 8. Project hygiene

- **SECURITY.md** modeled on forgo-auth's (supported versions, 48-72h ack SLA, ~90-day coordinated
  disclosure, and a "security design" section documenting deliberate choices) + GitHub **Private
  Vulnerability Reporting** enabled (and maintainer notifications turned on — off by default).
- **DCO** enforced via **cncf/dco2** — as shipped, the `DCO` check is a required status in the
  `main protections` ruleset alongside `ci-ok` and `lint-pr-title`. Per ADR 0003; note honestly:
  DCO grants no relicensing rights — if a commercial `ee/` tree ever lands, it gets
  CLA-or-no-external-contributions treatment, not DCO.
- **CODEOWNERS**: catch-all `* @forgo`, explicit ownership of `/.github/workflows/` (workflow
  changes are the attack surface).
- **Issue forms** (YAML, `blank_issues_enabled: false`, security routed to PVR); markdown PR
  template with a DCO/checklist footer.
- **SPDX headers + REUSE lint** — *(contested in general, but earns its keep for open-core: it
  makes the Apache-2.0 vs any-future-commercial boundary machine-checkable from day one, when
  adding headers is cheap)*.
- **Pre-commit hooks stay light** (fmt + obvious lint; CI is the source of truth) via **prek**
  (Rust rewrite of pre-commit, same config; adopted by CPython/Ruff/FastAPI) — installed by
  `just setup`, never mandatory.
- **OpenSSF Best Practices badge** (passing tier) — *status: **explicitly deferred**; CI has
  landed but the badge application has not been made (Scorecard runs and is badged in the README;
  Best Practices is the remaining external audit). Deferral inventory candidate for ROADMAP.md
  (#126).*
- Contributor Covenant; CONTRIBUTING.md that mostly points at `just` recipes and this doc.

## 9. What we deliberately did NOT adopt

Recorded so future us knows these were considered, not missed: cargo-vet/crev (org-scale),
tarpaulin (llvm-cov is strictly better), Bazel/Nx/moon (unwarranted complexity), husky (forces
Node on every contributor), Lit (per ADR 0005, revisit on evidence), per-commit commitlint
(squash+title-lint instead), harden-runner-as-boundary (telemetry only), workflow-level `paths:`
filtering (breaks required checks), `--allow-dirty` publishing (forgo-rust's one smell —
release-plz eliminates it), cargo-make (replaced by `just` for the same tasks-as-contract idea).

## 10. Amendments

- 2026-08-08 — v1.0 established (with ADR 0007).
- 2026-08-10 — §7 updated from planned to implemented (pre-release tier; issue #116):
  release-plz + vendored cargo-dist + versioned GHCR images live, graduation checklist in
  docs/RELEASING.md.
- 2026-08-11 — full aspiration-vs-reality pass (issue #123; walked checklist in the PR). Every
  practice now either exists in the tree/CI or carries an explicit status marker. Corrections:
  scheduled security surfaces live in `security.yml` + `scorecard.yml` + `codeql.yml` (not one
  file); DCO is a required `DCO` status check in the `main protections` ruleset; Renovate config
  is `renovate.json` at the repo root and Dependabot security alerts are a repo setting (no
  `dependabot.yml`). Marked deferred/dormant: `cargo auditable` (graduation tier), Miri (no
  `unsafe` exists), ASan/UBSan mode (FFI entered via bundled libhdf5; deferred),
  `custom-elements.json` manifest (until the components are published), harden-runner (not
  deployed), OpenSSF Best Practices badge (not applied for).
