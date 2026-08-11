# Swath

[![CI](https://github.com/forgo/swath/actions/workflows/ci.yml/badge.svg)](https://github.com/forgo/swath/actions/workflows/ci.yml) [![codecov](https://codecov.io/gh/forgo/swath/graph/badge.svg)](https://codecov.io/gh/forgo/swath) [![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/forgo/swath/badge)](https://scorecard.dev/viewer/?uri=github.com/forgo/swath)

**Satellite data comes in, and is immediately live on a map — and a data scientist can
derive a new product from that live flow and publish it the same way.**

![Capability-ladder diagram: derived products through the public API (X) versus dynamic tile serving (Y). TiTiler and xpublish-tiles sit at "dynamic tiles by design" on serving but at deploy-time or rendering-only rungs on derived products; openEO backends reach runtime graphs plus UDFs on derived products but deliver batch-first. Swath sits at "standard process graphs at runtime" served as measured, traced dynamic tiles — with the open frontier, UDFs at live latency, marked beyond it.](docs/media/wedge-a-quadrants.svg)

That upper-right quadrant is the wedge: openEO backends accept a user's process graph at
runtime but deliver batch-first; TiTiler and xpublish-tiles serve dynamic tiles by design but
neither accepts a user's process graph at runtime. Swath does both — a standard openEO graph in, live
measured tiles out. Every placement in the diagram is graded against each project's own
documentation, with rung definitions and pinned citations in
[`docs/media/wedge.notes.md`](docs/media/wedge.notes.md) and the full cited capability matrix
in [`docs/COMPARISON.md`](docs/COMPARISON.md).

## Try it

```sh
docker run -p 8080:8080 ghcr.io/forgo/swath serve --fixtures
```

Then open <http://localhost:8080> — the committed HLS fixtures and the embedded viewer. Every
published image passed this exact command in CI before pushing
([smoke test](.github/scripts/smoke-image.sh): `/healthz`, a rendered tile, the viewer).

![Swath viewer after publishing an authored layer: the openEO authoring panel open in the left rail with NDVI process steps and a plain-language narrative of the graph, and the newly published service selected in the layer rail, rendering a colormapped NDVI map live.](docs/media/screenshots/10-authoring-published.png)

*An authored openEO NDVI graph, published through the UI and serving on the map immediately —
no reload, nothing pre-baked. Twelve captured, perceptual-diff-verified screenshots in
[`docs/media/screenshots/`](docs/media/screenshots/index.md).*

More ways in — the full ingest-to-pixel demo from a checkout, and authoring your first layer
from the UI, each verified end-to-end in a fresh environment:
[`docs/QUICKSTART.md`](docs/QUICKSTART.md).

## What it is

Swath is an open-source geospatial tile platform with a pure-Rust serving core. It ingests
Earth-observation granules (COG and, via virtual references, archival HDF5), catalogs them,
and serves them as dynamic tiles through OGC API - Tiles and XYZ. A cost-aware planner decides
per tile between cache, overview, and live render under operator budget knobs, and every
decision is published as a trace — the x-ray overlay in the viewer and the integration-test
assertion surface are the same data.

Swath's serving path composes no external tiler. TiTiler, rio-tiler, GDAL, and morecantile
relate to Swath as validation oracles: the test suite renders the same tiles and tile-matrix
math through them and pixel-diffs the results against committed goldens
([`tests/oracle/`](tests/oracle/), `just oracle-verify`; the history of this decision is in
[`docs/CHARTER.md`](docs/CHARTER.md) §7 and [`docs/COMPARISON.md`](docs/COMPARISON.md)).

## Measured, not promised

Method, environment, and regeneration recipes for every figure:
[`docs/PERFORMANCE.md`](docs/PERFORMANCE.md).

- **Ingest-to-pixel: 646 ms** — from "a new granule lands" to "a correct, pixel-verified tile
  on the map," through the real stack. The north-star metric
  ([`docs/REQUIREMENTS.md`](docs/REQUIREMENTS.md) §3), enforced under a 10 s budget by
  `just e2e` on every commit ([`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) §4, artifact:
  [`docs/perf/i2p-baseline.json`](docs/perf/i2p-baseline.json)).
- **Hot-tile serving: p50 ~23 ms** at 32-way concurrency from the write-through cache; cold
  live renders of never-seen z15 tiles land at p50 ~660 ms
  ([`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) §6, artifact:
  [`docs/perf/load-baseline.md`](docs/perf/load-baseline.md)).
- **Referencer: 13.8 ms warm** to generate the virtual-reference manifest for a 1,551-chunk
  VIIRS HDF-EOS5 granule — **39.5×** faster than the VirtualiZarr sidecar it replaced, which
  remains the conformance reference ([`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) §7,
  artifact: [`docs/perf/referencer-baseline.json`](docs/perf/referencer-baseline.json)).
- **Head-to-head with TiTiler, honestly:** on the stateless render-vs-render scenarios —
  TiTiler's specialty — TiTiler is faster on the test machine; Swath's leads are the hot-tile
  path (its write-through cache) and control-plane latency. Full pre-committed protocol and
  results: [`docs/perf/load-h2h-titiler.md`](docs/perf/load-h2h-titiler.md).

## Status

**Alpha.** `v0.1.0-alpha.1` is published as a GitHub prerelease with a versioned container
image. Alphas ship with full build rigor and zero stability promises — anything may break
between alphas, and there is no support commitment until the graduation checklist in
[`docs/RELEASING.md`](docs/RELEASING.md) is met.

What is real today (each milestone links its evidence), what is deliberately parked (the
canonical deferral inventory — tile-cache GC, overview generation, a learned planner cost
model, and more), and the M7+ candidate list all live in [`docs/ROADMAP.md`](docs/ROADMAP.md).

## Documentation

Read in this order:

1. [`docs/REQUIREMENTS.md`](docs/REQUIREMENTS.md) — the north star: mission, non-negotiable
   requirements (R1-R10), success criteria. Changes rarely and deliberately.
2. [`docs/CHARTER.md`](docs/CHARTER.md) — the full vision: why now, the wedge, pillars,
   milestones, positioning.
3. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — ports & adapters, the build/adopt/bind
   boundary, component model, data flows.
4. [`docs/ENGINEERING.md`](docs/ENGINEERING.md) — repo standards: toolchains, linting,
   testing, CI/CD, release, security posture.
5. [`docs/QUICKSTART.md`](docs/QUICKSTART.md) — three verified tracks from nothing to tiles;
   [`docs/DEMO.md`](docs/DEMO.md) — the stopwatch demo and what the x-ray overlay shows.
6. [`docs/OPERATIONS.md`](docs/OPERATIONS.md) — the operator guide: store backends, the tile
   cache, ingest sources, observability — with [`docs/CONFIG.md`](docs/CONFIG.md) (every
   flag/env/TOML key, kept in sync by a CI-enforced drift test) and
   [`docs/ENDPOINTS.md`](docs/ENDPOINTS.md) (every mounted route, with captured examples).
7. [`docs/EXTENDING.md`](docs/EXTENDING.md) — the ports: adding a source adapter or an
   openEO process, and the oracle and test obligations that come with each.
8. [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) and [`docs/COMPARISON.md`](docs/COMPARISON.md)
   — the evidence behind every number and positioning claim in this README.
9. [`docs/decisions/`](docs/decisions/) — dated, immutable ADRs recording *why* Swath is
   shaped this way; [`prototypes/`](prototypes/) holds the dated experiments that produced
   the evidence.

## License

Apache-2.0 (see [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE)) — chosen for its explicit patent
grant (ADR 0003). Contributions require DCO sign-off.
