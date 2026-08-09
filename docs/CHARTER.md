# Swath — Project Charter

*Working document. Draft v0.2 — August 2026. Reconciled with ADRs 0001-0006: Apache-2.0 license,
pure-Rust core, Web Components + MapLibre frontend, anchor datasets. Where this charter and an ADR
disagree, the ADR wins; the north-star requirements live in `REQUIREMENTS.md`.*

---

## 1. What Swath is, in one sentence

Swath is an open-source, cloud-native geospatial platform where **satellite data comes in and is
immediately available on a map**, and where data scientists can **tap the live data flow, derive new
products, and publish them the same way** — all from a single, intuitive pane of glass that hides the
plumbing (STAC, tilers, cube formats) behind a clean, standards-based experience.

## 2. Why this, why now

The cloud-native geospatial stack has crossed a threshold. The primitives are done and battle-tested:

- **Serving:** TiTiler / rio-tiler (COG), `xpublish-tiles` and titiler-multidim/xarray (Zarr/NetCDF cubes).
- **Cataloging:** stac-fastapi + pgstac; bundled by **eoAPI**.
- **Storage:** Icechunk 1.0 (versioned, transactional Zarr), emerging **GeoZarr** + multiscale overviews.
- **Legacy bridging:** VirtualiZarr + kerchunk (turn existing NetCDF/HDF/GRIB into virtual cubes, no rewrite).
- **Processing standard:** **openEO** — as of May 2026 an official OGC Community Standard — plus OGC API - Processes.

What does **not** exist is a *product* that fuses these into one managed, low-latency, observable loop.
Today, standing up "data in → live on a map, with a place for scientists to build new products" means
hand-wiring eoAPI + a tiler + custom ingest + a bespoke UI, per deployment. NASA's VEDA and Development
Seed's eoAPI prove the pieces; they are building blocks and a NASA-specific dashboard, not a turnkey,
self-hostable platform with a control plane, a materialization brain, and a data-scientist publish loop.

That gap is the opportunity, and it was independently identified from two directions — the technical
survey below, and a practitioner (a founder building EO products for government) who put it plainly:
*"a way for data scientists to tap into that flow of data to generate new products with low latency,
and then host those products the same way — that's a place where we could build something. It hasn't
been addressed by anyone as far as I can see."*

## 3. The core thesis (the wedge)

There is exactly one hard, defensible, unbuilt kernel at the center of this, and it's worth stating precisely:

> **openEO / OGC API - Processes** can *define* a derived product (NDVI, a false-color composite, a
> reprojection) — but serves it as a **batch** job.
> **TiTiler / `xpublish-tiles`** can *serve* a raster or cube as **low-latency dynamic tiles** — but
> can't let a scientist define an **arbitrary** product.
> **Nobody compiles a data-scientist's process graph into a low-latency dynamic tile service backed by
> a cost-aware materialization cache.**

Building that bridge — and wrapping it in a single pane of glass and a glass-box observability layer —
is Swath. Everything else, we compose.

## 4. North-star metric

**Ingest-to-pixel latency:** seconds from *"a new granule arrives"* to *"a correct tile is visible on the map."*

It is measurable, demoable with a stopwatch, and it unifies every subsystem — ingest, catalog,
materialization, serving — under one number. The glass-box observability layer reports it continuously,
and our tests assert against it.

## 5. Who it's for

- **Agencies and their contractors** running EO data services (NOAA / NESDIS, NASA, and the startups who
  build derived products on their data). The first named use case is the weather-service / NESDIS pattern.
- **EO / geospatial startups** who currently glue eoAPI + TiTiler + custom code per project.
- **The existing eoAPI / pgstac / TiTiler community** — Swath is designed to layer *on top of* the exact
  stack they already run, so adoption is incremental, not a migration.
- **Data scientists** who want to turn "an idea for a product" into "a hosted, tiled, shareable layer"
  without a DevOps project each time.

## 6. UX principles — two audiences, one system

The design tension is that the platform serves both a non-expert who just wants a beautiful, responsive
map, and a developer who needs to *see the machine working*. Swath resolves this with a **glass-box**
architecture:

- **Default (everyone):** smooth, obvious interaction — pan, zoom, animate a time series, toggle a derived
  product — with no exposure to STAC, Zarr, projections, or tiling. It is always obvious *what* is
  happening on screen; it is never necessary to know *how*.
