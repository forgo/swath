# ADR 0016 — Extraction boundary: four crates ship, the product stays

**Status:** Accepted · **Date:** 2026-08-13 · **Source:** issue #185, maintainer decisions from
the planning round 3 portability exploration · **Design note:**
`docs/design/extraction-boundary.md` · **Graduates:** `docs/ROADMAP.md` §2's Icechunk record

## Context

M8 ("Ship the parts, join the ecosystem") begins with a boundary question that must be decided
*before* any extraction PR, because its costs are asymmetric and permanent: a crates.io name,
once published, is forever (crates.io never reuses names), and a published API is a promise
someone external may start depending on the same day. The planning round 3 portability
exploration examined every piece of the tree for one test: **does the ecosystem already have
this?** Four pieces fail that test — nothing on crates.io does what they do:

- **The warp kernel** (`crates/swath-render/src/warp.rs`): GDAL 3.12 `GDALWarpKernel` semantics
  replicated bit-for-bit in pure Rust — scaled triangle filters under decimation, per-axis
  scale snapping, GDAL's exact validity cutoffs — proven against the GDAL oracle goldens.
- **Manifest v1** (`crates/swath-core/src/manifest.rs`): the versioned, deny-unknown,
  snapshot-pinned virtual-reference contract with interchangeable generators (ADR 0006) and an
  executable equivalence check (`compare`).
- **The referencer** (`crates/swath-referencer`): the production pure-Rust virtual-reference
  generator — HDF5/NetCDF4 and GRIB2, byte-identical to the VirtualiZarr sidecar at 39.5× its
  speed, behind a conformance harness.
- **The planner** (`crates/swath-core/src/planner.rs`): the pure, explainable, property-tested
  cost model (44–70 ns) whose GDAL-calibrated overview rule and full-candidate trace have no
  library-shaped equivalent.

Everything else either has ecosystem equivalents (HTTP serving, tiling frontends, storage
abstractions) or *is* the product — the tiler, the Render IR + process compiler, the trace/x-ray,
the API/CLI control plane. Extracting those would be maintenance without an external consumer,
and would give away exactly the integration that differentiates Swath.

The publishing posture on record says the opposite of what M8 needs: ENGINEERING.md §7 —
"All crates stay `publish = false`" — was written when publishing had no purpose. RELEASING.md's
two-tier discipline (pre-releases ship now with full rigor and zero stability promises; the
first official release is graduation-gated) exists, but only for binaries and images. And the
roadmap's Icechunk section is explicitly a "first written record" — a deferral, not a plan.

## Decision

**Extract and publish four crates, as `0.x` alphas; everything else stays unpublished.**

- **Names** — `swath-warp`, `swath-manifest`, `swath-referencer`, `swath-planner`. Researched
  on crates.io 2026-08-13: the entire `swath*` namespace is unclaimed — the four names, their
  underscore variants, and `swath`, `swath-core`, `swath-render`, `swath-icechunk` all return
  404, and a full-text search for "swath" finds no geospatial crate. The working names are
  adopted as-is; no alternates needed. **No placeholder squatting:** each name is first
  published by its real extraction PR's release (#186–#189), never as an empty reservation —
  the unpublished names (`swath` itself included) stay unclaimed until they carry content.
