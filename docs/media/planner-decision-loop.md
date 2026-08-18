# Planner decision loop, with a real `plan.considered`

Hand-crafted SVG — [`planner-decision-loop.svg`](planner-decision-loop.svg) is both the
editable source and the export (canonical). The loop is `plan()` in
`crates/swath-planner/src/lib.rs` (the extracted planner crate, ADR 0016; re-exported at `swath_core::planner`) — pure, deterministic, no I/O. The payload on the right of
the figure is a **real capture, not a mock**: copied verbatim (abridged to the `plan` field)
from [`planner-trace.capture.json`](planner-trace.capture.json), the committed `GET /traces`
SSE frames recorded from the fixture stack. Provenance in
[`planner-decision-loop.notes.md`](planner-decision-loop.notes.md).

![The materialization planner decision loop next to a real captured plan.considered. Loop:
inputs are Budget plus Availability (the cache probe is a result, never a request); a probe
hit with cache enabled terminates as CacheHit with overview and live still recorded as "not
estimated: cache hit short-circuits"; otherwise candidate 1 (cache_hit) is recorded
inadmissible with the probe reason, candidate 2 (overview) takes the coarsest common factor
within the oversample threshold with cost = sum of ceil(cols/f) x ceil(rows/f) x bytes,
candidate 3 (live) is costed at factor 1 and is inadmissible over max_estimated_live_bytes;
if nothing is admissible the planner refuses with BudgetExceeded and emits no Trace, else the
cheapest admissible wins with ties broken by the fixed order cache_hit, overview, live, and
Trace.plan carries chosen plus all three candidates onto the GET /traces SSE stream. Captured
payload (tile 11/424/780, layer truecolor, fixture stack, no cache configured): cache_hit
inadmissible at 0 bytes ("no cache configured"); overview factor 2 admissible at 385 572
bytes ("coarsest overview within the oversample threshold") and chosen, executed as overview
level 2; live admissible at 1 542 288 bytes ("full-resolution
read").](planner-decision-loop.svg)

The overview won on price (385 572 < 1 542 288 bytes, three bands at factor 2 versus full
resolution). The capture's second event (`events[1]`, z12 NDVI) shows the other common
outcome: no eligible overview factor at that zoom, so `chosen` is `"live"`.
