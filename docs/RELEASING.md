# Releasing Swath

Two tiers, deliberately unequal (ENGINEERING.md §7, #116): **pre-releases**
(`v0.1.0-alpha.N`, later `-rc.N`) ship now with full build rigor and zero stability
promises; the **first official release** is gated behind the maintainer-signed
graduation checklist below. Everything in between is tooling; the only human actions
are dispatching the cut workflow and merging the PR it opens ("tag approval").

## What an alpha/rc promises — and what it doesn't

A pre-release **is** built from a tagged commit that passed the full CI gate,
**smoke-tested before anything is published**, checksummed, marked **prerelease**, and
changelogged. It is **not** a semver commitment (anything may break between alphas),
not a support commitment (the fix is the next alpha), and not a statement that the
README, security posture, or API docs are done (exactly what graduation checks).

## Cutting an alpha

Prerequisite (once): a `RELEASE_PLZ_TOKEN` fine-grained PAT secret with
**contents: write** and **pull requests: write** (`GITHUB_TOKEN`-pushed refs trigger
no workflows).

1. **Dispatch `Cut alpha`** (`cut-alpha.yml`): computes the next `v0.1.0-alpha.N`,
   applies it with `cargo set-version --workspace`, regenerates `CHANGELOG.md`, and
   opens a `chore(release):` PR. (release-plz cannot compute pre-release bumps — its
   detection only matches plain-semver tags, release-plz#2443 — which is why this
   workflow exists; see `release-plz.toml` for the split.)
2. **Merge the release PR** — this is tag approval; it runs the full CI gate.
3. Automation takes it from the squash commit: `release-plz.yml` pushes the tag (tag
   only); `release.yml` (vendored cargo-dist) builds mac-arm64 + linux-x64 binaries
   with the embedded viewer and creates the prerelease-marked GitHub release with
   checksums; `release-image.yml` **smoke-tests then publishes** the one immutable
   `ghcr.io/forgo/swath:v0.1.0-alpha.N` tag (`latest` keeps tracking main).
4. **Watch the runs** (`gh run watch`) and spot-check the release page and the
   versioned image (`serve --fixtures` serves a tile).

Tool pins: release-plz / git-cliff / cargo-edit in the `justfile`; cargo-dist in
`dist-workspace.toml` + the vendored `release.yml`; Renovate tracks all of them.

## Crates tier (ADR 0016)

Release tags also run `publish-crates.yml`: `just publish-dry` (part of `just check`)
plus an informational cargo-semver-checks report. Exactly four crates publish —
swath-manifest, swath-planner, swath-referencer, swath-warp — maintainer-executed
from the tag, dependency-ordered; credentials never in CI (trusted publishing:
decided at first publish).

## Recovery

A lost release-plz run: re-dispatch it — tagging is idempotent. `release.yml` /
`release-image.yml` failed after the tag exists: fix, re-run from the tag. Never move
or delete a pushed tag; a broken alpha is abandoned and the next alpha supersedes it.

## Graduation checklist — the first official release (`v0.1.0`)

Cutting any non-pre-release version is forbidden until every box is checked — each item
is the difference between "built and checksummed" and "other people can rely on this".

- [ ] **README rewrite complete** — install (binaries + image), quickstart, and honest
      scope/limitations reflect what the release actually does.
- [ ] **Security posture review** — SECURITY.md supported-versions table names the
      release line; threat model reviewed against the shipped surface; open advisories
      triaged to zero.
- [ ] **Semver commitment declared** — what the public surface *is* (CLI, config, HTTP
      API, cache format) is written down; from this release on, breaking it means a
      semver bump with deprecation notes.
- [ ] **Crates graduate per crate** — semver-checks findings turn merge-blocking
      for a crate at its first non-prerelease publish.
- [ ] **Release mechanics graduate** — release-plz's stock release-PR flow takes over;
      decide installers, whether `latest` tracks releases, and artifact attestation
      (SLSA L2 — ENGINEERING.md §7).
- [ ] **Maintainer sign-off** — the maintainer has reviewed this checklist and approves
      dropping the pre-release marker. No release PR for `v0.1.0` merges without it.

Update this file (and ENGINEERING.md §7) in the PR that graduates.
