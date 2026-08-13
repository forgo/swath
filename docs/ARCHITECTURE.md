# Swath — Architecture

_Working document. Draft v0.3 — August 2026. §§4, 6, 7 and 12 now describe the code as built and
carry a "last verified against" commit; the remaining sections are design intent. The charter (v0.2)
has been reconciled with the ADRs; where any doc disagrees with an ADR, the ADR wins. Engineering
standards (toolchains, CI, testing, release) live in `ENGINEERING.md`._

---

## 1. Purpose

Pin the shape of Swath before any code: the boundary between what we **build** and what we **adopt/bind**,
the **ports** (stable interfaces) the core depends on, the **adapters** behind them, the **standards** we
expose, and the **data flows** that produce the north-star metric (ingest-to-pixel latency). If this doc
is right, the scaffold is a transcription of it.

## 2. Design principles (locked)

1. **Hexagonal / ports-and-adapters.** A small, portable core depends only on narrow port traits and
   standard data types. Everything external is an adapter. Ecosystem churn is absorbed at adapters; the
   core never breaks because it only knows _standards_, not tools.
2. **Standards as interface contracts.** Ports are shaped like STAC, the OGC API family, and the
   openEO/OGC-Processes graph. Standards-as-interfaces _is_ the anti-lock-in mechanism.
3. **Pure-Rust core, single static binary.** The tiler + materialization engine + control-plane logic are
   Rust IP. Adopt Rust reader/codec crates; bind projection math; never reimplement PROJ or format drivers.
4. **Glass-box by construction.** Every render emits a structured decision **Trace**. The Trace powers the
   x-ray UI _and_ is the test oracle. Observability and correctness are one surface.
5. **Intuitive out of the box; resilient to extension.** One command, sane defaults. Extensions plug in at
   the ports with a small, well-documented surface — never by editing the core.
6. **Priorities, in order:** correctness → performance/memory → UX/intuitiveness → safety → docs →
   standards conformance breadth. (Breadth is last: we go deep on a vertical before wide.)

## 3. The build / adopt / bind / never boundary

| Layer                                                                                                            | Decision                             | Concretely                                                                            |
| ---------------------------------------------------------------------------------------------------------------- | ------------------------------------ | ------------------------------------------------------------------------------------- |
| Tiler brain (window/overview selection, warp+resample kernels, pixel ops, tile API, **per-tile decision hooks**) | **BUILD**                            | `swath-render`, `swath-core`                                                          |
| Materialization planner, catalog/ingest orchestration, trace model; process compiler + Render IR                 | **BUILD**                            | `swath-core` (planner, catalog, ingest, trace); `swath-render` (compiler + IR)        |
| COG / Zarr / virtual-reference reading                                                                           | **ADOPT**                            | `async-geotiff`/`async-tiff`, `zarrs` (+`zarrs_icechunk`), `object_store`             |
| Image encoding, HTTP, async runtime, vector/columnar                                                             | **ADOPT**                            | `image`/`png`/`webp`, `axum`, `tokio`, `geoarrow-rs`                                  |
| Projection / datum math                                                                                          | **BIND** (prefer pure-Rust)          | `proj4rs` (common CRS); `proj` C-bindings feature-gated for the long tail             |
| Legacy virtual-reference _generation_ (NetCDF/HDF → virtual Zarr)                                                | **ADOPT (Python, ingest-time only)** | `VirtualiZarr`/`kerchunk` as an ingest sidecar; Rust reads the manifest at serve time |
| Projection/datum catalog, universal format drivers, general GDAL warp                                            | **NEVER reimplement**                | (GDAL/rio-tiler live only in the test suite as a correctness oracle)                  |

## 4. Component model

Every node below names a real crate or module (crate in the subgraph title, module or component in
parentheses on the node). Nothing here is aspirational: deferred surfaces (Maps, Records, Processes,
EDR, Features, embeddings) appear only in the §7 phase tables and in the standards map
([`docs/media/standards-map.svg`](media/standards-map.svg), evidence ledger in
[`standards-map.notes.md`](media/standards-map.notes.md)).

