# Swath endpoint reference

Every route the `swath serve` process mounts. Examples were **captured
from the local fixture stack** (`tests/e2e/stack-up.sh`, base URL
`http://localhost:8080`) and are abridged; the running server is the
full reference. Configuration behind these routes: [`CONFIG.md`](CONFIG.md);
operational semantics: [`OPERATIONS.md`](OPERATIONS.md).

The route tables live in code at
[`crates/swath-api/src/routes.rs`](../crates/swath-api/src/routes.rs)
(OGC + control plane, always mounted),
[`granules.rs`](../crates/swath-api/src/granules.rs) and
[`openeo.rs`](../crates/swath-api/src/openeo.rs) (both merged beside the
OGC router **in catalog mode only** — static/fixtures serving has no
catalog, so those routes simply do not exist there).

Conventions, once: every route is `GET` unless noted (axum answers
`HEAD` from the same handlers); OGC-side errors are RFC 7807 problem
documents, openEO-side errors the spec's `{"code","message"}` format
with registry codes; CORS headers apply to every route when configured
and are absent by default (ADR 0011, [`OPERATIONS.md`](OPERATIONS.md)
§6).

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

**`GET /`** — the OGC landing page; in catalog mode the same JSON additionally
carries the openEO capabilities fields (both standards claim the root, so the
root speaks both). With the embedded UI, a browser `Accept` listing
`text/html` receives the viewer's `index.html`; the JSON stays byte-identical
for every other client.

**`GET /conformance`** — the honesty rule: a class is listed only when its
requirements are met and smoke-tested; `conformsTo` lists exactly the five
implemented ogcapi-tiles-1 classes (`core`, `tileset`, `tilesets-list`,
`dataset-tilesets`, `png`).

**`GET /tiles` and `GET /tilesets`** — the tilesets list, one entry per served
layer (published openEO services appear too, under their `xyz-…` id); `/tiles`
is the path OGC 20-057 requires on the dataset root, `/tilesets` the canonical
collection — same handler, same document. Entries carry `title`,
`dataType: "map"`, `crs` (EPSG:3857), `tileMatrixSetURI` (WebMercatorQuad),
and links.

