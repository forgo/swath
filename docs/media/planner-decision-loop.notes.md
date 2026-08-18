# Sidecar: planner-decision-loop.md — figure provenance

## The embedded payload

| Figure in diagram | Committed artifact | Verbatim value |
|---|---|---|
| The whole `plan` block (chosen + 3 candidates, costs 0 / 385 572 / 1 542 288 bytes, reasons) | `docs/media/planner-trace.capture.json`, `events[0].trace.plan` | copied verbatim, abridged only by extracting the `plan` field from the trace envelope |
| Tile / layer identity `11/424/780`, `truecolor` | `docs/media/planner-trace.capture.json`, `events[0].tile` / `events[0].layer` | `"11/424/780"`, `"truecolor"` |
| Executed decision `{"overview": {"level": 2}}` | `docs/media/planner-trace.capture.json`, `events[0].trace.decision` | `{"overview":{"level":2}}` |
| z12 counter-example: `chosen: "live"`, reason "no overview factor eligible at this zoom" | `docs/media/planner-trace.capture.json`, `events[1].trace.plan` | `{"chosen":"live","considered":[...,{"strategy":{"overview":{"factor":0}},...,"reason":"no overview factor eligible at this zoom"},...]}` |

## How the capture was made (reproducible)

Recorded 2026-08-10 at git_sha `27deca2d2f91778b53125e939e3e47efd3ccf693` (clean tree),
rustc 1.97.1, dev profile, Apple M2 Max. The events are the verbatim `data:` frames of the
SSE stream:

```sh
cargo run -p swath-cli -- serve --fixtures        # serves ./tests/fixtures on 127.0.0.1:8080
curl -sN http://127.0.0.1:8080/traces             # subscribe first; the stream is live-only
curl -s http://127.0.0.1:8080/tilesets/truecolor/tiles/11/780/424   # OGC order: z/row/col
curl -s http://127.0.0.1:8080/tilesets/ndvi/tiles/12/1561/848
```

The fixture stack configures no tile cache, so candidate 1's reason is
`"no cache configured"`; the `"cache miss"` / short-circuit variants shown in the loop need
the compose stack (`tests/e2e/stack-up.sh`), which mounts a writable cache. The capture's
`timings` are dev-profile and are not used in any diagram (the measured numbers live in
`docs/media/ingest-to-pixel-flow.md`).

## The loop's shape

| Diagram element | Committed artifact |
|---|---|
| Fixed candidate order and "exactly 3 candidates" invariant | `crates/swath-core/src/trace.rs` (`PlanTrace::considered` doc: "in the fixed evaluation order cache_hit, overview, live"); tests `all_three_candidates_are_always_recorded` in `crates/swath-planner/src/lib.rs` and trace assertions in `crates/swath-render/tests/tiler.rs` |
| Reason strings, admissibility rules, cost model, tie-break by `min_by_key` | `crates/swath-planner/src/lib.rs` (`plan()`, `cache_hit_plan()`, `common_overview_factor()`) |
| Refusal emits no trace, tiler raises BudgetExceeded | `crates/swath-core/src/trace.rs` (`PlanTraceExt::trace()`), `crates/swath-render/src/tiler.rs` |
| Design narrative | `docs/design/materialization-planner.md` |
