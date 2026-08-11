# Swath — Project Charter

*Working document. v0.3 — August 2026. Reconciled with the shipped system and ADRs 0001-0013;
§8 and §10 are written in achieved tense with evidence links where the thing exists. Where this
charter and an ADR disagree, the ADR wins; the north-star requirements live in `REQUIREMENTS.md`
(scope changes are recorded as dated amendments in its §10).*

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
is Swath. Everything else we compose, bind, or — where building proved cheaper and stronger than
composing — keep as a validation oracle (§8).

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
- **Compile & serve** it through the **materialization engine**, which lowers that graph to Swath's
  Render IR and serves it live through the owned Rust tiler, exposed via **OGC API - Tiles**.
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
shoulders of everything else. The v0.1 draft planned "~5 things to build"; all five were built. The
full build/adopt/bind/never table, verified against the tree, is `ARCHITECTURE.md` §3.

**Built (the defensible core — the 5 things):**

1. The **process-graph → tile-ops compiler** — openEO process-graph JSON lowered to the Render IR and
   served live (`crates/swath-render` `process`/`ir`; authoring surface per ADR 0010, conformance-tested
   in `crates/swath-api/tests/openeo_conformance.rs`).
2. The **cost-aware materialization planner** — live vs. overview vs. cache per tile, explicit
   per-layer budgets, every considered candidate's cost estimate recorded in the trace
   (`crates/swath-core` `planner`; spec: `docs/design/materialization-planner.md`; property-tested,
   benchmarked in `PERFORMANCE.md` §5 — planning costs tens of nanoseconds).
3. The **single-pane control plane** — datasets/layers API + Web-Components UI; writes pgstac
   underneath; STAC hidden (`crates/swath-api`, `web/src`, `crates/adapters/swath-catalog-pgstac`;
   domain contract: `docs/design/catalog-domain.md`).
4. The **glass-box observability / x-ray harness** — the per-tile trace model (decision, bytes read,
   chunk/byte-range provenance, timings), its SSE stream, and the overlay; and it is the test oracle:
   the e2e harness asserts against the same trace the overlay renders (`crates/swath-core` `trace`,
   `crates/swath-e2e`; `ARCHITECTURE.md` §9).
5. The **legacy virtualization ingest orchestration** — event-driven filedrop ingest plus a pure-Rust
   virtual-reference generator (`crates/swath-referencer`; ADRs 0006/0008), conformance-gated against
   the VirtualiZarr sidecar it replaced (`just test-referencer`) and ~40× faster per invocation
   (`PERFORMANCE.md` §7).

**Composed (as built):** less than the v0.1 draft planned, and the claim is stronger for it —
**pgstac** (Postgres) for the catalog and **object storage** (`object_store`; MinIO in the local
stack) for tiles, plus adopted reader/encoder/runtime crates (`async-tiff`, `image`/`png`, `axum`,
`tokio`; `zarrs` when the native-Zarr source lands) and **MapLibre GL** in the frontend. Projection
math is **bound**, never rewritten: `proj4rs` (ADR 0002).

**Demoted to test oracles:** the v0.1 draft composed TiTiler + rio-tiler, `xpublish-tiles`,
stac-fastapi, morecantile, and openEO reference tooling into the serving path; the pure-Rust core
(ADR 0002) outbuilt that list, and REQUIREMENTS §10 (amendment A2) records the demotion.
TiTiler/rio-tiler — like GDAL and morecantile — relate to Swath today as validation oracles: the test
suite renders the same tiles and tile-matrix math through them and pixel-diffs the results
(`tests/oracle/`, `just oracle-verify`, the committed goldens). VirtualiZarr remains the ingest-time
conformance reference for the Rust referencer (ADR 0006), not a runtime component.

**Prior art we align with rather than duplicate:** openEO (processing), pangeo-forge (ingest/ETL recipes),
eoAPI (catalog + serve). Each solves a slice; none fuses ingest + a low-latency publish loop + cost-aware
serving + a single pane. That fusion is ours — and it is measured, not promised: ingest-to-pixel is
646 ms end to end, budget-enforced on every commit (`PERFORMANCE.md` §4).

**Stack** *(as shipped; ADRs 0002, 0005, 0006 — the original draft described a Python core and a
React/deck.gl frontend; superseded):*

- **Core:** **pure Rust, single static binary** (`tokio` + `axum`) — the tiler, materialization planner,
  process compiler, control plane, and trace model are owned Rust IP. Adopted reader/codec crates
  (`async-tiff`, `object_store`); bound projection math (`proj4rs`; PROJ C-bindings feature-gated for
  the long tail); GDAL/PROJ never reimplemented. (ADR 0002)
