# Sidecar: product-loop.svg — figure provenance

The diagram carries no numbers by design (the README's measured figures live under
"Measured, not promised" with their markers). Every box and connector names a capability;
this table names the committed artifact that makes each one true.

| On the SVG | Committed artifact |
|---|---|
| **Granules land** — COG, or archival HDF5 served in place; S3 or disk | `crates/adapters/swath-source-cog`; `crates/adapters/swath-source-virtual` + `crates/swath-referencer` ("served in place, never converted": `virtual-reference.md`); store backends in `OPERATIONS.md` §2; events sources (watch dir, S3) in `OPERATIONS.md` §4 |
| **no per-scene work** | the ingest loop (`crates/swath-cli/src/serve.rs`) catalogs an arrival and serves it — `just e2e`'s `tile_live_within_60s_of_drop`; the north-star number in `PERFORMANCE.md` §4 |
| **Catalog** — STAC, filled in for you; every acquisition kept | `crates/swath-catalog-pgstac` (pgstac); `GET /collections`, `GET /datasets/{id}/granules` (`ENDPOINTS.md`); the derived temporal extent per ADR 0015; dataset registration `POST /datasets` |
| **the latest granule, per tile** | `crates/swath-api/src/provider.rs` (`resolve_template`, latest-at-or-before; one granule per branch for a join) |
| **Live tiles** — OGC API - Tiles, XYZ, `datetime=`; a planner picks cache / overview / live per tile, in budget | `crates/swath-api/tests/conformance.rs` (OGC schema-valid), `tiles_datetime.rs`; `crates/swath-planner` (`docs/media/planner-decision-loop.md`); the write-through cache (`crates/swath-core/src/cache.rs`); operator budgets in `CONFIG.md` |
| **any map client** | `RECIPES.md` (QGIS over a stock XYZ connection, `media/qgis-xyz-connection.png`); the embedded viewer (`web/`) |
| **One pane of glass** — layers, time slider, compare swipe, share link, QGIS via XYZ, phone-ready | `web/src/swath-shell.ts`, `swath-layer-list.ts`, `time-slider.ts`, `swath-compare.ts`, `view-state.ts` (the URL is the share link); the phone tier in `web/e2e/mobile.e2e.ts` and shots `m01–m04` |
| **pick bands, compose** → **Derive a product** — an openEO graph over the live layers: an index, a formula, a date-vs-date change, your own code sandboxed | `web/src/swath-authoring-panel.ts` + `authoring-dag.ts` (the editor); `crates/swath-render/src/process.rs` (the compiler's conformance statement: `ndvi`, `reduce_dimension` formulas, `merge_cubes` change detection, `run_udf` WebAssembly with fuel metering); `web/e2e/authoring.e2e.ts` |
| **publish** → **Published layer** — one request, a live tile URL, previewed first, beside the built-in layers | `POST /services` and `POST /result` (`ENDPOINTS.md`); `crates/swath-api/tests/openeo_services.rs` (the published NDVI serves byte-identical to the built-in layer), `openeo_result.rs` (preview ≡ publish) |
| **served the same way** | the same `CatalogLayers` provider and tile route serve built-in and published layers (`crates/swath-api/src/provider.rs`, `routes.rs`); rehydration on restart in `crates/swath-cli/src/serve.rs` |
| **every tile emits a trace** → **Per-tile trace** | `crates/swath-core/src/trace.rs` (`Trace`: decision, sources, provenance byte ranges, timings, temporal decision); the `x-swath-trace` header and `GET /traces` SSE (`ENDPOINTS.md`) |
| **X-ray in the viewer** — badges, why-view, bytes heatmap, trace feed, live analytics | `web/src/swath-xray.ts`, `xray-analytics.ts`; shots 04–07, 12; `DEMO.md` |
| **The same data in the tests** | `web/e2e/swath-xray.e2e.ts` asserts on the streamed traces; `crates/swath-api/tests/trace_stream.rs`, `tiles_datetime.rs` read the trace off the response — the viewer and CI consume one `Trace` |

REUSE: the SVG carries an inline SPDX header (same two-line form as source files) and is also
covered by the `docs/**` aggregate annotation in `REUSE.toml` (same holder and license).
