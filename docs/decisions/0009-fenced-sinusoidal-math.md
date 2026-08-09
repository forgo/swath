# ADR 0009 — Fenced spherical-sinusoidal math (narrow exception to the bind boundary)

**Status:** Accepted · **Date:** 2026-08-09 · **Refines:** ADR 0002 (bind, never build — projection math)

## Context

Serving VIIRS gridded products (#39, ADR 0008) requires the MODIS-heritage spherical sinusoidal
projection. proj4rs 0.1.10 does not implement `sinu` (verified empirically), and ADR 0002 reserves
PROJ C-bindings for a deliberate, feature-gated long-tail adapter we have not yet needed.

## Decision

Carry a **narrow, fenced spherical-sinusoidal module** in the proj4rs adapter crate: the two-line
spherical branch of PROJ's own `PJ_sinu.c` (`x = R·λ·cosφ`, `y = R·φ`), sphere-only, meters-only,
with PROJ-consistent antimeridian wrapping; everything beyond that scope refuses as `UnknownCrs`.
Validated point-for-point against real PROJ 9.5.1 via the committed pyproj truth table (VNP09GA
grid corners/centers, both directions vs EPSG:4326 and 3857).

## Consequences

- A deliberate, documented, *deletable* exception to "bind, never build": the module dies the day
  proj4rs grows `sinu` or the PROJ C-binding adapter lands — both recorded in the module docs.
- The reusable reproject conformance suite covers it, so any replacement must prove itself against
  the identical truth table.
- The bind boundary otherwise stands; this ADR exists precisely so the exception stays visible
  instead of becoming precedent by accident.
