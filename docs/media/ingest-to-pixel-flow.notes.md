# Sidecar: ingest-to-pixel-flow.md — figure provenance

Every number in the diagram, its committed artifact, and the verbatim value it was derived
from. Rounding: bench `median_ns` values are converted to ms/µs/ns and rounded to the shown
precision; nothing else is transformed.

| Figure in diagram | Committed artifact | Verbatim value |
|---|---|---|
| Planner 45–73 ns | `docs/perf/bench-baseline.json` | `plan_overview_single_band` median_ns 44.8; `plan_cache_hit` 53.2; `plan_miss_three_bands` 72.8 |
| Source window 13.4 µs | `docs/perf/bench-baseline.json` | `source_window_z12` median_ns 13437.4 |
| Warp 8.0 ms nearest / 8.9 ms bilinear | `docs/perf/bench-baseline.json` | `warp_nearest_fullres_z12` median_ns 7965875.0; `warp_bilinear_fullres_z12` median_ns 8875475.7 |
| Render IR 3.2 / 3.3 ms | `docs/perf/bench-baseline.json` | `eval_ndvi_grayscale` median_ns 3152751.3; `eval_ndvi_rdylgn` median_ns 3290739.6 |
| Encode PNG 13.0 ms | `docs/perf/bench-baseline.json` | `encode_png_ndvi_256` median_ns 12959458.2 |
| Full composite 37.3 ms median | `docs/perf/bench-baseline.json` | `composite/render_tile_ndvi_z12` median_ns 37276941.7 |
| Hot cache p50 22.27 ms | `docs/perf/load-baseline.json` (also `load-baseline.md`) | scenario `hot_cache_storm` p50_ms 22.27 (27 239 requests, 0 errors) |
| Cold live p50 653.88 ms | `docs/perf/load-baseline.json` (also `load-baseline.md`) | scenario `cold_live_burst` p50_ms 653.88 (128 unique z15 tiles, each once, decisions `{"live": 128}`) |
| Ingest-to-pixel 297 ms and 801 ms local, 535 ms CI | `docs/DEMO.md` ("Current measured numbers", issue #35); same numbers in the `I2P_BUDGET_MS` doc comment in `crates/swath-e2e/src/main.rs` | "Local (dev laptop): 297 ms, 801 ms"; "CI (GitHub runner): 535 ms" |
| Budget 10 000 ms | `crates/swath-e2e/src/main.rs` | `const I2P_BUDGET_MS: u64 = 10_000;` |
| Virtual-reference 14 ms warm / 29 ms cold (prototype-grade) | `prototypes/0001-2026-08-08-referencer-bakeoff/README.md` §HDF5 latency | "referencer-rs 29 ms cold / 14 ms warm" (Apple M2 Max, local file, dev-profile harness) |

Bench context (`docs/perf/bench-baseline.json` header): criterion medians, captured 2026-08-10
at git_sha `b7a775b`, rustc 1.97.1, Apple M2 Max, committed HLS fixtures, tile z12/848/1561;
`median_ns` is the criterion median, `mad_ns` the median absolute deviation. Load context
(`docs/perf/load-baseline.json` header): oha 1.15.0, generated 2026-08-10 at git_sha `b927977`,
Apple M2 Max 12 cores.

Stages shown with **no committed timing** (and therefore no number): granule-event detection,
register asset, catalog upsert, layer/RenderSpec resolve, isolated source byte read
(`source_window_z12` measures window *computation* only; the composite bench reads from an
in-memory store), and cache write-through. The per-stage `timings` values in
`crates/swath-core/src/trace.rs` tests are hand-written serde fixtures, not measurements, and
are deliberately not used here.
