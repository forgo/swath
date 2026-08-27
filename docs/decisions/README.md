# Architecture Decision Records (ADRs)

Each ADR captures **one significant decision**, dated and immutable. We don't edit a decision after the
fact; we supersede it with a new ADR that references the old one. This is the durable, historical record of
*why* Swath is shaped the way it is — the companion to the dated experiments in `../../prototypes/`.

Format per ADR: **Status · Date · Context · Decision · Consequences** (and, where relevant, the prototype
that produced the evidence).

## Index

| # | Date | Decision | Status |
|---|------|----------|--------|
| 0001 | 2026-08-08 | Hexagonal architecture; standards as interface contracts | Accepted |
| 0002 | 2026-08-08 | Pure-Rust, single-binary core; build/adopt/bind boundary | Accepted |
| 0003 | 2026-08-08 | License: Apache-2.0 with DCO; defer CLA | Accepted |
| 0004 | 2026-08-08 | Anchor datasets: HLS (clean), VIIRS (legacy-primary), MODIS (stretch) | Accepted |
| 0005 | 2026-08-08 | Frontend: Web Components + MapLibre GL; no framework/deck.gl | Accepted |
| 0006 | 2026-08-08 | Legacy referencer: staged Python→Rust behind one manifest port | Accepted (confirmed by prototype 0001) |
| 0007 | 2026-08-08 | Engineering standards & CI/CD foundation (see `docs/ENGINEERING.md`) | Accepted |
| 0008 | 2026-08-08 | Legacy-primary dataset: VNP09GA — VNP09 swath is HDF4 (amends 0004) | Accepted |
| 0009 | 2026-08-09 | Fenced spherical-sinusoidal math (narrow exception to ADR 0002) | Accepted |
| 0010 | 2026-08-09 | Authoring surface: native openEO API at a bounded profile | Accepted |
| 0011 | 2026-08-10 | UI ships inside the binary (`embedded-ui`); CORS opt-in, default off | Accepted |
| 0012 | 2026-08-10 | Render stays inline on the async runtime (resolves §16.7 with load evidence) | Accepted |
| 0013 | 2026-08-10 | Extension = compile-time features + openEO process graphs (closes §16.6/§14) | Accepted |
| 0014 | 2026-08-11 | Preview: openEO `POST /result` as a preview-bounded sync subset (extends 0010) | Accepted |
| 0015 | 2026-08-12 | Time dimension: frame selection via `datetime=`, latest-at-or-before (consumes roadmap row 7) | Accepted |
| 0016 | 2026-08-13 | Extraction boundary: swath-warp/-manifest/-referencer/-planner ship as 0.x alphas; the product stays | Accepted |
| 0017 | 2026-08-18 | Icechunk interop target: spec v2.1 via the icechunk crate | Accepted |
| 0018 | 2026-08-18 | `run_udf` as sandboxed WASM in the tile path | Accepted |
| 0019 | 2026-08-18 | Add-data goes through the engine; client-side COG rendering rejected | Accepted |
| 0020 | 2026-08-19 | Publish the UDF guest kit: `swath-udf-guest` is the fifth crate | Accepted |
| 0021 | 2026-08-26 | The UI system: shadow-DOM primitives on design tokens, one shell, modes in the URL (builds ADR 0005's reactive layer) | Accepted |
