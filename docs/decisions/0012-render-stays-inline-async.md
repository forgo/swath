# ADR 0012 — Render stays inline on the async runtime (§16.7 confirm-closed with load evidence)

**Status:** Accepted · **Date:** 2026-08-10 · **Resolves:** ARCHITECTURE §16.7 · **Evidence:** issue #101/#102 load baselines

## Context

§16.7 asked whether CPU-bound render work (warp/resample/eval/encode, ~37 ms per full tile —
`docs/perf/bench-baseline.json`) must move off tokio's async worker threads (`spawn_blocking`,
rayon, or a dedicated pool) to keep the control plane (`/healthz`, `/traces` SSE) responsive
under concurrent load. The code deferred it with "until a server actually feels the latency."
The `just load` harness (#101) made that criterion measurable, and per maintainer direction the
scenario was additionally rerun with the server pinned to **2 CPUs** (the constrained-VM shape
where starvation would most plausibly appear).

## Decision

**No code change: rendering stays inline on the tokio multi-thread runtime.** Measured, on both
hardware shapes, during a sustained mixed live-render storm (c=16, 40 s):

| Metric | 12 cores | 2 CPUs (pinned) |
|---|---|---|
| `/healthz` p99 under warps | 0.94 ms | 1.44 ms |
| `/healthz` worst single sample | 22 ms | 68.6 ms |
| SSE `/traces` through the storm | survived (932 events) | survived (422 events) |
| Mixed-storm tile p50 | 820 ms | 1 799 ms |
| Errors | 0 | 0 |

At 2 CPUs the *renders* starve exactly as arithmetic demands (there is simply less CPU), but the
control plane stays ≤1.44 ms p99 and the SSE stream never drops: the async loop itself is not
blocked, and `spawn_blocking` cannot manufacture CPU that is not there. Implementing it would
add thread-handoff complexity to the hot path with no demonstrable benefit on any measured shape.
Baselines: `docs/perf/load-baseline.json` (12-core, committed by #101) and
`docs/perf/load-2cpu-16.7-evidence.md` (this decision's constrained run).

## Reopen trigger

Reopen §16.7 if, on deployment-representative hardware, the standard load scenario (c) shows
**`/healthz` p99 > 50 ms** or a **dropped SSE subscription** — or if tile workloads change class
(e.g. ≥10× larger render units) such that per-poll compute blocks grow beyond the scheduler's
tolerance. The `just load` harness is the standing instrument for checking this.

## Consequences

- ARCHITECTURE §11's "CPU-bound warp/resample on a rayon pool or spawn_blocking" sketch is
  superseded by this evidence-based resolution; §16.7 is closed with a link here.
- The deferral notes in `swath-render`/`swath-api` docs update to point at this ADR.
