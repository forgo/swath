# Quickstart

From nothing to live satellite tiles, three ways — each track was executed
verbatim in a fresh environment before it was written down (the transcript
is attached to the PR that adds this file), and the Track 1 one-liner is
smoke-tested by CI before every image is published.

| Track | You get | You need |
|---|---|---|
| [1. Tiles in your browser](#track-1--tiles-in-your-browser-one-command) | The demo layers and the viewer, one command | Docker |
| [2. Author your first layer](#track-2--author-your-first-layer) | Your own NDVI variant, published from the UI and live on the map | The Track 3 stack |
| [3. The full demo from a checkout](#track-3--the-full-demo-from-a-checkout) | The complete ingest → catalog → serve motion, with the measured ingest-to-pixel number | Docker, `git`, `just`, Node + `pnpm` |

Going deeper afterwards: [`DEMO.md`](DEMO.md) (what the x-ray overlay is
telling you), [`OPERATIONS.md`](OPERATIONS.md) (running `swath` for real),
[`CONFIG.md`](CONFIG.md) (every flag/env/TOML key, verified mechanically),
[`ENDPOINTS.md`](ENDPOINTS.md) (every route, with captured examples).

## Track 1 — tiles in your browser (one command)

Runs the published image with the committed HLS fixtures baked in: two
layers (HLS true color and on-the-fly NDVI) plus the embedded viewer,
zero config, zero checkout. Every published image passed this exact
command in CI before it was pushed (the publish workflow smoke-tests
`/healthz`, a rendered tile, and the viewer).

```sh
docker run -p 8080:8080 ghcr.io/forgo/swath serve --fixtures
```

Expected output (first run pulls the image, ~120 MB):

```
Unable to find image 'ghcr.io/forgo/swath:latest' locally
latest: Pulling from forgo/swath
...
2026-08-11T07:13:30.421834Z  INFO embedded UI at http://localhost:8080/
2026-08-11T07:13:30.427983Z  INFO serving 2 layer(s) on 0.0.0.0:8080 (store: ./tests/fixtures); traces: http://localhost:8080/traces
```

Open <http://localhost:8080> — the viewer, with the layer rail on the
left and the fixture granule (a 512×512-pixel HLS subset over the
Rockies southwest of Denver) fitted in view. Switching layers in the rail
re-points the map; the NDVI layer is computed live from the band COGs on
every uncached tile request. It looks like
[`media/screenshots/01-landing-layer-rail.png`](media/screenshots/01-landing-layer-rail.png).

Prefer proof over pixels? From a second terminal:

```sh
curl -s http://localhost:8080/healthz
```

```
ok
```

```sh
curl -s -o tile.png -w '%{http_code} %{content_type} %{size_download} bytes\n' http://localhost:8080/tilesets/ndvi/tiles/12/1561/848
```

```
200 image/png 100314 bytes
```

`ctrl-c` stops the container. Every route this server mounts is in
[`ENDPOINTS.md`](ENDPOINTS.md).

## Track 2 — author your first layer

The authoring loop
([ADR 0010](decisions/0010-openeo-authoring-surface.md)): compose an
openEO process
graph in the UI, publish it, and watch it serve as a live tile layer —
same compiler, same serve path as the built-in layers.

**Prerequisite:** the full catalog-mode stack. The Track 1 container
serves a fixed layer set only (the authoring surface needs catalog mode),
so bring the stack up from a checkout — these are exactly Track 3's
commands, and Track 3 explains what they do:

```sh
git clone https://github.com/forgo/swath.git
cd swath
just setup-web
just demo
```

Wait for the countdown to finish and the imagery to appear (Track 3 shows
the expected output), then in the viewer the demo recipe told you to open
(`http://localhost:5173/demo/…`):

1. **Open the panel.** Click **Author a layer** in the rail. The palette
   and every form field are generated from the server's own
   `GET /processes` — nothing here is hard-coded in the UI. It looks like
   [`media/screenshots/08-authoring-form.png`](media/screenshots/08-authoring-form.png).
2. **Start from the NDVI template.** Click **Start from the NDVI
   template**: a working four-step pipeline (`load_collection → ndvi →
   linear_scale_range → save_result`) prefilled from the server's
   metadata. The narrative line under the form reads it back in plain
   words: *"Load hls-s30 (bands b8a,b04) → compute NDVI ((b8a − b04) /
   (b8a + b04)) → rescale -1..1 to 0..255 → save as png, colored with
   rdylgn."*
3. **Make it yours.** In the `save_result` step, change the colormap from
   `rdylgn` to `viridis` (the select lists exactly what the server
   offers: `grayscale`, `viridis`, `magma`, `rdylgn`), and give it a
   title — e.g. `NDVI (viridis)`. The narrative's tail updates as you
   choose: *"… save as png, colored with viridis."*
4. **Publish.** Click **Publish layer**. The button is enabled only while
   the graph is structurally valid — with the template it already is.
5. **See it render.** The new layer appears in the layer rail
   immediately — same page, no reload — selected, and the map recolors
   as its tiles arrive: NDVI in viridis instead of red-yellow-green.
   Like [`media/screenshots/10-authoring-published.png`](media/screenshots/10-authoring-published.png).

The published service survives restarts (it is persisted on the dataset's
catalog document) and can be deleted from the panel's **Published** list —
after which its tile URL honestly 404s.

### The same thing over curl

The panel is a pure client of the openEO API surface; the walkthrough
above is one `POST /services` (full surface: [`ENDPOINTS.md`](ENDPOINTS.md)).
The server assigns the service id, so capture it from the
`OpenEO-Identifier` header — this block is one paste:

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

```
HTTP/1.1 201 Created
location: http://localhost:8080/services/xyz-ea9411ed8a98
openeo-identifier: xyz-ea9411ed8a98
content-length: 0

published: xyz-ea9411ed8a98
```

(`outputMin`/`outputMax` are spelled out because the render path
quantizes to 8-bit — the server rejects any output range other than
`0..255`, and openEO's own default is `0..1`. The UI's smart defaults do
this for you.) The service serves on the next tile request:

```sh
curl -s -o authored.png -w '%{http_code} %{content_type} %{size_download} bytes\n' "http://localhost:8080/tilesets/$id/tiles/12/1561/848"
```

```
200 image/png 100873 bytes
```

## Track 3 — the full demo from a checkout

The stopwatch demo (R8: one command from a checkout): the full local
stack — `swath` in catalog mode, pgstac, MinIO — comes up, a granule
drops into the watched directory, and ingest → catalog → serve happens
with zero manual steps, timed. [`DEMO.md`](DEMO.md) is the full tour.

You need: Docker (with compose), `git`, [`just`](https://just.systems),
Node.js and `pnpm`. No Rust toolchain — the server builds inside Docker.

```sh
git clone https://github.com/forgo/swath.git
cd swath
just setup-web   # once: web deps + the Playwright chromium
just demo
```

Expected output (the first run builds the image — several minutes; later
runs start in seconds):

```
  Building and starting the stack (the first run takes a while).
  Open NOW and keep it visible:

      http://localhost:5173/demo/?xray&basemap=demo&layer=truecolor&center=-105.4475,39.2650&zoom=12

  The x-ray overlay is already on: every tile is annotated with
  its render decision, and the top-left readout shows ingest->pixel.
  The map is gray on purpose — the layer exists, its pixels don't
  (an honest 404). When the countdown ends, the granule drops.

...
stack healthy in 5s (pull/start -> all healthchecks green)
0.9.10
pgstac: migrations present
minio: live
  granule drops in 15s — watch the map
swath: granule dropped at 07:16:10 UTC
swath: tile went live with zero manual steps (R1)

  ==============================================

     INGEST-TO-PIXEL: 307 ms

     (CI asserts this same path under a 10000 ms
      budget on every commit — forever.)

  ==============================================
```

Open the printed URL while it builds: the footprint area is gray on
purpose — a tile of the empty catalog is an honest 404. When the
countdown ends, imagery appears on its own (the component auto-retries).
The x-ray overlay badges every tile with the server's own account of its
render — like
[`media/screenshots/04-xray-decisions.png`](media/screenshots/04-xray-decisions.png).
`ctrl-c` tears the whole stack down.

CI runs this same path (`just e2e`) on every PR and every push to `main`,
asserting the ingest-to-pixel budget, oracle-golden tile correctness, and
the honest pre-drop 404 — the demo cannot rot.

## Troubleshooting

**Docker isn't running.**

```
docker: Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?
```

Start Docker Desktop (macOS/Windows) or `systemctl start docker` (Linux)
and rerun.

**Port already in use.** Track 1 and the Track 3 stack both bind 8080
(the stack also takes 5173, 5432, 9000), so a Track 1 container still
running, a previous demo waiting on `ctrl-c`, or an e2e run will collide.
Docker reports it as:

```
docker: Error response from daemon: failed to set up container networking: ... Bind for 0.0.0.0:8080 failed: port is already allocated
```

and `just demo` refuses loudly before starting (one of):

```
FAIL: port 5173 is busy — a previous demo/vite is still running (ctrl-c it first)
FAIL: a swath stack is already up on :8080 — 'docker compose down -v' first
```

Fixes: `ctrl-c` the previous demo; `docker compose down -v` (from the
checkout) for a leftover stack; `docker ps` then `docker stop <name>` for
a Track 1 container; `lsof -ti :8080` to find anything else.

**The map is still gray after the countdown.** MapLibre won't refetch
tiles it already saw 404 in the same session if the auto-retry has
stopped — nudge the map (drag or zoom) to force fresh requests.