```mermaid
flowchart TB
  subgraph FE["Frontend — web/src (Web Components + MapLibre GL, no framework)"]
    MAP["Map viewer (swath-map)"]
    PANELS["Dataset / layer / authoring panels (swath-dataset-panel, swath-layer-panel, swath-authoring-panel)"]
    XRAY["X-ray overlay + analytics (swath-xray, xray-analytics)"]
  end

  subgraph IN["Inbound adapter — swath-api (axum)"]
    TILES["OGC API - Tiles (routes)"]
    OEO["openEO authoring surface (openeo)"]
    CP["Control plane: datasets/granules + Trace SSE (granules, traces)"]
    UIA["Embedded UI assets (ui)"]
  end

  subgraph RENDER["swath-render — tiler engine"]
    TILER["render_tile / render_tile_cached (tiler)"]
    WARP["Warp/resample kernels (warp, window, grid)"]
    COMP["Process compiler: openEO graph → Render IR (process)"]
    IRX["Render IR + evaluator (ir)"]
    ENC["Tile encoder (encode, colormaps)"]
  end

  subgraph CORE["swath-core — pure domain, no I/O"]
    PLAN["Materialization planner (planner)"]
    CATD["Catalog domain + STAC converters (catalog)"]
    ING["Ingest registration step (ingest)"]
    MANI["Virtual-manifest schema v1 (manifest)"]
    TMS["Tile / TMS / raster math (tile, crs, raster)"]
    TRACE[("Trace model (trace)")]
  end

  subgraph PORTS["Ports — traits in swath-core"]
    P_SRC[["RasterSource (source)"]]
    P_RPJ[["Reproject (reproject)"]]
    P_CAT[["Catalog (catalog)"]]
    P_CACHE[["TileCache (cache)"]]
    P_EVT[["EventSource (events)"]]
    P_REF[["IngestReferencer (ingest)"]]
  end

  subgraph ADS["Adapter crates — crates/adapters/* + swath-referencer"]
    A_COG["swath-source-cog"]
    A_VIRT["swath-source-virtual"]
    A_PYR["swath-pyramid-objectstore"]
    A_PROJ["swath-reproject-proj4rs"]
    A_PG["swath-catalog-pgstac"]
    A_OS["swath-cache-objectstore"]
    A_EVT["swath-events-filedrop"]
    A_REF["swath-referencer"]
  end

  CLI["swath-cli — the swath binary: wires adapters, serve + filedrop ingest loop"]
  EXT[("External: object storage, Postgres/pgstac, granule files")]

  FE --> IN
  IN --> RENDER
  IN --> CORE
  RENDER --> CORE
  RENDER --> PORTS
  CORE --> PORTS
  PORTS --> ADS
  ADS --> EXT
  CLI -. wires adapters into .-> IN
  TRACE -. streamed over SSE .-> XRAY
```

All nodes are implemented (no planned/phantom nodes remain, so no implemented-vs-planned styling is
needed). The Python `VirtualiZarr` sidecar (`python/sidecars/referencer`) is deliberately absent: it
is the conformance *reference* for `swath-referencer` (ADR 0006), not a runtime component.

_Last verified against `576324d`._

## 5. The Core (pure logic)

- **Tiler engine** (`swath-render`, orchestrated from `swath-core`): given a `ResolvedLayer`, a `TileCoord`,
  and a `RenderSpec`, produce an encoded tile + a `Trace`. Reads pixels via `RasterSource`, transforms via
  `Reproject`, applies the compiled render IR (resample → band math → composite → colormap → encode).
- **Materialization planner**: chooses, per `(layer, tile/zoom)`, one of `CacheHit | Overview | Live` under a
  per-layer storage-vs-latency **Budget**, consulting availability (cache, overviews). Emits its reasoning
  into the Trace. This is the systematized "on-the-fly vs cache" decision.
- **Process compiler → Render IR**: parses a standard process graph (openEO/OGC-Processes JSON) and lowers
  the supported subset into a typed, executable `RenderPlan` the tiler runs. The graph is _interchange_;
  the IR is _ours_.
- **Catalog service**: the "make STAC disappear" domain. Models `Dataset` and `Layer`; persists via the
  `Catalog` port (STAC-shaped) so users never touch STAC JSON.
- **Ingest orchestrator**: reacts to `EventSource`, registers clean assets or triggers legacy virtualization,
  upserts the catalog, optionally warms overviews. Owns the ingest-to-pixel timer.
- **Trace model**: a first-class structured record (see §9) returned with every render and streamed to the UI.

## 6. Ports — trait signatures (verbatim from source)

