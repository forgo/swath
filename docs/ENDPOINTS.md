# Swath endpoint reference

Every route the `swath serve` process mounts, with one example
request/response per route **captured from the local fixture stack**
(`tests/e2e/stack-up.sh`: the compose stack in catalog mode over the
committed HLS fixture granule, base URL `http://localhost:8080`). Long
bodies are abridged where marked; everything shown was returned by the
running server. Configuration behind these routes:
[`CONFIG.md`](CONFIG.md); operational semantics:
[`OPERATIONS.md`](OPERATIONS.md).

The route tables live in code at
[`crates/swath-api/src/routes.rs`](../crates/swath-api/src/routes.rs)
(OGC + control plane, always mounted),
[`granules.rs`](../crates/swath-api/src/granules.rs) and
[`openeo.rs`](../crates/swath-api/src/openeo.rs) (both merged beside the
OGC router **in catalog mode only** — static/fixtures serving has no
catalog, so those routes simply do not exist there).

Conventions, once:

- Every route is `GET` unless noted; axum answers `HEAD` from the same
  handlers.
- OGC-side errors are RFC 7807 problem documents
  (`{"type","title","status","detail"}`); openEO-side errors are the
  spec's `{"code","message"}` format with codes from the official
  registry.
- With CORS configured the headers apply to every route below; by
  default there are none (ADR 0011, [`OPERATIONS.md`](OPERATIONS.md) §6).

## Route table

The block below is mechanically checked against the axum routers by the
docs gate (`crates/swath-cli/src/docs_check/routes.rs`, `just docs-check`):
every mounted route must appear here — method set and mounting included —
and no phantom rows survive.

<!-- docs-check:begin routes -->
| Method | Path | Mounted | Purpose |
|---|---|---|---|
| GET | `/` | always | Landing page: OGC (+ openEO capabilities in catalog mode); HTML UI for browsers |
| GET | `/conformance` | always | OGC conformance declaration |
| GET | `/tiles` | always | Tilesets list (OGC dataset-tilesets path) |
| GET | `/tilesets` | always | Tilesets list (canonical resource collection; same document) |
| GET | `/tilesets/{layerId}` | always | Tileset metadata incl. derived bounding box |
| GET | `/tilesets/{layerId}/tiles/{tileMatrix}/{tileRow}/{tileCol}` | always | The tile: PNG bytes + `x-swath-trace`; optional `datetime=` frame selection (ADR 0015) |
| GET | `/traces` | always | X-ray SSE stream of every render |
| GET | `/healthz` | always | Liveness probe (process only) |
| GET/HEAD | *fallback* | always | Embedded UI assets; unknown paths are plain 404 |
| GET | `/datasets/{datasetId}/granules` | catalog mode | Granule browsing (paged, filterable) |
| GET | `/.well-known/openeo` | catalog mode | openEO version discovery |
| GET | `/collections` | catalog mode | openEO/STAC collections (one per dataset) |
| GET | `/collections/{collection_id}` | catalog mode | One collection document |
| GET | `/processes` | catalog mode | The supported openEO process subset |
| POST | `/result` | catalog mode | Preview: one bounded synchronous render of a process graph (PNG) |
| GET | `/service_types` | catalog mode | Secondary service types (`xyz`) |
| GET, POST | `/services` | catalog mode | List / publish secondary services |
| GET, DELETE | `/services/{service_id}` | catalog mode | Describe / delete one service |
<!-- docs-check:end routes -->

## OGC API - Tiles + control plane

### `GET /`

The OGC landing page; in catalog mode the same JSON document additionally
carries the openEO capabilities fields (both standards claim the root, so
the root speaks both). With the embedded UI, a browser `Accept` listing
`text/html` receives the viewer's `index.html` instead — the JSON stays
byte-identical for every other client.

```sh
curl -s http://localhost:8080/
```

