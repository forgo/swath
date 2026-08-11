# Head-to-head: Swath vs TiTiler on static COG serving (`just load-h2h`, issue #121)

**This is a laptop benchmark.** One developer machine (Apple M2 Max, 12 cores, Darwin 25.5.0 arm64; containers in Docker 29.3.1, Docker Desktop, 12 VM CPUs, aarch64), generated 2026-08-11T07:26:30Z at `38c2140`, oha 1.15.0. It is NOT a capacity-planning study; treat every number as one machine's evidence, reproducible with one command: `just load-h2h`.

**Pre-commitment.** Before this was first run, the maintainer committed (issue #121) to publishing the results REGARDLESS of which server wins, with honest framing. This document is that publication; the numbers below are whatever the run produced.

## What is (and is not) compared

Exactly one capability overlaps enough for a fair head-to-head: **serving a static,
already-ingested COG as WebMercatorQuad PNG tiles**. Both servers render the same two
products from the same committed HLS fixture COGs (`tests/fixtures/`, ~1.4 MB, real
Sentinel-2 data): truecolor (B04/B03/B02, rescale 0..3000) and NDVI ((B8A-B04)/(B8A+B04),
RdYlGn). A pre-flight check asserts both sides return 200 with a 256x256 PNG for both
products before anything is timed.

Explicitly **out of scope** here (COMPARISON.md and issue #120 own capability claims):

- **What TiTiler does that this does not test:** dynamic tiling of arbitrary COGs/STAC
  items/mosaics anywhere on the internet with zero pre-registration, xarray/zarr backends,
  many tile matrix sets and output formats, statistics/point endpoints, the plugin
  ecosystem. TiTiler is a general dynamic tiler; this test pins it to one narrow job.
- **What Swath does that TiTiler does not do (and is NOT scored here):** watch-dir
  ingest-to-pixel, openEO process products, per-tile provenance traces (`x-swath-trace`,
  SSE x-ray), the write-through tile cache as a *capability*, catalog/granule browsing.
- **Caching as a capability** is out of scope, but one scenario (repeated-tile) runs each
  architecture as designed — see the note under the table. Dynamic products and
  provenance are not exercised at all.

## Configuration (both sides disclosed, no strawman)

- **Resource matching:** each server's container pinned to --cpus 4 (Docker CPU quota); memory unlimited for both; servers run ONE AT A TIME on an otherwise idle machine.
- **Swath:** built from this commit's Dockerfile (cargo build --release); tests/e2e/swath-catalog.toml — the same compose stack `just e2e`/`just load` use (pgstac + minio sidecars up but idle-cost only; tiles read band COGs from a local mount).
- **TiTiler:** `ghcr.io/developmentseed/titiler@sha256:bf753ccf0fe0f231bc51a0ddbaebf7c0c82253a26db8ab25d1c30ea417e704ff` (release 2.2.1), uvicorn titiler.application.main:app --workers <pinned CPUs> — the docs' documented command (https://developmentseed.org/titiler/) with MORE workers than its `--workers 1` example (one per pinned CPU). GDAL environment set to the documented recommended values from its performance-tuning guide (<https://developmentseed.org/titiler/advanced/performance_tuning/>): `GDAL_CACHEMAX=200`, `VSI_CACHE=TRUE`, `VSI_CACHE_SIZE=5000000`, `GDAL_BAND_BLOCK_CACHE=HASHSET`, `GDAL_DISABLE_READDIR_ON_OPEN=EMPTY_DIR`, `GDAL_HTTP_MERGE_CONSECUTIVE_RANGES=YES`. Data access: same committed fixture COGs, read-only local mount; multi-asset products via its /stac router over a local STAC item (its canonical multi-COG composition path).
- **Scenario parameters** are `just load`'s own, imported from `tests/load/load.py` (one source of truth); URL mapping and TiTiler product queries: `tests/load/h2h.py`.

## Results

| scenario | server | requests | errors | rps | p50 ms | p95 ms | p99 ms | max ms |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| healthz — idle baseline | swath | 80807 | 0 | 16152.6 | 0.24 | 0.32 | 0.38 | 2.81 |
|  | titiler | 15945 | 0 | 3187.1 | 1.14 | 1.92 | 2.67 | 221.78 |
| repeated-tile storm (see note) | swath | 26603 | 0 | 1328.6 | 22.92 | 34.86 | 42.14 | 67.67 |
|  | titiler | 1329 | 0 | 65.3 | 481.04 | 645.55 | 696.31 | 763.48 |
| cold burst — 128 unique tiles, each once | swath | 128 | 0 | 11.4 | 669.81 | 1159.13 | 1204.0 | 1208.85 |
|  | titiler | 128 | 0 | 71.9 | 77.97 | 573.06 | 679.29 | 679.51 |
| heavy-tile storm — 6 heaviest products | swath | 1057 | 0 | 26.1 | 818.13 | 1285.7 | 1391.09 | 1446.85 |
|  | titiler | 3061 | 0 | 76.2 | 200.43 | 380.42 | 426.77 | 503.95 |

Same rows as throughput/latency ratios (who leads, by how much):

| scenario | rps | p50 | p99 |
|---|---|---|---|
| healthz — idle baseline | swath 5.1x | swath 4.8x | swath 7.0x |
| repeated-tile storm (see note) | swath 20.3x | swath 21.0x | swath 16.5x |
| cold burst — 128 unique tiles, each once | titiler 6.3x | titiler 8.6x | titiler 1.8x |
| heavy-tile storm — 6 heaviest products | titiler 2.9x | titiler 4.1x | titiler 3.3x |

**Bottom line (the pre-committed framing).** On the render-vs-render rows — stateless tile rendering, TiTiler's specialty — **TiTiler is faster on this machine**: Swath is within 6.3x of it on throughput at worst (see the ratio table). Swath's leads are the hot-tile path (its write-through cache) and control-plane latency. Neither fact cancels the other; both are published, as committed, and what each system does beyond this narrow overlap is deliberately not scored here.

### Scenario notes (read before quoting any row)

- **healthz** is each server's own liveness route; TiTiler's returns a versions document,
  Swath's a bare liveness body — a reference point, not a comparison of equals.
- **repeated-tile** is an *architecture contrast, not a render-vs-render comparison*:
  Swath serves its write-through cache (asserted `cache_hit` before the storm); TiTiler
  recomputes every request by design and delegates HTTP caching to the deployment layer.
  Read it as "what a client sees on a hot tile", nothing more.
- **cold burst** and **heavy storm** are the honest render-vs-render rows: every request
  renders. Swath's cache is cleared every 250 ms during the heavy storm (decision probes: {"live": 7, "cache_hit": 8}) and the cold burst is unique-by-construction (decisions: {"live": 128}); TiTiler never caches tiles.
- Each server pays its own per-request metadata cost as deployed: Swath resolves the
  granule through its catalog; TiTiler re-reads the local STAC item JSON. Both are how
  the servers actually serve.

## Regression policy

Internal baselines (`docs/perf/load-baseline.*`, PERFORMANCE.md) remain the regression
reference. This document is a point-in-time comparison, regenerated only deliberately
(`just load-h2h`), never a CI gate.
