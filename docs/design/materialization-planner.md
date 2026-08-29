# The cost-aware materialization planner (v1)

_Design spec for issue #37 — the `swath-core::planner` module. This resolves
ARCHITECTURE.md §16.4 for v1 and subsumes the tiler's inline strategy
decisions from #36 (cache-first check) and #38 (`select_overview`). After
this lands, `plan()` owns the per-tile strategy choice; the tiler executes
it and the Trace shows the work (CHARTER.md §6: "why did it decide that?").
**Decision recorded in [ADR 0024](../decisions/0024-planner-v1-explicit-knobs-transparent-estimates.md);
this note is the mechanics** — the contract, the cost model, the decision procedure._

## 1. Contract

```rust
pub fn plan(budget: &Budget, availability: &Availability) -> Plan
```

Pure, synchronous, allocation-light: no I/O, no clocks, no randomness. The
**caller** gathers availability (the cache probe result, `describe`
metadata, per-band window geometry) and executes the returned choice; the
planner only decides, and explains every candidate it weighed.

### Inputs

**`Availability`** — what is true about this `(layer, tile)` right now:

- `cache: CacheProbe` — the **result** of the caller's cache lookup, not a
  request to perform one. `plan()` must never cause a double fetch: the
  serve path already calls `TileCache::get` (which returns the payload on a
  hit), so the probe result is `NotConfigured | Disabled | Miss |
  Hit { payload_bytes }`. `Disabled` means the layer's budget opted out
  (`cache_enabled = false`); `NotConfigured` means the server runs without
  a cache at all.
- `tile_size: u32` — target tile side length in pixels.
- `bands: Vec<BandWindow>` — one entry per plan input band whose source
  window intersects its raster (off-raster bands read nothing and are not
  listed). Each carries the **full-resolution** source geometry the tiler
  already computes before any read: the fractional source-pixel extent of
  the tile boundary (`cols × rows`, pre-clip), the sample size in bytes
  (`DType::size_bytes`), and the asset's `overview_levels`.

**`Budget`** — the v1 knobs, per layer. Explicit and transparent
(§16.4 resolution, below); each knob trades storage against latency:

| knob | default | storage-vs-latency meaning |
| --- | --- | --- |
| `cache_enabled: bool` | `true` | `true`: spend object-store storage on encoded tiles to make repeats cheap (probe first, write fresh renders through). `false`: the layer opts out entirely — no probe, no write-through; every request pays the render, no storage grows. |
| `overview_oversample: f64` | `1.2` | GDAL's oversampling slack (`GDALBandGetBestOverviewLevel2`), promoted from the #38 constant to a knob: an overview factor is eligible when `factor <= desired_ratio × oversample`. Raising it serves coarser overviews at more zooms (fewer bytes, softer pixels); `1.0` demands strict decimation (sharper, costlier). The default is calibrated against the rio-tiler oracle (#38: a z11 tile of a 30 m source, ratio ~1.97, must serve the ×2 overview). |
| `max_estimated_live_bytes: Option<u64>` | `None` | A hard ceiling on the estimated cost of a live render. When the estimate exceeds it and nothing cheaper can serve, the planner **refuses** — an explicit error, never an unbounded full-res read. Protects the latency budget against absurd requests (a z0 tile over a continental mosaic). `None` = never refuse (today's behavior). |
| `max_udf_fuel_per_tile: u64` | `100_000_000` | The M9 cost axis (ADR 0018, #205): the deterministic wasmtime fuel a `run_udf` stage may consume per tile. The **executor** enforces it, not `plan()` — the materialization choice is fuel-independent — and a module that exhausts it fails that tile loudly (`UdfError::FuelExhausted`), reproducibly, never a hung worker or a degraded render. Calibration point, not a fit: fuel counts roughly one unit per WASM instruction, so 100 M is tens of milliseconds of CPU — comparable to the ~37 ms a full built-in tile costs end to end (ADR 0012) — while the reference NDVI UDF spends a few million per tile; the 250 ms epoch deadline stays the wall-clock backstop. The consumed fuel is `Trace::udf_fuel_used`; planner feedback from it is the deferred learning loop (ROADMAP). |

### Output

```rust
pub struct Plan {
    pub strategy: PlanChoice,            // CacheHit | Overview{factor} | Live | Refuse{..}
    pub considered: Vec<CandidateTrace>, // every candidate, always all three
}
pub struct CandidateTrace {
    pub strategy: PlannedStrategy,       // cache_hit | overview{factor} | live
    pub estimated_cost_bytes: u64,
    pub admissible: bool,
    pub reason: Cow<'static, str>,       // static, human-legible, deterministic
}
```

The Trace gains `plan: Option<PlanTrace>` (`chosen` + `considered`) — the
x-ray "why" payload. `Trace::decision` remains the executed strategy;
`plan` explains the choice against its alternatives.

## 2. The cost model (v1: transparent estimates, not learned)

Costs are **estimated source bytes decoded** — the quantity the Trace
already measures as `bytes_read`, so estimates are checkable against
reality (and tests do check, §5):

- **cache**: `payload_bytes` — the stored entry, already in hand from the
  probe.
- **overview at factor f**: `Σ_bands ceil(cols/f) × ceil(rows/f) ×
  bytes_per_sample × WARP_COST_WEIGHT`
- **live**: same with `f = 1`.

`WARP_COST_WEIGHT = 1.0`: warp cost scales linearly with source pixels
touched, so v1 folds it into the byte count rather than modeling CPU
separately. All constants are **documented calibration points, not learned
parameters**: the estimate is uncompressed window bytes over the pre-clip
boundary extent, while measured `bytes_read` is compressed COG tiles over
the clipped, margin-padded, tile-aligned window — for the committed HLS
fixtures (DEFLATE ~2:1) the two agree within ~2×, and the test bar is a
loose, documented 3×. Recalibrating a constant is a reviewed edit here,
never a runtime fit.

## 3. Decision procedure

Candidates are evaluated in the fixed order **cache_hit, overview, live**
and every one is recorded in `considered` with its estimate, admissibility,
and reason:

1. **Cache short-circuit.** If the probe is `Hit` and `cache_enabled`, the
   choice is `CacheHit` — terminal, by construction the cheapest: the
   payload fetch was already paid by the probe itself, so any re-render
   would only add cost on top of it. The overview/live candidates are
   recorded as inadmissible with reason "not estimated: cache hit
   short-circuits" (their geometry may not even have been gathered — a hit
   must stay free of source metadata I/O). This is the "cache always wins
   when available and enabled" invariant, property-tested.
2. **Overview candidate.** The eligible factor set is the intersection
   across all bands of `{f ∈ overview_levels : f > 1 and f ≤
   desired_ratio_band × overview_oversample}` where `desired_ratio_band =
   min(cols, rows) / tile_size` (the less-decimating axis is never starved
   — same rule as #38). The candidate is the **coarsest** common eligible
   factor; admissible iff one exists. Requiring a *common* factor keeps the
   #38 invariant "one tile, one honest decision", and strengthens it:
   execution now matches the Trace by construction (under #38 a mixed vote
   read overviews but reported `Live`; the planner picks the coarsest
   factor *every* band can serve, or none).
3. **Live candidate.** Admissible iff `max_estimated_live_bytes` is unset
   or the estimate is within it.
4. **Choice**: the admissible candidate with the smallest estimate; ties
   break by the fixed order above (cache over overview over live), so the
   decision is fully deterministic — same inputs, same `Plan`, always
   (property-tested).
5. **Refusal.** If nothing is admissible (live over the ceiling, no
   overview eligible, no cache hit), the plan is `Refuse { estimated_bytes,
   limit }`. The tiler surfaces it as an explicit `TileError::BudgetExceeded`
   — a loud, explained error tile decision instead of a budget-busting read.

A tile whose every band misses its raster has an empty `bands` list: the
live estimate is 0 and the choice is `Live` — the existing transparent-tile
path, unchanged and still explained.

## 4. Division of labor (what moved, what stayed)

- **Moved into `plan()`**: the cache-first check (#36's `get`-then-serve
  becomes probe → plan → execute) and the overview selection rule (#38's
  `select_overview` re-homed to `swath-core::planner` as the
  overview-candidate helper, threshold now the `overview_oversample` knob).
- **Stayed at the tiler**: window geometry (`source_extent`,
  `clip_to_raster`), the per-strategy read/warp/encode code paths, and the
  **write-through policy** — what to do with a fresh render is a serving
  concern, not a materialization choice (and a planner-owned write policy
  is named future work below).
- **Stayed at the API layer**: key computation, layer resolution, trace
  publication.

## 5. Validation bar

- **Property tests** (proptest, `swath-core`): chosen estimate ≤ every
  admissible candidate's; cache wins whenever hit + enabled; live is never
  chosen while an admissible overview exists (its estimate at `f ≥ 2` is
  strictly smaller); determinism; `max_estimated_live_bytes` admissibility
  respected, including refusal.
- **Trace-asserted integration** (the charter's promise, verbatim): the
  z11 fixture tile **must** be `Overview`, not `Live`; the second identical
  request **must** be `cache_hit` — asserted by reading the Trace, plus
  `plan.considered` carrying all three candidates with sane estimates
  (live > overview at z11).
- **Estimates vs reality**: for the fixture tile, estimated live and
  overview bytes are within **3×** of the measured `bytes_read` of actual
  renders (loose on purpose: compression and clipping live between the
  model and the wire; the bound documents the model's honesty, not its
  precision).
- **Compatibility**: at default knobs every existing golden and the e2e
  suite pass untouched — the planner reproduces #36/#38 behavior exactly on
  every previously-tested path.
- **Cost of deciding**: `plan()` itself benches in the nanosecond–microsecond
  range (criterion) — the decision must be free relative to any strategy it
  picks.

## 6. §16.4 resolved for v1, and future work

**Resolution**: v1 is **explicit per-layer knobs plus transparent cost
estimates** — no learned model, no global optimizer. Rationale: the x-ray
contract (R4) demands every choice be explainable from its inputs; three
knobs with documented storage-vs-latency semantics and a checkable byte
model deliver that today, and Trace history gives a learned model its
training data *later* without redesign.

**Recorded future work** (deliberately not v1; each item is tracked, with
its revisit trigger, in [`../ROADMAP.md`](../ROADMAP.md)'s deferral
inventory):

- **Learned cost model**: fit the estimate constants (compression ratio,
  warp weight, per-source latency) from accumulated Trace history
  (`plan.considered` estimates vs measured `bytes_read`/timings — the
  training pairs are already on the wire).
- **Planner-owned write policy**: today write-through is unconditional at
  the tiler when a cache is configured and enabled; a budget-aware policy
  ("cache only tiles whose live cost exceeds X") belongs to the planner
  once storage pressure is real.
- **Partial-mosaic invalidation**: unchanged from #36 — per-footprint
  invalidation lands with mosaics themselves (`swath-core::cache` docs).
- **Overview *generation*** (the batch-materialization path): shipped by
  issue #183 — `swath materialize` builds per-asset GeoZarr-shaped
  pyramids (`crates/adapters/swath-pyramid-objectstore`) and the
  `PyramidSource` overlay feeds them into `Availability` through
  `describe`, unchanged planner (`docs/ROADMAP.md` deferral row 6,
  closed).