```json
{
  "api_version": "1.2.0",
  "backend_version": "0.1.0-alpha.1",
  "description": "Live satellite imagery tiles: OGC API - Tiles over the Swath tiler.",
  "endpoints": [
    { "methods": ["GET"], "path": "/collections" },
    { "methods": ["GET"], "path": "/collections/{collection_id}" },
    { "methods": ["GET"], "path": "/conformance" },
    { "methods": ["GET"], "path": "/processes" },
    { "methods": ["POST"], "path": "/result" },
    { "methods": ["GET"], "path": "/service_types" },
    { "methods": ["GET", "POST"], "path": "/services" },
    { "methods": ["GET", "DELETE"], "path": "/services/{service_id}" }
  ],
  "id": "swath",
  "links": [
    { "href": "http://localhost:8080/", "rel": "self", "title": "This landing page", "type": "application/json" },
    { "href": "http://localhost:8080/conformance", "rel": "conformance", "title": "Conformance declaration", "type": "application/json" },
    { "href": "http://localhost:8080/conformance", "rel": "http://www.opengis.net/def/rel/ogc/1.0/conformance", "title": "Conformance declaration", "type": "application/json" },
    { "href": "http://localhost:8080/tiles", "rel": "http://www.opengis.net/def/rel/ogc/1.0/tilesets-map", "title": "Tilesets, one per layer", "type": "application/json" },
    { "href": "http://localhost:8080/collections", "rel": "data", "title": "Collections (openEO / STAC)", "type": "application/json" }
  ],
  "production": false,
  "stac_version": "1.1.0",
  "title": "Swath",
  "type": "Catalog"
}
```

Outside catalog mode the openEO fields (`api_version`, `endpoints`, …) are
absent and only the OGC `title`/`description`/`links` remain. Browser
negotiation:

```sh
curl -s -H 'Accept: text/html' http://localhost:8080/ | head -1
```

```
<!doctype html>
```

### `GET /conformance`

The honesty rule: a class is listed only when its requirements are met
and smoke-tested.

```sh
curl -s http://localhost:8080/conformance
```

```json
{
  "conformsTo": [
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/core",
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/tileset",
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/tilesets-list",
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/dataset-tilesets",
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/png"
  ]
}
```

### `GET /tiles` and `GET /tilesets`

The tilesets list, one entry per served layer — published openEO services
appear here too, under their `xyz-…` id. `/tiles` is the path OGC 20-057
requires on the dataset root; `/tilesets` is the canonical collection
self-links point into. Same handler, same document.

```sh
curl -s http://localhost:8080/tilesets
```

```json
{
  "tilesets": [
    {
      "title": "HLS NDVI",
      "dataType": "map",
      "crs": "http://www.opengis.net/def/crs/EPSG/0/3857",
      "tileMatrixSetURI": "http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad",
      "links": [
        { "href": "http://localhost:8080/tilesets/ndvi", "rel": "self", "type": "application/json", "title": "HLS NDVI tileset metadata" },
        { "href": "http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad", "rel": "http://www.opengis.net/def/rel/ogc/1.0/tiling-scheme", "type": "application/json", "title": "WebMercatorQuad tile matrix set definition" }
      ]
    },
    {
      "title": "Park Fire NDVI",
      "dataType": "map",
      "crs": "http://www.opengis.net/def/crs/EPSG/0/3857",
      "tileMatrixSetURI": "http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad",
      "links": [
        { "href": "http://localhost:8080/tilesets/park-fire-ndvi", "rel": "self", "type": "application/json", "title": "Park Fire NDVI tileset metadata" },
        { "href": "http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad", "rel": "http://www.opengis.net/def/rel/ogc/1.0/tiling-scheme", "type": "application/json", "title": "WebMercatorQuad tile matrix set definition" }
      ]
    },
    {
      "title": "HLS true color",
      "dataType": "map",
      "crs": "http://www.opengis.net/def/crs/EPSG/0/3857",
      "tileMatrixSetURI": "http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad",
      "links": [
        { "href": "http://localhost:8080/tilesets/truecolor", "rel": "self", "type": "application/json", "title": "HLS true color tileset metadata" },
        { "href": "http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad", "rel": "http://www.opengis.net/def/rel/ogc/1.0/tiling-scheme", "type": "application/json", "title": "WebMercatorQuad tile matrix set definition" }
      ]
    }
  ]
}
```

