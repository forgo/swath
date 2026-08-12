# ADR 0015 — The time dimension: frame selection, not aggregation

**Status:** Proposed · **Date:** 2026-08-12 · **Consumes:** `docs/ROADMAP.md` deferral row 7
(touches row 15) · **Source:** issue #178, v1 semantics selected by the maintainer in planning

## Context

Tile serving today has exactly one temporal behavior: **latest wins**. A catalog-backed layer
resolves each tile request against its dataset's latest granule — maximum acquisition
`datetime` at millisecond precision, ties by granule id, a total documented order
(`crates/swath-api/src/provider.rs`, `latest()`). The process compiler accepts
`load_collection`'s `temporal_extent` and **ignores** it (`crates/swath-render/src/process.rs`
module docs: "tile serving decides the window and the granule, not the product graph"), and
config-defined datasets carry a whole-world, open-ended placeholder extent
(`crates/swath-cli/src/config.rs`) — deferral rows 7 and 15 record both honestly.

Row 7's revisit trigger has arrived by decision: the charter promises "animate a time series",
the M7+ ordering puts the time dimension third ("the largest capability gap a user can *see*,
and it is the prerequisite for EDR"), and a dataset with more than one granule per footprint
makes "latest" the wrong answer for any historical comparison. Row 7 also names the design
constraint: time deserves a **designed surface that paves the road to EDR, not an ad-hoc query
param**.

The domain model is already time-shaped — this ADR adds no new vocabulary. Granules carry an
acquisition `datetime` (RFC 3339 UTC, `swath_core::catalog::Datetime`); `GranuleQuery` already
has an optional inclusive, optionally open-ended `datetime: TimeRange` filter that
`Catalog::find_granules` honors; `Extent.interval` is a `TimeRange` on every Dataset. What is
missing is purely the serving semantics: which granule does a *time-parameterized* frame show,
and how do requests say so.

Two shapes were on the table: **frame selection** ("which single granule backs this frame") and
**temporal aggregation** (reducing over a time window — means, composites, `reduce_dimension`
over `t`). Aggregation is a different machine: it needs multi-granule reads per tile, reducer
semantics the IR does not have, and it collides with the same open questions as user-defined
reducers. Frame selection needs only a resolution rule over machinery that exists.

## Decision

The v1 time dimension is **frame selection**: every rendered frame is backed by exactly one
granule, chosen by time. Aggregation is explicitly not attempted.

- **`datetime=` on the tile route.** `GET /tilesets/{layerId}/tiles/{tileMatrix}/{tileRow}/{tileCol}`
  gains one optional query parameter, `datetime`, with exactly the OGC API grammar (Features /
  Common Part 2, reused verbatim by EDR): an RFC 3339 instant (`2026-06-06T17:54:00Z`) or an
  interval (`start/end`, either side openable as `..`). Same name, same grammar, same
  double-open rejection as the standards demand — this is the "designed surface, paves EDR"
  requirement made concrete: when OGC API - EDR lands (M7+ item 11), its `datetime` parameter
  is *this* parameter, not a second one. A malformed value is a 400 naming the grammar.
- **Resolution rule: latest-at-or-before.** An instant `t` selects the latest granule with
  acquisition `datetime ≤ t` — the granule that was current at `t`. An interval resolves at
  its **end**: the latest granule whose datetime falls within the (inclusive) interval. An
  absent `datetime` is the fully open interval, which resolves to **latest** — today's
  behavior, unchanged byte-for-byte; the parameter is purely additive. A `datetime` that
  selects no granule (before the first acquisition, or outside a narrowed window) is a 404,
  the same shape as "no granule ingested yet". Mechanically, resolution stays one
  `find_granules` per request, now carrying the query's already-existing `datetime: TimeRange`
  filter instead of `GranuleQuery::default()`.
- **Graph-side windows constrain resolution** (the ADR 0010 narrowing pattern: standard
  parameters admitted at exactly the width the engine honestly supports, narrowing declared in
  the served process definitions). `load_collection`'s `temporal_extent`, and the standard
  `filter_temporal` process (`dimension` limited to the temporal dimension), stop being
  accepted-and-ignored: they compile into a **resolution window** on the derived layer — the
  request's `datetime` is intersected with the layer's window before the latest-at-or-before
  rule runs. They select *which frames the layer can show*, never *how pixels combine*. The
  served process descriptions state this narrowing honestly, exactly as the compiler's other
  narrowings are stated.
- **Dataset temporal extents become real** (the temporal half of deferral row 15, whose
  trigger — "EDR needs real temporal extents" — this ADR fires). `Extent.interval` is derived
  and maintained from ingested granules (min/max acquisition datetime) rather than left as the
  open-ended placeholder, and served where extents already flow: OGC collection/tileset
  metadata and the openEO/STAC collection documents. Clients need to know what times are
  askable before `datetime=` is usable; the spatial half of row 15 stays deferred.
- **Cache identity: no change needed.** The tile cache is already granule-scoped:
  `layer_version` prefixes the plan hash with the backing granule id for catalog-backed layers
  (`crates/swath-core/src/cache.rs`, `pub fn layer_version(granule: Option<&str>, …)` and the
  module's "`layer_version` v1 semantics" section). A time-parameterized frame keys under the
  version of the granule it *resolved to* — two requests resolving to the same granule share
  entries, historical frames are immutable under their keys, and no `datetime` string ever
  enters the key. The whole-version-bump invalidation story (and its recorded deferrals, rows
  2/3) is untouched.

**Explicitly out of scope, stated honestly:**

- **Temporal aggregation and reducers** — `reduce_dimension` over the temporal dimension,
  `aggregate_temporal*`, composites, "cloud-free mosaic". These require multi-granule reads
  and reducer semantics the IR does not have, and they share their real design questions with
  user-defined reducer/UDF semantics (the extension posture of ADR 0013); they are deferred
  **beside** that work, to be decided together, not smuggled in as a widened `datetime`.
- **Time-series endpoints** — position/area queries returning values over time are EDR's job
  (M7+ item 11), not the tile route's. This ADR deliberately paves that road — EDR mandates
  the same `datetime` parameter grammar adopted here, and it wants the real temporal extents
  this ADR makes true — but builds none of it.

## Consequences

- The user-visible capability gap closes at its wedge: animation and historical comparison are
  a client stepping `datetime=` across the dataset's now-served temporal extent, one cacheable
  frame per granule — no new endpoint, no new runtime shape, still exactly one granule read
  per tile.
- The tile route stays OGC-shaped: one standard optional parameter; requests without it are
  bitwise today's behavior, so nothing existing (viewer, e2e goldens, load baselines) moves.
- The compiler's conformance statement in `crates/swath-render/src/process.rs` changes
  honestly: `temporal_extent` graduates from accepted-and-ignored to a compiled resolution
  window, `filter_temporal` joins the supported subset (narrowed to frame-window semantics),
  and the served process definitions carry the narrowing per ADR 0010.
- Deferral bookkeeping: row 7 is consumed by this ADR (its reopen conditions now live below,
  which win per the roadmap's rule); row 15 splits — temporal extents land here, spatial
  extents remain deferred on the Records trigger.
- Single-granule-per-frame is preserved, so the cache's whole-version invalidation and the
  mosaic deferrals (rows 2/3) are exactly as right after this ADR as before it.
- Validation the established way: resolution-rule semantics (instant, interval, open ends,
  ties, empty window → 404) as table-driven tests over the catalog port; parameter grammar
  pinned against the OGC definition; the absent-parameter path proven identical against the
  existing goldens.

## Reopen / supersede conditions

Supersede this ADR when any of the following demand appears:

- **Aggregation demand** — a user needs pixels that combine granules (temporal mean, best-pixel
  composite, `reduce_dimension` over `t`): that is the deferred reducer/UDF design, and it must
  be decided there — never by quietly reinterpreting `datetime=` intervals as reductions.
- **Latest-at-or-before proves wrong** — a served dataset where nearest-in-time (or
  latest-*within* with different interval semantics) is demonstrably what users mean: redecide
  the resolution rule with the evidence, as a supersession, since clients will have encoded it.
- **EDR lands** — the EDR ADR must restate the division of labor: this parameter's grammar and
  the resolution rule are its foundation, and any tension EDR conformance surfaces reopens
  this decision rather than forking the parameter.
- **Multi-granule mosaics land** — "one granule per frame" is this ADR's load-bearing
  simplification; mosaic layers change frame identity, cache granularity (row 3), and the
  resolution rule together.
