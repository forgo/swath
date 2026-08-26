# Read-only public demo — the deploy recipe

The provider-agnostic recipe behind [`ROADMAP.md`](../ROADMAP.md) §3 item 8 (issue #212): one
Docker Compose file, one DNS name, ports 80/443 — nothing else assumed about the host. It runs
`swath serve --read-only` (#249: the write routes are *absent*, not 403'd; `POST /result`
stays) behind Traefik (ACME TLS, per-IP rate limits, a preview body cap), seeded from the
fixtures the image already carries and the reference NDVI `run_udf` module, so the demo opens
on a user-defined index animated over the 2024 Park Fire season with the x-ray one click away.

**Status (maintainer decision, 2026-08-25): the public deploy is deferred to the auth era.**
This recipe is exercised end to end (transcript below) and ready; the hosted URL is not live,
and the CI-tested one-liner in the [README](../../README.md) remains the demo.

## Files

| File | Role |
|---|---|
| [`compose.yml`](compose.yml) | The stack: `traefik` (the only published port), `swath` (read-only, no host port), `pgstac`; a `seed` profile with a writable `swath-seed` + the `seed` job |
| [`traefik/dynamic.yml`](traefik/dynamic.yml) | Two routers over one upstream: `/result` behind the tight preview limits, everything else behind the viewer limits; security headers |
| [`swath.toml`](swath.toml) | Catalog-mode config: the reference layers over the fire-season datasets, the tile cache, the `run_udf` module store, a byte ceiling |
| [`seed.sh`](seed.sh) | The seed job: hand the volumes to the swath uid, drop the granules (the filedrop convention), publish the UDF service, prove it renders |

## Bring-up

```sh
cd docs/deploy
cat > .env <<'EOF'
SWATH_DOMAIN=demo.example.org        # DNS A/AAAA record -> this host
ACME_EMAIL=you@example.org           # a real mailbox (see findings)
SWATH_PG_PASSWORD=<generate one>
EOF
docker compose --profile seed run --rm seed     # 1. seed: ingest + publish (writable, never published)
docker compose --profile seed stop swath-seed   #    the writable instance is not needed again
docker compose up -d                            # 2. serve: read-only swath + Traefik
```

The seed job prints the content-derived service id (`xyz-…`) and the deep link
`/?layer=<id>&xray`. Re-running the seed is idempotent (same graph, same id). To re-seed from
scratch, `docker compose --profile seed down -v` first.

Knobs (`.env`): `SWATH_IMAGE` (default `ghcr.io/forgo/swath:latest`; pin a digest for a
long-lived host), `HTTP_PORT`/`HTTPS_PORT`, `ACME_CASERVER` (point at Let's Encrypt staging
while rehearsing). Behind a cloud load balancer set the rate limiters'
`sourceCriterion.ipStrategy.depth`, or every visitor shares one bucket.

## Exercised locally — the transcript

Rehearsal on the maintainer's laptop (Docker 29.3.1, Compose v5.1.1, Traefik v3.7.11 by
digest), image built from this checkout, `SWATH_DOMAIN=localhost`, ports 8088/8443, ACME
against the staging directory. Excerpted; the labels in `== n.` lines are the proof script's.

