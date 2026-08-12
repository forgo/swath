# Load evidence — 2-CPU pinned rerun (`just load`, §16.7 / issue #102)

Generated 2026-08-10T15:15:28Z at `dfaa7f0` — Apple M2 Max host, Darwin 25.5.0 arm64, oha 1.15.0, **server pinned to 2 CPUs** (the constrained-VM shape of ADR 0012's maintainer-requested rerun; this run's numbers are the "2 CPUs (pinned)" column of that ADR's decision table). Recipe wall time to this point: 90s.

Regenerate with `just load` under the same 2-CPU pin (parameters and rationale: `tests/load/load.py`; scenarios: `tests/load/load.sh`). This file and the 12-core `load-baseline.json` are the committed evidence for ARCHITECTURE §16.7 (async-vs-blocking render boundary, issue #102), resolved by [ADR 0012](../decisions/0012-render-stays-inline-async.md).

| scenario | requests | errors | rps | p50 ms | p95 ms | p99 ms | max ms |
|---|---:|---:|---:|---:|---:|---:|---:|
| healthz — idle baseline | 80658 | 0 | 16124.0 | 0.24 | 0.32 | 0.43 | 3.86 |
| (a) hot-cache tile storm | 25582 | 0 | 1277.6 | 23.46 | 37.68 | 45.74 | 90.17 |
| (b) cold live-render burst | 128 | 0 | 8.0 | 965.57 | 1407.65 | 1477.65 | 1490.99 |
| (c) mixed tile storm | 408 | 0 | 9.9 | 1799.33 | 3252.83 | 3367.92 | 3398.07 |
| (c) healthz UNDER WARPS | 112653 | 0 | 5632.0 | 0.41 | 0.71 | 1.44 | 68.59 |

## §16.7: control plane under concurrent large warps

- `/healthz` p99 under warps: **1.44 ms** (idle: 0.43 ms); max 68.59 ms (idle: 3.86 ms). Scenario params: GET /healthz, c=4, 20s, started 5s into the mixed storm.
- SSE `/traces` subscription: SURVIVED the 45s window (422 trace events, 0 keepalives received).
- Storm decision probes (is the storm actually Live?): {"live": 9, "cache_hit": 6}; cold-burst decisions: {"live": 128}.
