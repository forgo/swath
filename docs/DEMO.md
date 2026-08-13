# The stopwatch demo — ingest-to-pixel, live

The Phase-1 exit demo (CHARTER.md §10): watch a new satellite granule go from *"file arrives"* to
*"correct pixels on the map"* with zero manual steps, and put a number on it — **ingest-to-pixel
latency**, the north-star metric (REQUIREMENTS.md §3).

## What you'll see

**Scale expectations first:** the demo granule is a deliberately tiny CI-sized fixture — a
**512×512-pixel (≈15×15 km) subset** of one HLS scene over the Rockies southwest of Denver
(mountains around the Platte River valley), not a whole-Earth archive. At the demo's zoom it
fills the viewport; zoom out and it's a small imagery patch on the world basemap — which is the
honest picture: this is one granule, freshly ingested, exactly the size the manifest declares.
The platform serves any number of real-size granules the same way; the fixture is small so the
demo (and CI, forever) runs in seconds from a clean checkout.

1. The full local stack comes up with one command: Swath (catalog mode) + pgstac + MinIO.
2. The viewer opens over the granule's true footprint, on a light world basemap (MapLibre's demo
   tiles — context only; Swath serves the imagery). The footprint area is **empty on purpose** —
   the layer exists, its pixels don't yet, and a tile of the empty catalog is an honest 404 (no
   placeholder, no pre-bake).
3. A countdown ends and the granule drops: five HLS band COGs land in the watched directory, the
   manifest is renamed into place last (the filedrop convention). Nobody touches anything after
   that.
4. Ingest → catalog → serve happens automatically; imagery appears where gray was. The measured
   ingest-to-pixel number prints big at the end.
5. Switch the layer control to **HLS NDVI**: `(B8A − B04) / (B8A + B04)` is computed **on the
   fly** from the band COGs at request time — nothing is pre-baked, and the x-ray proves it.

## Run it

```sh
just setup-web   # once: web deps
just demo        # bring-up, countdown, drop, number; ctrl-c tears down
```

The recipe prints the URL to open (`http://localhost:5173/demo/?xray&basemap=demo&layer=truecolor&center=-105.4475,39.2650&zoom=12`)
while the stack builds. Open it whenever you like — pre-drop tiles 404 (honestly empty), and the
component auto-retries every few seconds, so the imagery appears on its own moments after ingest;
no reloading or map-nudging needed. The recipe refuses to start if a previous demo is still
running (shared ports/stack made overlapping sessions look flaky).

## What the x-ray overlay is telling you

The overlay (on by default via `?xray`) is the glass box (REQUIREMENTS.md R4): it subscribes to
the server's `/traces` SSE stream and paints, per tile, the server's own account of the render —
the **decision** (`live`: computed from source bytes right now; no cache exists yet, so every
tile says so), and on click an inspector with the sources read, byte ranges, CRS hop, and stage
timings. The top-left readout shows the latest **ingest→pixel** number. Every fact it paints
comes from the same `Trace` the e2e suite asserts on — the overlay and the test read one oracle.

Since x-ray v1 (issue #42) the overlay has three display modes (top-left control): **decision**
(the colors above), **bytes** — a log-scale heatmap of source bytes read per tile, with a legend
showing the current min/max and a distinct dashed style for 0-byte cache hits, so panning between
zooms makes the overview/cache savings visibly obvious — and **off**. The inspector grew the
**why-view**: the planner's chosen strategy plus every candidate it weighed (estimated cost,
admissibility, reason). A collapsible **trace feed** drawer (bottom-right) streams one compact
line per received trace — bounded at 200 lines with a dropped counter, pausable, lagged gaps
marked inline — and clicking a line opens that tile's inspector.

## Watch a fire season evolve

The time dimension, live (ADR 0015, issue #182). The demo stack also ingests the **six-date
2024 Park Fire series** (HLS T10TFK subsets over Chico, CA — `tests/fixtures/README.md`), so the
same `just demo` session can scrub a real fire season:

1. In the layer rail, switch to **Park Fire NDVI**. A **time slider** appears bottom-center —
   its stops are the dataset's six acquisition dates, read straight from
   `GET /datasets/hls-s30-fire/granules` (a layer with a single date shows no slider at all,
   which is why the landing page looks exactly as before).
2. Scrub oldest → newest with the x-ray on. Every frame is one `datetime=` request re-pointing
   the raster source at the granule that was current then (latest-at-or-before — the server's
   rule, not the client's guess). Watch the vegetation index collapse between July 22 and
   August 16 as the burn scar appears, then barely recover through October.
3. The glass-box moment: on the **first pass**, every badge says `live` — each frame is rendered
   from source bytes the moment you ask for it, and the analytics card's **frame line** narrates
   the current frame's own plan mix (`frame 2024-08-16… · live N · …`). Scrub the season
   **again**: every badge flips to `cache_hit` — frames cache under the granule they resolved
   to, so the second loop replays instantly from the tile cache. Click any badge: the inspector
   names the frame (granule id, acquisition datetime, resolution rule).
4. Press **play**: the loop advances a frame per beat and prefetches the next frame's tiles
   before showing them, so once the season has been seen the animation cycles without a
   stutter.
5. The URL carries the frame: scrubbing writes `t=<instant>` into the share link
   (e.g. `?layer=park-fire-ndvi&t=2024-08-16T19:03:00Z&xray`), and that link alone reproduces
   the exact frame anywhere — byte-for-byte, like every other deep link on the page.

The regression guarantee extends here too: `just e2e` asserts `datetime=` frame selection
against committed oracle goldens, and the Playwright suite (`just e2e-web`,
`web/e2e/time-slider.e2e.ts`) drives this exact scrub-twice loop, asserting the first pass live
and the second cached through the same trace stream the overlay paints from.

## Current measured numbers

| Where                  | ingest-to-pixel | Notes                                                            |
| ---------------------- | --------------- | ---------------------------------------------------------------- |
| **Committed baseline** | **<!-- number:i2p-ms -->646 ms<!-- /number:i2p-ms -->** | `just perf-i2p`, stamped at <!-- number:i2p-sha -->`27deca2`<!-- /number:i2p-sha --> — [`docs/perf/i2p-baseline.json`](perf/i2p-baseline.json), method in [`PERFORMANCE.md`](PERFORMANCE.md) §4 |
| **Asserted budget**    | **10 000 ms**   | ~15x headroom over the committed baseline                        |

## The regression guarantee

This demo is not a demo script that rots: `just e2e` (CI on every PR and every push to `main`)
drives the **same path** — same stack bring-up, same granule drop (shared
`tests/e2e/stack-up.sh`), same tile URLs — and asserts it forever:

- the pre-drop tile is an honest 404, and the post-drop tile goes live with zero manual steps;
- the served truecolor **and NDVI** tiles perceptually match their committed GDAL/rio-tiler
  oracle goldens (a *correct* tile, not just any tile);
- the NDVI trace on the SSE stream carries `decision: "live"` — the on-the-fly proof;
- `ingest_to_pixel_ms` is printed on every run and asserted under the **10 000 ms** budget.

The budget is the permanent north-star guard. Tightening it is a deliberate, visible act: new
measurements are recorded in the `just e2e` recipe comment alongside the change.