### `GET /tilesets/{layerId}`

Tileset metadata; the bounding box derives from the layer's *resolved*
assets, so a catalog-backed layer whose dataset has no granules yet is
404 here while its identity still appears in the list above.

```sh
curl -s http://localhost:8080/tilesets/ndvi
```

```json
{
  "title": "HLS NDVI",
  "description": "(B8A - B04) / (B8A + B04), rescaled -1..1, RdYlGn colormap.",
  "dataType": "map",
  "crs": "http://www.opengis.net/def/crs/EPSG/0/3857",
  "tileMatrixSetURI": "http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad",
  "boundingBox": {
    "lowerLeft": [-105.53699286345432, 39.19542265081296],
    "upperRight": [-105.35806792254455, 39.33451047970236],
    "crs": "http://www.opengis.net/def/crs/OGC/1.3/CRS84",
    "orderedAxes": ["Lon", "Lat"]
  },
  "links": [
    { "href": "http://localhost:8080/tilesets/ndvi", "rel": "self", "type": "application/json", "title": "HLS NDVI tileset metadata" },
    { "href": "http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad", "rel": "http://www.opengis.net/def/rel/ogc/1.0/tiling-scheme", "type": "application/json", "title": "WebMercatorQuad tile matrix set definition" },
    { "href": "http://localhost:8080/tilesets/ndvi/tiles/{tileMatrix}/{tileRow}/{tileCol}", "rel": "item", "type": "image/png", "title": "HLS NDVI tiles (PNG)", "templated": true }
  ]
}
```

### `GET /tilesets/{layerId}/tiles/{tileMatrix}/{tileRow}/{tileCol}`

