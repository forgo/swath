# Extending Swath

REQUIREMENTS.md **R9** ("a new source, a new product, a new backend — without editing the
core") turned into concrete, checked steps. Two disciplines keep this document true:
**verified by construction** — the §2 walkthrough was executed step-by-step to build a real toy
adapter on the unmerged evidence branch
[`demo/125-toy-source-inmem`](https://github.com/forgo/swath/tree/demo/125-toy-source-inmem)
(§5) — and **verified against the sources** — each section carries a
`_Last verified against sources_` content fingerprint of its referenced files, checked by the
docs gate (`crates/swath-cli/src/docs_check/stamps.rs`, ARCHITECTURE.md §6's discipline); a PR
that changes a referenced source pastes the new fingerprint the failing gate prints. The
rustdoc on the named files is the normative contract and wins over this guide.

## 1. The extension model — and what is deliberately NOT supported

Decided by [ADR 0013](decisions/0013-extension-features-plus-openeo-graphs.md): **compile-time
Cargo crates/features for adapters, plus openEO process graphs at runtime**. Three kinds: a new
port implementation (§2), a new openEO process (§3), a new colormap (§4).

**Anti-goals** (each a decision with a recorded reopen condition): **no runtime plugin
loading** (deferred, not rejected — ADR 0013); **no sidecar-RPC adapter seam** (the Python
sidecar is an ingest-time conformance *reference*, ADR 0006); **no user-defined processes at
runtime** (the openEO profile is bounded,
[ADR 0010](decisions/0010-openeo-authoring-surface.md) — what a graph cannot express is a
compile-time process addition, §3); **no embedding of a third-party engine** (ADR 0002/0010 —
GDAL and friends appear ONLY as test oracles); **adapter selection is not feature-flagged**
(features gate optional *weight*, §2.6).

## 2. A new source adapter (`RasterSource`)

The port trait lives in [`crates/swath-core/src/source.rs`](../crates/swath-core/src/source.rs)
(`describe` + `read_window`; quoted in ARCHITECTURE.md §6); implementors write plain
`async fn`, and the trait is deliberately **not dyn-compatible** — the binary composes adapters
behind one dispatcher (§2.5). The port rustdoc spells out the obligations — read it in full
first: request **clipping**, full-resolution **coordinate spaces** (adapters own the overview
cover-rounding), **provenance honesty** (the byte ranges actually fetched, in order — the
Trace's raw material, R4), the `SourceError` **taxonomy**, **zero-based bands**.

### 2.1 Steps (each executed on the evidence branch, §5)

1. **Scaffold the crate** at `crates/adapters/swath-source-<name>/` (copy
   `swath-source-virtual`'s manifest pattern); depend on `swath-core` only.
2. **Register it** in the root `Cargo.toml` workspace members.
3. **Implement the trait**, discharging every obligation above.
4. **Write the oracle** (§2.2) and commit its truth table; **write the truth-table tests**
   (§2.3); **annotate generated test data** in `REUSE.toml`.
5. **Wire the binary** (§2.5) and, only for heavy optional weight, a feature gate (§2.6).
6. **Run the full gate**: `just check`.

### 2.2 The oracle obligation

Correctness against real formats is never self-certified (ADR 0002): a pinned, independent
oracle — a PEP 723 script under `tests/oracle/` — generates a JSON truth table the adapter must
match **exactly** (SHA-256 of the raw little-endian pixel bytes); precedents:
`swath-source-cog`, `swath-source-virtual`, the §5 toy. The window list must exercise at
minimum a full-grid read, an interior window, a nodata window, a single pixel, and an
out-of-bounds request that must clip; overview sources add a table replicating the
cover-rounding contract.

### 2.3 The test obligations

The shared schema and assertions live in
[`crates/swath-testsupport/src/truth.rs`](../crates/swath-testsupport/src/truth.rs). A new
adapter's suite must assert **pixel identity** against the oracle; **provenance honesty**
(in-bounds ranges summing to `bytes_read`, a count matching the access geometry); **`describe`
truth**; the **error taxonomy**; and, when servable end-to-end, a render test asserting the
Trace (`swath-source-virtual/tests/render_trace.rs` is the pattern) plus e2e coverage.

### 2.4 The other ports (same pattern, different contracts)

Every outbound port follows the identical recipe (signatures in ARCHITECTURE.md §6):

| Port | Trait file | Existing adapter | Test obligation precedent |
| --- | --- | --- | --- |
| `Reproject`/`CoordTransform` (sync, dyn-compatible) | `crates/swath-core/src/reproject.rs` | `swath-reproject-proj4rs` | pyproj truth tables + proptests |
| `Catalog` (domain-shaped; STAC only inside adapters) | `crates/swath-core/src/catalog.rs` | `swath-catalog-pgstac` | domain↔STAC round-trip proptests; live-Postgres integration |
| `TileCache` (content-derived keys, no TTL) | `crates/swath-core/src/cache.rs` | `swath-cache-objectstore` | key→path scheme + round-trip unit tests |
| `EventSource` (pull-shaped, `&mut self`) | `crates/swath-core/src/events.rs` | `swath-events-filedrop` | real-filesystem integration |
| `IngestReferencer` (sync; ADR 0006) | `crates/swath-core/src/ingest.rs` | `swath-referencer` | referencer-equivalence vs the Python reference (`tests/referencer/`) |

### 2.5 Wiring: how an adapter reaches the serving path

The API/render stack is generic over one `S: RasterSource`, so the binary supplies **one**
composite source dispatching per asset —
[`crates/swath-cli/src/source.rs`](../crates/swath-cli/src/source.rs) (`CompositeSource`).
Wiring is three mechanical edits in the binary: the dependency, a static
`handles(&AssetRef) -> bool` for the adapter's addressing convention, a field + dispatch arm.
Other ports wire analogously: adapters are chosen by configuration, never discovered at
runtime.

### 2.6 Feature flags: what they are for (and not for)

Features gate **optional weight**, not adapter selection; the two existing gates —
`embedded-ui` and `legacy-hdf5` (the bundled libhdf5 C build; default ON,
`just check-fast`/`just test-fast` as the opt-out, a loud error when a feature-off binary meets
`.h5`/`.nc`) — are the pattern. Add one only for comparable weight, default-ON, dependent tests
feature-gated, the opt-out documented in the justfile.

_Last verified against sources `2d91eaf957ed`._

## 3. A new openEO process (within the bounded profile)

A derived *product* needs no code — that is the point of ADR 0010: publish a graph via
`POST /services` and it serves as a live XYZ layer; the same compiler path backs `POST /result`
previews with identical diagnostics ([ADR 0014](decisions/0014-preview-bounded-sync-result.md)).
Extend the *process set* only when a product cannot be expressed with the current subset. The
compiler entry point is `compile` in
[`crates/swath-render/src/process.rs`](../crates/swath-render/src/process.rs); the supported
subset and each process's narrowing against openeo-processes 1.2.0 are a **conformance
statement** in that module's docs and nowhere else. There is no registry: a process is a match
arm plus pinned data.

### 3.1 Steps

1. **Pin the official definition** from openeo-processes 1.2.0 (commit
   `d0ce91fcd347360b907ea2d9589d7564a2c1e1e3`; provenance READMEs in both directories) into
   BOTH `crates/swath-render/tests/data/openeo/` and `crates/swath-api/data/openeo-processes/`
   — byte-identical (a test asserts it), never edited.
2. **Implement the lowering** in `process.rs` (an `eval_process` arm, `SUPPORTED`, the
   conformance statement), lowering to the existing Render IR; a missing IR operation is a
   deliberate IR extension first.
3. **Serve it**: the `include_str!` + profile note in `PROCESS_DEFINITIONS`
   (`crates/swath-api/src/openeo.rs`); the authoring panel is schema-driven (#148), so no
   frontend work.

### 3.2 The test obligations

Spec pins (`process_compiler.rs`); plan equality + byte-identical eval; perceptual-diff
goldens (`just render-goldens`); typed `CompileError` diagnostics pinned by snapshot;
structural proptests; surface honesty (`openeo_conformance.rs`).

### 3.3 Boundedness (what a process may not do)

A process is a *pure per-pixel/per-band lowering* into the Render IR: no I/O, no new data
sources (that is §2), no cross-granule pixels, no state between requests. Temporal arguments
compile into the product's granule-resolution *window* — frame selection per ADR 0015, never
how pixels combine. Products needing more are the reopen territory of ADR 0013.

_Last verified against sources `cdfbc457b1a0`._

## 4. A new colormap

The vocabulary, verbatim from [`crates/swath-render/src/ir.rs`](../crates/swath-render/src/ir.rs)
and [`colormaps.rs`](../crates/swath-render/src/colormaps.rs):

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
verbatim in `colormaps/luts.json`, applied by quantized index, interpolation deliberately off.
A colormap applies to gray results only; the plan validator rejects it on composites.

### 4.1 Steps (the same name through five vocabularies)

1. **Vendor the LUT** via the pinned regeneration script in
   `crates/swath-render/src/colormaps/README.md`; record the palette's license there and in
   `REUSE.toml`.
2. **Render vocabulary**: the `Colormap` variant in `ir.rs`; the table + `lut()` arm in
   `colormaps.rs`.
3. **Compiler**: the `save_result` `options.colormap` spelling in `process.rs`.
4. **Domain + config vocabularies**: the persisted catalog mirror (`catalog.rs`; snake_case is
   contractual) with the mapping in `plan.rs`, and `ColormapConfig` in
   `crates/swath-cli/src/config.rs`.
5. **Frontend**: the `COLORMAPS` list in `web/src/swath-authoring-panel.ts` (the one
   subtype-specialized widget).

### 4.2 The test obligations

Golden pixels (`colormaps.rs`: exact RGBA at five stops per variant, literals read off
matplotlib directly); the two-level relation to the oracle-validated gray tile
(`golden_ir.rs`); the openEO → plan → persisted metadata → recompiled round trips; the
diagnostics snapshots.

_Last verified against sources `ef67a86e77f1`._

## 5. The proof: a toy adapter built from §2

Branch [`demo/125-toy-source-inmem`](https://github.com/forgo/swath/tree/demo/125-toy-source-inmem)
(deliberately **unmerged**, kept as evidence) contains `swath-source-inmem`: a deterministic
in-memory `RasterSource` built by executing §2's steps in order — the diff is exactly the step
list and nothing else. When the port signatures or wiring change, rebuilding this toy is the
cheapest way to re-verify the guide before updating its stamps.
