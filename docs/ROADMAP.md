# Swath — Roadmap

_Working document. August 2026. This is the canonical record of three things: what has shipped
(milestone one-liners with their exit evidence), what is deliberately **deferred** (the single
deferral inventory — every "future work" note in the docs and module docs points here), and what
is **parked for M7+** (candidates proposed in priority order; the ordering is a maintainer
decision). The regression rule this document serves: the tree carries zero TODO/FIXME — a
deferral is a recorded decision here, never a code comment IOU._

Related reading: [`REQUIREMENTS.md`](REQUIREMENTS.md) (the north star),
[`CHARTER.md`](CHARTER.md) §10 (the original phase plan this roadmap operationalizes),
[`ARCHITECTURE.md`](ARCHITECTURE.md) §16 (the open-questions status ledger),
[`decisions/`](decisions/) (ADRs — where a deferral is ADR-governed, the ADR's reopen condition
wins).

---

## 1. Shipped

**M0 — Foundation scaffold** (complete). `ENGINEERING.md` transcribed into a real repo: Cargo
workspace with inherited lints, the `just` task contract, always-run CI with the `ci-ok` gate,
supply-chain and security posture (cargo-deny, CodeQL, Scorecard, zizmor, REUSE), DCO/squash
rulesets, hygiene files.

**M1 — Prototype 0001: referencer bake-off** (complete). Pure-Rust vs VirtualiZarr
virtual-reference generation measured on real HDF5 + GRIB2; concluded ADR 0006 (staged
Python→Rust behind one manifest port) with evidence.

**M2 — Walking skeleton: granule → live tile** (complete). The Phase-1 vertical slice: HLS COG
in, correct tile out via OGC API - Tiles, NDVI computed on the fly, MapLibre viewer + x-ray v0,
the ingest-to-pixel timer — every step validated against the GDAL oracle.

**M3 — Materialization brain and legacy path** (complete). The cost-aware planner
(`CacheHit | Overview | Live | Refuse` under per-layer budget knobs), content-keyed tile cache
with write-through, virtual-reference serving of VIIRS from manifests, and the openEO bounded
authoring profile (ADR 0010) — graph in, live XYZ out.

**M4 — Color, measure, ship a container** (complete). Colormap engine, criterion bench + HTTP
load harnesses with committed baselines, the typed Rust e2e harness, ADR 0012 (render stays
inline-async, load-evidenced), and the GHCR-published one-liner with the UI embedded in the
binary.

**M5 — Product surface and evidence** (complete). Landing page, dataset/layer browsing,
schema-driven openEO authoring panel, trace analytics; screenshots, the five diagrams,
`PERFORMANCE.md` with re-measured numbers, and the `v0.1.0-alpha.1` prerelease.