- **The standalone rule** — a published crate never depends on an unpublished workspace crate.
  `swath-manifest` is a leaf (serde + the georef vocabulary it already owns); `swath-referencer`
  depends on `swath-manifest` and stays wasmtime-free forever (#188's guard test); `swath-warp`
  and `swath-planner` take trait-shaped, self-contained inputs (IR-free, per #186/#189) instead
  of `swath-core` types. The port traits (`IngestReferencer`, the planner's trace integration)
  stay home in `swath-core`; the workspace consumes each published crate through a thin adapter
  shim, with zero behavior change proven by the existing goldens and e2e suite.
- **0.x-alpha semver policy** — published versions are the workspace version
  (`0.1.0-alpha.N`, lockstep, bumped by `cut-alpha` exactly as today; one version string,
  `[workspace.package]`). Externally, an alpha promises what RELEASING.md's prerelease tier
  promises, restated for crates: built from a tagged commit that passed the full CI gate
  including `cargo publish --dry-run` and a `cargo-semver-checks` report — and **not** an API
  stability commitment (anything may break between alphas; breakage is visible in the
  semver-checks report and deliberate, never accidental), not a support commitment (the fix is
  the next alpha). Graduating any crate to a non-prerelease version is per-crate, gated on the
  maintainer API-review boxes (#186/#189) and a crates-tier row added to RELEASING.md's
  graduation checklist; from graduation on, semver-checks findings are merge-blocking for that
  crate.
- **MSRV policy** — each published crate's `rust-version` is the inherited workspace MSRV
  (1.95 today; stable-minus-~2 per ENGINEERING.md §2, Renovate-trailed). During alpha, an MSRV
  bump may ride any release; after a crate graduates, an MSRV bump is at least a minor version.
- **REUSE / license posture** — Apache-2.0 (ADR 0003) unchanged. SPDX headers travel with the
  source; each published package includes its `LICENSE` so the packaged crate is
  REUSE-compliant standing alone, not only inside this repo's `REUSE.toml` aggregate.
- **cargo-deny discipline in extracted contexts** — the workspace `deny.toml` (all-features
  graph) keeps governing every published crate, and extraction itself may add **zero** new
  dependencies: an extracted crate's tree is a subset of what the gate already vets. A new dep
  in a published crate follows the written-justification checkpoint pattern (#190's zarrs
  review — the same pattern M9's wasmtime review reuses). The external-consumer CI smoke
  (#186: clean project, `cargo add`, reproduce a golden) proves each crate builds outside the
  workspace's lockfile and tooling.
- **What "publish" means operationally** — the M8.8 pipeline (#192): a tag-triggered,
  zizmor-clean workflow; `cargo publish --dry-run` joins `just check`; a `cargo-semver-checks`
  job over the published set; per-crate READMEs and docs.rs metadata. The actual
  `cargo publish` is **maintainer-executed** — no publish credentials live in CI (trusted
  publishing, if adopted, is decided and recorded in #192). RELEASING.md gains the crates tier
  in that PR, and ENGINEERING.md §7's "all crates stay `publish = false`" is amended there:
  exactly these four flip `publish = true` in their extraction PRs; every other crate stays
  `false`.
- **Icechunk graduates from record to plan** — the roadmap §2 Icechunk section stops being a
  "first written record" deferral and becomes executed interop: the referencer commits virtual
  chunk references to an Icechunk repo (M8.7, #191) and Swath serves tiles back from an
  Icechunk commit, byte-identical and traced (M8.9, #193), with the zarrs codec-chain adoption
  (M8.6, #190) as the enabling step. `swath-manifest` is the pivot: the manifest stops being a
  private-only format and joins the VirtualiZarr→Icechunk ecosystem. The versioned-layer
  *product UX* remainder stays demand-triggered (roadmap item 15).

## Consequences

- The boundary is a sentence: **libraries with no ecosystem equivalent ship; the product
  stays.** Future "should we extract X?" questions are answered by the same test, or they
  supersede this ADR.
- Namespace risk is retired first: the names are decided, verified free, and claimed only with
  real content — the one irreversible step is taken deliberately, before any code moves.
- The workspace keeps one version, one MSRV, one license gate, one deny graph — publishing
  adds a dry-run and a semver report to the existing gate rather than a parallel release
  system; the crates tier fits the two-tier discipline instead of inventing a third.
- Extraction PRs (#186–#189) have their contract fixed in advance: standalone rule, zero new
  deps, zero behavior change, goldens/proptests travel with the code, maintainer API review
  before publish.
- The planner's M9 evolution (the CPU-fuel cost axis) happens in public under semver gates —
  additive by review requirement (#189).

## Reopen / supersede conditions

- **Types gravity breaks the standalone rule** — if extraction pressure demands publishing
  `swath-core` itself (shared types outgrowing per-crate self-containment), that is a new
  boundary decision, not a quiet fifth crate.
- **A stay-home piece finds external demand** — a real consumer asking for the tiler, IR, or
  trace as a library reopens the boundary with evidence.
- **Icechunk spec drift** — #191 records the targeted Icechunk spec version (ADR addendum);
  a spec change that breaks the manifest-v1 mapping reopens the interop half.
- **Graduation** — the first non-prerelease publish of any crate supersedes the alpha policy
  section for that crate via RELEASING.md's crates-tier checklist.
