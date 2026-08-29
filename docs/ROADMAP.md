# Swath — Roadmap

_Working document. August 2026. Three things, kept apart: what has shipped (§1, one line per
milestone with its evidence), what is deliberately **deferred** (§2 — the single deferral
inventory; every "future work" note in the tree points here and nowhere else), and what is
**next** (§3 — open candidates only). The regression rule: zero TODO/FIXME in the tree — a
deferral is a recorded decision here, never a code comment IOU. Where ADR-governed, the ADR's
reopen condition wins ([`decisions/`](decisions/)); the phase plan is
[`CHARTER.md`](CHARTER.md) §10._

---

## 1. Shipped

| Milestone | Delivered | Evidence |
|---|---|---|
| **M0** Foundation scaffold | `ENGINEERING.md` as a real repo; the CI-mirrored `just check` gate | ADR 0007 |
| **M1** Referencer bake-off | Python vs pure-Rust virtual references, decided on evidence | `prototypes/0001`, ADR 0006/0008 |
| **M2** Walking skeleton | HLS COG in, correct tile out over OGC API - Tiles; NDVI on the fly; viewer + x-ray v0 | GDAL oracle goldens (`tests/oracle/`) |
| **M3** Materialization brain, legacy path | Cost-aware planner, content-keyed write-through cache, VIIRS served through virtual references, the openEO bounded profile | `design/materialization-planner.md`, ADR 0010 |
| **M4** Colour, measure, ship a container | Colormaps; bench + load baselines; the typed e2e harness; the GHCR one-liner | ADR 0012, `perf/` |
| **M5** Product surface and evidence | Landing, browsing, authoring panel, trace analytics, screenshots, diagrams, `PERFORMANCE.md`; `v0.1.0-alpha.1` | `media/`, `RELEASING.md` |
| **M6** Legibility | The self-description rewritten around the evidence; the docs gates (`docs_check`) that keep it true | `tools/docs-check/src/check/` |
| **M7** Time and overviews | Frame selection via `datetime=`; `swath materialize` pyramids | ADR 0015, #180/#181/#183 |
| **M8** Ship the parts | Five published crates; Icechunk interop, byte-identical and traced | ADR 0016/0017/0020, #190–#193 |
| **M9** `run_udf` | User code as sandboxed, fuel-metered WASM in the tile path; deterministic, load-tested, deployable read-only | ADR 0018, `PERFORMANCE.md` §9, `deploy/README.md` |
| **M10** UX product structure | One shell, shadow-DOM primitives on tokens, rail modes, palette, catalog thumbnails, canvas primitives, touch parity | ADR 0021, `design/ui-system.md` |
| **M11** Earn the DAG | `merge_cubes` at the bounded profile; the canvas a constrained DAG; change detection its first product | ADR 0022, `design/authoring-dag.md` |

## 2. Deferral inventory (canonical)

Every prose deferral in the tree, in one place: what is deferred, the site that records it
(linking back here), why, and the reopen trigger. "Deferred" means *decided against for now
with a named revisit condition* — not forgotten.

