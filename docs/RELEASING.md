# Releasing Swath

Two tiers, deliberately unequal (ENGINEERING.md §7, issue #116): **pre-releases**
(`v0.1.0-alpha.N`, later `-rc.N`) ship now with full build rigor and zero stability
promises; the **first official release** is gated behind the graduation checklist at the
bottom, which requires maintainer sign-off. Everything in between is produced by tooling;
the only human actions are dispatching the cut workflow and merging the PR it opens
("tag approval").

## What an alpha/rc promises — and what it doesn't

A pre-release **is**: built from a tagged commit that passed the full CI gate;
**tested before anything is published** (the GHCR image passes the same smoke test as
every main-branch image *before* the push); checksummed (per-artifact `.sha256` + an
aggregate), attached to a GitHub release marked **prerelease**; and changelogged (the
generated `CHANGELOG.md` section — squash-merged conventional PR titles, grouped).

A pre-release is **not**: a semver commitment (anything may break between alphas,
including CLI flags, config keys, HTTP surfaces, and on-disk/cache formats); a support
commitment (no backports — the fix is the next alpha); or a statement that the README,
security posture, or API docs are done (that is exactly what graduation checks).

## Cutting an alpha

Prerequisite (once): a `RELEASE_PLZ_TOKEN` repo secret — fine-grained PAT with
**contents: write** and **pull requests: write** (`GITHUB_TOKEN`-pushed branches and
tags trigger no workflows).

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
`dist-workspace.toml` + the vendored `release.yml`. Renovate tracks all of them.

## Recovery

- Release-plz run lost or skipped: re-dispatch it — tagging is idempotent.
- `release.yml` / `release-image.yml` failed after the tag exists: fix, then re-run the
  workflow from the tag. Never move or delete a pushed tag; a broken alpha is abandoned
  and the next alpha supersedes it.

## Graduation checklist — the first official release (`v0.1.0`)

Cutting any non-pre-release version is forbidden until every box below is checked. Each
item is the difference between "built and checksummed" and "other people can rely on
this".

- [ ] **README rewrite complete** — install (binaries + image), quickstart, and honest
      scope/limitations reflect what the release actually does.
- [ ] **Security posture review** — SECURITY.md supported-versions table names the
      release line; threat model reviewed against the shipped surface; open advisories
      triaged to zero.
- [ ] **Semver commitment declared** — what the public surface *is* (CLI, config, HTTP
      API, cache format) is written down; from this release on, breaking it means a
      semver bump with deprecation notes.
- [ ] **Release mechanics graduate** — release-plz's stock release-PR flow takes over;
      decide installers, whether `latest` tracks releases, and artifact attestation
      (SLSA L2 — ENGINEERING.md §7).
- [ ] **Maintainer sign-off** — the maintainer has reviewed this checklist and approves
      dropping the pre-release marker. No release PR for `v0.1.0` merges without it.

Update this file (and ENGINEERING.md §7) in the PR that graduates.
