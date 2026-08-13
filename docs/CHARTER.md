# Swath — Project Charter

*Working document. v0.4 — August 2026. Vision, principles, phases; delivery status lives in
`ROADMAP.md`, positioning in `PITCH.md`, the requirements in `REQUIREMENTS.md`. Where this
charter and an ADR disagree, the ADR wins.*

---

## 1. What Swath is, in one sentence

An open-source, cloud-native geospatial platform where **satellite data comes in and is
immediately available on a map**, and where data scientists **tap the live data flow, derive
new products, and publish them the same way** — from a single pane of glass that hides the
plumbing.

## 2. Why this, why now

The cloud-native geospatial primitives — serving, cataloging, storage, legacy bridging, and a
processing standard (openEO, an OGC Community Standard since May 2026) — are done and
battle-tested. What does **not** exist is a *product* fusing them into one managed,
low-latency, observable loop; today that means hand-wiring eoAPI + a tiler + custom ingest + a
bespoke UI per deployment. The gap was identified independently twice — by the technical
survey, and by a practitioner building EO products for government: *"a way for data scientists
to tap into that flow of data to generate new products with low latency, and then host those
products the same way … It hasn't been addressed by anyone as far as I can see."*

## 3. The core thesis (the wedge)

There is exactly one hard, defensible, unbuilt kernel:

> **openEO / OGC API - Processes** can *define* a derived product — but serves it as a **batch**
> job. **TiTiler / `xpublish-tiles`** can *serve* low-latency dynamic tiles — but can't let a
> scientist define an **arbitrary** product. **Nobody compiles a data-scientist's process graph
> into a low-latency dynamic tile service backed by a cost-aware materialization cache.**

Building that bridge — wrapped in a single pane of glass and glass-box observability — is
Swath; everything else we compose, bind, or keep as a validation oracle (§8).

## 4. North-star metric

**Ingest-to-pixel latency:** seconds from *"a new granule arrives"* to *"a correct tile is
visible on the map"* — measurable, stopwatch-demoable, unifying every subsystem under one
number the glass box reports and the tests assert against.

## 5. Who it's for

Agencies and their contractors running EO data services (the NESDIS pattern first); startups
who glue eoAPI + TiTiler + custom code per project; the eoAPI/pgstac/TiTiler community (Swath
layers *on top of* their stack); and data scientists who want "an idea for a product" to become
"a hosted, tiled, shareable layer" without a DevOps project.

## 6. UX principles — two audiences, one system

