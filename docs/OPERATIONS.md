# Swath operations guide

How to run and observe the `swath` binary in production-shaped
deployments. Companion references: [`CONFIG.md`](CONFIG.md) (every flag,
env var, and TOML key — mechanically verified against the code) and
[`ENDPOINTS.md`](ENDPOINTS.md) (every HTTP route, with captured
examples). Architecture background lives in
[`ARCHITECTURE.md`](ARCHITECTURE.md); the local compose stack in
[`../docker-compose.yml`](../docker-compose.yml) is the living example of
everything below.

## 1. The binary and its serving modes

`swath serve` is the single self-contained deployable: one process serves
OGC API - Tiles, the trace SSE stream, the liveness probe, the embedded
viewer UI, and — in catalog mode — the granule browsing and openEO
authoring surfaces. There are three ways to give it layers:

| Mode | Selected by | Layers come from | Assets resolve from |
|---|---|---|---|
| Fixtures | `--fixtures` | built-in HLS demo registry (`truecolor`, `ndvi`) | `./tests/fixtures` |
| Static | `--config` with `[[layers]]` | the config file | fixed asset URIs under the store root |
| Catalog | `catalog` (pgstac URL) with `[[datasets]]` | the config file, per dataset | the dataset's **latest ingested granule**, per tile |

Catalog mode is the production shape: datasets are upserted into pgstac
at startup (config is the source of truth for dataset identity — operators
write TOML, never STAC), granules arrive through ingest, and openEO
services published at runtime are persisted on the dataset and restored
on restart (a service whose graph no longer compiles against the dataset
is dropped loudly at startup, never served wrongly).

Shutdown is graceful on SIGINT and SIGTERM (the container stop signal):
the listener stops accepting and in-flight requests drain.

## 2. Store backends

`store-root` (and `cache`) accept one grammar, built by the same store
builder:

- **`s3://bucket[/prefix]`** — any S3-compatible object store.
  Credentials and endpoint come from the standard `object_store` AWS
  environment: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
  `AWS_DEFAULT_REGION` (or `AWS_REGION`), `AWS_ENDPOINT`, and
  `AWS_ALLOW_HTTP=true` for plain-HTTP endpoints such as local MinIO.
  Credentials are resolved lazily (per request), so a misconfigured
  environment surfaces on first read, not at startup.
- **anything else** — a local directory. There is no third scheme: a URL
  like `memory://x` falls through to the local branch and fails honestly
  at startup (`cannot open object store at ...`).

The server only ever **reads** from the store root — the data plane can
be mounted read-only (the compose stack does). The tile cache is the one
thing it writes; give it its own root.

Assets under the store root are either COGs (read via HTTP-style byte
ranges) or virtual-cube manifests (ADR 0006): legacy granules
(HDF5/NetCDF4, GRIB2) rendered from chunk-range reads into their original
files, dispatched per asset by the composite source.

## 3. The tile cache — behavior and the honest GC deferral

With `cache` configured, serving is **write-through**: the tile handler
consults the cache before rendering and writes fresh renders through; a
repeat request is a `cache_hit` (visible in the `x-swath-trace` header
and the trace stream). Without it, no cache is consulted and serving is
byte-for-byte the cache-less behavior.

Keys are **content-derived** (SHA-256 over layer id, layer version, the
render plan's canonical JSON, the tile matrix set, the tile coordinate,
and tile size — ARCHITECTURE.md §10). The layer version is derived from
the serving inputs: for catalog-backed layers, the latest granule id
joined with the plan hash; for static layers, the plan hash alone. So
invalidation needs no machinery — a new granule or an edited layer
definition changes the version, and every key under the old version
simply stops being asked for. Entries are immutable under their key
(same key ⇒ same bytes), which is why the cache carries no TTL.

**The deferral, stated honestly: there is no garbage collection.**
Superseded versions' objects are *orphaned* — never stale, only
unreachable — and linger until collected. GC (a sweep by age, or by
enumerating live versions) is deliberate future operational work, on the
known-deferral inventory that `docs/ROADMAP.md` carries (issue #126; the
semantics are documented in `swath-core`'s cache module and
ARCHITECTURE.md §10/§16.3). Until it lands, the operational mitigations:

- **Deleting the cache is always safe.** Every entry is a pure function
  of its key; wiping the cache root (or letting a bucket lifecycle rule
  expire old objects by age) costs exactly one cold render per tile,
  never correctness. An S3 lifecycle expiration rule on the cache prefix
  is the recommended stand-in for GC.
