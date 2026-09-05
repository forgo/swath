# Swath endpoint reference

Every route the `swath serve` process mounts; examples were **captured
from the local fixture stack** (`tests/e2e/stack-up.sh`) and abridged —
the running server is the full reference. Configuration:
[`CONFIG.md`](CONFIG.md); operations: [`OPERATIONS.md`](OPERATIONS.md).

The route tables live in code at
[`crates/swath-api/src/routes.rs`](../crates/swath-api/src/routes.rs)
(OGC + control plane, always mounted),
[`granules.rs`](../crates/swath-api/src/granules.rs) and
[`openeo/mod.rs`](../crates/swath-api/src/openeo/mod.rs) (both merged beside the
OGC router **in catalog mode only** — static/fixtures serving has no
catalog, so those routes do not exist there). Conventions: every route
is `GET` unless noted (axum answers `HEAD` from the same handlers);
OGC-side errors are RFC 7807 problem documents, openEO-side the spec's
registry-coded `{"code","message"}`; CORS headers apply when configured,
absent by default (ADR 0011).

## Route table

The block below is mechanically checked against the axum routers by the
docs gate (`tools/docs-check/src/check/routes.rs`): every mounted
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
| GET | `/traces` | always | SSE stream: every render (`trace`) and every source event (`ingest`, #416) |
| GET | `/healthz` | always | Liveness probe (process only) |
| GET/HEAD | *fallback* | always | Embedded UI assets; unknown paths are plain 404 |
| GET | `/sources` | catalog mode | Origins and their measured status (#417) |
| GET | `/sources/{sourceId}` | catalog mode | One origin and its measured status |
| GET | `/sources/register` | catalog mode | Endpoints offered for import, with what the allowlist permits (#420) |
| GET | `/datasets/{datasetId}/counts` | catalog mode | Matched granules bucketed by calendar step or CRS84 cell (#410) |
| GET | `/datasets/{datasetId}/facets` | catalog mode | What the granules in scope carry: discovered property keys with coverage, ranges and value counts (#409) |
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

Under `swath serve --read-only` (#198) the write routes — `POST /datasets`,
`POST /datasets/{datasetId}/granules`, `POST /services`,
`DELETE /services/{service_id}` — are **unmounted** (absent, not 403), the
capabilities document reflects it, and `POST /result` deliberately remains:
the preview is planner-budget-bounded by design (ADR 0014).

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
(ADR 0015; how the time slider, #182, discovers a layer's temporal domain),
plus `swath:window` — the compiled frame-selection window, the hull of the
branch windows for a two-source layer (ADR 0022) — and `swath:sources`, the
branch count; the slider offers only frames inside the window. Static
layers are a single timeless frame with none of these.

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
the granule's arrival time is known) is the north-star number; a `run_udf`
render adds its deterministic `udf_fuel_used` (ADR 0018).

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
(catalog-backed renders, ADR 0015) the `temporal` decision, and (UDF renders,
ADR 0018) `udf_ms` plus the deterministic `udf_fuel_used`. Slow
subscribers lose events (`lagged`), never delaying a tile.

The stream also carries `ingest` events — one per thing that happened to a
source (ADR 0030): `{"source","at","event","detail","coalesced"}`. `at` is the
**server's** instant, so freshness comes from when the thing happened, not a
client's clock. Routine events are coalesced to one per source per second and
the suppressed count rides on the next one; a failure is never suppressed. A
source with no events has none here, and the UI shows an em dash.

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
paths; rows carry `id`, `bbox`, `datetime`, `ingestedAt`, `assets`, and
`properties` — every other STAC property the item arrived with, verbatim and
opaque (ADR 0029), omitted when empty — wrapped with
`numberMatched`/`numberReturned` and `links`.

### `GET /datasets/{datasetId}/facets`

What the granules in scope actually carry. Keys are discovered from the
items, never from a fixed list: a key appears only because some granule
has it, so a control built from this response always has data behind it.
`bbox` and `datetime` scope the discovery as they scope the granule page;
`limit`/`offset` do not apply — this aggregates the whole match.

Each facet carries `key`, a `kind` (`number`, `string`, `boolean`, or
`other` for objects, arrays and mixed keys), and `coverage`: how many of
`total` granules carry the key, which keeps "no value" distinguishable
from "the value is zero". Numbers report `min`/`max`; strings and
booleans report `values` (distinct, most common first) and set
`truncated` past 25. An `other` facet claims only its coverage. Same
taxonomy: unknown dataset → 404, malformed parameter → 400.

### `GET /datasets/{datasetId}/counts`

How many granules match, bucketed — the question the timeline and the
density overlay both ask. `by=time&step=hour|day|week|month|year` counts
acquisition instants into calendar buckets, which partition the scope
exactly, so the counts sum to `total`. `by=cell&size=<degrees>` counts
footprints into a CRS84 lattice anchored at (-180, -90); a footprint
spanning several cells counts in each, so `overlapping` is true and the
buckets sum to at least `total`. Empty buckets are omitted — an absent
bucket is a zero.

`bbox` and `datetime` scope the count as they scope the granule page, and
`total` is the same number that scope's `numberMatched` reports. Cost is
one full scan per request, linear in matched rows. A bucketing that would
return more than 2000 buckets is refused with a 400 naming the number and
the way out, rather than answered slowly.

## Sources (catalog mode)

**`GET /sources`**, **`GET /sources/{sourceId}`** — what each origin is and
how it is doing (ADR 0030). Each row carries `id`, `title`, `kind`, `origin`
(`config` or `api`, so a config-owned source cannot look editable), the
datasets it feeds, the credential profile's **name** when there is one, whether it
`credentialResolved` (`null` = named but never checked; absent = no profile),
`requesterPays` and `consentedBy` when the source bills the reader, and a
`status`. No money figure appears anywhere: Swath cannot know one.

Every field of `status` is measured, derived from the recorded events: `state`,
`reachable` (`null` until something has looked — never a reassuring default),
`lastEvent`, `lastError` while a failure is still the last word, and the
`ingested`/`failures` counts. The probe behind it reads the target within a
5-second timeout every 30 seconds.

Two things are deliberately absent: the target **path** (only its `scheme` is
served — host paths do not leave the process, as with asset hrefs) and any
secret (there is no field one could occupy). **`GET /sources/register`** — the endpoints this deployment offers to import
from (#420), each with its `host`, whether the egress allowlist `allowed` it,
and `requesterPays`. `federationOff` says once when the allowlist is empty.
The register comes from the config, so adding an entry is an edit and a
restart rather than a release. Nothing is fetched here: an entry is an offer.

Read-only, and the mutating routes are
**absent rather than forbidden** until OIDC/RBAC lands (ADR 0031) — there is
no handler to authorise, which a middleware mistake cannot undo.

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
`linear_scale_range`, `load_collection`, `merge_cubes`, `multiply`, `ndvi`,
`reduce_dimension`, `save_result`, `subtract` — plus `run_udf` (ADR 0018)
exactly where `--udf-store` wires a module store: the module (inline
`data:` or an `http(s)` URL fetched once) is validated at publish time
and persisted by content hash. Temporal arguments are
real since ADR 0015: they compile into the layer's granule-resolution
window (frame selection, never how pixels combine); a window excluding
every granule 404s at `POST /result`, one that can never select
anything is 400 `ProcessParameterInvalid`. `merge_cubes` (ADR 0022) joins
two gray branches of one collection — two `load_collection` nodes, one
granule each — through a required `overlap_resolver`; the arithmetic
processes are admitted inside a reducer or a resolver.

### `POST /result`

The preview: the openEO synchronous-execute endpoint as a
**preview-bounded subset** ([ADR 0014](decisions/0014-preview-bounded-sync-result.md),
#170). The spec-shaped body compiles through the exact `POST /services`
path — same narrowing, same diagnostics — and answers **one** small
overview-backed `image/png` render covering the graph's
`spatial_extent` (null: the rendered granule's footprint — every branch's,
joined, for a two-source graph); nothing is persisted. Debug headers: `x-swath-trace`, and `x-swath-preview-tile`
naming the tile a published service serves the identical bytes under.
Compile failures answer the same registry codes as `POST /services`; a
live estimate over the bounded budget with no overview to serve it is
`ProcessGraphComplexity` (400) — refusal over degradation. A `run_udf`
graph previews under the per-tile fuel budget publishing enforces
(ADR 0018) — the validation loop: out of fuel or time is
`ProcessGraphComplexity`; a trap, declared failure, or malformed output
is `ProcessParameterInvalid` with the executor's diagnosis (400,
user-fixable).

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
