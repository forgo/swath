# Capability comparison — Swath, TiTiler, xpublish-tiles, openEO backends

This document grades four systems against four capabilities: **arbitrary derived
products**, **low-latency dynamic tiles**, **cost-aware cache**, and **per-tile
provenance**. It exists to make the README's positioning sentence auditable — every
cell is a citation, not an adjective.

**Fairness rules** (same rules as the wedge diagram, `docs/media/wedge.notes.md`):

1. Capability definitions were written **before** any cell was filled; cells grade
   against the definitions below, nothing else.
2. Every non-Swath cell cites that project's **own documentation** (pinned URL,
   access date, project version). No cell is filled from memory. No project is
   placed below what its own docs claim.
3. Every Swath cell cites an artifact **in this repository** — a CI-gated test, a
   spec'd source module, or a committed measurement file.
4. Each compared project gets a non-empty ["what they do better"](#what-they-do-better)
   section.
5. The maintainer signs off on fairness before this document merges ("I would be
   comfortable if the TiTiler maintainer read this").

## Compared versions (pinned)

| Project | Version compared | Released | Docs accessed |
|---|---|---|---|
| TiTiler | v2.2.1 | 2026-07-29 | 2026-08-11 |
| xpublish-tiles | v0.7.4 | 2026-08-07 | 2026-08-11 |
| openEO | API specification 1.2.0 (the spec backends implement; capabilities graded from the spec's own docs, so they reflect what a conformant backend *may* offer, not any single deployment) | — | 2026-08-11 |
| Swath | this repository, at the commit that contains this file (all Swath links are repo-relative and therefore pin with the commit) | — | — |

External projects move; if a citation below no longer matches a project's current
docs, the cell is stale and should be re-graded — the placement tracks their
documentation, not our memory.

## Capability definitions

Written before the matrix was filled. The first two ladders are shared verbatim
with the wedge diagram (`docs/media/wedge.notes.md`), so diagram and matrix cannot
drift apart.

### 1. Arbitrary derived products

What a *user of the running service* can publish through the public API, without
forking or redeploying it. Rungs, lowest to highest:

1. **Rendering & styling only** — the API selects, styles, rescales, colormaps
   existing variables; no new product can be defined through the API.
2. **Fixed band-math expressions** — per-request arithmetic over bands (e.g.
   numexpr strings).
3. **Operator-registered code (deploy-time)** — arbitrary code is possible, but it
   is registered by the operator when the service is built/deployed, not submitted
   by a user at runtime.
4. **Standard process graphs at runtime** — a user POSTs a standard (openEO)
   process graph to the running service and gets a served product back; no
   redeploy.
5. **Runtime graphs + arbitrary code (UDFs)** — rung 4 plus user-uploaded custom
   code executed server-side.

### 2. Low-latency dynamic tiles

How the most-derived product reaches a map. Rungs, lowest to highest:

1. **Batch jobs / pre-computed** — results are computed as jobs and stored, then
   accessed.
2. **On-demand services (where implemented)** — tiles/coverages computed on
   request via a service protocol, but offered optionally / per-backend, and not
   latency-engineered.
3. **Dynamic tiles by design** — the system's design center is answering tile
   requests dynamically, no pre-rendering step.
4. **Dynamic tiles, measured + traced** — rung 3 plus committed latency evidence
   and per-tile decision provenance (live vs. overview vs. cache, bytes, timings)
   as a product feature.

### 3. Cost-aware cache

Whether the tile-serving path includes a cache whose fill/serve decisions are made
by an **explicit, inspectable cost model** — per-request cost estimates for each
candidate strategy, operator-facing budget knobs, and the chosen/rejected estimates
recorded somewhere a user can read. Grades:

1. **None documented** — no caching claims in the project's docs.
2. **Read/data caching** — caches for source reads, grids, or intermediate data
   (block caches, HTTP caches), tuned by configuration; no per-request cost model
   over serving strategies.
3. **Caching recommended / cost estimated out-of-band** — the spec or docs
   recommend caching computed results and/or provide cost estimation as a separate
   feature (e.g. before running a job), without a per-request serving cost model.
4. **Cost-model-driven serving cache** — every tile request weighs
   cache/overview/live candidates by estimated cost under an operator-set budget,
   and the estimates are recorded per request.

Grades 2 and 3 are *different kinds* of caching, not a strict ladder — the cells
say which kind.

### 4. Per-tile provenance

Whether, for each served tile, the system records and exposes **which strategy
produced it and from what** — decision (cache/overview/live), source
bytes/ranges read, and stage timings — as a product feature a user can consume.
Grades:

1. **None documented.**
2. **Job-level logs** — logs attached to processing jobs, not per-tile.
3. **Request tracing via general observability** — spans/timings per request
   through a standard telemetry stack (e.g. OpenTelemetry), consumed with external
   tooling; not a domain-level "why this tile" record.
4. **First-class per-tile decision records** — decision, candidates, inputs, and
   timings emitted per tile through the product's own API, and asserted in CI.

## The matrix

Each cell: grade (bold) + citation key. Citations are listed
[below](#citations), one per cell claim.

| Capability | TiTiler v2.2.1 | xpublish-tiles v0.7.4 | openEO backends (API 1.2.0) | Swath |
|---|---|---|---|---|
| Arbitrary derived products | **3 — operator-registered code**: numexpr `expression` support, plus custom algorithms as Python classes extending `BaseAlgorithm`, registered via `algorithms.register(...)` at application construction; parameterized (not defined) at request time [T1] | **1 — rendering & styling**: colormaps, out-of-range colors, dimension-selection DSL, multiscale/GeoZarr level selection over existing variables; no band-math or derived-product API documented [X1] | **5 — runtime graphs + UDFs** (top of the ladder, above Swath): process graphs chain "pre-defined and user-defined processes"; users "upload custom code and have it executed" [O1] | **4 — standard process graphs at runtime**: `POST /services` accepts a standard openEO process graph and answers with a live XYZ tile URL; UDFs/jobs deliberately out of scope, so one rung below full openEO [S1] |
| Low-latency dynamic tiles | **3 — dynamic by design**: "a modern dynamic tile server built on top of FastAPI and Rasterio/GDAL"; continuous benchmark tracking exists (grade 4 requires committed latency evidence *plus per-tile provenance as a product feature*, which their docs don't claim) [T2] | **3 — dynamic by design**: OGC Tiles/WMS served directly from Xarray datasets across regular, curvilinear, unstructured, HEALPix and other grids, no pre-rendering step (no committed latency evidence claimed, so not grade 4) [X2] | **2 — on-demand where implemented**: secondary web services (OGC WMS/WCS, XYZ) where computations "often run on demand"; batch-first delivery; synchronous mode recommended only for "lightweight computations" [O2] | **4 — measured + traced**: committed load evidence — hot-cache tile storm p50 <!-- number:2cpu-hot-p50 -->23.46 ms<!-- /number:2cpu-hot-p50 --> / p95 <!-- number:2cpu-hot-p95 -->37.68 ms<!-- /number:2cpu-hot-p95 --> at <!-- number:2cpu-hot-rps -->1,277.6 req/s<!-- /number:2cpu-hot-rps -->, cold live-render p50 <!-- number:2cpu-cold-p50 -->965.57 ms<!-- /number:2cpu-cold-p50 -->, control-plane p99 <!-- number:2cpu-healthz-p99 -->1.44 ms<!-- /number:2cpu-healthz-p99 --> under concurrent warps — honest about cold costs, plus per-tile traces (next row's artifacts) [S2] |
| Cost-aware cache | **2 — read/data caching**: performance-tuning guide configures GDAL block cache, VSI cache, HTTP cache for source reads; no documented cost model over serving strategies [T3] | **2 — read/data caching**: internal grid-system cache (configurable size) and coordinate-transform reuse; no documented tile-serving cost model [X3] | **3 — caching recommended + out-of-band cost estimates**: "back-ends should make sure to cache processed data to avoid additional/high costs"; batch jobs are "the only mode that allows to get an estimate about time, volume and costs beforehand" [O3] | **4 — cost-model-driven**: every request weighs cache/overview/live by estimated bytes × warp cost under operator budget knobs; property-tested cheapest-admissible choice; estimates recorded per request in the trace [S3] |
| Per-tile provenance | **3 — request tracing via OpenTelemetry** (opt-in `[telemetry]` extra): traces "for all API endpoints" with spans for data access and image processing, exported to standard observability platforms [T4] | **1 — none documented**: the README (surveyed 2026-08-11) makes no provenance or observability claims; if such a feature exists undocumented, this cell moves up [X4] | **2 — job-level logs**: "log files are generated" for batch jobs; the spec's docs do not address per-tile provenance for secondary services [O4] | **4 — first-class per-tile records**: each render emits a `Trace` — executed decision (`live` / `cache_hit` / `overview`), all considered candidates with cost estimates, source-read provenance, stage timings — streamed over SSE `GET /traces` and asserted in CI [S4] |

Reading the matrix honestly: no column is dominated. openEO out-authors Swath
(5 vs. 4). TiTiler and xpublish-tiles are dynamic tile servers by design, same
rung 3 as Swath's design center — Swath's grade-4 claim is "committed evidence
and per-tile provenance", **not "faster than them"** (no numeric cross-project
comparison appears here; see [the TiTiler head-to-head](#titiler-head-to-head-issue-121)).
The cell Swath exists for is the *conjunction* — all four columns in one system —
which is exactly the README's positioning claim (the wedge-quadrant paragraph):
"Swath does both — a standard openEO graph in, live measured tiles out." Rows 1–2
are that sentence's two axes; rows 3–4 are the committed evidence behind
"live measured tiles".

## Citations

### TiTiler (docs accessed 2026-08-11, v2.2.1 released 2026-07-29)

- **[T1]** Algorithms guide — numexpr `expression`, `BaseAlgorithm`,
  `default_algorithms.register({...})` at application setup; parameters via query
  string at request time:
  <https://developmentseed.org/titiler/user_guide/algorithms/>; worked example:
  <https://developmentseed.org/titiler/examples/code/tiler_with_custom_algorithm/>
- **[T2]** Landing page tagline ("a modern dynamic tile server…"):
  <https://developmentseed.org/titiler/>; continuous benchmark dashboard
  (github-action-benchmark): <https://developmentseed.org/titiler/benchmark.html>
- **[T3]** Performance tuning — GDAL block cache, `VSI_CACHE`, HTTP cache,
  `GDAL_DISABLE_READDIR_ON_OPEN=EMPTY_DIR`:
  <https://developmentseed.org/titiler/advanced/performance_tuning/>
- **[T4]** Observability with OpenTelemetry — "automatically creating traces for
  all API endpoints", spans for data access and image processing:
  <https://developmentseed.org/titiler/advanced/telemetry/>
- Release pin: <https://github.com/developmentseed/titiler/releases> (v2.2.1,
  2026-07-29)

### xpublish-tiles (docs accessed 2026-08-11, v0.7.4 released 2026-08-07)

- **[X1] [X2] [X3] [X4]** Project README — OGC Tiles/WMS plugins serving tiles
  directly from Xarray datasets (regular, curvilinear, unstructured, HEALPix and
  other grids) with datashader-based rendering, GeoZarr/multiscale level
  selection, colormap/legend/dimension-selection features, and internal
  grid-system caching; no band-math, derived-product, provenance, or latency
  claims: <https://github.com/earth-mover/xpublish-tiles>; design write-up:
  <https://www.earthmover.io/blog/dynamic-map-tile-rendering-icechunk-zarr-data-xpublish-tiles>
- Release pin: <https://github.com/earth-mover/xpublish-tiles/releases> (v0.7.4,
  2026-08-07)

### openEO (glossary accessed 2026-08-11; API specification 1.2.0)

- **[O1] [O2] [O3] [O4]** openEO glossary — process graphs ("pre-defined and
  user-defined processes"), UDFs ("upload custom code and have it executed"),
  batch jobs (stored results, generated log files, upfront time/volume/cost
  estimates), synchronous processing ("lightweight computations"), secondary web
  services (OGC WMS/WCS/XYZ, computations "often run on demand"), and the caching
  recommendation ("back-ends should make sure to cache processed data to avoid
  additional/high costs and reduce waiting times for the user"):
  <https://openeo.org/documentation/1.0/glossary.html>

### Swath (this repository, at this commit)

- **[S1]** CI-gated test [`post_service_serves_tiles_byte_identical_to_the_builtin_ndvi`](../crates/swath-api/tests/openeo_services.rs)
  POSTs a standard openEO NDVI process graph to `POST /services` and asserts the
  served tiles are byte-identical to the built-in NDVI layer; profile bounds
  (openEO API 1.2.0, no UDFs/jobs/user-defined processes) in
  [ADR 0010](decisions/0010-openeo-authoring-surface.md).
- **[S2]** Committed load evidence
  [`docs/perf/load-2cpu-16.7-evidence.md`](perf/load-2cpu-16.7-evidence.md)
  (scenario table incl. hot p50 <!-- number:2cpu-hot-p50 -->23.46 ms<!-- /number:2cpu-hot-p50 --> / p95 <!-- number:2cpu-hot-p95 -->37.68 ms<!-- /number:2cpu-hot-p95 --> at <!-- number:2cpu-hot-rps -->1,277.6 req/s<!-- /number:2cpu-hot-rps --> and
  cold p50 <!-- number:2cpu-cold-p50 -->965.57 ms<!-- /number:2cpu-cold-p50 -->); regression reference
  [`docs/perf/load-baseline.md`](perf/load-baseline.md); serving-path tests
  [`crates/swath-api/tests/tiles.rs`](../crates/swath-api/tests/tiles.rs).
- **[S3]** Cost model spec and implementation in
  [`crates/swath-core/src/planner.rs`](../crates/swath-core/src/planner.rs)
  (bytes × warp-cost estimates, budget admission, `considered` candidate
  records); property test
  [`chosen_is_cheapest_admissible`](../crates/swath-core/tests/planner_properties.rs);
  cache behavior tests
  [`crates/swath-api/tests/tiles_cache.rs`](../crates/swath-api/tests/tiles_cache.rs)
  (`second_request_is_a_cache_hit_with_identical_bytes`,
  `cache_keys_are_layer_scoped`); operator budget knobs in
  [`docs/CONFIG.md` §`[budget]`](CONFIG.md).
- **[S4]** Trace model in
  [`crates/swath-core/src/trace.rs`](../crates/swath-core/src/trace.rs)
  (executed `decision`, `considered` candidates with cost estimates, source-read
  provenance, per-stage timings); SSE delivery asserted in
  [`crates/swath-api/tests/trace_stream.rs`](../crates/swath-api/tests/trace_stream.rs)
  (`rendered_tiles_stream_as_enveloped_traces`); endpoint reference
  [`docs/ENDPOINTS.md` §`GET /traces`](ENDPOINTS.md); requirement
  [R4 "Glass-box"](REQUIREMENTS.md).

## What they do better

Non-empty by rule — a comparison whose author can't name what the other side does
better hasn't looked hard enough.

### TiTiler

- **Format and source breadth.** COG, STAC, MosaicJSON, and Xarray backends across
  `titiler.core` / `titiler.xarray` / `titiler.mosaic` / `titiler.application`,
  riding the full Rasterio/GDAL format matrix. Swath serves its own catalog's
  virtualized datasets; TiTiler serves nearly anything GDAL can read, pointed at
  by URL.
- **Public, continuous benchmarking.** TiTiler tracks performance on every change
  via a published github-action-benchmark dashboard. Swath's numbers are
  committed evidence files regenerated by `just load` — honest, but not yet a
  public per-commit trend line.
- **Standard observability.** OpenTelemetry integration exports to the tooling
  operators already run (Jaeger, Datadog, …). Swath's per-tile trace is richer
  domain-wise but bespoke (SSE + x-ray overlay); TiTiler meets operators where
  their observability stack already is.
- **Maturity and ecosystem.** Years in production across many organizations,
  extensive deployment guides, and a large contributor community. Swath is
  pre-alpha.

### xpublish-tiles

- **Grid-type breadth.** Regular, rectilinear, curvilinear, unstructured/
  triangulated, cubed-sphere, HEALPix, polar radar sweeps, geostationary scan
  angles — far beyond Swath's serving-grid support today.
- **OGC WMS alongside OGC Tiles.** Standards-conformant WMS is a real interop
  surface Swath does not offer.
- **Native Xarray-ecosystem fit.** Serves live from any Xarray dataset via
  Xpublish plugins — scientists publish what they already have in memory, with
  CF-conventions awareness (categorical flag handling, dimension-selection DSL)
  and legend generation Swath lacks.

### openEO backends

- **Authoring breadth — the top of the ladder.** Full process catalogs,
  user-defined processes, and UDFs (user-uploaded code executed server-side).
  Swath deliberately implements a bounded profile (ADR 0010) and is honestly
  placed one rung below.
- **Upfront cost estimation.** Batch jobs can estimate time, volume, and cost
  *before* the user commits to running them. Swath's cost model is per-request
  and internal; it offers no pre-submission estimate to the end user.
- **A multi-implementation standard with client libraries.** One API served by
  many independent backends, with official Python/R/JavaScript clients — an
  interoperability story a single-vendor system cannot match. (Swath borrows
  exactly this strength by speaking the openEO API at its front door.)

## Relationship to other documents

- **README positioning sentence** ("Swath does both — a standard openEO graph in,
  live measured tiles out", the wedge-quadrant paragraph) is the conjunction claim
  of this matrix; if any cell here changes, re-check that sentence.
- **Wedge diagram** (`docs/media/wedge.notes.md`, `wedge-a-quadrants.svg`,
  `wedge-b-frontier.svg`): its two axes are capability rows 1–2 with the same
  rung definitions; rows 3–4 are the "measured + traced" content of its top rung.

## TiTiler head-to-head (issue #121)

The one honest overlapping capability — serving a static, already-ingested COG as
WebMercatorQuad PNG tiles — has its own numeric comparison, committed at
[`docs/perf/load-h2h-titiler.md`](perf/load-h2h-titiler.md): `just load-h2h` runs
identical scenarios (parameters imported from `just load`'s own single source of
truth) against Swath and a digest-pinned TiTiler v2.2.1 configured per its own
documented production guidance, both containers pinned to the same CPU quota on
the same machine, one at a time. Per the maintainer's pre-commitment, the results
are published **regardless of outcome** — in the committed run TiTiler leads the
render-vs-render scenarios, Swath leads the hot-tile (cache) path; the artifact
carries the full environment disclosure and framing. That head-to-head tests
**only** the overlap: it says nothing about TiTiler's breadth (arbitrary remote
COGs/STAC/mosaics with zero pre-registration — not tested there) nor about
Swath's distinguishing surface (rows 1, 3, and 4 of this matrix — not scored
there). Capability claims stay in the matrix above, cell by cell, with citations.