The tile. Path order is **OGC order, z/row/col** (`{z}/{y}/{x}`).
`Accept` absent, `*/*`, `image/*`, and `image/png` are acceptable;
anything else is an honest 406. Off-data tiles inside the matrix are a
transparent 200 PNG. The `x-swath-trace` header summarizes the render —
`decision` is `live`, `cache_hit`, or `overview`, and
`ingest_to_pixel_ms` (when the granule's arrival time is known) is the
north-star number, readable from a plain `curl -D`:

```sh
curl -s -D - -o tile.png http://localhost:8080/tilesets/ndvi/tiles/12/1561/848
```

```
HTTP/1.1 200 OK
content-type: image/png
x-swath-trace: {"decision":"live","bytes_read":546497,"total_ms":327,"ingest_to_pixel_ms":106175}
content-length: 100314
```

The same request again (with a cache configured) is a hit:

```
x-swath-trace: {"decision":"cache_hit","bytes_read":0,"total_ms":3}
```

**`datetime=` — the time dimension (ADR 0015).** One optional query
parameter selects **which granule backs the frame**, with exactly the
OGC API grammar (Features/Common Part 2, the grammar EDR reuses): an
RFC 3339 UTC instant (`2024-08-01T00:00:00Z`) or an interval
(`start/end`, either side openable as `..`, never both). An instant
resolves **latest-at-or-before** (the granule that was current then); an
interval resolves to the latest granule within it (inclusive bounds);
absent means the fully open interval — plain latest, byte-identical to a
request without the parameter. Frames cache under the granule they
*resolved to*: two spellings selecting the same granule share one cache
entry, different granules never collide. The trace on the SSE stream
carries the decision (`temporal`: resolved granule id + datetime, the
raw request, and the rule). Static (fixtures-mode) layers are a single
timeless frame: the grammar is validated, then every valid value serves
that frame. Askable times are served on the collection document (the
derived temporal extent, below).

```sh
# The 2024 Park Fire demo series: the same tile, before and after the burn.
curl -s -o pre.png  "http://localhost:8080/tilesets/park-fire-ndvi/tiles/13/3100/1326?datetime=2024-08-01T00:00:00Z"
curl -s -o post.png "http://localhost:8080/tilesets/park-fire-ndvi/tiles/13/3100/1326?datetime=2024-08-20T00:00:00Z"
```

A malformed value is a 400 naming the grammar; a window selecting no
granule (before the first acquisition, or a narrowed interval) is the
same honest 404 shape as "no granule ingested yet":

```sh
curl -s "http://localhost:8080/tilesets/park-fire-ndvi/tiles/13/3100/1326?datetime=yesterday"
```

```json
{"detail":"datetime `yesterday`: `yesterday` is not an RFC 3339 UTC (`Z`) timestamp","status":400,"title":"Bad Request","type":"about:blank"}
```

```sh
curl -s "http://localhost:8080/tilesets/park-fire-ndvi/tiles/13/3100/1326?datetime=2020-01-01T00:00:00Z"
```

```json
{
  "detail": "layer `park-fire-ndvi`: no granule of dataset `hls-s30-fire` has an acquisition datetime within [.., 2020-01-01T00:00:00Z]",
  "status": 404,
  "title": "Not Found",
  "type": "about:blank"
}
```

Error taxonomy (OGC 20-057 allows 404 or 400 for out-of-range; Swath's
split — addresses that *cannot exist* are 404, malformed ones 400):

```sh
curl -s http://localhost:8080/tilesets/truecolor/tiles/25/0/0    # beyond WebMercatorQuad
```

```json
{
  "detail": "tileMatrix `25` is not a WebMercatorQuad tile matrix (expected 0..=24)",
  "status": 404,
  "title": "Not Found",
  "type": "about:blank"
}
```

```sh
curl -s http://localhost:8080/tilesets/ndvi/tiles/12/x/0         # non-integer row
```

```json
{"detail":"tileRow `x` is not an integer","status":400,"title":"Bad Request","type":"about:blank"}
```

A non-PNG `Accept` (e.g. `application/json`) is `406 Not Acceptable`; an
unknown layer or an out-of-matrix row/col is 404.

### `GET /traces`

The x-ray SSE stream (`text/event-stream`): one `trace` event per render
from connection time on, with the full trace the header only summarizes —
sources, byte-range provenance, stage timings, the planner's considered
strategies with reasons, and (catalog-backed renders, ADR 0015) the
`temporal` decision: which granule the frame resolved to, the raw
`datetime` requested, and the rule (`latest`, `latest_at_or_before`, or
`latest_in_interval`). Slow subscribers lose events (`lagged`), never
delay a tile.

```sh
curl -N http://localhost:8080/traces
```

```
event: trace
id: 3
data: {"tile":"12/848/1562","layer":"truecolor","trace":{"decision":"live","source":"hlss30-t13sdd-2024158-b04.tif","sources":["hlss30-t13sdd-2024158-b04.tif","hlss30-t13sdd-2024158-b03.tif","hlss30-t13sdd-2024158-b02.tif"],"crs_from":32613,"crs_to":3857,"bytes_read":430088,"provenance":[{"path":"hlss30-t13sdd-2024158-b04.tif","offset":195269,"length":50384},{"path":"hlss30-t13sdd-2024158-b04.tif","offset":245661,"length":97869},{"path":"hlss30-t13sdd-2024158-b03.tif","offset":189844,"length":48780},{"path":"hlss30-t13sdd-2024158-b03.tif","offset":238632,"length":93462},{"path":"hlss30-t13sdd-2024158-b02.tif","offset":187079,"length":48101},{"path":"hlss30-t13sdd-2024158-b02.tif","offset":235188,"length":91492}],"timings":{"read_ms":206,"warp_ms":28,"pixel_ops_ms":0,"encode_ms":7,"total_ms":242},"ingest_to_pixel_ms":121492,"plan":{"chosen":"live","considered":[{"strategy":"cache_hit","estimated_cost_bytes":0,"admissible":false,"reason":"cache miss"},{"strategy":{"overview":{"factor":0}},"estimated_cost_bytes":0,"admissible":false,"reason":"no overview factor eligible at this zoom"},{"strategy":"live","estimated_cost_bytes":388620,"admissible":true,"reason":"full-resolution read"}]},"temporal":{"granule_id":"hlss30-t13sdd-2024158","granule_datetime":"2024-06-06T17:54:00Z","requested":null,"rule":"latest"}}}
```

### `GET /healthz`

Liveness only — the process is up; no registry, store, or catalog I/O
(see [`OPERATIONS.md`](OPERATIONS.md) §5).

```sh
curl -s http://localhost:8080/healthz
```

```
ok
```

### The fallback — embedded UI assets

`GET`/`HEAD` on any unrouted path is looked up in the embedded web bundle
(hashed assets under `/assets/…`, the entry page at `/` via negotiation).
API routes structurally outrank the bundle; there is no SPA fallback —
unknown paths return the plain empty 404 they always did.

## Granule browsing (catalog mode)

### `GET /datasets/{datasetId}/granules`

What has been ingested, newest first (acquisition datetime descending,
ties by id — a stable total order). Query parameters:
`bbox=west,south,east,north` (CRS84 degrees, footprint intersection),
`datetime=instant | start/end | ../end | start/..` (RFC 3339 UTC,
inclusive), `limit` (1..=1000, default 100), `offset` (default 0); a
`next` link appears while more pages remain. Unknown dataset → 404;
malformed parameter → 400; an existing dataset with no matches is an
empty 200 page. Asset `href`s are store keys, never serving-host paths.

```sh
curl -s http://localhost:8080/datasets/hls-s30/granules
```

```json
{
  "granules": [
    {
      "id": "hlss30-t13sdd-2024158",
      "bbox": [-105.537, 39.1954, -105.3581, 39.3345],
      "datetime": "2024-06-06T17:54:00Z",
      "ingestedAt": "2026-08-11T06:41:14.004Z",
      "assets": {
        "b02": { "href": "hlss30-t13sdd-2024158-b02.tif", "kind": "raster" },
        "b03": { "href": "hlss30-t13sdd-2024158-b03.tif", "kind": "raster" },
        "b04": { "href": "hlss30-t13sdd-2024158-b04.tif", "kind": "raster" },
        "b8a": { "href": "hlss30-t13sdd-2024158-b8a.tif", "kind": "raster" },
        "fmask": { "href": "hlss30-t13sdd-2024158-fmask.tif", "kind": "raster" }
      }
    }
  ],
  "numberMatched": 1,
  "numberReturned": 1,
  "links": [
    {
      "href": "http://localhost:8080/datasets/hls-s30/granules?limit=100&offset=0",
      "rel": "self",
      "type": "application/json",
      "title": "Granules of dataset hls-s30"
    }
  ]
}
```

## openEO surface (catalog mode)

The authoring loop of ADR 0010: read collections and processes, publish a
process graph as an `xyz` secondary service, and the service serves as a
tile layer immediately.

### `GET /.well-known/openeo`

Version discovery.

```sh
curl -s http://localhost:8080/.well-known/openeo
```

```json
{
  "versions": [
    { "api_version": "1.2.0", "production": false, "url": "http://localhost:8080/" }
  ]
}
```

### `GET /collections` and `GET /collections/{collection_id}`

One STAC collection per dataset (datacube extension: spatial/temporal
dimensions plus the band vocabulary a graph may load). The list wraps the
same documents with a `links` array; unknown ids are the standardized
`CollectionNotFound` (404). The **temporal extent is derived from
ingested granules** (min/max acquisition datetime, ADR 0015 — how a
client learns what `datetime=` values are askable; here the stack's one
ingested granule): `[null, null]` means no granule yet. The spatial
extent remains a whole-world placeholder (its derivation is deferred —
`docs/ROADMAP.md` row 15, Records trigger).

```sh
curl -s http://localhost:8080/collections/hls-s30
```

```json
{
  "cube:dimensions": {
    "bands": { "type": "bands", "values": ["b02", "b03", "b04", "b8a"] },
    "t": { "extent": ["2024-06-06T17:54:00Z", "2024-06-06T17:54:00Z"], "type": "temporal" },
    "x": { "axis": "x", "extent": [-180.0, 180.0], "reference_system": 4326, "type": "spatial" },
    "y": { "axis": "y", "extent": [-90.0, 90.0], "reference_system": 4326, "type": "spatial" }
  },
  "description": "Harmonized Landsat Sentinel-2, S30 product.",
  "extent": {
    "spatial": { "bbox": [[-180.0, -90.0, 180.0, 90.0]] },
    "temporal": { "interval": [["2024-06-06T17:54:00Z", "2024-06-06T17:54:00Z"]] }
  },
  "id": "hls-s30",
  "license": "CC0-1.0",
  "links": [
    { "href": "http://localhost:8080/collections/hls-s30", "rel": "self", "type": "application/json" },
    { "href": "http://localhost:8080/", "rel": "root", "type": "application/json" },
    { "href": "http://localhost:8080/", "rel": "parent", "type": "application/json" }
  ],
  "stac_extensions": ["https://stac-extensions.github.io/datacube/v2.2.0/schema.json"],
  "stac_version": "1.1.0",
  "summaries": {},
  "title": "HLS Sentinel-2 (S30)",
  "type": "Collection"
}
```

### `GET /processes`

The supported subset of openeo-processes 1.2.0 — the pinned official
definitions with a **Swath profile** note appended to each description
where v0 narrows the spec. Currently:

`add`, `array_element`, `divide`, `linear_scale_range`,
`load_collection`, `multiply`, `ndvi`, `reduce_dimension`,
`save_result`, `subtract`.

One entry, abridged (the full response is ~29 KB of the official
definitions):

```json
{
  "processes": [
    {
      "id": "ndvi",
      "summary": "Normalized Difference Vegetation Index",
      "description": "Computes the Normalized Difference Vegetation Index (NDVI). … **Swath profile:** `target_band` must be omitted or null (the bands dimension is dropped; the result is gray)."
    }
  ],
  "links": [
    { "href": "http://localhost:8080/processes", "rel": "self", "type": "application/json" }
  ]
}
```

### `POST /result`

The preview: the openEO synchronous-execute endpoint, implemented as a
**preview-bounded subset** ([ADR 0014](decisions/0014-preview-bounded-sync-result.md)
— added by #170). The spec-shaped body (`{"process": {"process_graph": …}}`)
compiles through the exact `POST /services` path — same narrowing, same
typed diagnostics — and answers **one** small overview-backed `image/png`
render covering the graph's `spatial_extent` (the referenced collection's
extent when null). Nothing is persisted: no service, no catalog write, no
trace-bus event. Two debug headers (not part of the openEO contract):
`x-swath-trace` summarizes the render, and `x-swath-preview-tile` names
the rendered tile — the address a published service would serve the
identical bytes under.

```sh
curl -s -D - -o preview.png -X POST http://localhost:8080/result \
  -H 'content-type: application/json' \
  --data '{
    "process": { "process_graph": {
      "load": { "process_id": "load_collection", "arguments": {
        "id": "hls-s30",
        "spatial_extent": { "west": -105.537, "south": 39.1954, "east": -105.3581, "north": 39.3345 },
        "temporal_extent": null,
        "bands": ["b8a", "b04"] } },
      "ndvi": { "process_id": "ndvi", "arguments": {
        "data": { "from_node": "load" }, "nir": "b8a", "red": "b04" } },
      "save": { "process_id": "save_result", "arguments": {
        "data": { "from_node": "ndvi" }, "format": "png" }, "result": true }
    } }
  }'
```

```
HTTP/1.1 200 OK
content-type: image/png
x-swath-trace: {"decision":"overview","bytes_read":139581,"total_ms":152,"ingest_to_pixel_ms":33441}
x-swath-preview-tile: 7/48/26
content-length: 883
```

Compile failures answer the same registry codes as `POST /services`
(`ProcessGraphInvalid`, `ProcessParameterInvalid`, …) — identical codes
for identical graphs on either endpoint. A body with no graph:

```sh
curl -s -X POST http://localhost:8080/result \
  -H 'content-type: application/json' --data '{"process":{}}'
```

```json
{"code":"ProcessGraphMissing","message":"Invalid process specified. It doesn't contain a process graph."}
```

When the preview's live estimate exceeds the bounded budget and no
overview can serve it, the server refuses with the spec's
`ProcessGraphComplexity` (400) — refusal over degradation; it never
silently renders a different extent than requested.

### `GET /service_types`

```sh
curl -s http://localhost:8080/service_types
```

```json
{
  "xyz": {
    "configuration": {
      "tile_size": {
        "default": 256,
        "description": "Tile side length in pixels.",
        "enum": [256, 512],
        "type": "integer"
      }
    },
    "description": "The published process graph served as live map tiles from the OGC API - Tiles endpoint. The service URL is a tile template ({z}/{y}/{x} — OGC order: tileMatrix/tileRow/tileCol).",
    "process_parameters": [],
    "title": "XYZ tiled web map (slippy map)"
  }
}
```

### `POST /services`

Publish a process graph as a live tile layer. Body: `type: "xyz"`
(case-insensitive), a `process` object carrying the `process_graph`,
optional `title`/`description`, optional
`configuration: {"tile_size": 256|512}`. The graph must load a known
collection with dataset band names and end in `save_result` (PNG); the
compile errors are the spec's registry codes (`ProcessGraphInvalid`,
`ProcessParameterInvalid`, …) as 400s.

