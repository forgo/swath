# Sidecar: wedge diagram candidates — placement provenance

The wedge/thesis diagram (issue #114, plan decision #3) positions Swath against openEO
backends, TiTiler, and xpublish-tiles on two axes: **arbitrary derived products** and
**low-latency dynamic serving**. Two candidate framings are drafted; the maintainer selects
(checkboxes in the PR). Fairness rule enforced here: **no competitor is placed below what its
own documentation claims** — every non-Swath placement cites that project's docs; every Swath
placement cites a committed artifact in this repo. External docs accessed **2026-08-10**.

The two dimensions are the first two capability columns of the planned COMPARISON matrix
(issue #120: arbitrary-products, low-latency dynamic tiles); the matrix's other two columns
(cost-aware cache, per-tile provenance) appear only inside the top rung of the serving ladder
("measured + traced"), so diagram and matrix cannot drift apart on definitions.

## Axis definitions (rungs defined before any placement)

**X — derived products through the public API** (what a *user of the running service* can
publish, without forking or redeploying it):

1. *rendering & styling only* — the API selects, styles, rescales, colormaps existing
   variables; no new product can be defined through the API.
2. *fixed band-math expressions* — per-request arithmetic over bands (e.g. numexpr strings).
3. *operator-registered code (deploy-time)* — arbitrary code is possible, but it is registered
   by the operator when the service is built/deployed, not submitted by a user at runtime.
4. *standard process graphs at runtime* — a user POSTs a standard (openEO) process graph to
   the running service and gets a served product back; no redeploy.
5. *runtime graphs + arbitrary code (UDFs)* — rung 4 plus user-uploaded custom code executed
   server-side.

**Y — dynamic tile serving** (how the most-derived product reaches a map):

1. *batch jobs / pre-computed* — results are computed as jobs and stored, then accessed.
2. *on-demand services (where implemented)* — tiles/coverages computed on request via a
   service protocol, but offered optionally / per-backend, and not latency-engineered.
3. *dynamic tiles by design* — the system's design center is answering tile requests
   dynamically, no pre-rendering step.
4. *dynamic tiles, measured + traced* — rung 3 plus committed latency evidence and per-tile
   decision provenance (live vs. overview vs. cache, bytes, timings) as a product feature.

## Candidate A — `wedge-a-quadrants.svg` ("capability ladders")

Discrete grid; each project sits at a rung intersection. Rhetoric: the top-right region
(runtime products × measured live serving) is the wedge. Through M8 the top-right *corner*
(UDFs at live latency) was honestly marked out of scope (ADR 0010); M9 shipped it
(ADR 0018), so the diamond now sits in the corner with a hollow diamond and an arrow recording
the move from its graphs-only position (issue #212).

### Placements and defenses

**TiTiler — X3 (operator-registered code), Y3 (dynamic tiles by design).**
TiTiler's own landing page calls it "a modern dynamic tile server built on top of FastAPI and
Rasterio/GDAL" serving COG/STAC/MosaicJSON — that is rung Y3 by definition, and we do not
place it lower despite it publishing no latency evidence (rung Y4 requires committed
measurements, which their docs don't claim). On X, TiTiler exceeds fixed expressions: its
Algorithm guide provides numexpr `expression` support *and* custom algorithms as Python
classes extending `BaseAlgorithm`, registered via `algorithms.register(...)` when the
application is constructed — arbitrary code, but wired in at deploy time by the operator, not
submitted by an API user at runtime. That is exactly rung X3, the highest their docs claim.
*Spot-check:* placement is at, not below, their claims — "dynamic tile server" ⇒ Y3;
deploy-time `register()` ⇒ X3. Sources: <https://developmentseed.org/titiler/>,
<https://developmentseed.org/titiler/user_guide/algorithms/>,
<https://developmentseed.org/titiler/examples/code/tiler_with_custom_algorithm/>.

**xpublish-tiles — X1 (rendering & styling only), Y3 (dynamic tiles by design).**
The project README describes standards-conformant OGC Tiles and OGC WMS plugins that serve
tiles directly from an Xarray dataset (regular, curvilinear, triangular, HEALPix grids)
"without a pre-rendering step", rendering via Datashader/Numba with async Zarr loading — the
design center is dynamic tile answering, rung Y3 (Y4 requires committed latency evidence +
per-tile provenance, which the README does not claim). On X, the README documents rendering
and styling of existing variables and multiscale/GeoZarr handling, and claims no API for
band math, computed variables, or derived products — rung X1 is therefore the highest its own
documentation claims. *Spot-check:* if xpublish-tiles documents a derived-product/band-math
API we missed, X moves right — the placement tracks their README, not our memory. Sources:
<https://github.com/earth-mover/xpublish-tiles>,
<https://www.earthmover.io/blog/dynamic-map-tile-rendering-icechunk-zarr-data-xpublish-tiles>.

**openEO backends — X5 (runtime graphs + UDFs), Y2 (on-demand services, where implemented),
with a whisker to Y1 (batch-first).**
openEO is placed at the *top* of the X ladder — above Swath: its glossary defines process
graphs chaining "pre-defined and user-defined processes" and UDFs where "users are able to
upload custom code and have it executed"; no other system in the diagram claims that. On Y,
the same glossary lists three delivery modes: batch jobs (results "can be pre-computed" and
stored), synchronous processing (recommended only for "lightweight computations"), and
secondary web services that "allow web-based access using different protocols such as OGC
WMS, OGC WCS or XYZ tiles" where "computations often run *on demand*". The filled point sits
at Y2 — the *highest* serving rung their spec claims (on-demand services exist, but are
optional per backend and carry no latency engineering/evidence claims); the dashed whisker to
Y1 records that delivery is batch-first, which their own docs present first and most fully.
*Spot-check:* placing openEO at Y1 alone would understate them (services are in the spec);
Y3 would overstate them (no openEO doc claims dynamic tiling as the design center). Source:
<https://openeo.org/documentation/1.0/glossary.html>.

**Swath — X5 (runtime graphs + arbitrary code (UDFs)), Y4 (dynamic tiles, measured + traced).**
X5 since M9 (through M8: X4). ADR 0010 implements the openEO API 1.2.0 at a *bounded profile* —
`POST /services` takes a standard openEO process graph, validates it through the process
compiler, and answers 201 with a live XYZ tile URL — and ADR 0018 admits user code into that
graph: `run_udf` (runtime `wasm`, version `1`) executes user-uploaded WASM, sandboxed
(zero-import, NaN-canonicalized, fuel-metered, memory-capped) in the live tile path, pixel
stage only. The rung is proven by committed artifacts, one per M9 step: the executor is
deterministic and fuel-reproducible (#262 — `crates/adapters/swath-udf-wasmtime/tests/determinism.rs`,
`engine_gate.rs`); the compiler persists modules by content hash (#263); the tile path serves a
`run_udf` NDVI byte-identical to the built-in band math with its fuel in the trace and refuses
fuel exhaustion as a pinned RFC 7807 problem (#265 — `crates/swath-api/tests/udf_tiles.rs`);
`POST /result` previews a UDF graph byte-identical to its published service (#267 —
`crates/swath-api/tests/openeo_result.rs`); and under a UDF storm `/healthz` p99 held at
<!-- number:udf-storm-healthz-p99 -->0.96 ms<!-- /number:udf-storm-healthz-p99 --> while a
fuel-bomb module was refused at
<!-- number:udf-fuelbomb-healthz-p99 -->0.92 ms<!-- /number:udf-fuelbomb-healthz-p99 -->
(#268 — `docs/perf/load-udf-baseline.md`, `PERFORMANCE.md` §9). *Spot-check:* X5's
definition is "user-uploaded custom code executed server-side" — met by WASM modules through
the public `POST /services`; openEO's own UDF story (Python runtimes, batch jobs, user-defined
processes) is broader, which is why Swath shares the rung and is never placed above openEO.
The graphs-only proof still stands beneath it: `post_service_serves_tiles_byte_identical_to_the_builtin_ndvi`
(`crates/swath-api/tests/openeo_services.rs`). Y4: committed load evidence
(`docs/perf/load-2cpu-16.7-evidence.md`) records hot-cache tile storm p50 <!-- number:2cpu-hot-p50 -->23.46 ms<!-- /number:2cpu-hot-p50 --> / p95
<!-- number:2cpu-hot-p95 -->37.68 ms<!-- /number:2cpu-hot-p95 --> at <!-- number:2cpu-hot-rps -->1,277.6 req/s<!-- /number:2cpu-hot-rps -->, cold live-render p50 <!-- number:2cpu-cold-p50 -->965.57 ms<!-- /number:2cpu-cold-p50 -->, and control-plane p99 <!-- number:2cpu-healthz-p99 -->1.44 ms<!-- /number:2cpu-healthz-p99 --> under
concurrent warps; per-tile decision provenance (live vs. cache) streams over SSE `/traces`
and is itself CI-gated (`crates/swath-api/tests/trace_stream.rs`,
`crates/swath-api/tests/tiles_cache.rs`), satisfying the "measured + traced" rung
(REQUIREMENTS.md R4; ingest-to-pixel north star §3). *Spot-check (against ourselves):* cold
live renders are ~1 s p50 on the committed 2-CPU-constrained scenario — the Y4 claim is
"measured and traced dynamic tiles", not "every tile in 25 ms", and the evidence file says so.

## Candidate B — `wedge-b-frontier.svg` ("the single-system frontier")

Continuous (ordinal) axes; a dashed frontier curve through the single-system placements, the
shaded region beyond it labeled with REQUIREMENTS.md §2's thesis ("standing up 'data in →
live on a map…' today means hand-wiring several projects per deployment"), and Swath placed
beyond the frontier with three receipts. Rhetoric: the gap is the product.

Positions are the *same rungs as Candidate A* mapped ordinally onto continuous axes (the
footnote in the SVG says so): xpublish-tiles (X1,Y3), TiTiler (X3,Y3), openEO backends as two
points — filled batch-first (X5,Y1) and hollow secondary-services (X5,Y2) — and Swath
(X5,Y4; X4 through M8). All defenses and spot-checks above apply unchanged. The four Swath
receipt bullets map to: `openeo_services.rs` byte-identical test + ADR 0010 (graph in → live
XYZ out); `udf_tiles.rs` + `docs/perf/load-udf-baseline.md` + ADR 0018 (run_udf on the live
path, fuel-metered, refused with zero collateral);
`docs/perf/load-2cpu-16.7-evidence.md` (23.5 ms p50 hot at 1,278 req/s) + `trace_stream.rs`
(decision traced); REQUIREMENTS.md §3 (ingest-to-pixel north star). The frontier curve passes
through the competitor placements by construction — it asserts nothing about them beyond
their cited rungs; the only claim it adds is the §2 thesis about *combinations*, which is
Swath's claim about the field, clearly attributed to Swath's own requirements doc.

## What neither candidate claims

- Not that Swath out-processes openEO (it doesn't; both sit at the top X rung since M9, and
  openEO's UDF story — Python runtimes, jobs, user-defined processes — is broader than Swath's
  WASM pixel-stage `run_udf` — stated in both SVGs' footnotes).
- Not that TiTiler or xpublish-tiles are slow — both sit at "dynamic tiles by design"; the
  Y4 rung Swath occupies is about *committed evidence and per-tile provenance*, hence
  "measured + traced", not "faster".
- No numeric performance comparison with any other project appears in either SVG (that is
  issue #121's head-to-head, with its own honest-framing gate).

## REUSE

Both SVGs carry an inline SPDX header (same two-line form as source files) and, with this
sidecar, are also covered by the `docs/**` aggregate annotation in `REUSE.toml` (same holder
and license).