- **Advanced / "x-ray" mode (developers & power users):** a DevTools-style overlay that makes the
  optimizations visible and verifiable, per tile:
  - the materialization decision — served **live** from the cube, from a **pre-computed overview**, or a **cache hit**;
  - a cache hit/miss heatmap painted on the map as you pan;
  - latency, bytes fetched, which Zarr **chunks** or COG **byte-ranges** were read;
  - a "why did it decide that?" trace from the planner, and the live **ingest-to-pixel** timer.

This overlay does triple duty and is treated as a **keystone feature**, not a nicety: it's the advanced
mode, it's the single best demo of the whole project, and — critically — **it is the test oracle.**
Integration tests assert against the same trace the overlay renders ("this tile at z3 must come from an
overview, not live"), so correctness and observability are one surface.

Engineering values: modern, typed, well-documented, well-tested, well-organized; bleeding-edge but
*measured* (no premature optimization, profile before reaching for Rust); standards-native wherever a
standard exists, so we interoperate instead of reinventing.

## 7. The three pillars + a frontier

### Pillar 1 — Ground-segment ingest spine

Event-driven ingest that behaves like a modern ground segment: a granule lands (an object-store event,
a new CMR record, a downlink drop), and the platform automatically processes, catalogs, and makes it
tileable — no human in the loop. Two ingest paths, deliberately:

- **Clean / modern path:** already-cloud-optimized data (COG, GeoZarr) is registered directly.
- **Legacy path:** decades of NetCDF / HDF / GRIB archives are absorbed **without a rewrite**, via
  **VirtualiZarr + Icechunk virtual references** — the old files stay in place, byte-range-referenced, and
  the engine sees a clean cloud-native cube. This is the "seamlessly ingest legacy file-based approaches
  into a modern architecture" requirement, done with real, current tooling.

Measured by ingest-to-pixel latency.

### Pillar 2 — Data-scientist product loop (the wedge, productized)

The surface where a scientist taps the live flow and publishes a new product:

- **Author** a product as an **openEO / OGC API - Processes** graph (NDVI, false-color, reprojection,
  band math) using tooling they may already know — not a bespoke DSL.
- **Compile & serve** it through the **materialization engine**, which turns that graph into on-the-fly
  tile operations over TiTiler / `xpublish-tiles` and exposes it via **OGC API - Tiles / Maps**.
- **Cost-aware materialization planner** (the novel heart): for each layer × zoom, estimate the on-the-fly
  cost and choose a strategy — serve **live**, pre-compute a **GeoZarr overview**, or **cache** tiles —
  under an explicit storage-vs-latency budget. This is the direct, systematized answer to the practitioner's
  own tension: *"some products you can do on the fly, some you must cache, but caching defeats the storage
  savings."* Swath makes that decision, per layer, and shows its work in the x-ray overlay.

### Pillar 3 — Single-pane control plane

The "make STAC disappear" layer. You manage **datasets** and **layers**; Swath writes the pgstac catalog
underneath. Its public contract is the **OGC API family**, which is what makes it both a coherent single
pane *and* instantly interoperable (QGIS, existing clients):

- **Records** — catalog/discovery
- **Tiles / Maps** — raster & derived-product serving
- **EDR** — point / time-series extraction straight out of the cube (key for weather/temporal data)
- **Features** — vector data (GeoParquet: fire perimeters, detections, footprints)
- **Processes** — the derivation/product authoring above

Format-plural by design: **COG + Zarr + GeoParquet**, raster *and* vector — a platform, not a raster viewer.

### Frontier — Geo-embeddings as a first-class product

The pillar that puts Swath a generation ahead of classic viz platforms. An embedding is *just another
product the DS loop can generate*: run a geospatial foundation model (Clay, Prithvi, AlphaEarth-style)
over incoming granules, store the embeddings (GeoParquet/Zarr), and the platform gains **semantic /
similarity search** ("find scenes that look like this") and ML-ready features over the same catalog.
The recent literature already frames "Earth embeddings as products," which is exactly the treatment here.
Architect for it now (embeddings = a product type + a vector index); ship it in a later phase.

## 8. Architecture — build vs. compose

The single most important discipline for finishing this: build only the defensible core; stand on the
shoulders of everything else.

**Build (the defensible core — ~5 things):**

1. The **process-graph → tile-ops compiler** (openEO/Processes graph → live tiling operations).
2. The **cost-aware materialization planner** (live vs. overview vs. cache, per layer × zoom, under budget).
3. The **single-pane control plane** (datasets/layers API + UI; writes pgstac underneath; hides STAC).
4. The **glass-box observability / x-ray harness** (also the test oracle).
5. The **legacy virtualization ingest orchestration** (VirtualiZarr/Icechunk pipelines, event-driven).

**Compose (never rebuild):** TiTiler + rio-tiler, `xpublish-tiles`, stac-fastapi + pgstac, VirtualiZarr +
kerchunk, Icechunk, morecantile, MapLibre GL / deck.gl, and — for the DS loop's standards surface —
openEO reference tooling / an OGC API - Processes engine.

**Prior art we align with rather than duplicate:** openEO (processing), pangeo-forge (ingest/ETL recipes),
eoAPI (catalog + serve). Each solves a slice; none fuses ingest + a low-latency publish loop + cost-aware
serving + a single pane. That fusion is ours.

**Stack** *(revised per ADRs 0002, 0005, 0006 — the original draft described a Python core and a
React/deck.gl frontend; superseded):*

- **Core:** **pure Rust, single static binary** (`tokio` + `axum`) — the tiler, materialization planner,
  process compiler, control plane, and trace model are owned Rust IP. Adopt Rust reader/codec crates
  (`async-geotiff`, `zarrs`, `object_store`); bind projection math (`proj4rs`, PROJ for the long tail);
  never reimplement GDAL/PROJ. (ADR 0002)
- **Ingest sidecars:** thin **Python** (`uv` + `ruff`) only for legacy virtual-reference *generation*
  (VirtualiZarr), staged toward Rust behind one manifest port. (ADR 0006)
- **Catalog / state:** pgstac (Postgres); object store + Icechunk for cubes.
- **Frontend:** TypeScript **Web Components** (no framework) + **MapLibre GL**; deck.gl deferred. (ADR 0005)
- **Deploy:** one-command local (`docker compose`) and IaC (Terraform / Helm) for cloud — the "stand up a
  real production instance" piece. Designed to layer onto an existing eoAPI/pgstac deployment.
- **Testing:** property-based tests (`proptest`) for the planner; perceptual-diff snapshots for rendered
  tiles vs a GDAL oracle; the x-ray trace as the assertion layer.

## 9. Reference datasets

- **Clean path — HLS (Harmonized Landsat Sentinel-2):** already COG, already in NASA CMR, `titiler-cmr`
  has a published HLS tiling + rendering-performance guide, and it ships real derived products (RGB
  composites *and* official NDVI/vegetation indices). Ideal day-one benchmark for the DS loop and the
  materialization engine.
- **Legacy path — MODIS or VIIRS (HDF/NetCDF):** virtualized via VirtualiZarr to prove seamless legacy
  ingest. (Pairing HLS + a legacy collection demonstrates the full ingest spectrum.)

## 10. Milestones

**Phase 0 — Foundations (this repo).**
Charter, architecture decisions, build-vs-compose boundary, dev environment, CI, testing harness skeleton.

**Phase 1 — MVP: "granule → live tile" (prove the north star).**
HLS ingest → true-color + NDVI derived on the fly → served via OGC API - Tiles → minimal MapLibre viewer.
Single pane lists *datasets/layers* (STAC hidden). Ship **x-ray v0** with the ingest-to-pixel timer and a
live/cache indicator. Success = a stopwatch demo of arrival-to-pixel, and NDVI computed on the fly, not pre-baked.

**Phase 2 — The materialization brain + legacy ingest + DS authoring.**
Cost-aware planner (live vs. overview vs. cache) with the storage-vs-latency budget. Legacy path via
VirtualiZarr (MODIS/VIIRS). openEO / OGC API - Processes product authoring compiled through the engine.
x-ray overlay grows the chunk/byte-range and decision-trace views.

**Phase 3 — Platform breadth + turnkey deploy.**
OGC API breadth (EDR time-series, Features/GeoParquet vector). Control-plane polish, auth (OIDC/RBAC),
multi-tenant. One-command IaC deploy; documented "adopt on top of your existing eoAPI" path.

**Phase 4 — Frontier: geo-embeddings.**
Embeddings as a first-class product type; a vector index; semantic/similarity search over the catalog.

## 11. Positioning & monetization

**Open-core.** A permissive, genuinely useful, self-hostable core (the platform, the engine, the control
plane, the x-ray tooling) that earns adoption and stars on its own. A commercial layer on top for teams
who don't want to run it themselves or need enterprise/government features:

- **Managed / hosted Swath** (the single pane + materialization SLAs, run for you).
- **Enterprise features:** SSO/SAML, fine-grained RBAC, audit, multi-tenant governance.
- **Government readiness:** compliance posture (FedRAMP-style controls), air-gapped/on-prem support.
- **Premium connectors & support:** priority data-source integrations, SLAs, professional services.

This mirrors proven models in the space — Development Seed's services around eoAPI, and Earthmover's
Arraylake around open-source Icechunk — but Swath's commercial wedge is distinct: the **managed single
pane + the materialization guarantees**, which is precisely the part that is hardest to operate well.

## 12. How this complements the founder conversation

This charter is framed as an independent open-source project, but it is deliberately the productized
platform layer that an EO-products government contractor most needs. Being the maintainer of the OSS core
makes its author the technical center of gravity for exactly that problem — more valuable than a second
pair of hands. And because Swath layers onto an existing pgstac/TiTiler deployment (e.g. a fire portal
already using that stack), it's adoptable there with near-zero migration cost. It is a way to *exceed*
the "just orchestrate what exists" framing by owning the two things that framing misses: making the loop
**standards-native** (openEO/OGC) and **embedding-aware**, plus the cost-aware materialization brain that
turns "assemble the tilers" into an actual product.

## 13. Open decisions

Decided since v0.1 (see `docs/decisions/`):

- ~~**License**~~ → **Apache-2.0 with DCO** (ADR 0003).
- ~~**Rust in the hot path**~~ → superseded by the **pure-Rust core** decision (ADR 0002); the question
  is no longer "when to add Rust" but "what to adopt vs build vs bind."
- ~~**Legacy dataset for the Phase-2 proof**~~ → **VIIRS primary, MODIS stretch** (ADR 0004).
- ~~**openEO surface**~~ → **native openEO API at a bounded profile** (ADR 0010): capabilities,
  collections, processes, and XYZ secondary services over the process compiler — real openEO
  clients author against Swath; jobs/batch/auth deferred until demanded.

Still open:

- **Embedding model for the frontier:** Clay vs. Prithvi vs. AlphaEarth-style — decide when Phase 4 nears.
- **Extension mechanism** beyond compile-time features (WASM plug-ins vs sidecar RPC) — see
  `ARCHITECTURE.md` §14/§16.

## 14. Glossary

- **STAC** — SpatioTemporal Asset Catalog; the standard for describing geospatial assets. Swath hides it.
- **COG** — Cloud-Optimized GeoTIFF; a 2D raster whose header lets a tiler byte-range-fetch just the tile it needs.
- **Zarr / cube** — chunked, cloud-native n-dimensional array storage (the "cubic data" source products derive from).
- **GeoParquet** — cloud-native columnar format for vector data.
- **OGC APIs** — modern web-API standards: Records, Tiles, Maps, EDR, Features, Processes.
- **Dynamic tiler** — serves map tiles on demand from source data rather than from a pre-rendered pyramid.
- **Materialization** — the live-vs-overview-vs-cache decision for how a given tile gets produced.
- **Ingest-to-pixel** — Swath's north-star metric: arrival of a granule → visible correct tile.
- **Geo-embeddings** — learned vectors from a foundation model over imagery; enable semantic/similarity search.

## 15. References

- eoAPI — https://eoapi.dev/ · https://developmentseed.org/projects/eoapi/
- TiTiler — https://developmentseed.org/titiler/ · titiler-cmr HLS guide — https://developmentseed.org/titiler-cmr/dev/datasets/hls_tiling/
- Earthmover `xpublish-tiles` — https://github.com/earth-mover/xpublish-tiles · blog — https://www.earthmover.io/blog/dynamic-map-tile-rendering-icechunk-zarr-data-xpublish-tiles
- Icechunk — https://icechunk.io/ · GeoZarr — https://geozarr.org/
- VirtualiZarr — https://virtualizarr.readthedocs.io/
- openEO as OGC Community Standard — https://www.ogc.org/announcement/openeo-api-ogc-community-standard/
- NASA VEDA — https://developmentseed.org/projects/nasa-impact-veda/
- "Earth Embeddings as Products" — https://arxiv.org/html/2601.13134v1
