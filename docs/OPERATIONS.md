# Swath operations guide

How to run and observe the `swath` binary in production-shaped deployments.
Companion references: [`CONFIG.md`](CONFIG.md) (every flag, env var, and TOML
key — mechanically verified against the code) and [`ENDPOINTS.md`](ENDPOINTS.md)
(every HTTP route, with captured examples). Architecture background:
[`ARCHITECTURE.md`](ARCHITECTURE.md); the local compose stack
([`../docker-compose.yml`](../docker-compose.yml)) is the living example of
everything below.

## 1. The binary and its serving modes

`swath serve` is the single self-contained deployable: one process serves
OGC API - Tiles, the trace SSE stream, the liveness probe, the embedded viewer
UI, and — in catalog mode — the granule browsing and openEO authoring surfaces.
Three ways to give it layers:

| Mode | Selected by | Layers come from | Assets resolve from |
|---|---|---|---|
| Fixtures | `--fixtures` | built-in HLS demo registry (`truecolor`, `ndvi`) | `./tests/fixtures` |
| Static | `--config` with `[[layers]]` | the config file | fixed asset URIs under the store root |
| Catalog | `catalog` (pgstac URL) with `[[datasets]]` | the config file, per dataset | the dataset's **latest ingested granule**, per tile |

Catalog mode is the production shape: datasets are upserted into pgstac at
startup (config is the source of truth — operators write TOML, never STAC),
granules arrive through ingest, and openEO services published at runtime are
persisted on the dataset and restored on restart (a service whose graph no
longer compiles is dropped loudly, never served wrongly). Shutdown is graceful
on SIGINT/SIGTERM.

## 2. Store backends

`store-root` (and `cache`) accept one grammar: **`s3://bucket[/prefix]`** (any
S3-compatible store; credentials/endpoint from the standard `object_store` AWS
environment, `AWS_ALLOW_HTTP=true` for plain-HTTP endpoints such as local
MinIO; credentials resolve lazily, so a misconfigured environment surfaces on
first read) — or **anything else**, a local directory (no third scheme; an
unknown URL fails honestly at startup). The server only ever **reads** from
the store root — mount it read-only; the tile cache is the one thing it
writes, so give it its own root. Assets are either COGs (byte-range reads) or
virtual-cube manifests (ADR 0006), dispatched per asset.

## 3. The tile cache — behavior and the honest GC deferral

With `cache` configured, serving is **write-through** (a repeat request is a
`cache_hit`); without it, serving is byte-for-byte the cache-less behavior.
Keys are **content-derived** (ARCHITECTURE.md §10), with the layer version
derived from the serving inputs — so invalidation needs no machinery: a new
granule or an edited layer definition changes the version, and old keys simply
stop being asked for. Entries are immutable under their key, which is why the
cache carries no TTL.

**The deferral, stated honestly: there is no garbage collection.** Superseded
versions' objects are *orphaned* — never stale, only unreachable — and linger
until collected. GC is deliberate future operational work, on the deferral
inventory in `docs/ROADMAP.md` (row 2; semantics in `swath-core`'s cache module
and ARCHITECTURE.md §10/§16.3). Until it lands, the mitigations: **deleting the
cache is always safe** (every entry is a pure function of its key; an S3
lifecycle expiration rule on the cache prefix is the recommended stand-in), and
**bound growth at the source** with `budget.cache-enabled = false` on layers
whose data changes often ([`CONFIG.md`](CONFIG.md)).

Cache failures are policy'd for availability: a failed read is a miss, a failed
write a logged warning — never a failed tile response. Related knobs
([`CONFIG.md`](CONFIG.md)): `overview-oversample` and
`max-estimated-live-bytes`.

## 4. Events sources — how granules arrive

The one built-in event source is the **filedrop watcher** (`watch-dir`, catalog
mode only): the directory is scanned every 250 ms, and each `<granule-id>.json`
manifest dropped in is ingested — registered against its (pre-existing)
dataset, stamped with its arrival time, and immediately eligible to serve (the
ingest-to-pixel path the e2e gate measures). Ingest errors are logged and never
stop the loop.

The **legacy path** rides the same drop: a granule whose assets are legacy
files (`.h5`/`.nc`/`.grib2`) gets a virtual-reference manifest generated
automatically and stored alongside (ADR 0006; also available manually as
`swath ingest reference <granule>`). Referencing reads local bytes, so it
lights up only for a **local** store root; with an `s3://` root the server
warns at startup and refuses legacy granules with an honest error.
Queue-backed event sources are a port (`EventSource`) with one adapter today;
see [`EXTENDING.md`](EXTENDING.md).

## 5. Observability — what the endpoints tell you

- **`GET /healthz`** — plain `200 ok`, **liveness only**: it touches no
  registry, store, or catalog, so healthchecks measure the process, not the
  data plane. No readiness probe yet — a catalog or store outage shows up as
  failing data-plane requests, not a failing `/healthz`.
- **`GET /traces`** — the x-ray SSE stream: the full render trace per tile
  (decision, provenance, timings, planner candidates, `ingest_to_pixel_ms`
  when known); slow subscribers lose events (`lagged`), never delay a tile.
  Every tile response carries a one-line `x-swath-trace` summary. Examples in
  [`ENDPOINTS.md`](ENDPOINTS.md).
- **Logs** — single-line `tracing` on stdout; `SWATH_LOG` selects the level
  (default `info`). Startup names the bound address, store root, layers,
  cache, CORS, and watch dir; the ingest loop logs every granule's latency.

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

Minimal static serve:

```sh
swath serve --store-root /data --config swath.toml   # [[layers]] in the file
```

Catalog mode (the compose shape): pgstac up first, then

```sh
SWATH_CATALOG=postgres://user:pass@host:5432/db \
swath serve --config swath.toml --watch-dir /data/drop --cache /cache
```

- Bind is loopback by default — set `bind = "0.0.0.0:8080"` explicitly to serve
  beyond the host.
- Set `base-url` to the externally reachable URL: it is minted into every
  OGC/openEO link.
- Mount the store root read-only; give the cache its own writable root.
- Point the orchestrator healthcheck at `/healthz`.
- Startup fails loudly on config errors (unknown TOML keys included) — see
  [`CONFIG.md`](CONFIG.md).
