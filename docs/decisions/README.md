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
| 0006 | 2026-08-08 | Legacy referencer: staged Python→Rust behind one manifest port | Accepted (pending prototype 0001) |
| 0007 | 2026-08-08 | Engineering standards & CI/CD foundation (see `docs/ENGINEERING.md`) | Accepted |
