# ADR 0023 — The catalog domain model: Dataset / Granule / Layer over a lossless STAC mapping

**Status:** Accepted · **Date:** 2026-08-29 (records the decision taken 2026-08-09 in
`docs/design/catalog-domain.md`; #340) · **Refs:** `docs/design/catalog-domain.md` (the mini-spec
and field tables), ARCHITECTURE.md §16.5 (resolved), REQUIREMENTS R2/R5, ADR 0010 (the openEO
surface serves STAC collections), issue #30

## Context

R2 says users manage *datasets* and *layers* and never see STAC; R5 says what Swath persists
must be a valid STAC catalog any third-party client can read. ARCHITECTURE §16.5 asked for the
exact schema that hides STAC yet round-trips to it losslessly. The decision was taken in a
design note and shipped as `swath_core::catalog` plus the pgstac adapter, but never recorded as
an ADR; the note is mutable, so the decision had no immutable home.

## Decision

1. **Three nouns, two of them STAC.** `Dataset` is a STAC Collection; `Granule` is a STAC Item;
   `Layer` is a serving definition stored as a `swath:layers` entry on the Collection. Nothing
   else is persisted.
2. **The identity is the contract.** `to_stac(from_stac(doc)) == doc` for every document Swath
   writes, and `from_stac(to_stac(d)) == d` for every in-bounds domain value; canonical ordering
   (sorted band sets, ordered layers) makes the round trip structural. A property test is the
   normative check; the pgstac integration suite is the R2/R5 bridge test.
3. **`Layer` stores a small storage-facing vocabulary (`PlanKind`), never the render IR.** The
   catalog sits below the render crate; lowering `PlanKind` → `RenderPlan` happens at serving
   wire-up so the persisted schema is decoupled from an IR that refactors freely.
4. **The `Catalog` port is domain-shaped and minimal** — upsert/get/list/find with a
   `GranuleQuery` of bbox + datetime range — not a STAC search façade. Breadth is added when a
   consumer exists.
5. **Errors name the seam.** `CatalogError::Stac` is the "someone else wrote to our database"
   signal, distinct from `Backend`; `StacError` names the JSON path.

## Consequences

- The word "STAC" stays out of Swath's own control plane and UI; it appears at the openEO
  boundary because openEO collections *are* STAC (REQUIREMENTS §10 A1).
- Adding a persisted field means adding it to the mapping and the property test together.
- Full JSON-Schema validation of STAC 1.1 is deliberately not vendored; pgstac validates on
  ingest and the integration gate reads back through plain `pgstac.search`.
- The design note keeps the field-by-field tables and the validation strategy as mechanics;
  changing any of the five points above means a superseding ADR.
