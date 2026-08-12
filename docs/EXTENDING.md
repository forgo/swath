# Extending Swath

How to extend the system at its interfaces — REQUIREMENTS.md **R9** ("a new source, a new
product, a new backend — without editing the core") turned into concrete, checked steps.

Two disciplines keep this document true:

- **Verified by construction.** The "new source adapter" walkthrough (§2) was executed
  step-by-step to build a real toy adapter; the resulting diff lives on the unmerged evidence
  branch [`demo/125-toy-source-inmem`](https://github.com/forgo/swath/tree/demo/125-toy-source-inmem)
  (§5). If a step below had been missing, that branch could not have passed the full gate.
- **Verified against a commit.** Every signature block is copied verbatim from the named source
  file and carries a `_Last verified against_` marker, the same drift discipline as
  ARCHITECTURE.md §6. The rustdoc on the named files is the normative contract; where this guide
  and the rustdoc disagree, the rustdoc wins.

## 1. The extension model — and what is deliberately NOT supported

The mechanism is decided by [ADR 0013](decisions/0013-extension-features-plus-openeo-graphs.md)
(the ARCHITECTURE §16.6 confirm-close): **compile-time Cargo crates/features for adapters, plus
openEO process graphs at runtime** as the user-facing extension surface. Three extension kinds
exist today, uniform across the shipped adapter set:

| Kind | Mechanism | Walkthrough |
| --- | --- | --- |
| New source adapter (or any other port impl) | New crate implementing a `swath-core` port trait, wired in `swath-cli` | §2 |
| New openEO process | Extend the bounded compiler subset + pinned definitions | §3 |
| New colormap | New `Colormap` variant + vendored LUT through every vocabulary | §4 |

**Anti-goals** (each is a decision with a recorded reopen condition, not an accident):

- **No runtime plugin loading.** No WASM host ABI, no `dlopen`, no operator-installed extensions
  on prebuilt binaries. Deferred, not rejected — the reopen condition (concrete demand for
  dynamic plugin loading) is recorded in ADR 0013. Until then, extending a *binary capability*
  means recompiling.
- **No sidecar-RPC adapter seam.** The Python `VirtualiZarr` sidecar is an ingest-time
  conformance *reference* (ADR 0006), not a runtime component; no language-agnostic adapter RPC
  exists (ADR 0013).
- **No user-defined processes at runtime.** The openEO profile is bounded
  ([ADR 0010](decisions/0010-openeo-authoring-surface.md)): no UDFs, no jobs/batch, no
  user-defined process registration, no `ProcessRegistry` port (ARCHITECTURE §6/§7 — process
  definitions are data resolved against a `CompileContext`, not an adapter seam). A product a
  graph cannot express within the profile is a compile-time process addition (§3) — or, per
  ADR 0013's reopen condition, eventually a superseding ADR.
- **No embedding of a third-party engine.** Composition happens at the standards boundary
  (openEO/OGC APIs), never by adopting someone else's openEO backend or linking GDAL into
  production code (ADR 0002, ADR 0010). GDAL and friends appear ONLY as test oracles.
- **Adapter selection is not feature-flagged.** Phase-1 adapters are direct dependencies of the
  binary; Cargo features gate optional *weight* (`embedded-ui`, `legacy-hdf5`), not which
  adapters exist (ARCHITECTURE §12). See §2.6 before reaching for a new feature flag.

## 2. A new source adapter (`RasterSource`)

The port, verbatim from [`crates/swath-core/src/source.rs`](../crates/swath-core/src/source.rs):

```rust
pub trait RasterSource: Send + Sync {
    fn describe(
        &self,
        asset: &AssetRef,
    ) -> impl Future<Output = Result<RasterInfo, SourceError>> + Send;

    fn read_window(
        &self,
        asset: &AssetRef,
        window: WindowRequest,
        band: BandSelection,
        level: ReadLevel,
    ) -> impl Future<Output = Result<WindowData, SourceError>> + Send;
}
```

Native async-in-trait: implementors write plain `async fn`, the compiler enforces the `Send`
bound at the impl site, and the trait is deliberately **not dyn-compatible** — consumers are
generic (`S: RasterSource`), and the binary composes compiled-in adapters behind one enum-like
dispatcher (§2.5). Contract obligations the port rustdoc spells out (read it in full before
implementing):

- **Clipping.** `read_window` clips the request to the raster grid; the returned
  `WindowData::window` is the intersection (possibly empty), never an error.
- **Coordinate spaces.** The `WindowRequest` is *always* in full-resolution pixel coordinates.
  For an overview read the adapter maps it by *covering* (`floor(off / factor)` /
  `ceil(end / factor)`, exact per-axis ratio) and returns the overview grid in
  `WindowData::grid` — callers never do overview math.
- **Provenance honesty.** `WindowData::provenance` reports the byte ranges *actually fetched*,
  in fetch order — real observed I/O, never estimates. This is the raw material of the
  glass-box Trace (R4).
- **Error taxonomy.** Translate library/storage errors into `SourceError`'s variants
  (`NotFound`, `Format`, `Unsupported`, `OverviewNotFound`, `BandOutOfRange`, `Io`) so
  consumers match on semantics, not adapter internals.
- **Zero-based bands**; adapters translate GDAL-style 1-based indexing at their boundary.

### 2.1 Steps (each executed on the evidence branch, §5)

1. **Scaffold the crate** at `crates/adapters/swath-source-<name>/` with the shared manifest
   pattern (`version.workspace = true` …, `publish = false`, `[lints] workspace = true`;
   copy `crates/adapters/swath-source-virtual/Cargo.toml`). Depend on `swath-core` only;
   dev-depend on `swath-testsupport`, `serde`, and `tokio` (`macros`, `rt-multi-thread`).
   Adapters choose their own runtime — `swath-core` never carries one.
2. **Register it** in the root `Cargo.toml` `[workspace] members` list (alphabetical).
3. **Implement the trait** as plain `async fn`s, discharging every obligation above. Every
   `.rs` file carries the SPDX header pair (`just reuse` gates it); the workspace lints are
   pedantic clippy + `missing_docs`, so document all public items.
4. **Write the oracle** (§2.2) and commit its generated truth table under `tests/data/`.
5. **Write the truth-table tests** (§2.3).
6. **Annotate generated test data** in the root `REUSE.toml` (add
   `"crates/adapters/swath-source-<name>/tests/data/*.json"` to the aggregate block).
7. **Wire the binary** (§2.5) and, only if the adapter adds heavy optional weight, a feature
   gate (§2.6).
8. **Run the full gate**: `just check` — fmt, clippy `-D warnings`, machete, the whole test
   suite (including doctests), deny, zizmor, reuse.

### 2.2 The oracle obligation

Correctness against real formats is never self-certified (ADR 0002): a pinned, independent
oracle generates a JSON truth table the adapter must match **exactly** — SHA-256 of the raw
little-endian pixel bytes, byte-for-byte. Precedents to copy:

| Adapter | Oracle | Truth tables |
| --- | --- | --- |
| `swath-source-cog` | `tests/oracle/window_truth.py` (pinned rasterio/GDAL) | `window_truth.json`, `overview_truth.json` |
| `swath-source-virtual` | h5py, inside the fixture maker | `window_truth.json` |
| `swath-source-inmem` (toy, §5) | `tests/oracle/inmem_truth.py` (pinned numpy) | `window_truth.json` |

The oracle is a PEP 723 script under `tests/oracle/` with pinned dependency versions, run via
`uv run`; it writes the truth table into the adapter's `tests/data/` deterministically, so
regeneration is reviewable. The window list must at minimum exercise: a full-grid read, an
interior window, a window containing nodata, a single pixel, and an out-of-bounds request that
must clip. If the source has overviews, add an overview table whose oracle replicates the
port's cover-rounding contract and reads the overview's *stored* samples explicitly (see
`tests/oracle/window_truth.py` for both).

### 2.3 The test obligations

The shared schema and assertions live in
[`crates/swath-testsupport/src/truth.rs`](../crates/swath-testsupport/src/truth.rs): every
truth table shares the `PixelCase` pixel-identity block (clipped window, dtype, nodata count,
valid sum, first/last samples, `sha256_le`); per-source keys ride alongside via
`#[serde(flatten)]`. A new adapter's integration suite must assert:

1. **Pixel identity** — `truth::assert_pixels_match` per case: exact clipped window, nodata
   sentinel, pixel count, and the SHA-256 of `PixelBuffer::to_le_bytes()` against the oracle.
2. **Provenance honesty** — non-empty, every range in-bounds for the underlying storage,
   `bytes_read` equal to the sum of range lengths, and a range count that matches the access
   geometry (tiles touched, chunks fetched, rows copied — whatever the format implies).
3. **`describe` truth** — grid, dtype, band count, nodata, and `overview_levels` as the
   format really reports them.
4. **Error taxonomy** — at minimum `NotFound`, plus whichever of
   `OverviewNotFound`/`BandOutOfRange`/`Format`/`Unsupported` the adapter can hit.
5. **(When servable end-to-end)** a render test that pulls a tile *through* the adapter and
   asserts the Trace (the pattern in
   `crates/adapters/swath-source-virtual/tests/render_trace.rs`), and e2e coverage via
   `just e2e` once the compose stack can serve the new source's assets.

### 2.4 The other ports (same pattern, different contracts)

Every outbound port follows the identical extension recipe — new crate, implement the trait,
oracle-or-property-based obligations, wire in `swath-cli`. Signatures verbatim in
ARCHITECTURE.md §6; contracts in the named files:

| Port | Trait file | Existing adapter | Test obligation precedent |
| --- | --- | --- | --- |
| `Reproject`/`CoordTransform` (sync, dyn-compatible) | `crates/swath-core/src/reproject.rs` | `swath-reproject-proj4rs` | pyproj truth tables + proptests (`crates/adapters/swath-reproject-proj4rs/tests/`, `tests/oracle/reproject_truth.py`) |
| `Catalog` (domain-shaped; STAC only inside adapters) | `crates/swath-core/src/catalog.rs` | `swath-catalog-pgstac` | domain↔STAC round-trip identity proptests; live-Postgres integration (`crates/adapters/swath-catalog-pgstac/tests/live.rs`) |
| `TileCache` (content-derived keys, no TTL) | `crates/swath-core/src/cache.rs` | `swath-cache-objectstore` | key→path scheme + get/put round-trip unit tests |
| `EventSource` (pull-shaped, `&mut self`) | `crates/swath-core/src/events.rs` | `swath-events-filedrop` | real-filesystem integration (`crates/adapters/swath-events-filedrop/tests/filedrop.rs`) |
| `IngestReferencer` (sync; virtual references, ADR 0006) | `crates/swath-core/src/ingest.rs` | `swath-referencer` | referencer-equivalence vs the Python conformance reference (`tests/referencer/`) |

### 2.5 Wiring: how an adapter reaches the serving path

The API/render stack is generic over one `S: RasterSource`, so the binary supplies **one**
composite source that owns every compiled-in adapter and dispatches per asset —
[`crates/swath-cli/src/source.rs`](../crates/swath-cli/src/source.rs) (`CompositeSource`).
Wiring a new source is three mechanical edits, all in the binary (the core stays
adapter-blind):

1. Add the crate to `crates/swath-cli/Cargo.toml` `[dependencies]`.
2. Give the adapter a static `handles(&AssetRef) -> bool` recognizing its addressing
   convention — `AssetRef` is an opaque URI; scheme support is purely an adapter concern
   (`swath-source-virtual` claims `*.vmanifest.json#<array>`; the toy claims `inmem:`).
3. Add a field + dispatch arm to `CompositeSource::new`/`describe`/`read_window`.

Other ports wire analogously where `swath-cli` constructs them (`serve.rs`, `ingest.rs`):
adapters are chosen by the binary's configuration, never discovered at runtime.

### 2.6 Feature flags: what they are for (and not for)

Features gate **optional weight**, not adapter selection (ARCHITECTURE §12). The two existing
gates in `crates/swath-cli/Cargo.toml` are the pattern:

- `embedded-ui = ["dep:include_dir"]` — compiles the web bundle into the binary; default ON.
- `legacy-hdf5 = ["swath-referencer/legacy-hdf5"]` — the statically bundled libhdf5 C build;
  default ON, with a documented fast-loop opt-out (`just check-fast` / `just test-fast`) and a
  loud runtime error when a feature-off binary meets `.h5`/`.nc`.

Add a feature only when an adapter drags comparable weight (a C toolchain, a large native
dependency), keep it default-ON so the shipped binary stays batteries-included, feature-gate
the dependent tests so both profiles compile, and document the opt-out in the justfile. The
full gate (`just check`) always runs the default profile.

_Last verified against `9ab35b8`._

## 3. A new openEO process (within the bounded profile)

A derived *product* needs no code at all — that is the point of ADR 0010: publish a process
graph via `POST /services` and it serves as a live XYZ layer; since
[ADR 0014](decisions/0014-preview-bounded-sync-result.md) the same compiler path also backs
`POST /result` previews, so a process added here is previewable and publishable through both
endpoints with identical diagnostics. Extend the *process set* only
when a product cannot be expressed with the current subset. The compiler entry point, verbatim
from [`crates/swath-render/src/process.rs`](../crates/swath-render/src/process.rs):

```rust
pub fn compile(graph: &Json, ctx: &CompileContext) -> Result<CompiledProduct, CompileError>;
```

The supported subset (`load_collection`, `reduce_dimension`, `array_element`,
`add`/`subtract`/`multiply`/`divide`, `linear_scale_range`, `ndvi`, `save_result`) is stated
as a **conformance statement** in that module's docs — the narrowing of each process against
openeo-processes 1.2.0 is documented there and nowhere else. There is no registry to add to:
a process is a match arm in the compiler plus pinned data.

### 3.1 Steps

1. **Pin the official definition.** Copy the process's JSON from openeo-processes **1.2.0**
   (commit `d0ce91fcd347360b907ea2d9589d7564a2c1e1e3` — provenance READMEs in both
   directories) into BOTH `crates/swath-render/tests/data/openeo/` (the compiler's oracle
   copy) and `crates/swath-api/data/openeo-processes/` (the served copy). The two sets must
   stay **byte-identical** — a test asserts it; the files are never edited.
2. **Implement the lowering** in `crates/swath-render/src/process.rs`: an arm in
   `eval_process`, the `SUPPORTED` diagnostics constant, and an update to the module-docs
   conformance statement saying exactly what is narrowed. The process must lower to the
   existing Render IR (`crates/swath-render/src/ir.rs` — `Expr`/`PixelOp`); if the IR lacks
   the operation, that is a deliberate IR extension first (the enums are `#[non_exhaustive]`
   for additive growth, but every `PixelOp` needs its own oracle-backed goldens).
3. **Serve it**: add the `include_str!` + Swath-profile narrowing note to
   `PROCESS_DEFINITIONS` in `crates/swath-api/src/openeo.rs` — definitions are served
   verbatim except the appended `**Swath profile:**` description note; honesty tests pin it.
   The web authoring panel builds its forms from `GET /processes` (schema-driven, #148), so
   it picks the new process up without frontend work.

### 3.2 The test obligations

- **Spec pins** — the committed 1.2.0 definition is the truth the compiler is tested against
  (parameter names, defaults, exception semantics), `crates/swath-render/tests/process_compiler.rs`.
- **Plan equality + eval equivalence** — the graph compiles to the exact hand-built
  `RenderPlan`, and evaluates byte-identically on synthetic and real warped fixtures (the
  "NDVI two ways" pattern).
- **Goldens** — if the process changes renderable output, the compiled plan's tiles must pass
  the perceptual-diff policy against the committed GDAL/rio-tiler oracle renders
  (`tests/oracle/render_reference.py`, regenerated via `just render-goldens`).
- **Diagnostics** — every new failure path is a typed `CompileError` naming the offending
  node, exercised by a minimal broken graph, its Display string pinned by insta snapshot
  (the strings are UX and map onto the standardized openEO error format at the API surface).
- **Properties** — structural invariants (DAG-ness, single result, reference resolution)
  extend `crates/swath-render/tests/process_properties.rs`.
- **Surface honesty** — `crates/swath-api/tests/openeo_conformance.rs`: `GET /processes`
  validates against the pinned openEO API 1.2.0 schema, the render↔api definition sets are
  identical, and the capabilities document lists only what exists.

### 3.3 Boundedness (what a process may not do)

A process is a *pure per-pixel/per-band lowering* into the Render IR. It cannot perform I/O,
introduce new data sources (that is §2), decide windows or granules (`spatial_extent`/
`temporal_extent` are accepted and ignored — tile serving decides), or require state between
requests. Products needing more than the IR's producing/transforming pipeline are the reopen
territory recorded in ADR 0013.

_Last verified against `9ab35b8`._

## 4. A new colormap

The serving vocabulary, verbatim from [`crates/swath-render/src/ir.rs`](../crates/swath-render/src/ir.rs)
and [`crates/swath-render/src/colormaps.rs`](../crates/swath-render/src/colormaps.rs):

```rust
#[non_exhaustive]
pub enum Colormap {
    Grayscale,
    Viridis,
    Magma,
    RdYlGn,
}

pub type Lut = [[u8; 3]; 256];
pub fn lut(map: Colormap) -> Option<&'static Lut>;
```

Palettes are **data with provenance**: matplotlib's published 256-entry byte LUTs vendored
verbatim in `crates/swath-render/src/colormaps/luts.json`, applied by quantized index with
interpolation deliberately off (`lut[q(gray)]`, the same `q` as the gray path). A colormap
applies to gray results only; the plan validator rejects it on composites.

### 4.1 Steps (the same name through five vocabularies)

1. **Vendor the LUT**: extend the regeneration script in
   `crates/swath-render/src/colormaps/README.md` (pinned `matplotlib==3.10.3`; byte-for-byte
   reproducible), rerun it, and record the palette's own data license in that README and in
   the root `REUSE.toml` annotation for `luts.json`.
2. **Render vocabulary**: add the variant to `Colormap` in `crates/swath-render/src/ir.rs`;
   add the parsed table + `lut()` arm in `crates/swath-render/src/colormaps.rs`.
3. **Compiler**: the `save_result` `options.colormap` spelling in
   `crates/swath-render/src/process.rs` (`save_options` — its "unknown colormap" diagnostic
   enumerates the set) and the module-docs conformance statement.
4. **Domain + config vocabularies**: the persisted catalog mirror
   (`crates/swath-core/src/catalog.rs`, snake_case spelling is contractual) with the mapping
   in `crates/swath-render/src/plan.rs` (`domain_colormap` and the `PlanKind` lowering), and
   the static-config spelling (`ColormapConfig` in `crates/swath-cli/src/config.rs`).
5. **Frontend**: the `COLORMAPS` list in `web/src/swath-authoring-panel.ts` (the one
   subtype-specialized widget; everything else is schema-driven).

### 4.2 The test obligations

- **Golden pixels** — `crates/swath-render/tests/colormaps.rs`: exact RGBA at five sample
  stops per variant, literals read off matplotlib directly (never off the committed JSON —
  the test pins the JSON *and* the indexing to the reference at once), plus the
  quantized-index semantics and the palette-needs-gray plan error.
- **Two-level relation** — `crates/swath-render/tests/golden_ir.rs`: the colormapped tile
  stays bit-relatable to the oracle-validated gray tile (`lut[q(gray)]`).
- **Round trips** — openEO option → compiled plan → persisted `swath:layers` metadata →
  recompiled plan (the colormap round-trip test in `crates/swath-api/src/openeo.rs`), the
  catalog's persisted-spelling snapshot/proptests, and `plan_roundtrip.rs`.
- **Diagnostics** — the compiler snapshots that enumerate accepted palette names, and the
  config-file error tests (`crates/swath-cli/src/config.rs`).

_Last verified against `9ab35b8`._

## 5. The proof: a toy adapter built from §2

Branch [`demo/125-toy-source-inmem`](https://github.com/forgo/swath/tree/demo/125-toy-source-inmem)
(deliberately **unmerged**, kept as evidence — diff:
[`main...demo/125-toy-source-inmem`](https://github.com/forgo/swath/compare/main...demo/125-toy-source-inmem))
contains `swath-source-inmem`: a deterministic in-memory `RasterSource` (`inmem:demo`, a 6×4
`UInt8` gradient with planted nodata) built by executing §2's steps in order. The diff is
exactly the guide's step list and nothing else:

| §2 step | Files in the evidence diff |
| --- | --- |
| 1. Scaffold | `crates/adapters/swath-source-inmem/Cargo.toml` |
| 2. Workspace member | `Cargo.toml`, `Cargo.lock` |
| 3. Implement the port | `crates/adapters/swath-source-inmem/src/lib.rs` (clipping, per-row provenance, typed errors, no overviews) |
| 4. Oracle | `tests/oracle/inmem_truth.py` (pinned numpy; independent of the Rust code) → `tests/data/window_truth.json` |
| 5. Truth-table tests | `crates/adapters/swath-source-inmem/tests/windows.rs` (exact SHA-256 pixel identity, provenance honesty, error taxonomy) |
| 6. REUSE annotation | `REUSE.toml` |
| 7. Wiring | `crates/swath-cli/Cargo.toml`, `crates/swath-cli/src/source.rs` (`handles`/dispatch) |
| 8. Full gate | `just check` green at the branch head |

No step outside this list was needed; every step in the list was needed. When the port
signatures or the wiring pattern change, rebuilding this toy (or repeating the exercise for
the changed kind) is the cheapest way to re-verify the guide before bumping its
`_Last verified against_` markers.
