# Swath configuration reference

Every knob the `swath` binary accepts: CLI flags, `SWATH_*` environment
variables, and the `--config` TOML file. The in-binary help remains the
source of truth; **this file is kept synchronized mechanically** — the
tables between `config-check` markers are diffed against the clap tree
and serde schema by the docs-drift tests
(`tools/docs-check/src/check/`): drift in either direction fails
the build.
Companions: [`OPERATIONS.md`](OPERATIONS.md), [`ENDPOINTS.md`](ENDPOINTS.md).

## Precedence

Layered, later layers overriding earlier scalar by scalar: **built-in
defaults** (loopback bind `127.0.0.1:8080`, no cache, no catalog, CORS
off) → the **TOML file** (`--config`; layers and datasets live *only*
here) → **environment / flags** (each scalar flag has a `SWATH_*`
variable). `--fixtures` conflicts with `--config` and serves the
built-in HLS registry from `./tests/fixtures`. The budget resolves knob
by knob: defaults → `[budget]` → the global flags/env → per-layer
`[layers.budget]`, most specific wins.

## `swath serve` — flags and environment

<!-- config-check:begin flags swath serve -->

| Flag | Env | Value | Meaning |
|---|---|---|---|
| `--config` | — | `PATH` | TOML config file (schema below); conflicts with `--fixtures`. |
| `--fixtures` | — | — | Serve the built-in HLS demo layers from `./tests/fixtures`, zero config. |
| `--bind` | `SWATH_BIND` | `ADDR:PORT` | Socket address to listen on. Default `127.0.0.1:8080`. |
| `--base-url` | `SWATH_BASE_URL` | `URL` | Base URL minted into OGC/openEO links. Default `http://localhost:<port>`. |
| `--store-root` | `SWATH_STORE_ROOT` | `ROOT` | Object-store root: local directory or `s3://bucket[/prefix]`; required unless `--fixtures`. |
| `--catalog` | `SWATH_CATALOG` | `URL` | Catalog mode: pgstac postgres URL; layers then come from `[[datasets]]`. |
| `--read-only` | `SWATH_READ_ONLY` | — | Write routes absent, not 403'd (`POST /datasets`, granule registration, `POST`/`DELETE /services`); `POST /result` stays (budget-bounded preview, ADR 0014). The auth-less hosted-demo slice (#198). |
| `--watch-dir` | `SWATH_WATCH_DIR` | `PATH` | Watch this directory for `<granule-id>.json` manifests (catalog mode only). |
| `--cache` | `SWATH_CACHE` | `ROOT` | Tile-cache root (same grammar as the store root); absent: no cache. |
| `--udf-store` | `SWATH_UDF_STORE` | `ROOT` | `run_udf` module-store root (same grammar): published WASM modules persist here by content hash (ADR 0018); absent: `run_udf` is not offered. |
| `--overview-oversample` | `SWATH_OVERVIEW_OVERSAMPLE` | `RATIO` | Global overview eligibility slack (default 1.2, GDAL's rule). |
| `--max-estimated-live-bytes` | `SWATH_MAX_ESTIMATED_LIVE_BYTES` | `BYTES` | Global live-render ceiling: refuse tiles estimated over this when nothing cheaper can serve; absent: never refuse. |
| `--max-udf-fuel-per-tile` | `SWATH_MAX_UDF_FUEL_PER_TILE` | `FUEL` | Global per-tile fuel a `run_udf` stage may consume (ADR 0018); default 100000000 (100 M). |
| `--cors-allowed-origins` | `SWATH_CORS_ALLOWED_ORIGINS` | `ORIGINS` | Comma-separated exact origins, or `*`; default empty — no CORS headers. |

<!-- config-check:end flags swath serve -->

### Environment beyond the flags

`SWATH_LOG` — max log level, `error`..`trace` (default `info`; an
unrecognized value falls back to `info` — logging config never takes the
server down). With an `s3://` store or cache root, credentials and
endpoint come from the standard `object_store` AWS environment
(`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_*_REGION`,
`AWS_ENDPOINT`, `AWS_ALLOW_HTTP=true` for plain-HTTP endpoints).

## `swath ingest reference` — flags

Generates a legacy granule's virtual-reference manifest (ADR 0006) — the
same generation the filedrop legacy path performs at ingest.

<!-- config-check:begin flags swath ingest reference -->

| Argument | Value | Meaning |
|---|---|---|
| `<granule>` | `PATH` | The legacy granule file (HDF5/NetCDF4, GRIB2) to reference. |
| `--output` | `PATH` | Where to write the manifest JSON. Default: `<granule>.vmanifest.json` beside the granule. |
| `--icechunk` | `DIR` | Also commit the virtual references to the Icechunk repository at this directory (created if absent, committed to `main` — ADR 0017). Chunk paths resolve against the granule's directory. |

<!-- config-check:end flags swath ingest reference -->

## `swath materialize` — flags

Materializes overview pyramids into `pyramids/` under the store root
(#183; layout in `crates/adapters/swath-pyramid-objectstore`), reading
the same config as `swath serve`. Idempotent and resumable; the serving
process picks new levels up with no restart; `nearest` layers aggregate
nearest, everything else averages, nodata-aware.

<!-- config-check:begin flags swath materialize -->

| Flag | Value | Meaning |
|---|---|---|
| `--config` | `PATH` | TOML config file (the same file `swath serve` reads). |
| `--store-root` | `ROOT` | Object-store root, overriding the config file's `store-root`. |
| `--layer` | `ID` | Materialize only this layer's assets. Default: every layer. |
| `--min-dim` | `PIXELS` | Coarsest-level bound: the ladder stops at the first level whose larger axis fits this many pixels. Default 256 (GDAL's overview-build default). |

<!-- config-check:end flags swath materialize -->

### `swath sources`

<!-- config-check:begin flags swath sources -->

| Flag | Env | Value | Meaning |
|---|---|---|---|
| `--config` | `SWATH_CONFIG` | `PATH` | TOML config file the egress allowlist is read from (the same file `swath serve` reads). |

<!-- config-check:end flags swath sources -->

<!-- config-check:begin flags swath sources allowlist -->

| Flag | Env | Value | Meaning |
|---|---|---|---|
| `--config` | `SWATH_CONFIG` | `PATH` | The config file to read the allowlist from. |

<!-- config-check:end flags swath sources allowlist -->

<!-- config-check:begin flags swath sources fetch -->

| Flag | Env | Value | Meaning |
|---|---|---|---|
| `--config` | `SWATH_CONFIG` | `PATH` | The config file to read the allowlist from. |
| `url` | — | `URL` | An `http(s)` URL whose host is on the allowlist. |

<!-- config-check:end flags swath sources fetch -->

<!-- config-check:begin flags swath sources prove -->

| Flag | Env | Value | Meaning |
|---|---|---|---|
| `--config` | `SWATH_CONFIG` | `PATH` | The config file to read the allowlist and the source from. |
| `id` | — | `ID` | The source being proved; its consent is checked first. |
| `url` | — | `URL` | An `http(s)` URL whose host is on the allowlist. |

<!-- config-check:end flags swath sources prove -->

<!-- config-check:begin flags swath sources consent -->

| Flag | Env | Value | Meaning |
|---|---|---|---|
| `--config` | `SWATH_CONFIG` | `PATH` | The config file the source is declared in. |
| `id` | — | `ID` | The source consented to. |

<!-- config-check:end flags swath sources consent -->

## The TOML config file

Kebab-case keys; **unknown keys are rejected** (`deny_unknown_fields`) —
a typo fails loudly at startup.

### Top-level keys

<!-- config-check:begin file -->

| Key | Type | Default | Meaning |
|---|---|---|---|
| `bind` | `"ADDR:PORT"` | `127.0.0.1:8080` | Socket address to listen on. |
| `base-url` | string | `http://localhost:<port>` | Base URL minted into OGC/openEO links. |
| `store-root` | string | — (required) | Object-store root: local directory or `s3://bucket[/prefix]`. |
| `cache` | string | none | Tile-cache root (same grammar as `store-root`). Absent: no cache. |
| `udf-store` | string | none | `run_udf` module-store root (same grammar). Absent: `run_udf` not offered. |
| `catalog` | string | none | Postgres URL of a pgstac database — presence selects catalog mode. |
| `watch-dir` | string | none | Drop directory watched for granule manifests (catalog mode only). Shorthand for one `[[sources]]` entry named `watch-dir`. |
| `sources` | array of tables | `[]` | Named origins to watch (`[[sources]]`, keys below; catalog mode only). |
| `register` | array of tables | `[]` | Endpoints offered in the import flow (`[[register]]`, keys below). |
| `egress-allowlist` | array of strings | `[]` (federation off) | Hosts the **server** may fetch from. Exact host names; no wildcard. |
| `cors-allowed-origins` | array of strings | `[]` (CORS off) | Origin allowlist; `["*"]` allows any origin. |
| `budget` | table | all knobs default | Global default materialization budget (`[budget]`, keys below). |
| `layers` | array of tables | `[]` | Static layer definitions (`[[layers]]`, keys below). Mutually exclusive with catalog mode. |
| `datasets` | array of tables | `[]` | Dataset definitions (`[[datasets]]`, catalog mode only). |

<!-- config-check:end file -->

Validation at startup: catalog mode requires `[[datasets]]` (and vice
versa, as do `watch-dir` and `[[sources]]`); static `[[layers]]` and
catalog mode are mutually exclusive; layer ids are unique across datasets
(shared URL space), dataset ids unique, source ids unique.

### `egress-allowlist` — what the server may reach

Empty by default: the server fetches nothing, as it did before it could fetch
at all. Listing a host turns federation on for that host and no other — names
match exactly, because a suffix rule would let `evil-example.com` through on an
`example.com` entry.

An unlisted host is refused before a connection is attempted; a redirect off
the allowlist is refused; a body past the size cap is abandoned mid-stream.
`swath sources allowlist` prints what is permitted, `swath sources fetch <url>`
retrieves one document under it. Both are operator actions: no HTTP route
reaches this, and none can until the auth interlock lifts (ADR 0031).

### `[[register]]` — what an operator can import from

The public register: STAC endpoints offered in the import flow. **Data, not
code** — adding one is an edit here and a restart, never a release of the web
app. An entry is an offer; nothing is fetched until an operator asks, and only
if its host is on `egress-allowlist`. An entry whose host is not allowed is
still listed, with the host named, so the fix is visible.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `id` | string | — (required) | Stable identifier; what a resumable import links to. |
| `title` | string | the id | What to call it on screen. |
| `url` | string | — (required) | The catalog's URL. |
| `requester-pays` | bool | `false` | Reading it bills you; the flow warns before the read. |

### Requester-pays

A source marked `requester-pays` bills whoever reads it, so Swath will not read
it until someone says they accept that: `swath sources consent <id>` records
who and when, and you write that name into `requester-pays-consented-by` so it
survives a restart. Without it a read is **refused before a connection is
attempted** — a read that never happened, not one that failed.

`swath sources prove <id> <url>` performs one bounded read and reports the
**bytes and requests** it actually made. It never states a money figure:
Swath does not know your rate card, your egress agreement or your region, and a
wrong number there would be worse than none.

### `[[sources]]` — where data comes from

Each entry is one watched origin with its own ingest task and its own
error state, so an unreachable or malformed origin records its failure and
its siblings keep running (ADR 0030).

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `id` | string | — (required) | Identifier, unique within the deployment. |
| `title` | string | the id | Human title for the Sources screen. |
| `watch-dir` | string | — (required) | Directory watched for `<granule-id>.json` manifests. |
| `datasets` | array of strings | `[]` | Which datasets this source feeds. Declared, so a source that has ingested nothing still says what it is for. |
| `credential-profile` | string | none | The **name** of a credential the operator provisions. Never a value. |
| `requester-pays` | bool | `false` | Reading this source bills you. Not read until consent is recorded. |
| `requester-pays-consented-by` | string | none | Who agreed to be billed. Written after `swath sources consent`. |

Swath resolves a profile by name and reports whether it resolved; it never
stores, logs, serves or traces the secret (ADR 0030). The environment
convention is `SWATH_CREDENTIAL_<PROFILE>` — upper-cased, non-alphanumerics
folded to `_` — and its presence is the whole answer. A profile that does not
resolve makes the source read as failing, naming the profile so you are sent to
the credential and not to the network. Putting a secret in `credential-profile`
would put it in your config file, which is what the field exists to avoid.

Top-level `watch-dir` is the same thing said shorter: it becomes a source
named `watch-dir`, and a one-directory deployment is unchanged.

**Adding or removing a source needs a restart**, deliberately: sources
declared in the file live in the file, and the process that reads it is
the one that watches them. Observed state does not survive a restart
either — a process that has just started has observed nothing, and says
`unknown` rather than replaying a claim that may no longer be true.

### `[budget]` — and `[layers.budget]` / `[datasets.layers.budget]`

Every knob is optional at every level; per-layer values outrank the
global default knob by knob (see Precedence). The resolved global
default also governs openEO published services and `POST /result`
previews, whose own ADR 0014 byte ceiling a `[budget]` cap can tighten,
never widen.

<!-- config-check:begin budget -->

| Key | Type | Default | Meaning |
|---|---|---|---|
| `cache-enabled` | bool | `true` | Consult/fill the tile cache for this layer (needs a configured cache root). |
| `overview-oversample` | float | `1.2` | Overview eligibility slack: an overview factor is eligible when `factor <= desired ratio × this value`. |
| `max-estimated-live-bytes` | integer | none (never refuse) | Refuse live renders estimated over this; per-layer values can set or tighten the ceiling, not clear it. |
| `max-udf-fuel-per-tile` | integer | `100000000` (100 M) | Deterministic fuel a `run_udf` stage may consume per tile (ADR 0018); exhausting it fails that tile loudly. Only UDF layers spend any. |

<!-- config-check:end budget -->

### `[[layers]]` — and `[[datasets.layers]]`

One schema, two contexts (the drift test pins it): static `[[layers]]`
`bands` values are **asset URIs under the store root**;
`[[datasets.layers]]` values are **dataset band names**, resolved per
tile from the latest ingested granule.

<!-- config-check:begin layer -->

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | — (required) | URL-safe identifier — the `{layerId}` path segment. |
| `title` | string | the id | Human-readable title. |
| `description` | string | `""` | Tileset-metadata description. |
| `kind` | string | — (required) | The pixel pipeline: `truecolor` \| `ndvi` (values below). |
| `bands` | table | — (required) | Band role → asset URI (static) or dataset band name (catalog); `truecolor` consumes `r`,`g`,`b`, `ndvi` consumes `nir`,`red` — exactly. |
| `rescale` | `[min, max]` | `truecolor`: none; `ndvi`: `[-1, 1]` | Linear rescale of pipeline output to 0..255. |
| `colormap` | string | `ndvi`: `rdylgn` | Colormap applied to the gray result — `ndvi` only (`truecolor` rejects the key). |
| `resampling` | string | `bilinear` | Warp kernel (values below). |
| `tile-size` | integer | `256` | Tile side length in pixels. |
| `budget` | table | resolved global default | This layer's budget (`[layers.budget]`), overriding the global default knob by knob. |

<!-- config-check:end layer -->

Enum vocabularies (unknown spellings fail at parse):

<!-- config-check:begin enum kind -->

| Value (kind) | Meaning |
|---|---|
| `truecolor` | RGB composite of bands `r`,`g`,`b`. |
| `ndvi` | Grayscale `(nir - red) / (nir + red)` of bands `nir`,`red`, colormapped. |

<!-- config-check:end enum kind -->

<!-- config-check:begin enum colormap -->

| Value (colormap) | Meaning |
|---|---|
| `grayscale` | The identity map: gray in, gray out. |
| `viridis` | Matplotlib's perceptually uniform sequential viridis. |
| `magma` | Matplotlib's perceptually uniform sequential magma. |
| `rdylgn` | ColorBrewer diverging red–yellow–green — the NDVI default. |

<!-- config-check:end enum colormap -->

<!-- config-check:begin enum resampling -->

| Value (resampling) | Meaning |
|---|---|
| `bilinear` | Bilinear, nodata excluded, weights renormalized. The default. |
| `nearest` | Nearest neighbor (categorical bands). |

<!-- config-check:end enum resampling -->

(`kind` is the walking-skeleton vocabulary; arbitrary pipelines are
authored through `POST /services`, ADR 0010.)

### `[[datasets]]` (catalog mode)

Upserted into the catalog at startup — config is the source of truth for
dataset identity (operators write TOML, never STAC); granules arrive via
ingest and require their dataset to pre-exist.

<!-- config-check:begin dataset -->

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | — (required) | Dataset identifier (the catalog collection id). |
| `title` | string | the id | Human-readable title. |
| `description` | string | `""` | Narrative description. |
| `license` | string | `other` | Data license (SPDX id). |
| `layers` | array of tables | `[]` | Serving layers over this dataset (`[[datasets.layers]]`, schema above). |

<!-- config-check:end dataset -->

## Example — catalog mode with filedrop ingest, cache, and budgets

The compose stack's config ([`tests/e2e/swath-catalog.toml`](../tests/e2e/swath-catalog.toml))
is the living example; in miniature (a static-layers file drops
`catalog`/`watch-dir`/`[[datasets]]` for `[[layers]]` whose `bands` are asset
URIs under the store root):

```toml
store-root = "/data"            # or "s3://bucket/prefix" (AWS_* env)
cache = "/cache"
catalog = "postgres://swath:swath-local-dev@pgstac:5432/swath"
watch-dir = "/data/drop"

[budget]
max-estimated-live-bytes = 50000000

[[datasets]]
id = "hls-s30"

[[datasets.layers]]
id = "ndvi"
kind = "ndvi"                   # colormap defaults to rdylgn
[datasets.layers.bands]         # dataset band names, resolved per tile
nir = "b8a"
red = "b04"
[datasets.layers.budget]
cache-enabled = false           # per-layer override, knob by knob
```
