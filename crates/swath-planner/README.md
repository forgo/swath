# swath-planner

Cost-aware materialization planner extracted from
[Swath](https://github.com/forgo/swath) (ADR 0016): `plan()` chooses, per
`(layer, tile)`, one of **cache-hit / overview / live** under an explicit
per-layer `Budget` — and returns *every* candidate it weighed, with its
estimate, admissibility, and reason. The decision is the API; the
explanation rides with it.

## Purity contract

`plan()` performs no I/O and consults no clocks. The caller gathers
`Availability` — the cache probe **result** (never a request to probe, so
planning can never double-fetch) plus per-band window geometry from
metadata it already holds — and executes the returned choice. Same inputs,
same `Plan`, always. Planning costs tens of nanoseconds; it is safe to
call per tile request.

## The cost model (v1: transparent, calibratable, not learned)

Costs are **estimated source bytes decoded**, the same quantity a serving
system can measure afterwards, so estimates stay checkable against
reality:

- cache: the stored payload length (already fetched by the probe);
- overview at factor `f`: `Σ_bands ceil(cols/f) · ceil(rows/f) ·
  bytes_per_sample`, warp-weighted;
- live: the same at `f = 1`.

The overview-eligibility rule is GDAL-calibrated (the 1.2 oversampling
slack), and every constant is a documented calibration point — never a
runtime fit. Property tests travel with the crate; the microbenchmark
pins the "trivially cheap" claim.

## Dependencies

Exactly `serde` (for the plan/trace payload shapes). Trait-shaped,
self-contained inputs — no engine types.

## Status

Published as a `0.1.0-alpha.N` — built from a tagged commit through the
full Swath CI gate, with no API stability promised between alphas.
Licensed Apache-2.0.