- **Bound growth at the source** where a layer's data changes often:
  `budget.cache-enabled = false` on that layer keeps it out of the cache
  entirely ([`CONFIG.md`](CONFIG.md), `[budget]`).

Cache failures are policy'd for availability: a failed read is a miss, a
failed write a logged warning — never a failed tile response.

Related knobs (see [`CONFIG.md`](CONFIG.md)): `overview-oversample`
(which overview factors may serve a zoomed-out tile) and
`max-estimated-live-bytes` (refuse live renders estimated over a byte
ceiling when nothing cheaper can serve — absent, never refuse).

## 4. Events sources — how granules arrive

The one built-in event source is the **filedrop watcher**
(`watch-dir`, catalog mode only): the directory is scanned every 250 ms,
and each `<granule-id>.json` granule manifest dropped in is ingested —
registered against its (pre-existing) dataset, stamped with its arrival
time, and immediately eligible to serve (the ingest-to-pixel path the e2e
gate measures). Ingest errors are logged and never stop the loop: one bad
manifest must not block the next granule.

The **legacy path** rides the same drop: a granule whose assets are
legacy files (`.h5`/`.nc`/`.grib2`) gets a virtual-reference manifest
generated automatically and stored alongside (ADR 0006). Referencing
reads local bytes, so it lights up only for a **local** store root; with
an `s3://` root the server warns at startup and refuses legacy granules
per-granule with an honest error. The same generation is available
manually as `swath ingest reference <granule>`.

Queue-backed event sources (SQS/Kafka-style arrival) are a port
(`EventSource`) with one adapter today; see
[`EXTENDING.md`](EXTENDING.md) for adding another.

## 5. Observability — what the endpoints tell you

- **`GET /healthz`** — plain `200 ok`. **Liveness only**: the process is
  up and serving HTTP. It deliberately touches no registry, store, or
  catalog, so orchestrator healthchecks measure the process, not the data
  plane. There is no readiness probe yet — a catalog or store outage
  shows up as failing data-plane requests (500s with RFC 7807 bodies),
  not as a failing `/healthz`.
- **`GET /traces`** — the x-ray SSE stream: one `trace` event per
  rendered tile from connection time on, carrying the full render trace —
  the decision (`live` | `cache_hit` | `overview`), per-asset byte-range
  provenance, bytes read, stage timings, the planner's considered
  strategies with reasons, and `ingest_to_pixel_ms` (the north-star
  number) when the granule's arrival time is known. Slow subscribers
  lose events (reported as `lagged`), never delay a tile. Every tile
  response also carries a one-line summary in its `x-swath-trace` header
  — readable from a plain `curl -D`. Examples in
  [`ENDPOINTS.md`](ENDPOINTS.md).
- **Logs** — single-line `tracing` output on stdout; `SWATH_LOG` selects
  the level (`error`..`trace`, default `info`). Startup logs name the
  bound address, store root, layer count, cache root, CORS origins, and
  watched drop directory; the ingest loop logs every granule with its
  ingest latency.

## 6. CORS and serving the UI

**Same-origin is the default story and needs no configuration.** The
production binary embeds the web viewer (build via `just build-full`;
feature `embedded-ui`, default-on): browsers get `index.html` at `/`
through content negotiation (an `Accept` listing `text/html`), hashed
assets serve from the router fallback, and API clients see byte-identical
JSON — API routes structurally outrank any file the bundle could ship,
and there is no SPA fallback (unknown paths stay plain 404). A build
without `web/dist` degrades honestly to serving no UI (a startup warning
says so).

**CORS is opt-in and off by default** (ADR 0011): no CORS headers at all
unless `cors-allowed-origins` is set. Turn it on only when browsers on
another origin call the API directly (a separately hosted frontend, or
cross-origin dev without the vite proxy): list exact
`scheme://host[:port]` origins, or `*` for any (a dev convenience). The
origin list is the whole policy — methods and request headers mirror the
request, credentials stay off (the API is a public read surface plus the
openEO authoring routes; there are no cookies to protect).

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

- Bind is loopback by default — set `bind = "0.0.0.0:8080"` explicitly to
  serve beyond the host (the compose file does).
- Set `base-url` to the externally reachable URL: it is minted into every
  OGC/openEO link and service URL.
- Mount the store root read-only; give the cache its own writable root.
- Point the orchestrator healthcheck at `/healthz`.
- Startup fails loudly on config errors (unknown TOML keys included) —
  see [`CONFIG.md`](CONFIG.md) for the validation rules.
