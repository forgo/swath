# Load baseline (`just load`, issue #101)

Generated 2026-08-10T13:39:23Z at `b927977` — Apple M2 Max (12 cores), Darwin 25.5.0 arm64, oha 1.15.0. Recipe wall time to this point: 154s.

Regenerate with `just load` (parameters and rationale: `tests/load/load.py`; scenarios: `tests/load/load.sh`). This file and `load-baseline.json` are the committed evidence for ARCHITECTURE §16.7 (async-vs-blocking render boundary, issue #102).

| scenario | requests | errors | rps | p50 ms | p95 ms | p99 ms | max ms |
|---|---:|---:|---:|---:|---:|---:|---:|
| healthz — idle baseline | 83235 | 0 | 16638.2 | 0.24 | 0.3 | 0.36 | 2.73 |
| (a) hot-cache tile storm | 27239 | 0 | 1360.3 | 22.27 | 34.65 | 41.57 | 77.69 |
| (b) cold live-render burst | 128 | 0 | 11.5 | 653.88 | 1200.51 | 1232.39 | 1236.41 |
| (c) mixed tile storm | 917 | 0 | 22.5 | 820.02 | 1934.54 | 2040.75 | 2093.14 |
| (c) healthz UNDER WARPS | 190856 | 0 | 9541.0 | 0.39 | 0.64 | 0.94 | 22.14 |

## §16.7: control plane under concurrent large warps

- `/healthz` p99 under warps: **0.94 ms** (idle: 0.36 ms); max 22.14 ms (idle: 2.73 ms). Scenario params: GET /healthz, c=4, 20s, started 5s into the mixed storm.
- SSE `/traces` subscription: SURVIVED the 45s window (932 trace events, 0 keepalives received).
- Storm decision probes (is the storm actually Live?): {"live": 12, "cache_hit": 3}; cold-burst decisions: {"live": 128}.
