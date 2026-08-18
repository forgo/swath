# Swath endpoint reference

Every route the `swath serve` process mounts; examples were **captured
from the local fixture stack** (`tests/e2e/stack-up.sh`) and abridged —
the running server is the full reference. Configuration:
[`CONFIG.md`](CONFIG.md); operations: [`OPERATIONS.md`](OPERATIONS.md).

The route tables live in code at
[`crates/swath-api/src/routes.rs`](../crates/swath-api/src/routes.rs)
(OGC + control plane, always mounted),
[`granules.rs`](../crates/swath-api/src/granules.rs) and
[`openeo.rs`](../crates/swath-api/src/openeo.rs) (both merged beside the
OGC router **in catalog mode only** — static/fixtures serving has no
catalog, so those routes do not exist there). Conventions: every route
is `GET` unless noted (axum answers `HEAD` from the same handlers);
OGC-side errors are RFC 7807 problem documents, openEO-side the spec's
registry-coded `{"code","message"}`; CORS headers apply when configured,
absent by default (ADR 0011).

## Route table

The block below is mechanically checked against the axum routers by the
docs gate (`crates/swath-cli/src/docs_check/routes.rs`): every mounted
route must appear — methods and mounting included — and no phantom rows
survive.

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
| GET, POST | `/datasets/{datasetId}/granules` | catalog mode | Granule browsing (paged, filterable); POST registers one (asset map or inline STAC Item — headers validated, extents derived, #196) |
| POST | `/datasets` | catalog mode | Register a dataset (id, title, bands) — #196; 409 on existing id |
| GET | `/.well-known/openeo` | catalog mode | openEO version discovery |
| GET | `/collections` | catalog mode | openEO/STAC collections (one per dataset) |
| GET | `/collections/{collection_id}` | catalog mode | One collection document |
| GET | `/file_formats` | catalog mode | openEO output formats: PNG only (the ADR 0014 preview) |
| GET | `/processes` | catalog mode | The supported openEO process subset |
| POST | `/result` | catalog mode | Preview: one bounded synchronous render of a process graph (PNG) |
| GET | `/service_types` | catalog mode | Secondary service types (`xyz`) |
| GET, POST | `/services` | catalog mode | List / publish secondary services |
| GET, DELETE | `/services/{service_id}` | catalog mode | Describe / delete one service |
<!-- docs-check:end routes -->

## OGC API - Tiles + control plane

**`GET /`** — the OGC landing page; in catalog mode the same JSON also
carries the openEO capabilities fields (both standards claim the root). A
browser `Accept` listing `text/html` receives the viewer's `index.html`; the
JSON stays byte-identical for every other client.

**`GET /conformance`** — the honesty rule: a class is listed only when met and
smoke-tested; `conformsTo` lists exactly the five implemented ogcapi-tiles-1
classes (`core`, `tileset`, `tilesets-list`, `dataset-tilesets`, `png`).

**`GET /tiles` and `GET /tilesets`** — the tilesets list, one entry per served
layer (published openEO services appear under their `xyz-…` id); `/tiles` is
the path OGC 20-057 requires on the dataset root, `/tilesets` the canonical
collection — same handler, same document. Entries carry `title`, `dataType`,
`crs`, `tileMatrixSetURI` (WebMercatorQuad), and links.

**`GET /tilesets/{layerId}`** — tileset metadata; the bounding box derives
from the layer's *resolved* assets, so a catalog-backed layer with no
granules yet is 404 here while its identity still appears in the list.
Catalog-backed layers also carry a `granules` link — the listing whose
acquisition datetimes are exactly the frames `datetime=` can select
(ADR 0015; how the time slider, #182, discovers a layer's temporal domain);
static layers are a single timeless frame with no such link.

### `GET /tilesets/{layerId}/tiles/{tileMatrix}/{tileRow}/{tileCol}`

The tile. Path order is **OGC order, z/row/col** (`{z}/{y}/{x}`);
`Accept` absent, `*/*`, `image/*`, and `image/png` are acceptable,
anything else 406; off-data tiles inside the matrix are a transparent
200 PNG. The `x-swath-trace` header summarizes the render, readable from
a plain `curl -D`:

```
x-swath-trace: {"decision":"live","bytes_read":546497,"total_ms":327,"ingest_to_pixel_ms":106175}
```

The same request again (with a cache configured) is a hit
(`"decision":"cache_hit","bytes_read":0`); `ingest_to_pixel_ms` (when
the granule's arrival time is known) is the north-star number.

**`datetime=` — the time dimension (ADR 0015).** One optional query
parameter selects **which granule backs the frame**, with exactly the
OGC API grammar (Features/Common Part 2, reused by EDR): an RFC 3339
UTC instant or an interval (`start/end`, either side openable as `..`,
never both). An instant resolves **latest-at-or-before**; an interval,
the latest granule within it (inclusive); absent means plain latest.
Frames cache under the granule they *resolved to* (same granule, one
entry; different granules never collide), and the SSE trace carries the
`temporal` decision (granule id + datetime, raw request, rule). Static
layers are a single timeless frame; askable times are the collection
document's derived temporal extent. A malformed value is a 400 naming
the grammar; a window selecting no granule is the same honest 404 as
"no granule ingested yet".

Error taxonomy (OGC 20-057 allows 404 or 400 for out-of-range; Swath's
split): addresses that *cannot exist* are 404 (unknown layer,
out-of-matrix row/col or tileMatrix); malformed ones are 400
(non-integer row, bad grammar).

**`GET /traces`** — the x-ray SSE stream (`curl -N`): one `trace` event per
render, with the full trace the header only summarizes — sources, byte-range
provenance, stage timings, the planner's considered strategies with reasons,
and (catalog-backed renders, ADR 0015) the `temporal` decision. Slow
subscribers lose events (`lagged`), never delaying a tile.

**`GET /healthz`** — liveness only, `200 ok`; no registry, store, or catalog
I/O ([`OPERATIONS.md`](OPERATIONS.md) §5).

**The fallback** — `GET`/`HEAD` on any unrouted path is looked up in the
embedded web bundle; API routes structurally outrank it, and there is no SPA
fallback — unknown paths return the plain empty 404 they always did.

## Granule browsing (catalog mode)

### `GET /datasets/{datasetId}/granules`

What has been ingested, newest first (acquisition datetime descending,
ties by id). Query parameters: `bbox` (CRS84 footprint intersection),
`datetime` (instant or interval, RFC 3339 UTC, inclusive), `limit`
(1..=1000, default 100), `offset`; a `next` link appears while pages
remain. Unknown dataset → 404; malformed parameter → 400; no matches →
an empty 200 page. Asset `href`s are store keys, never serving-host
paths; rows carry `id`, `bbox`, `datetime`, `ingestedAt`, and `assets`,
wrapped with `numberMatched`/`numberReturned` and `links`.

## openEO surface (catalog mode)

The ADR 0010 authoring loop: read collections and processes, publish a graph
as an `xyz` service, and it serves as a tile layer immediately.

**`GET /.well-known/openeo`** — version discovery: one entry,
`api_version 1.2.0`, pointing at the root.

### `GET /collections` and `GET /collections/{collection_id}`

One STAC collection per dataset (datacube extension: spatial/temporal
`cube:dimensions` plus the band vocabulary a graph may load); unknown
ids are `CollectionNotFound` (404). The **temporal extent derives from
ingested granules** (ADR 0015 — how a client learns which `datetime=`
values are askable; `[null, null]` means no granule yet); the spatial
extent remains a whole-world placeholder (derivation deferred —
`docs/ROADMAP.md` row 15, Records trigger).

### `GET /processes`

The supported subset of openeo-processes 1.2.0 — pinned official
definitions with a **Swath profile** note where v0 narrows the spec:
`add`, `array_element`, `divide`, `filter_temporal`,
`linear_scale_range`, `load_collection`, `multiply`, `ndvi`,
`reduce_dimension`, `save_result`, `subtract`. Temporal arguments are
real since ADR 0015: they compile into the layer's granule-resolution
window (frame selection, never how pixels combine); a window excluding
every granule 404s at `POST /result`, one that can never select
anything is 400 `ProcessParameterInvalid`.

### `POST /result`

The preview: the openEO synchronous-execute endpoint as a
**preview-bounded subset** ([ADR 0014](decisions/0014-preview-bounded-sync-result.md),
#170). The spec-shaped body compiles through the exact `POST /services`
path — same narrowing, same typed diagnostics — and answers **one**
small overview-backed `image/png` render covering the graph's
`spatial_extent` (the collection's extent when null); nothing is
persisted. Two debug headers: `x-swath-trace`, and
`x-swath-preview-tile` naming the tile a published service would serve
the identical bytes under. Compile failures answer the same registry
codes as `POST /services`; a live estimate over the bounded budget with
no overview to serve it is `ProcessGraphComplexity` (400) — refusal
over degradation, never silently rendering a different extent.

### `GET /service_types`

One type: `xyz` — "the published process graph served as live map tiles
from the OGC API - Tiles endpoint", with a `tile_size` configuration
(integer, 256 or 512, default 256). The service URL is a tile template
(`{z}/{y}/{x}` — OGC order).

### `POST /services`

Publish a process graph as a live tile layer. Body: `type: "xyz"`, a
`process` object carrying the `process_graph`, optional
`title`/`description`, optional `configuration: {"tile_size": 256|512}`.
The graph must load a known collection with dataset band names and end
in `save_result` (PNG); compile errors are the spec's registry codes as
400s (a full graph is exercised by
`crates/swath-api/tests/openeo_services.rs`). The response is
`201 Created` with `location` and `openeo-identifier` naming the
assigned `xyz-…` id; the service serves on the next tile request,
appears in the tilesets list, and survives restarts (persisted on the
dataset's catalog document).

### `GET /services`, `GET /services/{service_id}`, `DELETE /services/{service_id}`

The list (id, title, type, `enabled`, the tile-template `url`), the full
service document (the stored graph, `configuration`, `attributes`), and
removal — `204 No Content` on delete, after which the layer stops
serving; unknown ids are `ServiceNotFound` (404).
