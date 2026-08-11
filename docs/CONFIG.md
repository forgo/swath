# Swath configuration reference

Every knob the `swath` binary accepts: CLI flags, `SWATH_*` environment
variables, and the `--config` TOML file. The in-binary help
(`swath serve --help` and the `swath --help` after-help) remains the
source of truth; **this file is kept synchronized mechanically** — the
tables between `config-check` markers are diffed against the actual clap
command tree and serde config schema by `swath-cli`'s docs-drift tests
(`crates/swath-cli/src/docs_check.rs`, run by `just test` and CI). A flag
or TOML key added, renamed, or removed without updating this file fails
the build; so does documenting a key the code does not accept.

Companion docs: [`OPERATIONS.md`](OPERATIONS.md) (what the knobs mean
operationally), [`ENDPOINTS.md`](ENDPOINTS.md) (the HTTP surface they
configure).

## Precedence

Configuration is layered, later layers overriding earlier ones scalar by
scalar:

1. **Built-in defaults** — bind `127.0.0.1:8080` (loopback: never all
   interfaces unless asked), base URL `http://localhost:<port>`, no cache,
   no catalog, CORS off.
2. **TOML file** (`--config <PATH>`). Layers and datasets live *only*
   here — a layer is a structure, and structures are not encoded in
   environment variables.
3. **Environment / flags** (one surface: each scalar flag has a `SWATH_*`
   variable via clap's `env` attribute; either outranks the file).

`--fixtures` conflicts with `--config` (clap rejects the pair) and serves
the built-in HLS demo registry with the store root defaulted to
`./tests/fixtures`.

The materialization budget resolves knob by knob: built-in defaults →
top-level `[budget]` → `--overview-oversample` /
`--max-estimated-live-bytes` (or their env vars) → per-layer
`[layers.budget]`, most specific wins.

## `swath serve` — flags and environment

<!-- config-check:begin flags swath serve -->

| Flag | Env | Value | Meaning |
|---|---|---|---|
| `--config` | — | `PATH` | TOML config file (schema below). Conflicts with `--fixtures`. |
| `--fixtures` | — | — | Serve the built-in HLS demo layers (`truecolor`, `ndvi`) from `./tests/fixtures`, zero config. |
| `--bind` | `SWATH_BIND` | `ADDR:PORT` | Socket address to listen on. Default `127.0.0.1:8080`. |
| `--base-url` | `SWATH_BASE_URL` | `URL` | Base URL minted into OGC/openEO links. Default `http://localhost:<port>`. |
| `--store-root` | `SWATH_STORE_ROOT` | `ROOT` | Object-store root: a local directory or `s3://bucket[/prefix]`. Required unless `--fixtures`. |
| `--catalog` | `SWATH_CATALOG` | `URL` | Catalog mode: postgres URL of a pgstac database. Layers then come from `[[datasets]]`. Conflicts with `--fixtures`. |
| `--watch-dir` | `SWATH_WATCH_DIR` | `PATH` | Watch this directory for `<granule-id>.json` manifests (catalog mode only). Conflicts with `--fixtures`. |
| `--cache` | `SWATH_CACHE` | `ROOT` | Tile-cache root (same grammar as the store root). Absent: no cache is consulted. |
| `--overview-oversample` | `SWATH_OVERVIEW_OVERSAMPLE` | `RATIO` | Global default for the planner's overview eligibility slack (default 1.2, GDAL's rule). |
| `--max-estimated-live-bytes` | `SWATH_MAX_ESTIMATED_LIVE_BYTES` | `BYTES` | Global default live-render ceiling: refuse tiles estimated over this many bytes when nothing cheaper can serve. Absent: never refuse. |
| `--cors-allowed-origins` | `SWATH_CORS_ALLOWED_ORIGINS` | `ORIGINS` | Comma-separated exact origins, or `*` for any. Default: empty — no CORS headers at all. |

<!-- config-check:end flags swath serve -->

### Environment beyond the flags