The signatures below are copied verbatim from the named files (doc comments and method bodies
elided); the rustdoc on each trait and its module is the normative contract — including the
design points that superseded the v0.1 sketches (async-in-trait shape and dyn-compatibility per
each port's module docs; the domain-shaped `Catalog` per
[`docs/design/catalog-domain.md`](design/catalog-domain.md), §16.5; the pull-shaped
`EventSource`; no `ProcessRegistry` port, ADR 0010; the `IngestReferencer` port, ADR 0006).

```rust
// crates/swath-core/src/source.rs
pub trait RasterSource: Send + Sync {
    fn describe(
        &self,
        asset: &AssetRef,
    ) -> impl Future<Output = Result<RasterInfo, SourceError>> + Send;

    fn read_window(
        &self,
        asset: &AssetRef,
        window: WindowRequest,
        band: BandSelection,
        level: ReadLevel,
    ) -> impl Future<Output = Result<WindowData, SourceError>> + Send;
}

// crates/swath-core/src/reproject.rs
pub trait CoordTransform: Send + Sync {
    fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError>;

    fn transform_slice(&self, points: &mut [(f64, f64)]) -> Result<(), ReprojectError> { /* default: per-point loop */ }
}

pub trait Reproject: Send + Sync {
    fn transformer(&self, from: &Crs, to: &Crs) -> Result<Box<dyn CoordTransform>, ReprojectError>;
}

// crates/swath-core/src/catalog.rs
pub trait Catalog: Send + Sync {
    fn upsert_dataset(
        &self,
        dataset: &Dataset,
    ) -> impl Future<Output = Result<(), CatalogError>> + Send;

    fn upsert_granules(
        &self,
        granules: &[Granule],
    ) -> impl Future<Output = Result<(), CatalogError>> + Send;

    fn get_dataset(
        &self,
        id: &DatasetId,
    ) -> impl Future<Output = Result<Option<Dataset>, CatalogError>> + Send;

    fn list_datasets(&self) -> impl Future<Output = Result<Vec<Dataset>, CatalogError>> + Send;

    fn find_granules(
        &self,
        dataset: &DatasetId,
        query: &GranuleQuery,
    ) -> impl Future<Output = Result<Vec<Granule>, CatalogError>> + Send;
}

// crates/swath-core/src/cache.rs — no TTL by design: content-derived keys
// never go stale (§10, §16.3)
pub trait TileCache: Send + Sync {
    fn get(
        &self,
        key: &TileKey,
    ) -> impl Future<Output = Result<Option<CachedTile>, CacheError>> + Send;

    fn put(
        &self,
        key: &TileKey,
        bytes: &[u8],
        content_type: &str,
    ) -> impl Future<Output = Result<(), CacheError>> + Send;
}

// crates/swath-core/src/events.rs
pub trait EventSource: Send {
    fn next_event(
        &mut self,
    ) -> impl Future<Output = Result<Option<GranuleEvent>, EventError>> + Send;
}

// crates/swath-core/src/ingest.rs
pub trait IngestReferencer: Send + Sync {
    fn handles(&self, granule: &Path) -> bool;

    fn generate(&self, granule: &Path) -> Result<VirtualManifest, ReferencerError>;
}
```

Core entry points (not ports — this is the logic itself; same files are normative):

```rust
// crates/swath-core/src/planner.rs — PlanChoice is
// CacheHit | Overview { factor } | Live | Refuse { .. } (#[non_exhaustive])
pub fn plan(budget: &Budget, availability: &Availability) -> Plan;

// crates/swath-render/src/process.rs (Json = serde_json::Value)
pub fn compile(graph: &Json, ctx: &CompileContext) -> Result<CompiledProduct, CompileError>;

// crates/swath-render/src/tiler.rs — free functions generic over the ports,
// not a Tiler struct; the cached variant owns the probe + write-through
pub async fn render_tile<S: RasterSource, R: Reproject + ?Sized>(
    source: &S,
    reproject: &R,
    request: &TileRequest,
) -> Result<(EncodedTile, Trace), TileError>;

pub async fn render_tile_cached<S, R, C>(
    source: &S,
    reproject: &R,
    cache: &C,
    key: &TileKey,
    request: &TileRequest,
) -> Result<(EncodedTile, Trace), TileError>
where
    S: RasterSource,
    R: Reproject + ?Sized,
    C: TileCache;
```

_Last verified against `6b83794`._

## 7. Adapters and inbound APIs

**Adapters (outbound, behind ports):**

