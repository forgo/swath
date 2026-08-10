# Releasing Swath

Two tiers, deliberately unequal (ENGINEERING.md §7, issue #116): **pre-releases**
(`v0.1.0-alpha.N`, later `-rc.N`) ship now with full build rigor and zero stability
promises; the **first official release** is gated behind the graduation checklist at the
bottom, which itself requires maintainer sign-off. Everything in between — version
strings, tags, changelog, artifacts, images — is produced by tooling. The only human
actions in a release are dispatching the cut workflow and merging the PR it opens
("tag approval").

## What an alpha/rc promises — and what it doesn't

A pre-release **is**:

- **Built** from a tagged commit that passed the full CI gate on main.
- **Tested** before anything is published: the GHCR image passes the same smoke test as
  every main-branch image (liveness, a real rendered tile, the embedded UI) *before* the
  push; binaries are built by the same `dist` pipeline that will build official releases.
- **Checksummed**: every artifact ships with a `.sha256`, plus an aggregate `sha256.sum`,
  attached to a GitHub release marked **prerelease**.
- **Changelogged**: the release body is the generated `CHANGELOG.md` section for that
  version — squash-merged conventional PR titles, grouped.

A pre-release is **not**:

- a semver commitment — anything may break between alphas, including CLI flags, config
  keys, HTTP surfaces, and on-disk/cache formats;
- a support commitment — no backports, no patch releases; the fix is the next alpha;
- a statement that the README, security posture, or API docs are done (that is exactly
  what graduation checks).

## Cutting an alpha

Prerequisite (once): a `RELEASE_PLZ_TOKEN` repo secret — fine-grained PAT for
`forgo/swath` with **contents: write** and **pull requests: write**. It exists because
`GITHUB_TOKEN`-pushed branches and tags trigger no workflows: the release PR would get
no CI and the tag would build no artifacts.

1. **Dispatch `Cut alpha`** (`cut-alpha.yml`). It computes the next
   `v0.1.0-alpha.N` from existing tags, applies it with `cargo set-version --workspace`
   (workspace version + every inter-crate requirement + `Cargo.lock`), regenerates
   `CHANGELOG.md` with git-cliff, and opens a release PR titled
   `chore(release): v0.1.0-alpha.N`. release-plz cannot compute pre-release bumps — its
   release detection only matches plain-semver tags (upstream discussion
   release-plz#2443) — which is the whole reason this workflow exists; see
   `release-plz.toml` for the split of responsibilities.
2. **Merge the release PR** — this is tag approval. It runs the full CI gate like any
   other PR.
3. Automation takes it from the squash commit:
   - `release-plz.yml` sees the `chore(release):` commit on main and `release-plz
     release` creates and pushes the `v0.1.0-alpha.N` tag (config: `release-plz.toml` —
     tag only; no crates.io, no GitHub release of its own).
   - `release.yml` (vendored cargo-dist workflow, pinned in `dist-workspace.toml`)
     builds `swath` for **aarch64-apple-darwin** and **x86_64-unknown-linux-gnu** with
     the embedded viewer, and creates the GitHub release: artifacts + checksums,
     marked **prerelease** automatically because the version carries a pre-release
     suffix, body from `CHANGELOG.md`.
   - `release-image.yml` builds the container image, **smoke-tests it before pushing**,
     and publishes exactly one immutable tag: `ghcr.io/forgo/swath:v0.1.0-alpha.N`.
     `latest` keeps tracking main (publish-image.yml) until graduation says otherwise.
4. **Watch the runs** (`gh run watch`) and spot-check the release page: prerelease flag
   set, artifacts + `.sha256` files present, and
   `docker run -p 8080:8080 ghcr.io/forgo/swath:v0.1.0-alpha.N serve --fixtures` serves
   a tile.

Tool pins: release-plz / git-cliff / cargo-edit in the `justfile` (crates datasource),
cargo-dist in `dist-workspace.toml` + the vendored `release.yml` installer URLs
(grouped as one Renovate PR). Renovate tracks all of them.

## Recovery

- Release-plz run lost or skipped: re-dispatch `Release-plz` — tagging is idempotent
  (an already-tagged or `0.0.0` version is a no-op).
- `release.yml` / `release-image.yml` failed after the tag exists: fix, then re-run the
  workflow from the tag. Never move or delete a pushed tag; a broken alpha is
  abandoned (delete the GitHub release if one was half-created) and the next alpha
  supersedes it.

## Graduation checklist — the first official release (`v0.1.0`)

Cutting any non-pre-release version is forbidden until every box below is checked.
The checklist is not a formality: each item is the difference between "built and
checksummed" and "other people can rely on this".

- [ ] **README rewrite complete** — install (binaries + image), quickstart, and honest
      scope/limitations reflect what the release actually does.
- [ ] **Security posture review** — SECURITY.md supported-versions table names the
      release line; threat-model/"security design" section reviewed against the shipped
      surface; open advisories triaged to zero.
- [ ] **Semver commitment declared** — what the public surface *is* (CLI, config, HTTP
      API, cache format) is written down, and from this release on, breaking it means a
      major/minor bump per semver, with deprecation notes in the changelog.
- [ ] **Release mechanics graduate** — release-plz's stock release-PR flow takes over
      version bumps and changelog (plain-semver tags match its detection); decide
      installers (shell/Homebrew), whether `latest` should track releases instead of
      main, and artifact attestation (`actions/attest-build-provenance`, SLSA L2 —
      ENGINEERING.md §7).
- [ ] **Maintainer sign-off** — the maintainer has reviewed this checklist and approves
      dropping the pre-release marker. No release PR for `v0.1.0` merges without it.

Update this file (and ENGINEERING.md §7) in the PR that graduates.
