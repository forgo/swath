# Swath

[![CI](https://github.com/forgo/swath/actions/workflows/ci.yml/badge.svg)](https://github.com/forgo/swath/actions/workflows/ci.yml) [![codecov](https://codecov.io/gh/forgo/swath/graph/badge.svg)](https://codecov.io/gh/forgo/swath) [![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/forgo/swath/badge)](https://scorecard.dev/viewer/?uri=github.com/forgo/swath)

**Satellite data comes in, and is immediately live on a map — from one pane of glass.**

Swath is an open-source, cloud-native geospatial data platform. It ingests Earth-observation
data like a modern ground segment, catalogs and serves it as dynamic map tiles, and lets data
scientists derive *new* products from the live data flow and publish them the same way — all
managed from a single, intuitive control plane that hides the plumbing.

It doesn't reinvent the excellent primitives the community already built (TiTiler, `xpublish-tiles`,
stac-fastapi/pgstac, VirtualiZarr, Icechunk). It fuses them into the one thing nobody has shipped:
a managed platform where **ingest -> derive -> serve** is a single, low-latency, observable motion.

## The one thing that's genuinely new

> openEO / OGC API - Processes can *define* a derived product (e.g. NDVI) but serves it as a batch job.
> TiTiler can *serve* a raster as low-latency tiles but can't let a scientist define an arbitrary product.
> **Nobody compiles a data-scientist's process graph into a low-latency dynamic tile service with a
> cost-aware cache.** That bridge is Swath.

## North-star metric

**Ingest-to-pixel latency** -- seconds from "a new granule lands" to "it's a visible, correct tile on the map."
Everything in the platform optimizes and reports this number.

## Status

Pre-alpha. Design phase.

## Install

Run the demo container — the committed HLS fixtures and the embedded viewer,
live at <http://localhost:8080>:

```sh
docker run -p 8080:8080 ghcr.io/forgo/swath serve --fixtures
```

Every published image passed this exact command in CI (the publish workflow
smoke-tests `/healthz`, a rendered tile, and the viewer before pushing).

## Documentation

Read in this order:

1. [`docs/REQUIREMENTS.md`](docs/REQUIREMENTS.md) — the north star: mission, non-negotiable
   requirements (R1-R10), success criteria. Changes rarely and deliberately.
2. [`docs/CHARTER.md`](docs/CHARTER.md) — the full vision: why now, the wedge, pillars, milestones,
   positioning.
3. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — ports & adapters, the build/adopt/bind boundary,
   component model, data flows.
4. [`docs/ENGINEERING.md`](docs/ENGINEERING.md) — repo standards: toolchains, linting, testing,
   CI/CD, release, security posture.
5. [`docs/DEMO.md`](docs/DEMO.md) — the stopwatch demo: `just demo`, what the x-ray overlay
   shows, the measured ingest-to-pixel numbers, and how CI asserts the same path forever.
6. [`docs/decisions/`](docs/decisions/) — dated, immutable ADRs recording *why* Swath is shaped
   this way; [`prototypes/`](prototypes/) holds the dated experiments that produced the evidence.

## License

Apache-2.0 (see [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE)) — chosen for its explicit patent
grant (ADR 0003). Contributions require DCO sign-off.
