# Swath — Roadmap

_Working document. August 2026. The canonical record of what has shipped, what is deliberately
**deferred** (the single deferral inventory — every "future work" note points here), and what
is **parked for M7+** (proposed order; a maintainer decision). The regression rule: zero
TODO/FIXME in the tree — a deferral is a recorded decision here, never a code comment IOU.
Where ADR-governed, the ADR's reopen condition wins ([`decisions/`](decisions/)); the phase
plan is [`CHARTER.md`](CHARTER.md) §10, the open-questions ledger
[`ARCHITECTURE.md`](ARCHITECTURE.md) §16._

---

## 1. Shipped

**M0 — Foundation scaffold**: `ENGINEERING.md` transcribed into a real repo. **M1 — Prototype
0001: referencer bake-off**: concluded ADR 0006 with evidence. **M2 — Walking skeleton**: HLS
COG in, correct tile out via OGC API - Tiles, NDVI on the fly, viewer + x-ray v0 — validated
against the GDAL oracle. **M3 — Materialization brain and legacy path**: the cost-aware
planner, content-keyed write-through cache, virtual-reference serving of VIIRS, the openEO
bounded profile (ADR 0010). **M4 — Color, measure, ship a container**: colormaps, bench + load
baselines, the typed e2e harness, ADR 0012, the GHCR one-liner. **M5 — Product surface and
evidence**: landing page, browsing, the authoring panel, trace analytics, screenshots,
diagrams, `PERFORMANCE.md`, the `v0.1.0-alpha.1` prerelease. (M0-M5 complete.)
**M6 — Legibility: the docs tell the truth** (in progress): rewriting the project's
self-description around the now-existing evidence and killing every audit-flagged drift
(#117-#126: README, QUICKSTART, operator guide, COMPARISON, the TiTiler head-to-head,
ARCHITECTURE/EXTENDING/CHARTER/ENGINEERING/REQUIREMENTS reconciliation, the screenshot suite,
this roadmap).

## 2. Deferral inventory (canonical)

Every prose deferral in the tree, in one place: what is deferred, the site that records it
(linking back here), why, and the reopen trigger. "Deferred" means *decided against for now
with a named revisit condition* — not forgotten.

