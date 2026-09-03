# Architecture Decision Records (ADRs)

Each ADR captures **one significant decision**, dated and immutable. We don't edit a decision after the
fact; we supersede it with a new ADR that references the old one. This is the durable, historical record of
*why* Swath is shaped the way it is — the companion to the dated experiments in `../../prototypes/`.

Format per ADR: **Status · Date · Context · Decision · Consequences** (and, where relevant, the prototype
that produced the evidence). A status may gain a dated annotation (Proposed → Accepted, a
prototype's confirmation); the decision text never changes. Design notes under `../design/`
prepare or carry the mechanics of a decision; the ADR is the ruling.

## Index

| # | Date | Decision | Status | Supersedes / amends |
|---|------|----------|--------|---|
| 0001 | 2026-08-08 | Hexagonal architecture; standards as interface contracts | Accepted |  |
| 0002 | 2026-08-08 | Pure-Rust, single-binary core; build/adopt/bind boundary | Accepted | Refined by 0009 |
| 0003 | 2026-08-08 | License: Apache-2.0 with DCO; defer CLA | Accepted |  |
| 0004 | 2026-08-08 | Anchor datasets: HLS (clean), VIIRS (legacy-primary), MODIS (stretch) | Accepted | Amended by 0008 |
| 0005 | 2026-08-08 | Frontend: Web Components + MapLibre GL; no framework/deck.gl | Accepted | Built on by 0021 |
| 0006 | 2026-08-08 | Legacy referencer: staged Python→Rust behind one manifest port | Accepted (confirmed by prototype 0001) |  |
| 0007 | 2026-08-08 | Engineering standards & CI/CD foundation (see `docs/ENGINEERING.md`) | Accepted |  |
| 0008 | 2026-08-08 | Legacy-primary dataset: VNP09GA — VNP09 swath is HDF4 (amends 0004) | Accepted | Amends 0004 |
| 0009 | 2026-08-09 | Fenced spherical-sinusoidal math (narrow exception to ADR 0002) | Accepted | Refines 0002 |
| 0010 | 2026-08-09 | Authoring surface: native openEO API at a bounded profile | Accepted | Extended by 0014 |
| 0011 | 2026-08-10 | UI ships inside the binary (`embedded-ui`); CORS opt-in, default off | Accepted |  |
| 0012 | 2026-08-10 | Render stays inline on the async runtime (resolves §16.7 with load evidence) | Accepted |  |
| 0013 | 2026-08-10 | Extension = compile-time features + openEO process graphs (closes §16.6/§14) | Accepted | Superseded by 0018 for the pixel stage |
| 0014 | 2026-08-11 | Preview: openEO `POST /result` as a preview-bounded sync subset (extends 0010) | Accepted | Extends 0010 |
| 0015 | 2026-08-12 | Time dimension: frame selection via `datetime=`, latest-at-or-before (consumes roadmap row 7) | Accepted |  |
| 0016 | 2026-08-13 | Extraction boundary: swath-warp/-manifest/-referencer/-planner ship as 0.x alphas; the product stays | Accepted |  |
| 0017 | 2026-08-18 | Icechunk interop target: spec v2.1 via the icechunk crate | Accepted |  |
| 0018 | 2026-08-18 | `run_udf` as sandboxed WASM in the tile path | Accepted | Supersedes 0013 for the pixel stage |
| 0019 | 2026-08-18 | Add-data goes through the engine; client-side COG rendering rejected | Accepted |  |
| 0020 | 2026-08-19 | Publish the UDF guest kit: `swath-udf-guest` is the fifth crate | Accepted |  |
| 0021 | 2026-08-26 | The UI system: shadow-DOM primitives on design tokens, one shell, modes in the URL (builds ADR 0005's reactive layer) | Accepted | Builds on 0005 |
| 0022 | 2026-08-28 | The two-cube join: `merge_cubes` at the bounded profile (gray × gray, same collection, resolver required; `datetime=` intersects every branch) | Accepted | Extends 0025 to a graph |
| 0023 | 2026-08-29 | Catalog domain model: Dataset / Granule / Layer over a lossless STAC mapping (records `design/catalog-domain.md`, 2026-08-09) | Accepted | Resolves ARCHITECTURE §16.5 |
| 0024 | 2026-08-29 | Planner v1: explicit per-layer knobs, transparent byte estimates, every candidate explained (records `design/materialization-planner.md`) | Accepted | Resolves ARCHITECTURE §16.4 |
| 0025 | 2026-08-29 | Authoring model B: the always-valid canvas (records `design/authoring-ux.md` §8, 2026-08-11) | Accepted | Extended by 0022 |
| 0026 | 2026-09-03 | The M12 freeze is contract-asserted, not path-asserted (amends 0021) | Accepted | Amends 0021 |

Historical pointers: ADRs 0002, 0003 and 0005 each say they supersede a note in "CHARTER.md §8";
the charter has since been rewritten and those notes no longer exist — the ADRs are the record.
