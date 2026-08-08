# Swath

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

Pre-alpha. Design phase. See [`docs/CHARTER.md`](docs/CHARTER.md) for the full vision, architecture,
build-vs-compose boundary, and roadmap.

## License

MIT (see [`LICENSE`](LICENSE)). Under review -- may move to Apache-2.0 for its explicit patent grant
before first release.