| # | Deferral | Recorded at | Why deferred | Revisit when |
|---|---|---|---|---|
| 1 | **WebP tile encoding** | `crates/swath-render/src/ir.rs` (`TileFormat`) | Every extra `image` codec is supply-chain surface; PNG serves every current consumer | A consumer actually needs WebP |
| 2 | **Cache GC of orphaned entries** | `crates/swath-core/src/cache.rs`, `ARCHITECTURE.md` §16.3 | Content-derived keys never go stale, only unreachable; no current storage pressure | Measured storage growth in a real deployment |
| 3 | **Partial-mosaic invalidation** | `crates/swath-core/src/cache.rs`, `docs/design/materialization-planner.md` §6 | Single-granule serving makes the whole-version bump exactly right | Multi-granule mosaic layers land, plus measured re-render cost |
| 4 | **Learned planner cost model** | `docs/design/materialization-planner.md` §6, `crates/swath-planner/src/lib.rs` | The x-ray contract (R4) demands explainable choices; the Trace already carries the training pairs | A Trace corpus shows the fixed estimate constants materially wrong |
| 5 | **Budget-aware (planner-owned) write policy** | `docs/design/materialization-planner.md` §6, `crates/swath-render/src/tiler.rs` | Write-through is unconditional; conditional caching earns nothing yet | Real storage pressure on the tile cache |
| 6 | ~~**Overview *generation***~~ — **closed by #183**: `swath materialize` + `crates/adapters/swath-pyramid-objectstore` | `docs/design/materialization-planner.md` §6, `ARCHITECTURE.md` §10 | — | — |
| 7 | ~~**Time dimension**~~ — **consumed by [ADR 0015](decisions/0015-time-dimension-frame-selection.md)**; serving shipped by #180, graph-side windows by #181 | the ADR, `crates/swath-render/src/process.rs` | — | The ADR's reopen/supersede conditions |
| 8 | **Non-WebMercator target TMS / multi-CRS mosaics** | `crates/swath-render/src/tiler.rs` | `WebMercatorQuad` is the only TMS every client asks for | A client needs another TMS, or cross-CRS mosaics land |
| 9 | **COG metadata caching** | `crates/adapters/swath-source-cog/src/lib.rs` | Amortizing header/IFD walks changes no observable result | `describe` overhead visible at realistic asset counts |
| 10 | **GRIB2 georeferencing** | `crates/swath-referencer/src/grib.rs` | Real work with its own known-answer tests; no GRIB dataset serves (ADR 0008) | A GRIB dataset is put on the serving path |
| 11 | **CF coordinate interpretation (plain HDF5/NetCDF4)** | `crates/swath-referencer/src/hdf.rs` | Non-EOS arrays carry no georef today — recorded honestly, not guessed from CF | Demand to serve a non-EOS HDF5/NetCDF4 collection |
| 12 | **HDF-EOS parsing scope widening** | `crates/swath-referencer/src/eos.rs` | The parser reads exactly what VNP09GA uses; widening needs new known-answer tests | A supported product line outside the scope fence |
| 13 | **PROJ C-binding long-tail adapter** | `crates/adapters/swath-reproject-proj4rs/src/lib.rs`, ADR 0002 | proj4rs + the fenced sinusoidal module (ADR 0009) cover every CRS actually served | A required CRS falls outside proj4rs + `sinu` |
| 14 | **CDN-pointable (extension-keyed) cache layout** | `crates/adapters/swath-cache-objectstore/src/lib.rs` | The framed-payload layout works on every backend; directly-servable objects need an external reader | A CDN or external reader is pointed at the cache |
| 15 | **Dataset *spatial* extents derived from granules** (the temporal half shipped with #180 per [ADR 0015](decisions/0015-time-dimension-frame-selection.md)) | `crates/swath-cli/src/config.rs` | A whole-world placeholder is honest; real extents are bookkeeping without a consumer | Discovery (Records) needs real spatial extents |
| 16 | **Header/metadata fetch-provenance port extension** | `crates/swath-render/src/tiler.rs` | `RasterSource` reports provenance for pixel reads; header accounting would widen the port for a number nobody reads yet | Header I/O accounting is actually wanted in the Trace |

**ADR-governed deferrals** carry their reopen conditions in the ADR, listed only for
completeness: WASM plug-ins and sidecar-RPC adapters (ADR 0013); openEO
jobs/batch/user-defined-processes/files and auth (ADR 0010 — also why no openEO conformance
class is claimed); OGC API - Maps (`ARCHITECTURE.md` §7); GPU/GDAL warp offload
(`ARCHITECTURE.md` §16.2 — GDAL stays test-oracle-only); deck.gl (ADR 0005).

### Icechunk (graduated: executed interop in M8)

First recorded here as a deferral; **graduated by
[ADR 0016](decisions/0016-extraction-boundary-published-crates.md)** into an executed-interop
plan. M8 ships the interop rather than an adapter wish: the referencer commits virtual chunk
references to an Icechunk repo instead of owning a private-only format (M8.7, #191, with an
icechunk-python/xarray conformance gate), and Swath serves tiles back from an Icechunk commit,
byte-identical to the manifest path and trace-visible (M8.9, #193; all executed), with the
zarrs codec-chain adoption (M8.6, #190) as the enabling step. What remains demand-triggered (item 15 below): the
versioned-layer product UX — time-travel surfacing, transactional multi-granule updates (which
would also change row 3's story) — and the native-Zarr `RasterSource` adapter
(`ARCHITECTURE.md` §7, which also reopens `RasterSource`-vs-`CubeSource`, §16.1).
**Revisit when:** a user needs versioned/transactional layers surfaced, or the native Zarr
adapter lands.

## 3. M7+ candidates (proposed order — maintainer decision pending)

Everything parked for after M6, in proposed order — **not final until the maintainer approves
it** (the PR that introduced this file carries the approval checkbox).

1. **#156 — openeo `save_result` profile-note drift** (smallest truth-telling fix).
2. **#139 — linux/arm64 GHCR manifest** (the one-liner fails natively on Apple Silicon).
3. **Time dimension** (row 7). *Decided by ADR 0015; shipped in #180/#181.*
4. **Overview generation** (row 6). *Shipped by #183.*
5. **#151 — authoring UX rethink** (guided flow, live preview-before-publish).
6. **Dataset-creation API** — completes the "single pane of glass" claim.
7. **Auth (OIDC/RBAC)** — Charter Phase 3; gates multi-tenancy, the hosted demo, the openEO
   conformance class.
8. **Hosted public demo** — needs auth (7) and the ops learnings (9).
9. **Performance beyond the laptop** — the gaps `PERFORMANCE.md` §9 declines to claim.
10. **Cache operations bundle** (rows 2, 3, 5) — real together, with mosaics and storage
    pressure.
11. **OGC API - EDR** — rides on the time dimension (3).
12. **OGC API - Records** — wants real dataset extents (row 15).
13. **OGC API - Features** — vector/GeoParquet.
14. **OGC API - Maps** — lowest-demand surface.
15. **Icechunk adapter** (above). *Interop executed in M8 per ADR 0016/0017: refs committed
    (#191) and tiles served back from a commit, byte-identical and traced (#193); only the
    versioned-layer product UX remainder stays demand-triggered.*
16. **Engine breadth bundle** (rows 8, 9, 10) — demand-triggered.
17. **WebP** (row 1) — deliberately cheap late.
18. **Learned planner cost model** (row 4) — needs a real-operation Trace corpus.
19. **Embeddings frontier** (Charter Phase 4) — the biggest bet, deliberately last.

Inventory rows without a numbered entry (11–16) are demand-triggered maintenance: scheduled
when their revisit trigger fires, not by milestone.
