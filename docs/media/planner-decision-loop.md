# Planner decision loop, with a real `plan.considered`

The loop below is `plan()` in `crates/swath-core/src/planner.rs` — pure, deterministic, no
I/O. The payload on the right of the diagram is a **real capture, not a mock**: it is copied
verbatim (abridged to the `plan` field) from
[`planner-trace.capture.json`](planner-trace.capture.json), the committed `GET /traces` SSE
frames recorded from the fixture stack. Provenance in
[`planner-decision-loop.notes.md`](planner-decision-loop.notes.md).

```mermaid
flowchart TD
    IN["Inputs: Budget + Availability<br/>cache probe result, tile size, per-band windows<br/>probe is a result, never a request"] --> HIT{"probe is Hit<br/>and cache enabled?"}
    HIT -->|"yes"| SHORT["Terminal: CacheHit<br/>candidate 1 admissible, cost = payload bytes;<br/>overview and live recorded inadmissible:<br/>'not estimated: cache hit short-circuits'"]
    HIT -->|"no"| C1["Candidate 1 — cache_hit, inadmissible<br/>reason: no cache configured, disabled, or miss"]
    C1 --> C2["Candidate 2 — overview<br/>coarsest common factor f with<br/>f ≤ desired ratio × oversample, across all bands;<br/>cost = Σ ceil(cols/f)·ceil(rows/f)·bytes per sample"]
    C2 --> C3["Candidate 3 — live, cost at f = 1<br/>admissible unless it exceeds<br/>max_estimated_live_bytes"]
    C3 --> ANY{"any admissible<br/>candidate?"}
    ANY -->|"yes"| PICK["Choose cheapest admissible<br/>ties break by fixed order:<br/>cache_hit, overview, live"]
    ANY -->|"no"| REFUSE["Refuse: BudgetExceeded<br/>no Trace emitted"]
    PICK --> TR["Trace.plan = chosen + considered<br/>invariant: exactly 3 candidates,<br/>always in order cache_hit, overview, live"]
    SHORT --> TR
    TR --> SSE["Rides the GET /traces SSE stream untouched"]
```

## The captured payload (abridged)

Tile `11/424/780`, layer `truecolor`, fixture stack (`swath serve --fixtures`, no cache
configured — hence candidate 1's reason). From `planner-trace.capture.json`, `events[0]`:

```json
{
  "chosen": { "overview": { "factor": 2 } },
  "considered": [
    {
      "strategy": "cache_hit",
      "estimated_cost_bytes": 0,
      "admissible": false,
      "reason": "no cache configured"
    },
    {
      "strategy": { "overview": { "factor": 2 } },
      "estimated_cost_bytes": 385572,
      "admissible": true,
      "reason": "coarsest overview within the oversample threshold"
    },
    {
      "strategy": "live",
      "estimated_cost_bytes": 1542288,
      "admissible": true,
      "reason": "full-resolution read"
    }
  ]
}
```

Both admissible candidates were costed; the overview won on price (385 572 < 1 542 288 bytes,
three bands at factor 2 versus full resolution), and the executed decision in the same trace is
`{"overview": {"level": 2}}`. The capture's second event (`events[1]`, z12 NDVI) shows the
other common outcome: no eligible overview factor at that zoom, so `chosen` is `"live"`.