| Port                           | Implemented adapter (crate)                                                                                     | Planned adapters                                  |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| `RasterSource`                 | `swath-source-cog` (COG/HLS over `object_store`); `swath-source-virtual` (virtual-reference manifests); `swath-pyramid-objectstore` (materialized-pyramid overlay over either) | `zarrs` (native Zarr stores)                      |
| `Reproject`                    | `swath-reproject-proj4rs` (pure Rust)                                                                            | `proj` C-bindings (geostationary/exotic)          |
| `Catalog`                      | `swath-catalog-pgstac` (Postgres + pgstac)                                                                       | —                                                 |
| `TileCache`                    | `swath-cache-objectstore` (local/S3)                                                                             | Redis hot-tile cache                              |
| `EventSource`                  | `swath-events-filedrop` (watched drop directory)                                                                 | S3 notifications, CMR polling                     |
| `IngestReferencer`             | `swath-referencer` (pure Rust: HDF-EOS, GRIB2; Python `VirtualiZarr` sidecar as conformance reference, ADR 0006) | HDF5/NetCDF4 breadth                              |
| `EmbeddingModel`/`VectorIndex` | — (frontier; no port trait defined yet)                                                                          | Clay/Prithvi/AlphaEarth + vector index            |

There is no `ProcessRegistry` port (see §6): the openEO process subset is compiled in-core against
a `CompileContext` (ADR 0010). An external OGC Processes backend for batch materialization remains
a possible later seam and would get its own port when it lands.

**Inbound APIs (standards), by phase** (implementation status per the standards map,
[`docs/media/standards-map.svg`](media/standards-map.svg) /
[`standards-map.notes.md`](media/standards-map.notes.md)):

| API                                     | Purpose                             | Target phase | Status                                          |
| --------------------------------------- | ----------------------------------- | ------------ | ----------------------------------------------- |
| OGC API - Tiles                         | raster + derived-product tiles      | 1            | implemented (core, tileset, tilesets-list, dataset-tilesets, png) |
| Control-plane REST + Trace SSE          | datasets/layers mgmt + x-ray stream | 1            | implemented                                     |
| openEO (bounded authoring profile)      | product authoring (ADR 0010); preview `POST /result` (ADR 0014) | 1            | implemented                                     |
| OGC API - Maps                          | styled map imagery                  | deferred     | not implemented — an earlier draft paired it with Tiles at phase 1, but no endpoints, conformance classes, or tests exist; the standards map records the final call |
| OGC API - Records                       | catalog/discovery                   | 2            | not started                                     |
| OGC API - Processes                     | batch/externalized processing       | 2            | not started (authoring is openEO-only, ADR 0010) |
| OGC API - EDR                           | point/time-series from cubes        | 3            | not started                                     |
| OGC API - Features                      | vector/GeoParquet                   | 3            | not started                                     |

_Last verified against `6b83794`._

## 8. Data flows

**Tile-serve hot path (the materialization decision):**

```mermaid
flowchart LR
  REQ[GET tile z/x/y - layer] --> API[Tiles API]
  API --> RES[Resolve layer + RenderSpec]
  RES --> PLAN{Planner: strategy?}
  PLAN -- CacheHit --> C[(TileCache)] --> OUT[Encoded tile + Trace]
  PLAN -- Overview --> RD1[RasterSource: overview window]
  PLAN -- Live --> RD2[RasterSource: full-res window]
  RD1 --> WARP[Reproject + resample]
  RD2 --> WARP
  WARP --> PIX[Render IR: band math -> composite -> colormap]
  PIX --> ENC[Encode PNG/WebP]
  ENC --> WT[[write-through cache?]] --> OUT
```

**Ingest-to-pixel (north star):**

```mermaid
flowchart LR
  EV[Granule event] --> ING[Ingest orchestrator]
  ING -->|clean COG/Zarr| REG[Register asset]
  ING -->|legacy NetCDF/HDF| VIRT[VirtualiZarr sidecar -> virtual manifest] --> REG
  REG --> CAT[Catalog upsert - pgstac]
  CAT --> WARM[[optional overview warm]]
  WARM --> LIVE[Layer servable]
  EV -. timer .-> LIVE
```

**Data-scientist product publish:**

```mermaid
flowchart LR
  DS[Scientist authors openEO graph] --> PAPI[Processes/openEO API]
  PAPI --> COMP[Compile graph -> Render IR]
  COMP --> LREG[Register derived Layer]
  LREG --> SERVE[Served via the same tile path; planner decides materialization]
```

