# ADR 0022 — The two-cube join: `merge_cubes` at the bounded profile

**Status:** Accepted (proposed 2026-08-28; accepted 2026-08-29 with #300/#301 shipped and 33 files depending on it) · **Date:** 2026-08-28 · **Refs:** issue #294 (M11 — Earn the DAG),
`docs/design/authoring-dag.md` (the reasoning), ADR 0010 (bounded openEO profile), ADR 0014
(preview budget), ADR 0015 (frame selection), ADR 0021 (the UI system; its "M11" section defers
this decision here)

## Context

The process compiler (`crates/swath-render/src/process.rs`) admits a single-cube chain: one
`load_collection`, cube operations, one `save_result`. Every served frame is backed by exactly one
granule (ADR 0015). The authoring surface is therefore a line, and the first product that needs
two inputs — change detection between two frames of one collection — cannot be authored, only
approximated by publishing two layers.

A DAG editor without a join is a worse chain (`authoring-dag.md` §1). The join has to come first,
and it has to come at the profile's width: ADR 0010 admits standard processes exactly as wide as
the engine honestly supports and states the narrowing in the served definitions. Scalar arithmetic
is served verbatim (`subtract.json`: `x`, `y` are numbers) and admitted only inside a reducer's
child graph — widening it to cubes would contradict the pinned definitions.

## Decision

Extend the bounded profile with **`merge_cubes`**, the standard's two-cube process, narrowed:

- **`cube1` and `cube2` are gray** — cubes reduced to one value per pixel (`ndvi`,
  `reduce_dimension`, `run_udf` outputs). Multi-band inputs are rejected with a typed diagnostic
  naming the fix ("reduce to one value per pixel first").
- **Same collection on both branches.** Each branch traces back to a `load_collection` of the
  same `id`; different collections are `MergeCubesMismatch`. Same collection means the same
  grid, CRS and band vocabulary — the engine's one-granule-per-branch read stays a plain read.
- **`overlap_resolver` is required**, a reducer child graph over the pair bound as `x` (from
  `cube1`) and `y` (from `cube2`) — the existing child-graph mechanism where `add`/`subtract`/
  `multiply`/`divide` are already admitted. A missing resolver is `MissingResolver`, not a
  default (openEO's default "error on overlap" would be the always-failing case here).
- **`context` is absent.** Not admitted; stated in the served definition.
- **Both branches are frame-selected.** Each branch resolves to exactly one granule (ADR 0015's
  rule per branch, its `temporal_extent` / `filter_temporal` window applied), so the resolver
  applies to every pixel pair; there is no multi-granule read and no aggregation.
- **`datetime=` intersects every branch.** A request's `datetime` is intersected with each
  branch's window before that branch's latest-at-or-before rule runs. The served layer's
  askable frames are the intersection of both windows; a `datetime` that leaves either branch
  without a granule is the tile route's 404, unchanged in shape. (Alternative declined: letting
  `datetime=` move only one branch — it would make "which frame changed" a hidden parameter.)
- **Cache identity:** `layer_version` extends from one granule id to the ordered pair the
  branches resolved to; two requests resolving to the same pair share entries, and no
  `datetime` string enters the key (the ADR 0015 pattern, one id per branch).
- **Trace:** the render trace carries one temporal record per branch (`temporal[]`), so the
  x-ray can show which two granules a tile's pixels came from (issue #296).
- **Preview:** `POST /result` renders the pair under ADR 0014's one budget; a two-branch graph
  that exceeds it is refused in plain words, not degraded.

The served `merge_cubes` definition states every narrowing above in its description, as the
profile's other entries do.

## Consequences

- The compiler grows from a chain to a **two-source DAG**: graph parsing, type checking and
  lowering accept a node with two cube inputs; everything else (single-source chains, UDFs,
  frame selection, caching) is unchanged byte-for-byte — the byte-identical NDVI check stays the
  regression net.
- Authoring can become a constrained DAG editor (`authoring-dag.md` §4) whose first product is
  change detection (#300). B10 (dead steps) moves from unconstructible to explained + gated;
  every other bad state keeps or improves its status (§6 there).
- The x-ray, time slider and compare must understand two-source layers (#296, #301).

## Reopen / supersede conditions

- **`mask`** (a second cube gating the first): reopen when a product needs it *and* the IR has
  a nodata/replacement vocabulary to express it honestly.
- **Band-wise merge** (two multi-band cubes into one wider cube): reopen when a consumer needs a
  composite across collections; it needs band-namespace rules this ADR does not decide.
- **Cross-CRS / cross-grid branches**: reopen when a second collection must join the first; it
  needs a resampling step in the IR (the warp port exists, its placement in a graph does not).
- **N > 2 joins**: reopen with the first three-input product; chaining two `merge_cubes` is the
  interim answer and may be enough.
- **The alternative `datetime=` rule** (per-branch time parameters): reopen if the intersection
  rule proves too restrictive in the device pass — with the trace evidence of which frames users
  actually pair.
- **Temporal aggregation** stays deferred beside UDF/reducer semantics (ADR 0015's out-of-scope
  list); this join selects frames, it never combines more than one granule per branch.