- **Ingest sidecars:** the staged Python→Rust plan of ADR 0006 completed — the production
  virtual-reference generator is Rust (`swath-referencer`); the thin **Python** (`uv` + `ruff`)
  VirtualiZarr sidecar is retained as its conformance reference.
- **Catalog / state:** pgstac (Postgres); object store for tiles; Icechunk/GeoZarr cubes are ahead
  (§10 Phase 3+).
- **Frontend:** TypeScript **Web Components** (no framework) + **MapLibre GL**; deck.gl deferred. (ADR 0005)
- **Deploy:** one-command local (`docker compose`) and a no-checkout GHCR demo one-liner exist;
  cloud IaC (Terraform / Helm) and the documented "layer onto an existing eoAPI/pgstac deployment"
  path are still ahead (§10 Phase 3).
- **Testing:** property-based tests (`proptest`) on the planner; pixel-diff goldens for rendered tiles
  vs the GDAL/rio-tiler oracle (`tests/oracle/`); the x-ray trace as the assertion layer
  (`crates/swath-e2e`). All running in CI today.

## 9. Reference datasets

- **Clean path — HLS (Harmonized Landsat Sentinel-2):** already COG, already in NASA CMR, `titiler-cmr`
  has a published HLS tiling + rendering-performance guide, and it ships real derived products (RGB
  composites *and* official NDVI/vegetation indices). Ideal day-one benchmark for the DS loop and the
  materialization engine.
- **Legacy path — MODIS or VIIRS (HDF/NetCDF):** virtualized via VirtualiZarr to prove seamless legacy
  ingest. (Pairing HLS + a legacy collection demonstrates the full ingest spectrum.)

## 10. Milestones

_Delivery status against these phases — shipped milestones, the canonical deferral inventory,
and the parked M7+ candidates — lives in [`ROADMAP.md`](ROADMAP.md)._

**Phase 0 — Foundations. Done.**
Charter, requirements, ADRs 0001-0007, build-vs-compose boundary, dev environment, the `just check`
gate mirrored by CI (`ENGINEERING.md`), testing harness. Evidence: `docs/decisions/`, `.github/workflows/ci.yml`.

**Phase 1 — MVP: "granule → live tile". Done — the stopwatch demo exists and is enforced.**
HLS ingest → true-color + NDVI derived on the fly → served via OGC API - Tiles → MapLibre viewer with
datasets/layers panels (STAC hidden). X-ray v0 shipped with the ingest-to-pixel timer and live/cache
indicator. The stopwatch number: **646 ms** arrival-to-correct-pixel, measured end to end through the
real stack and asserted under budget on every commit (`PERFORMANCE.md` §4; `crates/swath-e2e`). NDVI
is computed on the fly and pixel-verified against oracle goldens — not pre-baked.

**Phase 2 — The materialization brain + legacy ingest + DS authoring. Done.**
Cost-aware planner (live vs. overview vs. cache under explicit budgets) —
`docs/design/materialization-planner.md`, benchmarked in `PERFORMANCE.md` §5. Legacy path proven on
VIIRS (VNP09GA, ADR 0008) — and a step beyond the plan: virtualization by the pure-Rust referencer,
with VirtualiZarr demoted to conformance reference (ADR 0006, `PERFORMANCE.md` §7). openEO product
authoring compiled through the engine (ADR 0010; end-to-end from the authoring panel,
`web/e2e/authoring.e2e.ts`). The x-ray overlay carries the chunk/byte-range provenance and
planner decision-trace views (`ARCHITECTURE.md` §9).

**Phase 3 — Platform breadth + turnkey deploy. In progress.**
Shipped: versioned GHCR images with a CI-smoke-tested no-checkout docker one-liner, one-command local
stack, and the pre-release pipeline (`docs/RELEASING.md`). Ahead: OGC API breadth (EDR time-series,
Features/GeoParquet vector), auth (OIDC/RBAC), multi-tenant, cloud IaC deploy, and the documented
"adopt on top of your existing eoAPI" path.

**Phase 4 — Frontier: geo-embeddings. Not started.**
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
- ~~**Extension mechanism**~~ → **compile-time features/crates for adapters + openEO process graphs
  at runtime for products** (ADR 0013); WASM plug-ins and RPC sidecars deferred, not rejected — the
  reopen condition is recorded in the ADR. See `ARCHITECTURE.md` §14.

Still open:

- **Embedding model for the frontier:** Clay vs. Prithvi vs. AlphaEarth-style — decide when Phase 4 nears.

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