## 9. Trace / observability model (the x-ray keystone)

Every `render_tile` returns a `Trace` and streams it over SSE. Shape (illustrative):

```rust
pub struct Trace {
    pub decision: Strategy,            // Live | Overview{level} | CacheHit
    pub source: AssetRef,
    pub crs_from: Crs, pub crs_to: Crs,
    pub bytes_read: u64,
    pub chunks_or_ranges: Vec<Provenance>, // Zarr chunks or COG byte-ranges touched
    pub timings_ms: Timings,           // read, warp, resample, pixel_ops, encode
    pub cache_key: TileKey,
    pub ingest_to_pixel_ms: Option<u64>, // when this tile reflects a just-ingested granule
}
```

Uses: the overlay paints per-tile decisions + a cache-hit heatmap and shows timings/bytes; the test suite
asserts against the same struct ("z3 must be `Overview`, not `Live`"); a perceptual-diff test compares the
encoded tile to a GDAL-rendered reference.

## 10. Materialization & cache model

- **Cache key** = hash of `(layer_version, render_spec, tile_coord, tms)`. A `layer_version` bump (new data
  or new graph) invalidates cleanly — no scattered invalidation.
- **Overview/artifact store** (shipped, issue #183): per-asset GeoZarr-shaped pyramids (plain Zarr v2
  over `object_store`, `pyramids/` under the store root), produced by `swath materialize` — batch,
  idempotent, resumable (`crates/adapters/swath-pyramid-objectstore`). The `PyramidSource` overlay
  merges materialized factors into `describe` and serves them from stored chunks, so the planner
  prefers an overview at low zoom without any planner change; COG-embedded overviews keep serving
  from the asset itself and are never duplicated.
- **Budget**: per-layer policy trading storage vs latency; the planner's cost estimate (bytes × warp cost)
  vs. overview/cache availability decides `Live | Overview | CacheHit`. Every choice is traced.

## 11. Runtime & concurrency

`tokio` + `axum`. Async I/O via `object_store`; CPU-bound warp/resample runs **inline on the async
runtime** (the v0.1 `rayon`/`spawn_blocking` sketch is superseded —
[ADR 0012](decisions/0012-render-stays-inline-async.md)). Cancellation propagates from dropped requests down to in-flight
reads. Single process; horizontal scale by running N stateless instances behind a load balancer (state lives
in Postgres + object store + optional Redis).

## 12. Crate / repo layout (as built)

```
swath/                          # Cargo workspace
  crates/
    swath-core/                 # domain types, port traits, planner, tile/TMS math, manifest schema, Trace — no I/O
    swath-render/               # warp/resample kernels, Render IR + evaluator, openEO process compiler, tiler, encoding
    swath-api/                  # inbound axum surface: OGC API - Tiles, openEO authoring, control plane + Trace SSE, embedded UI
    swath-cli/                  # the `swath` binary: `swath serve` (catalog mode runs the filedrop ingest loop) / `swath ingest`
    swath-referencer/           # pure-Rust virtual-reference generator (`IngestReferencer` impl: HDF-EOS, GRIB2; ADR 0006)
    swath-e2e/                  # end-to-end assertion harness over the live compose stack (`just e2e`)
    swath-testkit/              # perceptual-diff library + `pdiff` binary for oracle comparisons (never shipped)
    swath-testsupport/          # shared test plumbing: GDAL/h5py truth tables, temp dirs, env-gated skips (never shipped)
    adapters/
      swath-source-cog/         # `RasterSource`: COG over object_store
      swath-source-virtual/     # `RasterSource`: virtual-reference manifests
      swath-pyramid-objectstore/ # `RasterSource` overlay: materialized GeoZarr pyramids + batch writer
      swath-reproject-proj4rs/  # `Reproject`: pure-Rust proj4rs
      swath-catalog-pgstac/     # `Catalog`: Postgres + pgstac
      swath-cache-objectstore/  # `TileCache`: object_store (local/S3)
      swath-events-filedrop/    # `EventSource`: watched drop directory
  web/                          # Web Components + MapLibre GL frontend (TypeScript, no framework; ADR 0011)
  python/                       # uv workspace: ingest-time sidecars (VirtualiZarr conformance reference)
  tests/                        # e2e stack scripts, oracle + referencer-equivalence fixtures, load
  prototypes/                   # dated experiments, immutable once concluded
  docs/                         # requirements, architecture, ADRs, design docs, media
```