- `SWATH_LOG` — max log level: `error` | `warn` | `info` | `debug` |
  `trace` (default `info`). An unrecognized value falls back to `info` —
  logging config never takes the server down.
- With an `s3://` store or cache root, credentials and endpoint come from
  the standard `object_store` AWS environment: `AWS_ACCESS_KEY_ID`,
  `AWS_SECRET_ACCESS_KEY`, `AWS_DEFAULT_REGION` (or `AWS_REGION`),
  `AWS_ENDPOINT`, and `AWS_ALLOW_HTTP=true` for plain-HTTP endpoints such
  as local MinIO.

## `swath ingest reference` — flags

Generates a legacy granule's virtual-reference manifest (ADR 0006)
without registering anything — the same generation the filedrop legacy
path performs automatically at ingest.

<!-- config-check:begin flags swath ingest reference -->

| Argument | Value | Meaning |
|---|---|---|
| `<granule>` | `PATH` | The legacy granule file (HDF5/NetCDF4, GRIB2) to reference. |
| `--output` | `PATH` | Where to write the manifest JSON. Default: `<granule>.vmanifest.json` beside the granule. |

<!-- config-check:end flags swath ingest reference -->

## The TOML config file

Kebab-case keys; **unknown keys are rejected** (`deny_unknown_fields`) —
a typo fails loudly at startup, never silently falls back to a default.

### Top-level keys

<!-- config-check:begin file -->

| Key | Type | Default | Meaning |
|---|---|---|---|
| `bind` | `"ADDR:PORT"` | `127.0.0.1:8080` | Socket address to listen on. |
| `base-url` | string | `http://localhost:<port>` | Base URL minted into OGC/openEO links. |
| `store-root` | string | — (required) | Object-store root: local directory or `s3://bucket[/prefix]`. |
| `cache` | string | none | Tile-cache root (same grammar as `store-root`). Absent: no cache. |
| `catalog` | string | none | Postgres URL of a pgstac database — presence selects catalog mode. |
| `watch-dir` | string | none | Drop directory watched for granule manifests (catalog mode only). |
| `cors-allowed-origins` | array of strings | `[]` (CORS off) | Origin allowlist; `["*"]` allows any origin. |
| `budget` | table | all knobs default | Global default materialization budget (`[budget]`, keys below). |
| `layers` | array of tables | `[]` | Static layer definitions (`[[layers]]`, keys below). Mutually exclusive with catalog mode. |
| `datasets` | array of tables | `[]` | Dataset definitions (`[[datasets]]`, catalog mode only). |

<!-- config-check:end file -->

Validation rules enforced at startup: catalog mode requires at least one
`[[datasets]]`; `[[datasets]]` and `watch-dir` require `catalog`; static
`[[layers]]` and catalog mode are mutually exclusive; layer ids must be
unique across all datasets (they share URL space), dataset ids unique.

### `[budget]` — and `[layers.budget]` / `[datasets.layers.budget]`

Every knob is optional at every level; resolution is knob by knob with
per-layer values outranking the global default (see Precedence above).

<!-- config-check:begin budget -->

| Key | Type | Default | Meaning |
|---|---|---|---|
| `cache-enabled` | bool | `true` | Consult/fill the tile cache for this layer (only effective when a cache root is configured at all). |
| `overview-oversample` | float | `1.2` | Overview eligibility slack: an overview factor is eligible when `factor <= desired ratio × this value`. |
| `max-estimated-live-bytes` | integer | none (never refuse) | Refuse live renders estimated over this many bytes. Per-layer values can set or tighten a global ceiling, not clear it. |

<!-- config-check:end budget -->

### `[[layers]]` — and `[[datasets.layers]]`

One schema, two contexts (the drift test pins that they stay one schema).
In static `[[layers]]`, `bands` values are **asset URIs relative to the
store root**; in `[[datasets.layers]]` they are **dataset band names**
(granule asset keys, e.g. `r = "b04"`), resolved per tile from the
dataset's latest ingested granule.

