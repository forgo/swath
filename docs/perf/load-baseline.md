# Load baseline (`just load`, issue #101)

Generated 2026-08-10T17:52:37Z at `27deca2` — Apple M2 Max (12 cores), Darwin 25.5.0 arm64, oha 1.15.0. Recipe wall time to this point: 89s.

Regenerate with `just load` (parameters and rationale: `tests/load/load.py`; scenarios: `tests/load/load.sh`). This file and `load-baseline.json` are the committed evidence for ARCHITECTURE §16.7 (async-vs-blocking render boundary, issue #102).

| scenario | requests | errors | rps | p50 ms | p95 ms | p99 ms | max ms |
|---|---:|---:|---:|---:|---:|---:|---:|
| healthz — idle baseline | 84450 | 0 | 16883.2 | 0.23 | 0.3 | 0.36 | 3.84 |
| (a) hot-cache tile storm | 26306 | 0 | 1313.7 | 23.33 | 34.91 | 41.79 | 72.57 |
| (b) cold live-render burst | 128 | 0 | 11.5 | 660.61 | 1140.7 | 1221.21 | 1231.08 |
| (c) mixed tile storm | 813 | 0 | 20.1 | 852.82 | 1983.87 | 2051.72 | 2096.35 |
| (c) healthz UNDER WARPS | 190707 | 0 | 9533.8 | 0.39 | 0.64 | 0.97 | 30.35 |

## §16.7: control plane under concurrent large warps

- `/healthz` p99 under warps: **0.97 ms** (idle: 0.36 ms); max 30.35 ms (idle: 3.84 ms). Scenario params: GET /healthz, c=4, 20s, started 5s into the mixed storm.
- SSE `/traces` subscription: SURVIVED the 45s window (828 trace events, 0 keepalives received).
- Storm decision probes (is the storm actually Live?): {"live": 13, "cache_hit": 2}; cold-burst decisions: {"live": 128}.