| # | Deferral | Recorded at | Why deferred | Revisit when |
|---|---|---|---|---|
| 1 | **WebP tile encoding** | `crates/swath-render/src/ir.rs` (`TileFormat`) | Every extra `image` codec is supply-chain surface; PNG serves every current consumer | A consumer actually needs WebP |
| 2 | **Cache GC of orphaned entries** | `crates/swath-core/src/cache.rs`, `ARCHITECTURE.md` §16.3 | Content-derived keys never go stale, only unreachable; no current storage pressure | Measured storage growth in a real deployment |
| 3 | **Partial-mosaic invalidation** | `crates/swath-core/src/cache.rs` | Single-granule serving makes the whole-version bump exactly right | Multi-granule mosaic layers land, plus measured re-render cost |
| 4 | **Learned planner cost model** | `crates/swath-planner/src/lib.rs` | The x-ray contract (R4) demands explainable choices; the Trace already carries the training pairs | A Trace corpus shows the fixed estimate constants materially wrong |
| 5 | **Budget-aware (planner-owned) write policy** | `crates/swath-render/src/tiler.rs` | Write-through is unconditional; conditional caching earns nothing yet | Real storage pressure on the tile cache |
| 6 | ~~**Overview *generation***~~ — **closed by #183**: `swath materialize` + `crates/adapters/swath-pyramid-objectstore` | `ARCHITECTURE.md` §10 | — | — |
| 7 | ~~**Time dimension**~~ — **consumed by [ADR 0015](decisions/0015-time-dimension-frame-selection.md)**; serving shipped by #180, graph-side windows by #181 | the ADR, `crates/swath-render/src/process.rs` | — | The ADR's reopen/supersede conditions |
| 8 | **Non-WebMercator target TMS / multi-CRS mosaics** | `crates/swath-render/src/tiler.rs` | `WebMercatorQuad` is the only TMS every client asks for | A client needs another TMS, or cross-CRS mosaics land |
| 9 | **COG metadata caching** | `crates/adapters/swath-source-cog/src/lib.rs` | Amortizing header/IFD walks changes no observable result | `describe` overhead visible at realistic asset counts |
| 10 | **GRIB2 georeferencing** | `crates/swath-referencer/src/grib.rs` | Real work with its own known-answer tests; no GRIB dataset serves (ADR 0008) | A GRIB dataset is put on the serving path |
| 11 | **CF coordinate interpretation (plain HDF5/NetCDF4)** | `crates/swath-referencer/src/hdf.rs` | Non-EOS arrays carry no georef today — recorded honestly, not guessed from CF | Demand to serve a non-EOS HDF5/NetCDF4 collection |
| 12 | **HDF-EOS parsing scope widening** | `crates/swath-referencer/src/eos.rs` | The parser reads exactly what VNP09GA uses; widening needs new known-answer tests | A supported product line outside the scope fence |
| 13 | **PROJ C-binding long-tail adapter** | `crates/adapters/swath-reproject-proj4rs/src/lib.rs`, ADR 0002 | proj4rs + the fenced sinusoidal module (ADR 0009) cover every CRS actually served | A required CRS falls outside proj4rs + `sinu` |
| 14 | **CDN-pointable (extension-keyed) cache layout** | `crates/adapters/swath-store-objectstore/src/tile_cache.rs` | The framed-payload layout works on every backend; directly-servable objects need an external reader | A CDN or external reader is pointed at the cache |
| 15 | **Dataset *spatial* extents derived from granules** — *shipped with #196 for API-registered datasets and granule registration (union over granules; temporal half was #180 per [ADR 0015](decisions/0015-time-dimension-frame-selection.md))* | `crates/swath-api/src/datasets.rs` | Config-declared datasets keep their declared boxes until a registration touches them | Discovery (Records) reads them ready-made |
| 16 | **Header/metadata fetch-provenance port extension** | `crates/swath-render/src/tiler.rs` | `RasterSource` reports provenance for pixel reads; header accounting would widen the port for a number nobody reads yet | Header I/O accounting is actually wanted in the Trace |
| 17 | **UDF module-store GC** (ADR 0018 §v2) | `crates/swath-core/src/udf.rs`, `crates/adapters/swath-store-objectstore/src/module_store.rs` | Content-addressed modules never go stale; a deleted service's module costs bytes, nothing else | Measured module-store growth |
| 18 | **In-browser UDF authoring (Rust playground)** | `web/src/swath-authoring-panel.ts` (#208) | The canvas uploads compiled `.wasm`; the guest kit (ADR 0020) already closes the edit loop locally, and a browser toolchain is a product of its own | Authors ask for it in numbers |
| 19 | **`mask` as a second join** | [ADR 0022](decisions/0022-two-cube-join-merge-cubes.md) | A gated special case; the IR has no nodata/replacement vocabulary | A product needs masking the IR can express honestly |
| 20 | **Band-wise `merge_cubes`** | [ADR 0022](decisions/0022-two-cube-join-merge-cubes.md) | v1 joins gray × gray; cross-cube band namespaces are undecided | A cross-collection composite is needed |
| 21 | **Cross-CRS / cross-grid join branches** | [ADR 0022](decisions/0022-two-cube-join-merge-cubes.md) | Same collection = same grid; no resampling node in the graph yet | A second collection must join the first |
| 22 | **N > 2 joins** | [ADR 0022](decisions/0022-two-cube-join-merge-cubes.md) | Two `merge_cubes` in series cover the foreseeable products | The first three-input product |
| 23 | **Per-branch `datetime=`** | [ADR 0022](decisions/0022-two-cube-join-merge-cubes.md) | Intersecting every branch keeps "which frames changed" in one parameter | The device pass finds it too restrictive, with trace evidence |
| 24 | **Splitting `process.rs`/`ir.rs` by compiler stage** | `crates/swath-render/src/process.rs` | Reviewability only; `tests/process_compiler.rs` is the spec | A feature forces a new stage |
| 25 | **`ci.yml` as reusable workflows** | `.github/workflows/ci.yml` | One 24-job file is readable; splitting buys nothing until a second caller exists | A second workflow needs the same jobs |

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

## 3. Next (open candidates, proposed order — a maintainer decision)

1. **#156 — openeo `save_result` profile-note drift** (smallest truth-telling fix).
2. **#139 — linux/arm64 GHCR manifest** (the one-liner fails natively on Apple Silicon).
3. **M12 — UX design language**: theme values, typography, motion, light/high-contrast —
   mechanically a token-value swap (ADR 0021's freeze). **M13 — Consolidate**: one source of
   truth per fact across docs, tests, crates and tooling (the milestone's issues carry the
   invariant contract).
4. **Dataset-creation API** — completes the "single pane of glass" claim.
5. **Auth (OIDC/RBAC)** — Charter Phase 3; gates multi-tenancy, *writable* demos (maintainer
   decision 2026-08-12), the openEO conformance class.
6. **Hosted public demo** — the read-only recipe shipped and was exercised end to end
   ([`deploy/README.md`](deploy/README.md), #212); the hosted URL is parked to the auth era —
   the maintainer picks a host when (5) lands, and the CI-tested one-liner stays the demo until
   then (maintainer, 2026-08-25).
7. **Performance beyond the laptop** — the gaps `PERFORMANCE.md` §10 declines to claim; the
   #212 run's ops findings are the first input.
8. **Cache operations bundle** (rows 2, 3, 5) — real together, with mosaics and storage
   pressure.
9. **OGC API - EDR** — rides on the time dimension (ADR 0015).
10. **OGC API - Records** — wants real dataset extents (row 15).
11. **OGC API - Features** — vector/GeoParquet.
12. **OGC API - Maps** — lowest-demand surface.
13. **Versioned-layer product UX** — the Icechunk remainder (§2): time-travel surfacing,
    transactional multi-granule updates, the native-Zarr `RasterSource` adapter.
14. **Engine breadth bundle** (rows 8, 9, 10) — demand-triggered. *UDF operational deferrals
    (ADR 0018 §v2): halo/f32 ABI v2, Python UDFs, module-store GC, planner fuel feedback,
    `Module::serialize` cache — demand-triggered with it.*
15. **WebP** (row 1) — deliberately cheap late.
16. **Learned planner cost model** (row 4) — needs a real-operation Trace corpus.
17. **Embeddings frontier** (Charter Phase 4) — the biggest bet, deliberately last.

Inventory rows without a numbered entry are demand-triggered maintenance: scheduled when their
revisit trigger fires, not by milestone. deck.gl stays out (ADR 0005).