```sh
curl -s -D - -X POST http://localhost:8080/services \
  -H 'content-type: application/json' \
  --data '{
    "title": "NDVI (published)",
    "type": "xyz",
    "process": { "process_graph": {
      "load": { "process_id": "load_collection", "arguments": {
        "id": "hls-s30", "spatial_extent": null, "temporal_extent": null,
        "bands": ["b8a", "b04"] } },
      "ndvi": { "process_id": "ndvi", "arguments": {
        "data": { "from_node": "load" }, "nir": "b8a", "red": "b04" } },
      "save": { "process_id": "save_result", "arguments": {
        "data": { "from_node": "ndvi" }, "format": "png" }, "result": true }
    } }
  }'
```

```
HTTP/1.1 201 Created
location: http://localhost:8080/services/xyz-3913a754300a
openeo-identifier: xyz-3913a754300a
content-length: 0
```

The service serves on the next tile request
(`GET /tilesets/xyz-3913a754300a/tiles/12/1561/848` → `200 image/png`),
appears in the tilesets list, and survives restarts (persisted on the
dataset's catalog document).

### `GET /services`

```sh
curl -s http://localhost:8080/services
```

```json
{
  "links": [
    { "href": "http://localhost:8080/services", "rel": "self", "type": "application/json" }
  ],
  "services": [
    {
      "description": "",
      "enabled": true,
      "id": "xyz-3913a754300a",
      "title": "NDVI (published)",
      "type": "xyz",
      "url": "http://localhost:8080/tilesets/xyz-3913a754300a/tiles/{z}/{y}/{x}"
    }
  ]
}
```

### `GET /services/{service_id}` and `DELETE /services/{service_id}`

The full service document (the stored process graph included), and
removal — `204 No Content` on delete, after which the layer stops
serving; unknown ids are the standardized `ServiceNotFound` (404).

```sh
curl -s http://localhost:8080/services/xyz-3913a754300a
```

```json
{
  "attributes": {},
  "configuration": { "tile_size": 256 },
  "description": "",
  "enabled": true,
  "id": "xyz-3913a754300a",
  "process": {
    "process_graph": {
      "load": { "arguments": { "bands": ["b8a", "b04"], "id": "hls-s30", "spatial_extent": null, "temporal_extent": null }, "process_id": "load_collection" },
      "ndvi": { "arguments": { "data": { "from_node": "load" }, "nir": "b8a", "red": "b04" }, "process_id": "ndvi" },
      "save": { "arguments": { "data": { "from_node": "ndvi" }, "format": "png" }, "process_id": "save_result", "result": true }
    }
  },
  "title": "NDVI (published)",
  "type": "xyz",
  "url": "http://localhost:8080/tilesets/xyz-3913a754300a/tiles/{z}/{y}/{x}"
}
```

```sh
curl -s -X DELETE -o /dev/null -w '%{http_code}\n' http://localhost:8080/services/xyz-3913a754300a
```

```
204
```
