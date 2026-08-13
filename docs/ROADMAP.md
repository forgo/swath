# Swath — Roadmap

_Working document. August 2026. The canonical record of three things: what has shipped, what is
deliberately **deferred** (the single deferral inventory — every "future work" note in the docs
and module docs points here), and what is **parked for M7+** (candidates in proposed priority
order; the ordering is a maintainer decision). The regression rule this document serves: the
tree carries zero TODO/FIXME — a deferral is a recorded decision here, never a code comment IOU._

Related reading: [`REQUIREMENTS.md`](REQUIREMENTS.md) (the north star),
[`CHARTER.md`](CHARTER.md) §10 (the phase plan this roadmap operationalizes),
[`ARCHITECTURE.md`](ARCHITECTURE.md) §16 (the open-questions ledger),
[`decisions/`](decisions/) (ADRs — where a deferral is ADR-governed, the ADR's reopen condition
wins).

---

## 1. Shipped

**M0 — Foundation scaffold** (complete): `ENGINEERING.md` transcribed into a real repo.
**M1 — Prototype 0001: referencer bake-off** (complete): concluded ADR 0006 with evidence.
**M2 — Walking skeleton** (complete): HLS COG in, correct tile out via OGC API - Tiles, NDVI on
the fly, viewer + x-ray v0, the ingest-to-pixel timer — validated against the GDAL oracle.
**M3 — Materialization brain and legacy path** (complete): the cost-aware planner, content-keyed
write-through cache, virtual-reference serving of VIIRS, the openEO bounded profile (ADR 0010).
**M4 — Color, measure, ship a container** (complete): colormaps, bench + load baselines, the
typed e2e harness, ADR 0012, the GHCR one-liner with the UI embedded.
**M5 — Product surface and evidence** (complete): landing page, browsing, the authoring panel,
trace analytics, screenshots, diagrams, `PERFORMANCE.md`, the `v0.1.0-alpha.1` prerelease.
**M6 — Legibility: the docs tell the truth** (in progress): rewriting the project's
self-description around the now-existing evidence and killing every audit-flagged drift. Landed:
ARCHITECTURE reconciliation (#122, #124), EXTENDING.md (#125), the screenshot suite, this
roadmap (#126). Remaining: README rewrite (#117), QUICKSTART (#118), operator guide (#119),
COMPARISON.md (#120), the TiTiler head-to-head (#121), and the CHARTER/ENGINEERING/REQUIREMENTS
reconciliation (#123).

## 2. Deferral inventory (canonical)

Every prose deferral in the tree, in one place. Each row: what is deferred, the site whose docs
record it (that site links back here), why, and the trigger that reopens it. "Deferred" means
*decided against for now with a named revisit condition* — not forgotten.

| # | Deferral | Recorded at | Why deferred | Revisit when |
|---|---|---|---|---|
| 1 | **WebP tile encoding** | `crates/swath-render/src/ir.rs` (`TileFormat`), `encode.rs`, `crates/swath-api/src/lib.rs` | Every extra `image` codec is supply-chain surface; PNG serves every current consumer | A consumer actually needs WebP |
| 2 | **Cache GC of orphaned entries** | `crates/swath-core/src/cache.rs`, `crates/adapters/swath-cache-objectstore/src/lib.rs`, `ARCHITECTURE.md` §16.3 | Content-derived keys never go stale, only unreachable; no current storage pressure | Measured storage growth in a real deployment |
| 3 | **Partial-mosaic invalidation** | `crates/swath-core/src/cache.rs`, `docs/design/materialization-planner.md` §6, `ARCHITECTURE.md` §16.3 | Single-granule serving makes the whole-version bump exactly right | Multi-granule mosaic layers land, plus measured re-render cost |
| 4 | **Learned planner cost model** | `docs/design/materialization-planner.md` §6, `ARCHITECTURE.md` §16.4, `crates/swath-core/src/planner.rs` | The x-ray contract (R4) demands explainable choices; the Trace already carries the training pairs | A Trace corpus shows the fixed estimate constants materially wrong |
| 5 | **Budget-aware (planner-owned) write policy** | `docs/design/materialization-planner.md` §6, `crates/swath-render/src/tiler.rs` | Write-through is unconditional; conditional caching earns nothing yet | Real storage pressure on the tile cache |
| 6 | ~~**Overview *generation* (GeoZarr pyramids / batch materialization)**~~ — **closed by #183**: `swath materialize` + `crates/adapters/swath-pyramid-objectstore` | `docs/design/materialization-planner.md` §6, `ARCHITECTURE.md` §10 (both updated) | — | — |
| 7 | ~~**Time dimension**~~ — **consumed by [ADR 0015](decisions/0015-time-dimension-frame-selection.md)** (frame selection, not aggregation); serving side shipped by #180, graph-side windows by #181 | `docs/decisions/0015-time-dimension-frame-selection.md`, `crates/swath-render/src/process.rs` | — | The ADR's reopen/supersede conditions, which win per this file's rule |
| 8 | **Non-WebMercator target TMS / multi-CRS mosaics** | `crates/swath-render/src/tiler.rs` | `WebMercatorQuad` is the only TMS every current client asks for | A client needs another target TMS, or mosaics spanning source CRSs land |
| 9 | **COG metadata caching** | `crates/adapters/swath-source-cog/src/lib.rs` | Header + IFD walks are per-asset bookkeeping; amortizing them changes no observable result | Trace/`describe` overhead visible at realistic asset counts |
| 10 | **GRIB2 georeferencing** | `crates/swath-referencer/src/grib.rs` | Real work with its own known-answer tests, and no GRIB dataset is on the serving path (ADR 0008) | A GRIB dataset is put on the serving path |
| 11 | **CF coordinate interpretation (plain HDF5/NetCDF4)** | `crates/swath-referencer/src/hdf.rs` | Non-EOS arrays carry no georef today — recorded honestly, not guessed from CF | Demand to serve a non-EOS HDF5/NetCDF4 collection |
| 12 | **HDF-EOS parsing scope widening** | `crates/swath-referencer/src/eos.rs` | The parser reads exactly what VNP09GA uses; widening needs new known-answer tests | A supported product line outside the scope fence |
| 13 | **PROJ C-binding long-tail adapter** | `crates/adapters/swath-reproject-proj4rs/src/lib.rs`, ADR 0002 | proj4rs + the fenced sinusoidal module (ADR 0009) cover every CRS actually served | A required CRS falls outside proj4rs + `sinu` |
| 14 | **CDN-pointable (extension-keyed) cache layout** | `crates/adapters/swath-cache-objectstore/src/lib.rs` | The framed-payload layout works on every backend; directly-servable objects need an external reader | A CDN or external reader is pointed at the cache |
| 15 | **Dataset *spatial* extents derived from ingested granules** — the row split by [ADR 0015](decisions/0015-time-dimension-frame-selection.md): the temporal half shipped with #180; the spatial half stays deferred | `crates/swath-cli/src/config.rs` | A whole-world placeholder is honest; maintaining real spatial extents is bookkeeping without a consumer | Discovery (Records) needs real spatial extents |
| 16 | **Header/metadata fetch-provenance port extension** | `crates/swath-render/src/tiler.rs` | `RasterSource` reports provenance for pixel reads; header accounting would widen the port for a number nobody reads yet | Header I/O accounting is actually wanted in the Trace |

**ADR-governed deferrals** carry their reopen conditions in the ADR (immutable; supersede to
reopen) and are listed here only for completeness: **WASM plug-ins and sidecar-RPC adapters**
(ADR 0013); **openEO jobs / batch / user-defined processes / files** and **openEO auth
endpoints** (ADR 0010 / Charter Phase 3 — also why no openEO conformance class is claimed);
**OGC API - Maps** (deferred at the standards map, `ARCHITECTURE.md` §7); **GPU/GDAL warp
offload** (`ARCHITECTURE.md` §16.2 — GDAL stays test-oracle-only); **deck.gl** (ADR 0005).

### Icechunk (first written record)

An **Icechunk adapter** would be a `RasterSource` (or future cube-source) adapter over
`zarrs` + `zarrs_icechunk`, reading versioned, transactional Zarr stores — including virtual
references *committed to an Icechunk repo* rather than loose manifest JSON — buying versioned
layers with time-travel, transactional multi-granule updates (which would also change row 3's
partial-mosaic story), and alignment with the Earthmover stack. Deferred because manifest v1 +
`object_store` covers every current dataset and no versioned-store demand exists; it is the
natural companion to the planned native-Zarr adapter (`ARCHITECTURE.md` §7), which also reopens
`RasterSource`-vs-`CubeSource` (§16.1). **Revisit when:** the native Zarr adapter lands, or a
user needs versioned/transactional layer updates.

## 3. M7+ candidates (proposed order — maintainer decision pending)

Everything parked for after M6, in the order this document proposes; the ordering is **not final
until the maintainer approves it** (the PR that introduced this file carries the approval
checkbox).

1. **#156 — openeo `save_result` profile-note drift** (smallest truth-telling fix).
2. **#139 — linux/arm64 GHCR manifest** (the one-liner fails natively on Apple Silicon).
3. **Time dimension** (row 7). *Decided by ADR 0015; shipped in #180/#181.*
4. **Overview generation** (row 6). *Shipped by #183.*
5. **#151 — authoring UX rethink** (guided flow, live preview-before-publish).
6. **Dataset-creation API** — completes the "single pane of glass" claim.
7. **Auth (OIDC/RBAC)** — Charter Phase 3; gates multi-tenancy, the hosted demo, the openEO
   conformance class.
8. **Hosted public demo** — needs auth (7) and the ops learnings (9).
9. **Performance beyond the laptop** — the honest gaps `PERFORMANCE.md` §9 declines to claim.
10. **Cache operations bundle** (rows 2, 3, 5) — real together, when mosaics and storage
    pressure arrive.
11. **OGC API - EDR** — rides on the time dimension (3).
12. **OGC API - Records** — wants real dataset extents (row 15).
13. **OGC API - Features** — vector/GeoParquet.
14. **OGC API - Maps** — lowest-demand OGC surface.
15. **Icechunk adapter** (section above).
16. **Engine breadth bundle** (rows 8, 9, 10) — demand-triggered.
17. **WebP** (row 1) — deliberately cheap to do late.
18. **Learned planner cost model** (row 4) — needs a real-operation Trace corpus.
19. **Embeddings frontier** (Charter Phase 4) — the biggest bet, deliberately last.

Inventory rows without a numbered entry above (11–16) are demand-triggered maintenance items:
they get scheduled when their revisit trigger fires, not by milestone.
