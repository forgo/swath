# The extraction boundary (M8)

_Design note for issue #185, the mechanics companion to
[ADR 0016](../decisions/0016-extraction-boundary-published-crates.md). The ADR decides what
ships and what it promises; this note records the per-crate extraction shape the M8 issues
(#186–#193) execute, and the research behind the names. **Decision recorded in the ADR; this
note is the mechanics, and its work items are shipped.**_

## 1. The boundary, per crate

| Crate | Source today | What travels | Published deps | Workspace consumption |
|---|---|---|---|---|
| `swath-warp` | `crates/swath-render/src/warp.rs` (+ its grid/window geometry) | Kernel, GDAL-oracle goldens as crate tests, the GDAL-equivalence README contract | none (self-contained input types) | `swath-render` re-exports/wraps; existing goldens prove zero behavior change (#186) |
| `swath-manifest` | `crates/swath-core/src/manifest.rs` | Types/serde/validation, `compare()`, the schema snapshot test, a normative spec doc | serde, serde_json | `swath-referencer` + `swath-source-virtual` consume it (#187) |
| `swath-referencer` | `crates/swath-referencer` (whole crate) | Library + optional CLI feature, the VirtualiZarr conformance harness docs, measured claims via the marker discipline | `swath-manifest`; hdf5-metno behind `legacy-hdf5`, gribberish | The `IngestReferencer` port impl stays a thin shim in-tree (#188) |
| `swath-planner` | `crates/swath-core/src/planner.rs` | `plan()` + Budget/Availability model, proptests, the calibration constants and their documented basis | none (trait-shaped, IR-free inputs) | `swath-core` keeps trace integration behind the port (#189) |

Two rules bind every row (ADR 0016): a published crate never depends on an unpublished
workspace crate, and extraction adds zero new dependencies — each extracted tree is a subset of
what `deny.toml` already vets. Where a module today leans on `swath-core` types
(`RasterInfo`, `CoordTransform`, `PixelBuffer` in the warp kernel; the planner's trace types),
the extracted crate defines its own minimal input surface and the workspace adapts at the shim —
the same move in both directions keeps `swath-core` unpublished without duplicating logic.

## 2. Name research (crates.io, 2026-08-13)

The whole `swath*` namespace is unclaimed: `swath`, `swath-warp`, `swath-manifest`,
`swath-referencer`, `swath-planner`, `swath-core`, `swath-render`, `swath-icechunk`, and the
underscore variants all 404 on the crates.io API, and a full-text search for "swath" (18 hits)
returns no geospatial crate. The working names are adopted unchanged. Per the ADR, no name is
claimed by a placeholder: first publish ships real content from the extraction PRs, and
unpublished names stay unclaimed.

## 3. Versioning, MSRV, license — the crates tier in one place

- **Version**: lockstep with `[workspace.package] version` (`0.1.0-alpha.N`), bumped by the
  existing `cut-alpha` flow; no per-crate version divergence during alpha.
- **Alpha promise** (mirrors `docs/RELEASING.md`'s prerelease tier): built from a tagged commit
  through the full gate, `cargo publish --dry-run` green, `cargo-semver-checks` report
  attached — and no API/support stability between alphas. Graduation is per-crate and
  checklist-gated (ADR 0016).
- **MSRV**: the inherited workspace `rust-version` (1.95; stable-minus-~2, ENGINEERING.md §2).
  Alpha releases may bump it freely; a graduated crate bumps MSRV only with at least a minor.
- **License/REUSE**: Apache-2.0 with SPDX headers traveling in-source; each package carries
  `LICENSE` so it is REUSE-compliant standalone, outside this repo's `REUSE.toml` aggregate.

## 4. The publish pipeline (M8.8, #192)

Tag-triggered, zizmor-clean workflow; `cargo publish --dry-run` joins `just check`;
`cargo-semver-checks` over the published set; per-crate READMEs + docs.rs metadata. The final
`cargo publish` is maintainer-executed — publish credentials never live in CI (trusted
publishing, if chosen, is decided in #192). That PR also adds the crates tier to RELEASING.md
and amends ENGINEERING.md §7's blanket `publish = false` to name the four exceptions.

## 5. Icechunk interop (M8.6/M8.7/M8.9)

The roadmap §2 Icechunk section graduates from first written record to an executed plan
(ADR 0016): zarrs replaces the hand-rolled codec chain in `swath-source-virtual` (#190, also
the supply-chain rehearsal), the referencer commits virtual chunk references to an Icechunk
repo with an icechunk-python/xarray conformance gate (#191, spec-version target recorded as an
ADR addendum), and serving reads tiles back from an Icechunk commit byte-identical to the
manifest path, trace-visible (#193). The versioned-layer product UX remainder stays
demand-triggered in `docs/ROADMAP.md` item 15.
