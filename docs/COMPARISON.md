# Capability comparison — Swath, TiTiler, xpublish-tiles, openEO backends

This document grades four systems against four capabilities: **arbitrary derived
products**, **low-latency dynamic tiles**, **cost-aware cache**, and **per-tile
provenance**. It exists to make the README's positioning sentence auditable — every
cell is a citation, not an adjective.

**Fairness rules** (same rules as the wedge diagram, `docs/media/wedge.notes.md`):
definitions written **before** any cell was filled; every non-Swath cell cites that
project's **own documentation** (pinned URL, access date, version — never memory,
and no project placed below what its own docs claim); every Swath cell cites an
artifact **in this repository**; each compared project gets a non-empty
["what they do better"](#what-they-do-better) section; and the maintainer signs off
on fairness before this document merges ("I would be comfortable if the TiTiler
maintainer read this").

## Compared versions (pinned)

| Project | Version compared | Released | Docs accessed |
|---|---|---|---|
| TiTiler | v2.2.1 | 2026-07-29 | 2026-08-11 |
| xpublish-tiles | v0.7.4 | 2026-08-07 | 2026-08-11 |
| openEO | API specification 1.2.0 (graded from the spec's own docs, so cells reflect what a conformant backend *may* offer, not any single deployment) | — | 2026-08-11 |
| Swath | this repository, at the commit that contains this file (all Swath links are repo-relative and therefore pin with the commit) | — | — |

External projects move; if a citation below no longer matches a project's current
docs, the cell is stale and should be re-graded.

## Capability definitions

Written before the matrix was filled. The first two ladders — **arbitrary derived
products** (1 rendering & styling only → 2 fixed band-math expressions → 3
operator-registered code at deploy time → 4 standard process graphs at runtime →
5 runtime graphs + UDFs) and **low-latency dynamic tiles** (1 batch/pre-computed →
2 on-demand where implemented → 3 dynamic tiles by design → 4 dynamic, measured +
traced) — are defined in full in [`docs/media/wedge.notes.md`](media/wedge.notes.md)
and shared verbatim with the wedge diagram, so diagram and matrix cannot drift
apart. The other two:

**3. Cost-aware cache** — whether tile serving includes a cache whose fill/serve
decisions are made by an **explicit, inspectable cost model**: **1** none
documented; **2** read/data caching (block/HTTP caches, tuned by configuration);
**3** caching recommended and/or cost estimated out-of-band, without a per-request
serving cost model; **4** cost-model-driven serving cache — every request weighs
cache/overview/live candidates by estimated cost under an operator-set budget,
estimates recorded per request. (2 and 3 are *different kinds*, not a strict
ladder — the cells say which kind.)

**4. Per-tile provenance** — whether, per served tile, the system exposes **which
strategy produced it and from what** as a product feature: **1** none documented;
**2** job-level logs; **3** request tracing via general observability (e.g.
OpenTelemetry, consumed with external tooling); **4** first-class per-tile
decision records emitted through the product's own API and asserted in CI.

## The matrix

Each cell: grade (bold) + citation key. Citations are listed
[below](#citations), one per cell claim.

| Capability | TiTiler v2.2.1 | xpublish-tiles v0.7.4 | openEO backends (API 1.2.0) | Swath |
|---|---|---|---|---|
| Arbitrary derived products | **3 — operator-registered code**: numexpr `expression` plus custom algorithms registered at application construction; parameterized (not defined) at request time [T1] | **1 — rendering & styling**: colormaps, dimension selection, level selection; no band-math or derived-product API documented [X1] | **5 — runtime graphs + UDFs** (top of the ladder, above Swath): graphs chain "pre-defined and user-defined processes"; users "upload custom code and have it executed" [O1] | **4 — standard process graphs at runtime**: `POST /services` accepts a standard openEO graph and answers with a live XYZ tile URL; UDFs/jobs deliberately out of scope, one rung below full openEO [S1] |
| Low-latency dynamic tiles | **3 — dynamic by design**: "a modern dynamic tile server built on top of FastAPI and Rasterio/GDAL"; continuous benchmark tracking exists (grade 4 requires committed latency evidence *plus per-tile provenance as a product feature*, which their docs don't claim) [T2] | **3 — dynamic by design**: OGC Tiles/WMS served directly from Xarray datasets across many grid types, no pre-rendering step (no committed latency evidence claimed, so not grade 4) [X2] | **2 — on-demand where implemented**: secondary web services where computations "often run on demand"; batch-first delivery; synchronous mode recommended only for "lightweight computations" [O2] | **4 — measured + traced**: committed load evidence — hot-cache tile storm p50 <!-- number:2cpu-hot-p50 -->23.46 ms<!-- /number:2cpu-hot-p50 --> / p95 <!-- number:2cpu-hot-p95 -->37.68 ms<!-- /number:2cpu-hot-p95 --> at <!-- number:2cpu-hot-rps -->1,277.6 req/s<!-- /number:2cpu-hot-rps -->, cold live-render p50 <!-- number:2cpu-cold-p50 -->965.57 ms<!-- /number:2cpu-cold-p50 -->, control-plane p99 <!-- number:2cpu-healthz-p99 -->1.44 ms<!-- /number:2cpu-healthz-p99 --> under concurrent warps — honest about cold costs, plus per-tile traces (next row's artifacts) [S2] |
| Cost-aware cache | **2 — read/data caching**: GDAL block/VSI/HTTP caches for source reads; no documented cost model over serving strategies [T3] | **2 — read/data caching**: internal grid-system cache; no documented tile-serving cost model [X3] | **3 — caching recommended + out-of-band estimates**: "back-ends should make sure to cache processed data to avoid additional/high costs"; batch jobs are "the only mode that allows to get an estimate about time, volume and costs beforehand" [O3] | **4 — cost-model-driven**: every request weighs cache/overview/live by estimated bytes × warp cost under operator budget knobs; property-tested cheapest-admissible choice; estimates recorded per request [S3] |
| Per-tile provenance | **3 — request tracing via OpenTelemetry** (opt-in `[telemetry]` extra): traces "for all API endpoints" with spans for data access and image processing [T4] | **1 — none documented**: the README (surveyed 2026-08-11) makes no provenance or observability claims; if such a feature exists undocumented, this cell moves up [X4] | **2 — job-level logs**: "log files are generated" for batch jobs; the spec's docs do not address per-tile provenance for secondary services [O4] | **4 — first-class per-tile records**: each render emits a `Trace` — executed decision, all considered candidates with cost estimates, source-read provenance, stage timings — streamed over SSE `GET /traces` and asserted in CI [S4] |

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

- **[T1]** Algorithms guide:
  <https://developmentseed.org/titiler/user_guide/algorithms/>
- **[T2]** Landing page tagline: <https://developmentseed.org/titiler/>;
  benchmark dashboard: <https://developmentseed.org/titiler/benchmark.html>
- **[T3]** Performance tuning:
  <https://developmentseed.org/titiler/advanced/performance_tuning/>
- **[T4]** OpenTelemetry telemetry:
  <https://developmentseed.org/titiler/advanced/telemetry/>
- Release pin: <https://github.com/developmentseed/titiler/releases>

### xpublish-tiles (docs accessed 2026-08-11, v0.7.4 released 2026-08-07)

- **[X1] [X2] [X3] [X4]** Project README (the four cells' claims and absences):
  <https://github.com/earth-mover/xpublish-tiles>; design write-up:
  <https://www.earthmover.io/blog/dynamic-map-tile-rendering-icechunk-zarr-data-xpublish-tiles>
- Release pin: <https://github.com/earth-mover/xpublish-tiles/releases>

### openEO (glossary accessed 2026-08-11; API specification 1.2.0)

- **[O1] [O2] [O3] [O4]** openEO glossary (process graphs, UDFs, batch jobs,
  synchronous processing, secondary web services, and the caching
  recommendation quoted in the cells):
  <https://openeo.org/documentation/1.0/glossary.html>

### Swath (this repository, at this commit)

- **[S1]** CI-gated test [`post_service_serves_tiles_byte_identical_to_the_builtin_ndvi`](../crates/swath-api/tests/openeo_services.rs);
  profile bounds (openEO API 1.2.0, no UDFs/jobs/user-defined processes) in
  [ADR 0010](decisions/0010-openeo-authoring-surface.md).
- **[S2]** Committed load evidence
  [`docs/perf/load-2cpu-16.7-evidence.md`](perf/load-2cpu-16.7-evidence.md)
  (scenario table incl. hot p50 <!-- number:2cpu-hot-p50 -->23.46 ms<!-- /number:2cpu-hot-p50 --> / p95 <!-- number:2cpu-hot-p95 -->37.68 ms<!-- /number:2cpu-hot-p95 --> at <!-- number:2cpu-hot-rps -->1,277.6 req/s<!-- /number:2cpu-hot-rps --> and
  cold p50 <!-- number:2cpu-cold-p50 -->965.57 ms<!-- /number:2cpu-cold-p50 -->); regression reference
  [`docs/perf/load-baseline.md`](perf/load-baseline.md); serving-path tests
  [`crates/swath-api/tests/tiles.rs`](../crates/swath-api/tests/tiles.rs).
- **[S3]** Cost model in
  [`crates/swath-core/src/planner.rs`](../crates/swath-core/src/planner.rs);
  property test
  [`chosen_is_cheapest_admissible`](../crates/swath-core/tests/planner_properties.rs);
  cache behavior tests
  [`crates/swath-api/tests/tiles_cache.rs`](../crates/swath-api/tests/tiles_cache.rs);
  operator budget knobs in [`docs/CONFIG.md` §`[budget]`](CONFIG.md).
- **[S4]** Trace model in
  [`crates/swath-core/src/trace.rs`](../crates/swath-core/src/trace.rs); SSE
  delivery asserted in
  [`crates/swath-api/tests/trace_stream.rs`](../crates/swath-api/tests/trace_stream.rs);
  endpoint reference [`docs/ENDPOINTS.md` §`GET /traces`](ENDPOINTS.md);
  requirement [R4 "Glass-box"](REQUIREMENTS.md).

## What they do better

Non-empty by rule — a comparison whose author can't name what the other side does
better hasn't looked hard enough.

**TiTiler:** format and source breadth (the full Rasterio/GDAL matrix, pointed at
by URL, vs Swath's own catalog); public per-commit benchmarking (Swath's numbers
are committed evidence files, not a public trend line); standard OpenTelemetry
observability (Swath's per-tile trace is richer domain-wise but bespoke); and
maturity — years in production, a large community. Swath is pre-alpha.

**xpublish-tiles:** grid-type breadth (curvilinear, unstructured, cubed-sphere,
HEALPix, radar sweeps — far beyond Swath's serving grids); OGC WMS alongside
Tiles; and native Xarray-ecosystem fit with CF-conventions awareness and legend
generation Swath lacks.

**openEO backends:** authoring breadth — the top of the ladder (UDFs and
user-defined processes; Swath's bounded profile is honestly one rung below);
upfront cost estimation before a job runs (Swath's cost model is per-request and
internal); and a multi-implementation standard with official client libraries —
a strength Swath borrows by speaking the openEO API at its front door.

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
[`docs/perf/load-h2h-titiler.md`](perf/load-h2h-titiler.md) (`just load-h2h`:
identical scenarios, a digest-pinned TiTiler configured per its own production
guidance, same CPU quota, one at a time). Per the maintainer's pre-commitment the
results are published **regardless of outcome** — TiTiler leads the
render-vs-render scenarios, Swath leads the hot-tile (cache) path; the artifact
carries the full environment disclosure. The head-to-head tests **only** the
overlap; capability claims stay in the matrix above, cell by cell, with
citations.
