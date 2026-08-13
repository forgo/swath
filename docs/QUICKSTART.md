# Quickstart

From nothing to live satellite tiles, three ways — each track was executed
verbatim in a fresh environment before it was written down; the Track 1
one-liner is smoke-tested by CI before every image is published.

| Track | You get | You need |
|---|---|---|
| [1. Tiles in your browser](#track-1--tiles-in-your-browser-one-command) | The demo layers and the viewer, one command | Docker |
| [2. Author your first layer](#track-2--author-your-first-layer) | Your own NDVI variant, published from the UI and live on the map | The Track 3 stack |
| [3. The full demo from a checkout](#track-3--the-full-demo-from-a-checkout) | The complete ingest → catalog → serve motion, with the measured ingest-to-pixel number | Docker, `git`, `just`, Node + `pnpm` |

Going deeper afterwards: [`DEMO.md`](DEMO.md) (what the x-ray overlay is telling
you), [`OPERATIONS.md`](OPERATIONS.md) (running `swath` for real),
[`CONFIG.md`](CONFIG.md) (every flag/env/TOML key, verified mechanically),
[`ENDPOINTS.md`](ENDPOINTS.md) (every route, with captured examples).

## Track 1 — tiles in your browser (one command)

Runs the published image with the committed HLS fixtures baked in: two layers
(HLS true color and on-the-fly NDVI) plus the embedded viewer, zero config,
zero checkout.

```sh
docker run -p 8080:8080 ghcr.io/forgo/swath serve --fixtures
```

The first run pulls the image (~120 MB); the server logs
`serving 2 layer(s) on 0.0.0.0:8080`. Open <http://localhost:8080> — the viewer,
with the layer rail on the left and the fixture granule (a 512×512-pixel HLS
subset over the Rockies southwest of Denver) fitted in view. The NDVI layer is
computed live from the band COGs on every uncached tile request. It looks like
[`media/screenshots/01-landing-layer-rail.png`](media/screenshots/01-landing-layer-rail.png).

Prefer proof over pixels? From a second terminal:

```sh
curl -s http://localhost:8080/healthz          # -> ok
curl -s -o tile.png -w '%{http_code} %{content_type} %{size_download} bytes\n' \
  http://localhost:8080/tilesets/ndvi/tiles/12/1561/848
# -> 200 image/png 100314 bytes
```

`ctrl-c` stops the container. Every route this server mounts is in
[`ENDPOINTS.md`](ENDPOINTS.md).

## Track 2 — author your first layer

The authoring loop ([ADR 0010](decisions/0010-openeo-authoring-surface.md)):
compose an openEO process graph in the UI, publish it, and watch it serve as a
live tile layer — same compiler, same serve path as the built-in layers.

**Prerequisite:** the full catalog-mode stack (the Track 1 container serves a
fixed layer set only), so run Track 3's commands first, then in the viewer:
**Author a layer** in the rail (the palette and every form field are generated
from the server's own `GET /processes` — nothing is hard-coded in the UI) →
**Start from the NDVI template** (a working four-step pipeline prefilled from
the server's metadata, with a plain-words narrative line under the form) →
change the colormap from `rdylgn` to `viridis` and give it a title →
**Publish** (enabled only while the graph is structurally valid) — and the new
layer appears in the rail immediately, no reload, the map recoloring as its
tiles arrive
([`media/screenshots/10-authoring-published.png`](media/screenshots/10-authoring-published.png)).
The published service survives restarts (persisted on the dataset's catalog
document) and can be deleted from the panel's **Published** list — after which
its tile URL honestly 404s.

### The same thing over curl

The panel is a pure client of the openEO API surface; the walkthrough above is
one `POST /services` (full surface: [`ENDPOINTS.md`](ENDPOINTS.md)). The server
assigns the service id, so capture it from the `OpenEO-Identifier` header:

```sh
id=$(curl -s -D - -o /dev/null -X POST http://localhost:8080/services \
  -H 'content-type: application/json' \
  --data '{
    "title": "NDVI (viridis, curl)",
    "type": "xyz",
    "process": { "process_graph": {
      "load": { "process_id": "load_collection", "arguments": {
        "id": "hls-s30", "spatial_extent": null, "temporal_extent": null,
        "bands": ["b8a", "b04"] } },
      "ndvi": { "process_id": "ndvi", "arguments": {
        "data": { "from_node": "load" }, "nir": "b8a", "red": "b04" } },
      "scale": { "process_id": "linear_scale_range", "arguments": {
        "x": { "from_node": "ndvi" }, "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 255 } },
      "save": { "process_id": "save_result", "arguments": {
        "data": { "from_node": "scale" }, "format": "png",
        "options": { "colormap": "viridis" } }, "result": true }
    } }
  }' | tr -d '\r' | tee /dev/stderr | sed -n 's/^openeo-identifier: //p')
echo "published: $id"
```

(`outputMin`/`outputMax` are spelled out because the render path quantizes to
8-bit — the server rejects any output range other than `0..255`, and openEO's
own default is `0..1`. The UI's smart defaults do this for you.) The service
serves on the next tile request:

```sh
curl -s -o authored.png -w '%{http_code}\n' "http://localhost:8080/tilesets/$id/tiles/12/1561/848"
# -> 200
```

## Track 3 — the full demo from a checkout

The stopwatch demo (R8: one command from a checkout): the full local stack —
`swath` in catalog mode, pgstac, MinIO — comes up, a granule drops into the
watched directory, and ingest → catalog → serve happens with zero manual steps,
timed. [`DEMO.md`](DEMO.md) is the full tour.

You need: Docker (with compose), `git`, [`just`](https://just.systems), Node.js
and `pnpm`. No Rust toolchain — the server builds inside Docker.

```sh
git clone https://github.com/forgo/swath.git
cd swath
just setup-web   # once: web deps + the Playwright chromium
just demo
```

The first run builds the image (several minutes; later runs start in seconds).
Open the printed viewer URL while it builds: the footprint area is gray on
purpose — a tile of the empty catalog is an honest 404. When the countdown
ends the granule drops, imagery appears on its own (the component
auto-retries), the x-ray overlay badges every tile with the server's own
account of its render, and the measured **ingest-to-pixel** number prints big
at the end. `ctrl-c` tears the whole stack down. CI runs this same path
(`just e2e`) on every PR and push, asserting the budget, oracle-golden tile
correctness, and the honest pre-drop 404 — the demo cannot rot.

## Troubleshooting

- **Docker isn't running** (`Cannot connect to the Docker daemon…`): start
  Docker Desktop (macOS/Windows) or `systemctl start docker` (Linux) and rerun.
- **Port already in use.** Track 1 and the Track 3 stack both bind 8080 (the
  stack also takes 5173, 5432, 9000); `just demo` refuses loudly before starting
  when it detects a previous demo or stack. Fixes: `ctrl-c` the previous demo;
  `docker compose down -v` for a leftover stack; `docker ps` then
  `docker stop <name>` for a Track 1 container; `lsof -ti :8080` for anything
  else.
- **The map is still gray after the countdown.** MapLibre won't refetch tiles it
  already saw 404 once the auto-retry has stopped — nudge the map (drag or zoom)
  to force fresh requests.