<!-- config-check:begin layer -->

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | — (required) | URL-safe identifier — the `{layerId}` path segment. |
| `title` | string | the id | Human-readable title. |
| `description` | string | `""` | Tileset-metadata description. |
| `kind` | string | — (required) | The pixel pipeline: `truecolor` \| `ndvi` (values below). |
| `bands` | table | — (required) | Band role → asset URI (static) or dataset band name (catalog). `truecolor` consumes `r`,`g`,`b`; `ndvi` consumes `nir`,`red` — exactly. |
| `rescale` | `[min, max]` | `truecolor`: none (raw values clamp); `ndvi`: `[-1, 1]` | Linear rescale of pipeline output to 0..255. |
| `colormap` | string | `ndvi`: `rdylgn` | Colormap applied to the gray result — `ndvi` only (`truecolor` renders RGB directly and rejects the key). |
| `resampling` | string | `bilinear` | Warp kernel (values below). |
| `tile-size` | integer | `256` | Tile side length in pixels. |
| `budget` | table | resolved global default | This layer's materialization budget (`[layers.budget]`, keys above), overriding the global default knob by knob. |

<!-- config-check:end layer -->

Enum vocabularies (an unknown spelling fails at parse):

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
| `bilinear` | Bilinear, nodata excluded and weights renormalized (the continuous-band kernel of the golden suites). The default. |
| `nearest` | Nearest neighbor (categorical bands). |

<!-- config-check:end enum resampling -->

The `kind` enum is the walking-skeleton vocabulary for operator-authored
layers; arbitrary pipelines are authored through the openEO services
surface (`POST /services`, ADR 0010), which compiles a process graph into
a served layer.

### `[[datasets]]` (catalog mode)

The dataset is upserted into the catalog at startup — config is the
source of truth for dataset identity and serving layers (operators write
TOML, never STAC). Granules arrive later via ingest and require their
dataset to pre-exist.

<!-- config-check:begin dataset -->

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | — (required) | Dataset identifier (the catalog collection id). |
| `title` | string | the id | Human-readable title. |
| `description` | string | `""` | Narrative description. |
| `license` | string | `other` | Data license (SPDX id). |
| `layers` | array of tables | `[]` | Serving layers over this dataset (`[[datasets.layers]]`, schema above). |

<!-- config-check:end dataset -->

## Examples

### Static layers over a local store

```toml
bind = "127.0.0.1:8080"
store-root = "/data"

[[layers]]
id = "truecolor"
title = "True color"
kind = "truecolor"
rescale = [0.0, 3000.0]
[layers.bands]                 # asset URIs under store-root
r = "granule-b04.tif"
g = "granule-b03.tif"
b = "granule-b02.tif"
```

### Catalog mode with filedrop ingest, cache, and budgets

The compose stack's config ([`tests/e2e/swath-catalog.toml`](../tests/e2e/swath-catalog.toml))
is the living example; in miniature:

```toml
bind = "0.0.0.0:8080"
base-url = "http://localhost:8080"
store-root = "/data"            # or "s3://bucket/prefix" (AWS_* env)
cache = "/cache"
catalog = "postgres://swath:swath-local-dev@pgstac:5432/swath"
watch-dir = "/data/drop"

[budget]
overview-oversample = 1.2
max-estimated-live-bytes = 50000000

[[datasets]]
id = "hls-s30"
title = "HLS Sentinel-2 (S30)"
license = "CC0-1.0"

[[datasets.layers]]
id = "truecolor"
kind = "truecolor"
rescale = [0.0, 3000.0]
[datasets.layers.bands]         # dataset band names, resolved per tile
r = "b04"
g = "b03"
b = "b02"

[[datasets.layers]]
id = "ndvi"
kind = "ndvi"                   # colormap defaults to rdylgn
[datasets.layers.bands]
nir = "b8a"
red = "b04"
[datasets.layers.budget]
cache-enabled = false           # per-layer override, knob by knob
```
