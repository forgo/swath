# ADR 0024 — The materialization planner v1: explicit per-layer knobs, transparent byte estimates, every candidate explained

**Status:** Accepted · **Date:** 2026-08-29 (records the decision taken for issue #37 in
`docs/design/materialization-planner.md`; #340) · **Refs:** `docs/design/materialization-planner.md`
(contract, cost model, decision procedure), ARCHITECTURE.md §16.4 (resolved), REQUIREMENTS R4,
ADR 0012 (render inline), ADR 0018 (the fuel axis), ROADMAP §2 rows 3–5

## Context

The tiler chose live / overview / cache inline (#36, #38). ARCHITECTURE §16.4 asked what the
planner's budget semantics should be: a learned cost model, a global optimizer, or explicit
knobs. R4 (glass box) demands that every choice be explainable from its inputs and that the
explanation be the same data the tests assert on. The decision shipped as `swath-planner` and
resolved §16.4 in the design note only.

## Decision

1. **`plan(budget, availability) -> Plan` is pure** — no I/O, clocks or randomness; the caller
   gathers availability (the cache probe *result*, `describe` metadata, per-band window
   geometry) and executes the choice. A plan never causes a second fetch.
2. **Three explicit per-layer knobs, each a storage-vs-latency trade:** `cache_enabled`,
   `overview_oversample` (GDAL's slack, calibrated against the rio-tiler oracle), and
   `max_estimated_live_bytes` — a ceiling above which the planner **refuses** rather than reads.
   `max_udf_fuel_per_tile` is enforced by the executor, not the planner (ADR 0018).
3. **Costs are estimated source bytes decoded**, the quantity the Trace measures as
   `bytes_read`, so estimates are checkable against reality; constants are documented
   calibration points, never runtime fits.
4. **Fixed candidate order — cache hit, overview, live — and every candidate is recorded** in
   `Plan::considered` with its estimate, admissibility and a static reason. A cache hit
   short-circuits; the overview candidate is the coarsest factor *every* band can serve; the
   choice is the cheapest admissible candidate with deterministic tie-breaks. Same inputs, same
   plan, property-tested.
5. **No learned model and no global optimizer in v1.** Trace history already carries the
   training pairs; a learned model is deferred with its trigger (ROADMAP §2 row 4).

## Consequences

- The x-ray's "why" view is `Plan::considered` verbatim; the e2e and the planner's tests assert
  on the same structure (R4 by construction).
- Execution matches the Trace by construction: under #38 a mixed vote could read overviews and
  report `Live`; the common-factor rule removes that case.
- `TileError::BudgetExceeded` is a loud, explained refusal, never an unbounded full-resolution
  read.
- Recalibrating a constant, changing the candidate order, or adding a knob is a reviewed edit
  to the crate and this ADR's successor; the planner-owned write policy and partial-mosaic
  invalidation remain deferred (ROADMAP §2 rows 5, 3).
