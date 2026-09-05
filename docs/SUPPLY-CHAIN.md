# Swath — Supply chain

_Why each notable dependency is in the tree, what was deliberately left off, and the
checkpoint discipline behind the choices. `Cargo.toml` names the version; this document
carries the reasoning it used to carry inline. The gate is `cargo deny` (`deny.toml`:
licenses, advisories, bans, sources) on every PR and nightly; Renovate bumps under a
release-age cooldown; `just publish-dry` proves the five published crates package on their
own._

## The rule

Every crate is supply-chain surface the license gate must then carry, so the workspace adds
a dependency only with **default features off** and exactly the features a consumer names,
records what the dependency is *for* and what was rejected, and — for the heavy ones — an
itemized residual cost in the PR that introduced it. A dependency that reaches the published
crates (ADR 0016) must be publishable on its own; the published crates never depend on an
unpublished workspace crate.

## Checkpoints (the heavy ones)

**zarrs** — codec-chain decode for the virtual-reference source: the manifest's HDF5 filter
pipeline (`zlib:<level>`, `shuffle`) decodes through zarrs's codec implementations, the same
crate a native-Zarr adapter would bind, so codec behaviour is maintained upstream rather than
duplicated. Feature-trimmed hard: default features off (no filesystem store, ndarray,
blosc/zstd/crc32c codecs, sharding, transpose) and exactly `zlib` on — the one gated codec
the manifests use (`shuffle` is ungated; gzip stays off until a manifest can emit it). The
residual cost is zarrs's mandatory core (rayon, moka, num, half, inventory, …), measured and
itemized in the introducing PR; this dependency was the rehearsal for the wasmtime review.

**wasmtime** — the `run_udf` runtime (ADR 0018), trimmed to the ADR's commitments and nothing
else: `runtime` + `cranelift` (compile and execute core modules), `pooling-allocator`
(per-request instantiation at tile rates), `std`. Explicitly absent, by default and by
intent: `wasi` (zero-import modules — determinism is structural), `component-model`, `async`
(execution is bounded by fuel and epoch, inline per ADR 0012), `cache`/`incremental-cache`
(module compilation is content-addressed by Swath's own store), `parallel-compilation` (rayon
stays out of the serve path), and the gc/profiling/debug machinery. License: Apache-2.0 WITH
LLVM-exception (`deny.toml` allows the exact expression). Only the UDF adapter crate may name
this dependency; the referencer's wasmtime-free guard test pins the isolation.

**icechunk** — the manifest → Icechunk committer writes virtual chunk references natively
(spec v2.1, the version icechunk 2.1.x writes; ADR 0016 and its addendum). Default features
off drops the AWS SDK, the GCS/Azure/HTTP object_store backends and reqwest; exactly
`object-store-fs` on — the local-filesystem repo the conformance gate and `swath ingest` write
today (S3 arrives with a real deployment, feature-flagged then). The residual cost is
icechunk's core — tokio, flatbuffers/rmp-serde/zstd (the on-disk format), chrono — itemized
in the introducing PR. Unpublished adapter only: the published `swath-referencer` stays free
of all of it. `url` rides along for filesystem-path → `file://` construction (already in the
tree transitively; a direct use, not new surface).

**sqlx** — the Postgres driver for the pgstac catalog adapter, over tokio-postgres: the
pooled, typed-bind, one-crate surface (`PgPool` + jsonb binds via `json`) versus hand-rolling
pooling, TLS and typed binds on tokio-postgres + deadpool — two more dependencies for less.
Feature-trimmed hard: `postgres` + `runtime-tokio` + `json` only; no TLS backend compiled
(local/dev and in-cluster plaintext for now — a `tls-*` feature is a deliberate later
addition); no macros/migrate/derive (the queries are three static SQL strings calling pgstac
functions; compile-time checking would drag a `DATABASE_URL` into every build).

**hdf5-metno + gribberish** — the production referencer (ADR 0006). `gribberish` reads GRIB2
section metadata (offsets, lengths, grids, packing templates) — referencing needs no field
decode, so default features stay off. `hdf5-metno` in `static` mode builds the bundled
libhdf5 from source, so no system HDF5 is required on any platform (the single-binary story;
prototype 0001); versions are pinned to the ones the bake-off validated. The whole HDF5 half
sits behind `legacy-hdf5` (default on; the fast dev-loop profile turns it off and compiles
without a C toolchain).

## Adopted and bound (the light ones)

- **async-tiff** (COG/TIFF, adopt per ARCHITECTURE §3): parses headers/IFDs/GeoKeys and
  decodes tiles; default features off — Swath drives it through its own object_store-backed
  reader (no reqwest, no coupling to the object_store version its optional feature pins).
  `async-trait` comes with it: the `AsyncFileReader` trait is an `#[async_trait]` trait.
- **reqwest** — outbound HTTP, and the only place Swath makes one (ADR 0030 §5,
  #419): the STAC adapter's allowlisted reads. Already in the tree via
  object_store's `http` feature; `swath-sources-stac` names it directly so the
  egress policy has somewhere to live. Default features off, `rustls` only, and
  redirects disabled — the allowlist is re-checked at every hop. Only that
  adapter may name it.
- **object_store** — the one storage abstraction (local fs, in-memory, S3-compatible behind
  one trait); `http` only where the UDF module fetcher needs it.
- **proj4rs** — projection math is bind, never build (ADR 0002): pure-Rust proj4 for the
  common CRS set, PROJ C-bindings feature-gated later for the long tail; default features off
  (they only add binaries and NAD-grid machinery the adapter never uses).
- **axum / tower / tower-http / futures-core / http-body-util** — inbound HTTP
  (ARCHITECTURE §11): the API crate builds routers and serializes JSON; the server wiring
  (`http1`/`tokio`) arrives with the binary; exactly `cors` from the middleware grab bag;
  the `Stream` trait alone (never futures-util's combinator surface); body collection in
  tests only.
- **tokio** — the adapter/test async runtime; **never** a dependency of `swath-core`.
- **sha2** — promoted from a test-only truth-table hash to a runtime dependency of
  `swath-core` by the tile-cache key: the key names objects in a store, so it must be stable
  across Rust releases and architectures — SHA-256, never `DefaultHasher`. Pure computation;
  the core stays runtime-free.
- **base64** — `run_udf`'s inline module form (`data:application/wasm;base64,…`) decodes in
  the process compiler; already in the tree transitively; pure computation.
- **image** (PNG only; the perceptual-diff oracle and the tile encoder), **serde_json**
  (`float_roundtrip`: the catalog's lossless STAC round trip is a contract on exact f64
  identity through JSON text), **clap** (`derive` + `env`; config layering stays hand-rolled
  on clap + toml + serde — figment rejected: three crates do the job), **include_dir**
  (the embedded UI, ADR 0011), **tracing/tracing-subscriber** (compact single-line output;
  level selection is a hand-parsed `SWATH_LOG`, not an env-filter DSL).
- **Dev-only:** proptest, insta, criterion (default features off; `async_tokio` for the
  full-tile bench), jsonschema (the OGC conformance smoke tests).

## What was rejected

cargo-vet/crev (ENGINEERING §9), figment, futures-util, deadpool + tokio-postgres, a
system-HDF5 build, wasmtime's WASI/component/async/cache features, and every optional
object_store backend not yet deployed. Each stays out until a deployment needs it; the
ENGINEERING §5 posture (SHA-pinned actions, OIDC publishing, Renovate cooldown, Scorecard,
dependency review, zizmor) is the surrounding discipline.
