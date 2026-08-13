# Temporal + overview load baseline (`just load-temporal`, issue #184)

Generated 2026-08-13T08:18:58Z at `3f44d55` — Apple M2 Max (12 cores), Darwin 25.5.0 arm64. Recipe wall time to this point: 211s.

Regenerate with `just load-temporal` (parameters and rationale: `tests/load/temporal.py`; scenarios: `tests/load/temporal.sh`). This file and `temporal-baseline.json` are the committed M7 evidence (ADR 0015 frame-serving + the #183/#218 pyramid path) quoted by `docs/PERFORMANCE.md`.

| scenario | requests | errors | rps | p50 ms | p95 ms | p99 ms | max ms |
|---|---:|---:|---:|---:|---:|---:|---:|
| (d) frame loop, cold (all Live) | 54 | 0 | 14.6 | 345.99 | 606.77 | 619.69 | 619.69 |
| (d) frame loop, hot (all cache hits) | 54 | 0 | 150.7 | 7.91 | 270.43 | 271.12 | 271.12 |
| (e) z12 — Live (full resolution) | 24 | 0 | 3.7 | 259.24 | 269.31 | 288.11 | 288.11 |
| (e) z10 pre-materialize (embedded ov. x2) | 24 | 0 | 4.0 | 230.59 | 257.88 | 290.6 | 290.6 |
| (e) z11 post-materialize (pyramid ov. x2) | 24 | 0 | 3.7 | 246.59 | 296.56 | 317.68 | 317.68 |
| (e) z10 post-materialize (pyramid ov. x4) | 24 | 0 | 6.6 | 134.75 | 146.19 | 162.52 | 162.52 |

- `swath materialize --min-dim 64`: 578 ms wall for the whole store (every layer of both datasets, run once between the pre- and post-materialize rungs).
- Frame decisions (from `x-swath-trace`): cold {"live": 54}, hot {"cache_hit": 54} — the cold pass is all Live renders, the hot pass all granule-scoped cache hits.
- Overview-rung decisions (SSE envelopes, level included): live_z12 {"live": 24}; embedded_z10 {"overview:2": 24}; pyramid_z11 {"overview:2": 24}; pyramid_z10 {"overview:4": 24}.
