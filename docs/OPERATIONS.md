# Swath operations guide

How to run and observe the `swath` binary in production-shaped deployments.
Companions: [`CONFIG.md`](CONFIG.md) (every knob) and
[`ENDPOINTS.md`](ENDPOINTS.md) (every route); the compose stack
([`../docker-compose.yml`](../docker-compose.yml)) is the living example.

## 1. The binary and its serving modes

`swath serve` is the single deployable: one process serves OGC API - Tiles,
the trace SSE stream, `/healthz`, the embedded viewer, and — in catalog
mode — granule browsing and openEO authoring. Layers come three ways:

| Mode | Selected by | Layers come from | Assets resolve from |
|---|---|---|---|
| Fixtures | `--fixtures` | built-in HLS demo registry (`truecolor`, `ndvi`) | `./tests/fixtures` |
| Static | `--config` with `[[layers]]` | the config file | fixed asset URIs under the store root |
| Catalog | `catalog` (pgstac URL) with `[[datasets]]` | the config file, per dataset | the dataset's **latest ingested granule**, per tile |

Catalog mode is the production shape: datasets are upserted into pgstac at
startup (config is the source of truth — operators write TOML, never STAC),
granules arrive through ingest, and published openEO services persist on the
dataset and restore on restart (a no-longer-compiling service is dropped
loudly, never served wrongly). Shutdown is graceful on SIGINT/SIGTERM.

## 2. Store backends

`store-root` (and `cache`) accept one grammar: **`s3://bucket[/prefix]`**
(credentials/endpoint from the standard `object_store` AWS environment,
`AWS_ALLOW_HTTP=true` for plain-HTTP endpoints; credentials resolve lazily, so
a misconfigured environment surfaces on first read) — or **anything else**, a
local directory (no third scheme; an unknown URL fails honestly at startup).
The server only ever **reads** from the store root — mount it read-only; the
tile cache is the one thing it writes. Assets are either COGs (byte-range
reads) or virtual-cube manifests (ADR 0006), dispatched per asset.

## 3. The tile cache — behavior and the honest GC deferral

With `cache` configured, serving is **write-through** (a repeat request is a
`cache_hit`); without it, byte-for-byte the cache-less behavior. Keys are
**content-derived** (ARCHITECTURE.md §10), the layer version derived from the
serving inputs — invalidation needs no machinery: a new granule or edited
layer changes the version and old keys stop being asked for. Entries are
immutable under their key; the cache carries no TTL.

**The deferral, stated honestly: there is no garbage collection.** Superseded
versions' objects are *orphaned* — never stale, only unreachable — and linger;
GC is deliberate future work on the deferral inventory in `docs/ROADMAP.md`
(row 2). Until it lands: **deleting the cache is always safe** (every entry is
a pure function of its key; an S3 lifecycle rule is the recommended stand-in),
and **bound growth at the source** with `budget.cache-enabled = false` on
fast-changing layers. Cache failures are policy'd for availability: a failed
read is a miss, a failed write a logged warning — never a failed tile.
Related knobs ([`CONFIG.md`](CONFIG.md)): `overview-oversample`,
`max-estimated-live-bytes`.

## 4. Events sources — how granules arrive

The one built-in event source is the **filedrop watcher** (`watch-dir`,
catalog mode only): scanned every 250 ms, each `<granule-id>.json` manifest is
registered against its (pre-existing) dataset, stamped with its arrival time,
and immediately eligible to serve — the ingest-to-pixel path the e2e gate
measures. Ingest errors are logged and never stop the loop.

The **legacy path** rides the same drop: a granule with legacy assets
(`.h5`/`.nc`/`.grib2`) gets a virtual-reference manifest generated
automatically and stored alongside (ADR 0006; also manual via
`swath ingest reference <granule>`). Referencing reads local bytes, so it
lights up only for a **local** store root; with an `s3://` root the server
warns at startup and refuses legacy granules honestly. Queue-backed event
sources are a port (`EventSource`) with one adapter today
([`EXTENDING.md`](EXTENDING.md)).

## 5. Observability — what the endpoints tell you

**`GET /healthz`** — plain `200 ok`, **liveness only** (no registry, store,
or catalog I/O; no readiness probe yet — an outage shows as failing
data-plane requests). **`GET /traces`** — the x-ray SSE stream: the full
render trace per tile; slow subscribers lose events (`lagged`), never delay a
tile; every tile response carries a one-line `x-swath-trace` summary
([`ENDPOINTS.md`](ENDPOINTS.md)). **Logs** — single-line `tracing` on stdout,
`SWATH_LOG` selecting the level; startup names the bound address, store root,
layers, cache, CORS, and watch dir; the ingest loop logs every granule's
latency.

![Trace analytics in the viewer: rolling p50/p95 render latency, plan mix, and cache hit rate over the session's tiles — the /traces stream, consumed.](media/screenshots/12-analytics-under-load.png)

## 6. CORS and serving the UI

**Same-origin is the default story and needs no configuration.** The binary
embeds the web viewer (`embedded-ui`, default-on): browsers get `index.html`
at `/` through content negotiation, hashed assets serve from the fallback, API
clients see byte-identical JSON, and there is no SPA fallback. A build without
`web/dist` degrades honestly to serving no UI. **CORS is opt-in and off by
default** (ADR 0011): no CORS headers unless `cors-allowed-origins` is set —
exact origins or `*` (a dev convenience); the origin list is the whole policy
(methods/headers mirror the request, credentials stay off).

## 7. Deployment checklists

Minimal static serve: `swath serve --store-root /data --config swath.toml`
(`[[layers]]` in the file). Catalog mode (the compose shape), pgstac up first:

```sh
SWATH_CATALOG=postgres://user:pass@host:5432/db \
swath serve --config swath.toml --watch-dir /data/drop --cache /cache
```

Bind is loopback by default — set `bind = "0.0.0.0:8080"` explicitly to serve
beyond the host; set `base-url` to the externally reachable URL; mount the
store root read-only with a separate writable cache root; point the
healthcheck at `/healthz`. Startup fails loudly on config errors (unknown
TOML keys included) — [`CONFIG.md`](CONFIG.md).

**Public read-only demo** — the exercised, provider-agnostic recipe in
[`deploy/README.md`](deploy/README.md): compose + Traefik (ACME TLS, per-IP
rate limits, a preview body cap), `swath serve --read-only` with the `run_udf`
module store, seeded from the fixtures the image already carries.