**M6 — Legibility: the docs tell the truth** (in progress). Rewriting the project's
self-description around the now-existing evidence and killing every audit-flagged drift.
Landed: ARCHITECTURE reconciliation (#122, #124), EXTENDING.md (#125), the screenshot suite
(#112 follow-through), this roadmap (#126). Remaining: README rewrite (#117), QUICKSTART
(#118), operator guide (#119), COMPARISON.md (#120), the TiTiler head-to-head (#121), and the
CHARTER/ENGINEERING/REQUIREMENTS reconciliation (#123).

## 2. Deferral inventory (canonical)

Every prose deferral in the tree, in one place. Each row: what is deferred, the site whose
docs record it (that site links back here), why it is deferred, and the trigger that reopens
it. "Deferred" means *decided against for now with a named revisit condition* — not forgotten.

| # | Deferral | Recorded at | Why deferred | Revisit when |
|---|---|---|---|---|
| 1 | **WebP tile encoding** | `crates/swath-render/src/ir.rs` (`TileFormat`), `crates/swath-render/src/encode.rs`, `crates/swath-api/src/lib.rs` | Every extra `image` codec is supply-chain surface the license gate must carry; PNG serves every current consumer | A consumer actually needs WebP (bandwidth pressure at tile scale) |
| 2 | **Cache GC of orphaned entries** | `crates/swath-core/src/cache.rs`, `crates/adapters/swath-cache-objectstore/src/lib.rs`, `ARCHITECTURE.md` §16.3 | Content-derived keys never go stale, only unreachable — superseded versions linger harmlessly; a sweep is operational work with no current storage pressure | Measured storage growth from superseded layer versions in a real deployment |
| 3 | **Partial-mosaic invalidation** | `crates/swath-core/src/cache.rs`, `docs/design/materialization-planner.md` §6, `ARCHITECTURE.md` §16.3 | Single-granule serving (latest wins) makes the whole-version bump exactly right; per-footprint invalidation buys nothing yet | Multi-granule mosaic layers land, plus measured re-render cost of whole-layer misses under a realistic granule cadence |
| 4 | **Learned planner cost model** | `docs/design/materialization-planner.md` §6, `ARCHITECTURE.md` §16.4, `crates/swath-core/src/planner.rs` | The x-ray contract (R4) demands explainable choices; three explicit knobs + a checkable byte model deliver that today, and the Trace already carries the training pairs | An accumulated Trace corpus shows the fixed estimate constants materially wrong |
| 5 | **Budget-aware (planner-owned) write policy** | `docs/design/materialization-planner.md` §6, `crates/swath-render/src/tiler.rs` | Write-through is unconditional at the tiler; "cache only tiles whose live cost exceeds X" earns nothing until storage pressure is real | Real storage pressure on the tile cache |
| 6 | ~~**Overview *generation* (GeoZarr pyramids / batch materialization)**~~ — **closed by #183**: `swath materialize` + `crates/adapters/swath-pyramid-objectstore` (GeoZarr-shaped Zarr v2 pyramids over `object_store`; idempotent, resumable) | `docs/design/materialization-planner.md` §6, `ARCHITECTURE.md` §10 (both updated to the shipped state) | — | — |
| 7 | ~~**Time dimension (temporal selection in graphs and serving)**~~ — **consumed by [ADR 0015](decisions/0015-time-dimension-frame-selection.md)** (frame selection, not aggregation). Serving side shipped by #180: `datetime=` on the tile route (latest-at-or-before), the temporal decision on the Trace, granule-scoped cache identity, derived temporal extents. Graph-side windows shipped by #181: `temporal_extent` / `filter_temporal` compile into the granule-resolution window, intersected with the request's `datetime` at resolution | `docs/decisions/0015-time-dimension-frame-selection.md`, `crates/swath-render/src/process.rs` (the conformance statement, updated to the shipped state) | — | The ADR's reopen/supersede conditions, which win per this file's rule |
| 8 | **Non-WebMercator target TMS / multi-CRS mosaics** | `crates/swath-render/src/tiler.rs` | `WebMercatorQuad` is the only TMS every current client asks for; widening `TileRequest` is mechanical once demanded | A client needs another target TMS, or mosaics spanning source CRSs land |
| 9 | **COG metadata caching** | `crates/adapters/swath-source-cog/src/lib.rs` | Header + IFD walks are per-asset bookkeeping, not pixel I/O; amortizing them changes no observable result, so it waits for evidence | Trace/`describe` overhead visible at realistic asset counts |
| 10 | **GRIB2 georeferencing** | `crates/swath-referencer/src/grib.rs` | The grid-definition-template → CRS/transform mapping is real work with its own known-answer tests, and no GRIB dataset is on the serving path (VNP09GA is HDF-EOS, ADR 0008) | A GRIB dataset is put on the serving path |
| 11 | **CF coordinate interpretation (plain HDF5/NetCDF4)** | `crates/swath-referencer/src/hdf.rs` | Non-EOS arrays carry no georef today — recorded honestly rather than guessed from CF conventions | Demand to serve a non-EOS HDF5/NetCDF4 collection |
| 12 | **HDF-EOS parsing scope widening** (other projections/origins, swath/point structures) | `crates/swath-referencer/src/eos.rs` | The parser reads exactly what the VNP09GA product line uses; widening is deliberate work with new known-answer tests, not a parser tweak | A supported product line outside the current scope fence |
| 13 | **PROJ C-binding long-tail adapter** | `crates/adapters/swath-reproject-proj4rs/src/lib.rs`, ADR 0002 | proj4rs + the fenced sinusoidal module (ADR 0009) cover every CRS actually served; the long tail is a feature-gated adapter that must pass the same accuracy suite | A required CRS falls outside proj4rs + `sinu` scope |
| 14 | **CDN-pointable (extension-keyed) cache layout** | `crates/adapters/swath-cache-objectstore/src/lib.rs` | The framed-payload layout works identically on every `object_store` backend; directly-servable objects only matter when something other than the swath serve path reads the cache | A CDN or external reader is pointed at the tile cache |
| 15 | **Dataset *spatial* extents derived from ingested granules** — the row split by [ADR 0015](decisions/0015-time-dimension-frame-selection.md): the temporal half shipped with #180 (`Extent.interval` derived from granule min/max acquisition datetimes, maintained on ingest, served on collection documents); the spatial half stays deferred | `crates/swath-cli/src/config.rs` | A whole-world spatial placeholder is honest for a dataset whose coverage is whatever granules arrive; maintaining real spatial extents is bookkeeping without a consumer | Discovery (Records) needs real spatial extents |
| 16 | **Header/metadata fetch-provenance port extension** | `crates/swath-render/src/tiler.rs` | `RasterSource` reports provenance for pixel reads; header accounting would widen the port for a number nobody reads yet | Header I/O accounting is actually wanted in the Trace |

**ADR-governed deferrals** carry their reopen conditions in the ADR (immutable; supersede to
reopen) and are listed here only for completeness: **WASM plug-ins and sidecar-RPC adapters**
(ADR 0013 — reopen on concrete demand for dynamic plugin loading); **openEO jobs / batch /
user-defined processes / files** (ADR 0010 — the bounded authoring profile); **openEO auth
endpoints** (ADR 0010 / Charter Phase 3 — also why no openEO conformance class is claimed);
**OGC API - Maps** (deferred at the standards map, `ARCHITECTURE.md` §7); **GPU/GDAL warp
offload** (`ARCHITECTURE.md` §16.2 — GDAL stays test-oracle-only); **deck.gl**
(ADR 0005 — until a genuine GPU-scale visualization need).

### Icechunk (first written record)

Until this document, Icechunk existed in Swath only as charter vocabulary (`CHARTER.md` §§2, 7)
and a "zero code hits" row in the standards-map evidence ledger — no decision had ever been
written down. Recording it now: an **Icechunk adapter** would be a `RasterSource` (or future
cube-source) adapter over `zarrs` + `zarrs_icechunk`, reading versioned, transactional Zarr
stores — including virtual references *committed to an Icechunk repo* rather than shipped as
loose manifest JSON (the v1 `VirtualManifest` this tree serves today). That buys versioned
layers with time-travel semantics, transactional multi-granule updates (which would also change
the partial-mosaic invalidation story, row 3), and alignment with the Earthmover stack the
charter positions alongside. It is deferred because the serving path's manifest v1 +
`object_store` covers every current dataset, and no versioned-store demand exists yet; it is
the natural companion to the planned native-Zarr (`zarrs`) adapter (`ARCHITECTURE.md` §7),
which is also the trigger to reopen `RasterSource`-vs-`CubeSource` (§16.1). **Revisit when:**
the native Zarr adapter lands, or a user needs versioned/transactional layer updates.

## 3. M7+ candidates (proposed order — maintainer decision pending)

Everything parked for after M6, in the order this document proposes. One line of rationale per
item; the ordering is **not final until the maintainer approves it** (the PR that introduced
this file carries the approval checkbox).

1. **#156 — openeo `save_result` profile-note drift.** Smallest truth-telling fix in the M6
   spirit; a served process definition under-documents a real capability.
2. **#139 — linux/arm64 GHCR manifest.** Removes the single biggest first-run friction: the
   README one-liner currently fails natively on every Apple Silicon machine.
3. **Time dimension** (inventory row 7). The charter promises "animate a time series"; it is
   the largest capability gap a user can *see*, and it is the prerequisite for EDR. *Decided
   by ADR 0015 (row 7 consumed); serving side shipped in #180, graph-side windows in #181.*
4. **Overview generation** (row 6). Completes the planner's third strategy with real GeoZarr
   pyramids — the planner brain exists; give it something to choose. *Shipped by #183 (row 6
   closed).*
5. **#151 — authoring UX rethink** (guided flow, live preview-before-publish). The wedge is
   the product; M5 review found the ceiling of form-based UX for non-experts.
6. **Dataset-creation API.** Datasets today come from config files; an API (and UI) for
   creating them completes the "single pane of glass" claim.
7. **Auth (OIDC/RBAC).** Charter Phase 3; gates multi-tenancy, the hosted demo, and the openEO
   conformance class (ADR 0010's declared absence).
8. **Hosted public demo.** Evidence-first positioning wants a URL, not just a one-liner; needs
   auth (7) and the ops learnings below (9).
9. **Performance beyond the laptop.** The maintainer intends to improve the honest gaps
   `PERFORMANCE.md` §8 declines to claim: behavior behind a real ingress (CDN/TLS), multi-node
   horizontal scale, and larger-than-fixture datasets.
10. **Cache operations bundle** (rows 2, 3, 5: GC, partial-mosaic invalidation, budget-aware
    writes). These become real together, when mosaics and storage pressure arrive.
11. **OGC API - EDR.** First standards-breadth item; rides directly on the time dimension (3).
12. **OGC API - Records.** Catalog discovery; wants real dataset extents (row 15).
13. **OGC API - Features.** Vector/GeoParquet — the "platform, not a raster viewer" claim.
14. **OGC API - Maps.** Styled imagery; deferred at the standards map, lowest-demand OGC
    surface.
15. **Icechunk adapter** (section above). Waits on the native Zarr adapter or versioned-store
    demand.
16. **Engine breadth bundle** (rows 8, 9, 10: multi-CRS mosaics/TMS, COG metadata caching,
    GRIB georeferencing). Each is demand-triggered; none has demand yet.
17. **WebP** (row 1). A variant addition when a consumer needs it — deliberately cheap to do
    late.
18. **Learned planner cost model** (row 4). Needs the accumulated Trace corpus that only real
    operation produces.
19. **Embeddings frontier** (Charter Phase 4). The biggest bet, deliberately last: a product
    type + vector index once the platform beneath it is boring.

Inventory rows without a numbered entry above (11–16: CF coordinates, HDF-EOS widening, the
PROJ long tail, the CDN cache layout, derived extents, header provenance) are demand-triggered
maintenance items: they get scheduled when their revisit trigger fires, not by milestone.