The platform serves both a non-expert who wants a responsive map and a developer who needs to
*see the machine working* — the **glass-box** architecture: smooth default interaction with no
exposure to STAC/Zarr/projections/tiling, plus the **x-ray** overlay making the optimizations
visible per tile (the materialization decision, a cache heatmap, latency/bytes/chunks, the
planner's "why", the live ingest-to-pixel timer). The overlay is a **keystone feature**: the
advanced mode, the single best demo, and — critically — **the test oracle**; integration tests
assert against the same trace it renders.

## 7. The three pillars + a frontier

**Pillar 1 — Ground-segment ingest spine.** Event-driven: a granule lands and is automatically
processed, cataloged, tileable — no human in the loop; cloud-optimized data registers directly,
and decades of NetCDF/HDF/GRIB archives are absorbed **without a rewrite** via virtual
references. Measured by ingest-to-pixel latency.

**Pillar 2 — Data-scientist product loop (the wedge, productized).** Author a product as an
openEO graph; the **materialization engine** lowers it to Render IR and serves it live via
OGC API - Tiles, the **cost-aware planner** (the novel heart) choosing live / overview / cache
per layer × zoom under an explicit storage-vs-latency budget — showing its work in the x-ray
overlay.

**Pillar 3 — Single-pane control plane.** The "make STAC disappear" layer: manage **datasets**
and **layers**; Swath writes the pgstac catalog underneath. The public contract is the OGC API
family — a coherent single pane, instantly interoperable; format-plural by design (COG + Zarr +
GeoParquet, raster *and* vector) — a platform, not a raster viewer.

**Frontier — Geo-embeddings as a first-class product.** An embedding is *just another product
the DS loop can generate*: run a foundation model over incoming granules and the platform gains
semantic/similarity search over the same catalog. Architect for it now (a product type + a
vector index); ship it later.

## 8. Architecture — build vs. compose

The single most important discipline: build only the defensible core; stand on the shoulders
of everything else. The v0.1 draft planned "~5 things to build"; all five were built — the
build/adopt/bind/never table and shipped stack inventory are `ARCHITECTURE.md` §§3-4 and
§12.

**Demoted to test oracles:** the v0.1 draft composed TiTiler + rio-tiler, `xpublish-tiles`,
stac-fastapi, morecantile, and openEO reference tooling into the serving path; the pure-Rust
core (ADR 0002) outbuilt that list, and REQUIREMENTS §10 (A2) records the demotion. They relate
to Swath today as validation oracles: the test suite renders the same tiles and tile-matrix
math through them and pixel-diffs the results (`tests/oracle/`, `just oracle-verify`, the
committed goldens); VirtualiZarr remains the ingest-time conformance reference (ADR 0006).

**Prior art we align with rather than duplicate:** openEO (processing), pangeo-forge (ingest
recipes), eoAPI (catalog + serve) — each solves a slice; none fuses them. That fusion is ours,
measured: ingest-to-pixel is <!-- number:i2p-ms -->646 ms<!-- /number:i2p-ms --> end to end,
budget-enforced on every commit (`PERFORMANCE.md` §4).

## 9. Reference datasets

**Clean path — HLS** (already COG, real derived products like NDVI): the day-one benchmark.
**Legacy path — MODIS or VIIRS (HDF/NetCDF)**, virtualized to prove seamless legacy ingest.
Pairing the two demonstrates the full spectrum.

## 10. Milestones

_Phases as scope; delivery status lives in [`ROADMAP.md`](ROADMAP.md) §1._

**Phase 0 — Foundations**: charter, requirements, ADRs 0001-0007, the CI-mirrored
`just check` gate. **Phase 1 — MVP "granule → live tile"**: HLS ingest → NDVI on the fly →
OGC API - Tiles → viewer with STAC hidden; x-ray v0; the stopwatch demo asserted under budget.
**Phase 2 — the materialization brain + legacy ingest + DS authoring**: the cost-aware planner
(`docs/design/materialization-planner.md`), the legacy path on VIIRS (ADRs 0006/0008), openEO
authoring (ADR 0010), the x-ray provenance views. **Phase 3 — platform breadth + turnkey
deploy**: versioned images and the release pipeline (`docs/RELEASING.md`), OGC API breadth,
auth, multi-tenant, cloud deploy, the "adopt on top of your existing eoAPI" path. **Phase 4 —
frontier**: geo-embeddings with a vector index.

## 11. Open decisions

Decided since v0.1 (`docs/decisions/`): license (ADR 0003), the pure-Rust core (ADR 0002), the
legacy dataset (ADR 0004), the bounded openEO surface (ADR 0010), the extension mechanism
(ADR 0013 — WASM/RPC deferred, reopen condition in the ADR). Still open: the frontier's
embedding model — decide when Phase 4 nears.

## 12. Glossary

**Materialization** — the live-vs-overview-vs-cache decision for how a tile gets produced.
**Ingest-to-pixel** — the north-star metric: arrival of a granule → visible correct tile.
**Geo-embeddings** — learned vectors from a foundation model over imagery, enabling
semantic/similarity search. (STAC, COG, Zarr, GeoParquet, and the OGC APIs are the ecosystem's
standard vocabulary; their specs are the reference.)

## 13. References

eoAPI — https://eoapi.dev/ · TiTiler — https://developmentseed.org/titiler/ · `xpublish-tiles`
— https://github.com/earth-mover/xpublish-tiles · Icechunk — https://icechunk.io/ · GeoZarr —
https://geozarr.org/ · VirtualiZarr — https://virtualizarr.readthedocs.io/ · openEO as OGC
Community Standard — https://www.ogc.org/announcement/openeo-api-ogc-community-standard/ ·
NASA VEDA — https://developmentseed.org/projects/nasa-impact-veda/ · "Earth Embeddings as
Products" — https://arxiv.org/html/2601.13134v1
