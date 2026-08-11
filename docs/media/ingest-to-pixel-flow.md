# Ingest-to-pixel flow, with measured stage timings

Hand-crafted SVG — [`ingest-to-pixel-flow.svg`](ingest-to-pixel-flow.svg) is both the editable
source and the export (canonical; there is no separate generated form). Every number on it
comes from a committed artifact — see
[`ingest-to-pixel-flow.notes.md`](ingest-to-pixel-flow.notes.md) for the figure-by-figure
provenance. Stages labeled *(no committed timing)* have never been measured in a committed
artifact and carry no number on purpose.

![Ingest to pixel, the measured path. Ingest half: granule event (north-star clock starts) to
ingest orchestrator, which routes clean COG/Zarr straight to asset registration and legacy
HDF through virtual-reference manifest generation (14 ms warm / 29 ms cold, prototype-grade),
then catalog upsert to a servable layer; the whole pipeline measures 297 and 801 ms locally,
535 ms in CI, against a 10 000 ms budget. Serve half: GET tile, resolve layer and RenderSpec,
planner (45-73 ns); a cache hit serves the stored tile (storm p50 22.27 ms end-to-end), else
the render path runs source window computation (13.4 microseconds), byte-range reads, warp
(8.0 ms nearest / 8.9 ms bilinear per band), render IR eval (3.2-3.3 ms), PNG encode
(13.0 ms), cache write-through, and returns the encoded tile plus Trace; the composite bench
of that chain is 37.3 ms median, and cold live requests measure p50 653.88 ms end-to-end
under load.](ingest-to-pixel-flow.svg)

Reading notes:

- The stage benches deliberately do **not** sum to the 37.3 ms composite: the composite reads
  two bands concurrently from an in-memory store and includes trace assembly, while the warp
  bench measures a single band.
- The hot-cache and cold-live figures are end-to-end HTTP percentiles under concurrency
  (oha storms), not single-request stage sums.
- The ingest-half total (297/801/535 ms) has no committed per-stage breakdown; only the total
  is measured, and up to ~500 ms of it is e2e poll granularity.
