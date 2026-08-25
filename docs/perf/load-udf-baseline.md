# `run_udf` live-latency baseline (`just load-udf`, issue #207)

Generated 2026-08-25T17:41:27Z at `df7aab1` — Apple M2 Max (12 cores), Darwin 25.5.0 arm64, oha 1.15.0. Recipe wall time to this point: 400s.

Regenerate with `just load-udf` (parameters and rationale: `tests/load/load_udf.py`; scenarios: `tests/load/load_udf.sh`). This file and `load-udf-baseline.json` are the committed evidence for `run_udf` under the ADR 0012 guard (ADR 0018 tile path).

| scenario | requests | errors | rps | p50 ms | p95 ms | p99 ms | max ms |
|---|---:|---:|---:|---:|---:|---:|---:|
| (u) UDF mixed storm (Live NDVI + cache, buster on) | 4147 | 0 | 102.4 | 5.45 | 1230.98 | 1357.67 | 1453.55 |
| (u) healthz UNDER the UDF storm | 188264 | 0 | 9411.9 | 0.39 | 0.66 | 0.96 | 30.13 |
| (f) fuel-bomb storm — every tile refused | 637 | 637 | 15.7 | 886.7 | 1340.67 | 1370.81 | 1410.99 |
| (f) healthz UNDER the fuel-bomb refusals | 190614 | 0 | 9529.5 | 0.39 | 0.63 | 0.92 | 20.7 |

## ADR 0012 signals (recorded — a trip is a maintainer lane-decision)

- **UDF storm** `/healthz` p99: **0.96 ms** (trigger: 50.0 ms) — holds. SSE `/traces`: SURVIVED (4162 trace events). Storm exercised the module (probe mix, buster racing the sampler): {"live": 8, "cache_hit": 7}, every Live sample charging the same deterministic udf_fuel_used 12260531.
- **Fuel bomb** — refused with ZERO collateral. `/healthz` p99 while the runaway module is being refused: **0.92 ms** (holds); SSE SURVIVED (0 trace events). Tile path: 500 RFC 7807 fuel problem; preview `POST /result`: 400 `ProcessGraphComplexity`.