Phase-1 adapters are direct dependencies of the binary (Cargo features gate the embedded UI bundle
and HDF5 support, not adapter selection). See §14 for third-party extension beyond compile time.

_Last verified against `576324d`._

## 13. Frontend architecture

- **Vanilla Web Components / Custom Elements**, TypeScript, a tiny in-house reactive/state layer — no React,
  no deck.gl. **MapLibre GL** is the single necessary dependency (WebGL map renderer; BSD; framework-agnostic).
- **X-ray overlay** as a MapLibre **custom WebGL layer** (or Canvas overlay) fed by the Trace SSE stream:
  per-tile decision coloring, cache-hit heatmap, timing/bytes inspector.
- Deck.gl stays out until a genuine GPU-scale vector/point/3D need lands (e.g. dense embeddings scatter),
  and then only as an isolated, optional visualization module.

## 14. Extension model (decided — ADR 0013)

**Compile-time Cargo features/crates for adapters, plus openEO process graphs at runtime** as the
primary user-facing extension surface —
[ADR 0013](decisions/0013-extension-features-plus-openeo-graphs.md) records the three candidate
mechanisms weighed (features, WASM plug-ins, RPC sidecars), the WASM/RPC deferral, and the reopen
condition.

## 15. Deployment topology

Single binary `swath` + **Postgres (pgstac)** + an **object store** (S3/MinIO/local) as the only required
infra; optional Redis for a hot-tile cache. Local: `docker compose up`. Cloud: the binary (N stateless
replicas) + managed Postgres + bucket. The pure-Rust core keeps the image tiny and the cold-start fast.

## 16. Open questions — status ledger

Each row carries exactly one status — **Resolved**, **Open**, or **Closed-by-ADR** — and links
the artifact that owns the full rationale (ADR, design doc, module docs, or data). This section
is a ledger, not the argument: an Open row's link states exactly what evidence would resolve it,
and a Closed row's ADR records its reopen condition. ADRs are immutable; a Resolved/Closed item
reopens only via a superseding ADR.

| # | Item | Status | Rationale lives at |
| --- | --- | --- | --- |
| 1 | Port granularity: one `RasterSource`, or split (metadata vs. read, raster vs. cube `CubeSource`)? | Resolved | As-built port set, §6 (reconciliation #152): one port, no split; the cube half returns only when a native `zarrs` N-dim adapter lands (§7) — reopen trigger recorded in [`ROADMAP.md`](ROADMAP.md) |
| 2 | Where warp lives: pure kernels over a minimal `Reproject`, or a richer `Warp` offload port? | Resolved | As built, §6 (#152): kernels in `swath-render`, no `Warp` port; load evidence in [ADR 0012](decisions/0012-render-stays-inline-async.md); GPU/GDAL offload is an ADR-governed deferral in [`ROADMAP.md`](ROADMAP.md) (GDAL stays test-oracle-only, §3) |
| 3 | Cache key & invalidation | Open (narrowed by #36) | `swath-core` `cache` module docs: content-derived `layer_version` decided for v1, plus what evidence resolves the remainder; GC and partial-mosaic invalidation tracked in [`ROADMAP.md`](ROADMAP.md)'s inventory |
| 4 | Planner budget semantics | Resolved | [`docs/design/materialization-planner.md`](design/materialization-planner.md) (issue #37): explicit per-layer knobs + transparent cost estimates; its recorded future work is tracked in [`ROADMAP.md`](ROADMAP.md) |
| 5 | Control-plane domain model ("make STAC disappear") | Resolved | [`docs/design/catalog-domain.md`](design/catalog-domain.md): lossless domain↔STAC mapping; the `Catalog` port is domain-shaped |
| 6 | Extension mechanism (§14) | Closed-by-ADR | [ADR 0013](decisions/0013-extension-features-plus-openeo-graphs.md): compile-time features + openEO graphs; reopen condition recorded there |
| 7 | Async vs blocking render boundary | Resolved | [ADR 0012](decisions/0012-render-stays-inline-async.md) (M4 load evidence, #101/#102; data under `docs/perf/`; reopen trigger recorded there) |
| 8 | The Python ingest seam | Resolved | [ADR 0006](decisions/0006-legacy-referencer-staged.md): staged Python→Rust behind one manifest port; Rust stage shipped (`swath-referencer`, §7) |
