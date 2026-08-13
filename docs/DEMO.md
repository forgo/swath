# The stopwatch demo — ingest-to-pixel, live

The Phase-1 exit demo (CHARTER.md §10): watch a new satellite granule go from *"file arrives"* to
*"correct pixels on the map"* with zero manual steps, and put a number on it — **ingest-to-pixel
latency**, the north-star metric (REQUIREMENTS.md §3).

## What you'll see

**Scale expectations first:** the demo granule is a deliberately tiny CI-sized fixture — a
**512×512-pixel (≈15×15 km) subset** of one HLS scene over the Rockies — so the demo (and CI,
forever) runs in seconds from a clean checkout; the platform serves real-size granules the same
way.

The full local stack (Swath in catalog mode + pgstac + MinIO) comes up with one command. The
viewer opens over the granule's true footprint, **empty on purpose** — the layer exists, its
pixels don't yet, and a tile of the empty catalog is an honest 404. A countdown ends and the
granule drops (five HLS band COGs, manifest renamed into place last); ingest → catalog → serve
happens with nobody touching anything, imagery appears where gray was, and the measured
ingest-to-pixel number prints big at the end. Switch to **HLS NDVI**: `(B8A − B04)/(B8A + B04)`
is computed **on the fly** at request time — nothing is pre-baked, and the x-ray proves it.

## Run it

```sh
just setup-web   # once: web deps
just demo        # bring-up, countdown, drop, number; ctrl-c tears down
```

The recipe prints the URL to open while the stack builds. Pre-drop tiles 404 (honestly empty)
and the component auto-retries, so the imagery appears on its own moments after ingest. The
recipe refuses to start if a previous demo is still running.

## What the x-ray overlay is telling you

The overlay (on by default via `?xray`) is the glass box (REQUIREMENTS.md R4): it subscribes to
the server's `/traces` SSE stream and paints, per tile, the server's own account of the render —
the **decision**, and on click an inspector with the sources read, byte ranges, CRS hop, and
stage timings. The top-left readout shows the latest **ingest→pixel** number. Every fact it
paints comes from the same `Trace` the e2e suite asserts on — the overlay and the test read one
oracle.

Since x-ray v1 (issue #42) the overlay has three display modes — **decision**, **bytes** (a
log-scale heatmap of source bytes read per tile), **off** — plus the inspector's **why-view**
(the planner's chosen strategy and every candidate it weighed) and a bounded, pausable **trace
feed** drawer whose lines open each tile's inspector.

## Watch a fire season evolve

The time dimension, live (ADR 0015, issue #182). The demo stack also ingests the **six-date
2024 Park Fire series** (HLS T10TFK subsets over Chico, CA — `tests/fixtures/README.md`), so the
same `just demo` session can scrub a real fire season:

1. Switch to **Park Fire NDVI**. The map **auto-frames the layer's data**, and a **time
   slider** appears — its stops are the dataset's six acquisition dates, read straight from
   `GET /datasets/hls-s30-fire/granules` (a single-date layer shows no slider at all).
2. Scrub oldest → newest with the x-ray on. Every frame is one `datetime=` request resolved
   latest-at-or-before — the server's rule, not the client's guess. Watch the vegetation index
   collapse as the burn scar appears.
3. The glass-box moment: on the **first pass** every badge says `live`; scrub **again** and
   every badge flips to `cache_hit` — frames cache under the granule they resolved to. Click a
   badge: the inspector names the frame (granule id, acquisition datetime, resolution rule).
4. Press **play**: the loop prefetches the next frame's tiles, so a seen season animates without
   a stutter; scrubbing writes `t=<instant>` into the share link, which alone reproduces the
   exact frame anywhere.

The regression guarantee extends here too: `just e2e` asserts `datetime=` frame selection
against committed oracle goldens, and the Playwright suite (`web/e2e/time-slider.e2e.ts`) drives
this exact scrub-twice loop through the same trace stream the overlay paints from.

## Current measured numbers

| Where                  | ingest-to-pixel | Notes                                                            |
| ---------------------- | --------------- | ---------------------------------------------------------------- |
| **Committed baseline** | **<!-- number:i2p-ms -->646 ms<!-- /number:i2p-ms -->** | `just perf-i2p`, stamped at <!-- number:i2p-sha -->`27deca2`<!-- /number:i2p-sha --> — [`docs/perf/i2p-baseline.json`](perf/i2p-baseline.json), method in [`PERFORMANCE.md`](PERFORMANCE.md) §4 |
| **Asserted budget**    | **10 000 ms**   | ~15x headroom over the committed baseline                        |

## The regression guarantee

This demo is not a demo script that rots: `just e2e` (CI on every PR and every push to `main`)
drives the **same path** — same stack bring-up, same granule drop, same tile URLs — and asserts
it forever: the pre-drop tile is an honest 404; the served truecolor **and NDVI** tiles
perceptually match their committed GDAL/rio-tiler oracle goldens; the NDVI trace carries
`decision: "live"` — the on-the-fly proof; and `ingest_to_pixel_ms` is asserted under the
**10 000 ms** budget. The budget is the permanent north-star guard; tightening it is a
deliberate, visible act.