**`GET /tilesets/{layerId}`** — tileset metadata; the bounding box derives
from the layer's *resolved* assets, so a catalog-backed layer with no granules
yet is 404 here while its identity still appears in the list. Catalog-backed
layers also carry a `granules` link — the backing dataset's granule listing,
whose acquisition datetimes are exactly the frames `datetime=` can select
(ADR 0015): how the viewer's time slider (#182) discovers a layer's temporal
domain. Static layers are a single timeless frame and carry no such link.

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
```

The same request again (with a cache configured) is a hit:
`{"decision":"cache_hit","bytes_read":0,"total_ms":3}`.

**`datetime=` — the time dimension (ADR 0015).** One optional query
parameter selects **which granule backs the frame**, with exactly the
OGC API grammar (Features/Common Part 2, the grammar EDR reuses): an
RFC 3339 UTC instant (`?datetime=2024-08-01T00:00:00Z`) or an interval
(`start/end`, either side openable as `..`, never both). An instant
resolves **latest-at-or-before**; an interval resolves to the latest
granule within it (inclusive); absent means plain latest, byte-identical
to a request without the parameter. Frames cache under the granule they
*resolved to*: two spellings selecting the same granule share one cache
entry, different granules never collide. The SSE trace carries the
`temporal` decision (resolved granule id + datetime, the raw request,
the rule). Static layers are a single timeless frame: the grammar is
validated, then every valid value serves that frame. Askable times are
served on the collection document (the derived temporal extent, below).
A malformed value is a 400 naming the grammar; a window selecting no
granule is the same honest 404 shape as "no granule ingested yet".

Error taxonomy (OGC 20-057 allows 404 or 400 for out-of-range; Swath's
split): addresses that *cannot exist* are 404 — an unknown layer, an
out-of-matrix row/col, ``tileMatrix `25` is not a WebMercatorQuad tile
matrix (expected 0..=24)``; malformed ones are 400 — ``tileRow `x` is
not an integer``. A non-PNG `Accept` (e.g. `application/json`) is 406.

**`GET /traces`** — the x-ray SSE stream (`text/event-stream`, `curl -N`):
one `trace` event per render from connection time on, with the full trace the
header only summarizes — sources, byte-range provenance, stage timings, the
planner's considered strategies with reasons, and (catalog-backed renders,
ADR 0015) the `temporal` decision. Slow subscribers lose events (`lagged`),
never delay a tile.

**`GET /healthz`** — liveness only, `200 ok`; no registry, store, or catalog
I/O ([`OPERATIONS.md`](OPERATIONS.md) §5).

**The fallback** — `GET`/`HEAD` on any unrouted path is looked up in the
embedded web bundle (hashed assets under `/assets/…`); API routes structurally
outrank the bundle, and there is no SPA fallback — unknown paths return the
plain empty 404 they always did.

## Granule browsing (catalog mode)

### `GET /datasets/{datasetId}/granules`

What has been ingested, newest first (acquisition datetime descending,
ties by id — a stable total order). Query parameters: `bbox` (CRS84,
footprint intersection), `datetime` (instant or interval, RFC 3339 UTC,
inclusive), `limit` (1..=1000, default 100), `offset`; a `next` link
appears while more pages remain. Unknown dataset → 404; malformed
parameter → 400; no matches → an empty 200 page. Asset `href`s are store
keys, never serving-host paths; rows carry `id`, `bbox`, `datetime`,
`ingestedAt`, and the `assets` map, wrapped with
`numberMatched`/`numberReturned` and `links`.

## openEO surface (catalog mode)

The authoring loop of ADR 0010: read collections and processes, publish a
process graph as an `xyz` secondary service, and the service serves as a
tile layer immediately.

**`GET /.well-known/openeo`** — version discovery: one entry,
`api_version 1.2.0`, pointing at the root.

### `GET /collections` and `GET /collections/{collection_id}`

One STAC collection per dataset (datacube extension: spatial/temporal
`cube:dimensions` plus the band vocabulary a graph may load). The list
wraps the same documents with a `links` array; unknown ids are the
standardized `CollectionNotFound` (404). The **temporal extent is
derived from ingested granules** (min/max acquisition datetime,
ADR 0015 — how a client learns what `datetime=` values are askable):
`[null, null]` means no granule yet. The spatial extent remains a
whole-world placeholder (its derivation is deferred — `docs/ROADMAP.md`
row 15, Records trigger).

### `GET /processes`

The supported subset of openeo-processes 1.2.0 — the pinned official
definitions with a **Swath profile** note appended where v0 narrows the
spec. Currently: `add`, `array_element`, `divide`, `filter_temporal`,
`linear_scale_range`, `load_collection`, `multiply`, `ndvi`,
`reduce_dimension`, `save_result`, `subtract`. Temporal arguments are
real since ADR 0015: `temporal_extent` and `filter_temporal` compile
into the published layer's granule-resolution window (frame selection,
never how pixels combine); a window excluding every ingested granule
makes `POST /result` answer 404 `NotFound`, and a window that can never
select anything is 400 `ProcessParameterInvalid`.

### `POST /result`

The preview: the openEO synchronous-execute endpoint, implemented as a
**preview-bounded subset** ([ADR 0014](decisions/0014-preview-bounded-sync-result.md)
— added by #170). The spec-shaped body (`{"process": {"process_graph": …}}`)
compiles through the exact `POST /services` path — same narrowing, same
typed diagnostics — and answers **one** small overview-backed `image/png`
render covering the graph's `spatial_extent` (the referenced
collection's extent when null). Nothing is persisted. Two debug headers
(not part of the openEO contract): `x-swath-trace` summarizes the
render, and `x-swath-preview-tile` names the rendered tile — the address
a published service would serve the identical bytes under. Compile
failures answer the same registry codes as `POST /services`; a body with
no graph is `ProcessGraphMissing`. When the preview's live estimate
exceeds the bounded budget and no overview can serve it, the server
refuses with the spec's `ProcessGraphComplexity` (400) — refusal over
degradation; it never silently renders a different extent than
requested.

### `GET /service_types`

One type: `xyz` — "the published process graph served as live map tiles
from the OGC API - Tiles endpoint", with a `tile_size` configuration
(integer, 256 or 512, default 256). The service URL is a tile template
(`{z}/{y}/{x}` — OGC order).

### `POST /services`

Publish a process graph as a live tile layer. Body: `type: "xyz"`
(case-insensitive), a `process` object carrying the `process_graph`,
optional `title`/`description`, optional
`configuration: {"tile_size": 256|512}`. The graph must load a known
collection with dataset band names and end in `save_result` (PNG); the
compile errors are the spec's registry codes (`ProcessGraphInvalid`,
`ProcessParameterInvalid`, …) as 400s. A worked full-body example (NDVI,
published over curl) is in [`QUICKSTART.md`](QUICKSTART.md) Track 2. The
response is `201 Created` with `location` and `openeo-identifier`
headers naming the assigned `xyz-…` id; the service serves on the next
tile request, appears in the tilesets list, and survives restarts
(persisted on the dataset's catalog document).

### `GET /services`, `GET /services/{service_id}`, `DELETE /services/{service_id}`

The list (`services` rows: id, title, type, `enabled`, the tile-template
`url`), the full service document (the stored process graph,
`configuration`, `attributes`), and removal — `204 No Content` on
delete, after which the layer stops serving; unknown ids are the
standardized `ServiceNotFound` (404).