```text
$ docker compose --profile seed run --rm seed
seed: dropping the single-date HLS granule
seed: dropping the six-date Park Fire series
seed: waiting for the series to catalog and serve
seed: park fire series live
seed: publishing the reference NDVI UDF service over hls-s30-fire
seed: done — UDF service id: xyz-c152a0cd87b9
seed: demo deep link: /?layer=xyz-c152a0cd87b9&xray
(6.6 s wall)

$ docker compose up -d --wait && docker compose ps
swath-demo-pgstac-1    Up (healthy)   5432/tcp
swath-demo-swath-1     Up (healthy)   8080/tcp
swath-demo-traefik-1   Up             0.0.0.0:8088->80/tcp, 0.0.0.0:8443->443/tcp
swath: udf runtime ready: wasmtime 0; run_udf modules persist at /udf (ADR 0018)
swath: restored openEO service xyz-c152a0cd87b9 on dataset hls-s30-fire
swath: read-only: write routes unmounted (#198)
swath: serving 4 layer(s) on 0.0.0.0:8080 (store: /data); traces: https://localhost/traces

== 1. TLS termination + HTTP->HTTPS redirect
GET /healthz over https: 200 (Traefik's default certificate — ACME cannot validate "localhost")
GET http://:8088/ -> 301 Location: https://localhost/
  header: strict-transport-security: max-age=31536000; includeSubDomains
  header: x-content-type-options: nosniff
  header: x-frame-options: DENY

== 2. tiles serve: the seeded run_udf service, dated frames
run_udf xyz-c152a0cd87b9 datetime=2024-06-07T19:03:00Z: 200 image/png
  trace={"decision":"live","bytes_read":194486,"total_ms":90,"ingest_to_pixel_ms":71464,"udf_fuel_used":12260531}
run_udf xyz-c152a0cd87b9 datetime=2024-10-15T00:00:00Z: 200 image/png
  trace={"decision":"live","bytes_read":195477,"total_ms":95,"ingest_to_pixel_ms":37029,"udf_fuel_used":12260531}
tileset title: Park Fire NDVI (run_udf)   links: self, tiling-scheme, item, granules
minted base: https://localhost/tilesets/xyz-c152a0cd87b9/...

== 3. write routes absent (read-only), preview present
POST /services -> 405        POST /datasets -> 404
POST /datasets/hls-s30-fire/granules -> 405        DELETE /services/xyz-c152a0cd87b9 -> 405
capabilities advertise: GET /collections, GET /collections/{id}, GET /conformance,
  GET /datasets/{id}/granules, GET /file_formats, GET /processes, GET /service_types,
  GET /services, GET /services/{id}, POST /result
POST /result (run_udf NDVI over the fire collection): 200 image/png 6531B
POST /result (fuel bomb): 400 {"code":"ProcessGraphComplexity", ... "exceeded the per-tile fuel budget (100000000 fuel)"}
POST /result with a 300 KB body -> 413 (Traefik body cap 256 KiB)

== 4. rate limit engages
viewer bucket (60/s, burst 120): 400 requests, 40-way parallel, one tile ->  205 x 200, 195 x 429
preview bucket (2/s, burst 4): 12 back-to-back previews -> 200 200 200 200 200 429 429 429 429 429 429 429
after a 3 s pause: tile -> 200

== 5. the read-only mounts, from inside the swath container
touch: cannot touch '/data/x': Read-only file system
touch: cannot touch '/udf/x': Read-only file system
```

Torn down with `docker compose --profile seed down -v`.

## Findings (working-agreement rule 7)

1. **Published services and previews ignore the operator budget — issue #272.**
   `compile_service_layer` gives every published service `Budget::default()`, and the preview's
   fuel axis "rides along at the budget default"; the `[budget]` table and the global
   `--max-udf-fuel-per-tile` / `--max-estimated-live-bytes` govern *declared* layers only.
   Reproduced: with `[budget] max-udf-fuel-per-tile = 10000000` the reference module (~12.3 M
   fuel) still rendered a never-seen frame through the service and the preview answered 200.
   Consequence for this recipe: the fuel cap on user code is the binary's built-in 100 M (+ the
   250 ms epoch backstop, 64 MiB), stated honestly in `swath.toml`; the reverse proxy's
   per-IP and body limits carry the rest until #272 lands.
2. **Absent write routes answer 405 where a read route shares the path** (`/services`,
   `/datasets/{id}/granules`), 404 where nothing does (`POST /datasets`). Both are "absent, not
   403'd" per [`ENDPOINTS.md`](../ENDPOINTS.md); the capabilities document lists neither.
3. **`datetime=` selects the latest granule at or before the instant**: a frame request at
   `2024-06-07T00:00:00Z` (before the first 19:03 overpass) is an honest 404 naming the
   window; the deep links and the seed use the acquisition instants.
4. **ACME needs a real contact address even on staging**: Let's Encrypt's staging directory
   rejected `ops@example.invalid` at account registration (400 on `new-acct`), Traefik logged it
   and served its default self-signed certificate — which is exactly what makes a loopback
   rehearsal possible (`curl -k`). With a public DNS name and a real mailbox nothing else
   changes.
5. **Fresh named volumes are root-owned** while the image runs as uid 65534: the seed job runs
   as root for the one `chown`, then everything else is unprivileged. A host that pre-creates
   the volumes needs the same step once.
6. **Non-default HTTPS port drops out of the redirect**: Traefik's entrypoint redirect targets
   `https://<host>/` — correct on 443, a rehearsal-only artefact on 8443.
7. **The landing's zero-config pick is "first playable tileset in id order"**: `park-fire-ndvi`
   sorts before `xyz-…`, so the UDF index opens through the deep link the seed prints (a deep
   link is never overridden by the cinematic default), the x-ray flag riding along.
8. **Ops learnings for ROADMAP item 9** (performance beyond the laptop): a viewport burst at
   40-way parallelism from one address tripped the viewer bucket at exactly the sizing (burst
   120, 60/s) — real-world tuning wants a measured tiles-per-viewport figure per zoom, and a
   CDN in front turns cache hits into edge hits (deferral row 14, the CDN-pointable cache
   layout, becomes worth its price there); the seed phase is a 7 s job, so a host can be
   rebuilt from nothing in the time a certificate takes to issue.
