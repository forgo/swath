// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The OGC API - Tiles surface: Swath's first standards-facing HTTP
//! interface (REQUIREMENTS.md R5, ARCHITECTURE.md §7 — Tiles/Maps is a
//! Phase-1 inbound API).
//!
//! # An inbound adapter, nothing more
//!
//! This crate is standards-shaped HTTP over the core: handlers translate
//! OGC API requests into [`swath_render::render_tile`] calls and domain
//! types into OGC JSON documents. No rendering, projection, or planning
//! logic lives here — a handler that starts computing pixels is a bug.
//! Like the tiler itself, the crate is generic over the two ports it
//! consumes ([`RasterSource`](swath_core::source::RasterSource) and
//! [`Reproject`](swath_core::reproject::Reproject)); concrete adapters are
//! wired by the binary (issue #29) and by the test suite, never here.
//!
//! # Routes
//!
//! | Path | Resource |
//! |------|----------|
//! | `GET /` | OGC API landing page |
//! | `GET /conformance` | conformance declaration |
//! | `GET /tiles` | tilesets list (the standard's dataset-tilesets path) |
//! | `GET /tilesets` | tilesets list (same representation, canonical path) |
//! | `GET /tilesets/{layerId}` | tileset metadata (`WebMercatorQuad`) |
//! | `GET /tilesets/{layerId}/tiles/{tileMatrix}/{tileRow}/{tileCol}` | a PNG tile |
//! | `GET /traces` | the x-ray Trace SSE stream (control-plane, issue #28) |
//! | `GET /healthz` | liveness probe: plain 200 `ok` (operational, non-OGC, issue #29) |
//!
//! Users address **layers** (R2): a layer id is the only name a client
//! ever sees — band assets, plans, and catalog plumbing stay behind the
//! [`LayerRegistry`].
//!
//! **Tile path order is the OGC order**: `{tileMatrix}/{tileRow}/{tileCol}`
//! = z/**y**/**x** — *not* the XYZ `z/x/y` habit. The integration suite
//! pins this explicitly (the classic Tiles-API bug).
//!
//! # Conformance (declared honestly)
//!
//! `/conformance` lists exactly the OGC API - Tiles 1.0 (OGC 20-057)
//! classes this surface implements: `Core`, `TileSet`, `TileSets List`,
//! `Dataset TileSets`, and `PNG` (see [`CONFORMANCE_CLASSES`]). OGC API -
//! Common Core is deliberately **not** declared: we serve its landing-page
//! and conformance shapes but no `OpenAPI` definition document, so claiming
//! the class would be dishonest. The conformance smoke tests validate
//! every JSON response against the committed official OGC schemas
//! (`tests/data/ogc/`).
//!
//! # Behavioral choices (documented, tested)
//!
//! - A tile inside the tile matrix but outside the layer's data footprint
//!   is **200 with a fully transparent PNG** — OGC 20-057 `/req/core/tc-error`
//!   permits 204 or a 200 "blank response"; 200 matches `render_tile`'s
//!   "a served empty tile is still explained" semantics (R4).
//! - Out-of-range `{tileMatrix}/{tileRow}/{tileCol}` → 404; malformed
//!   (non-integer) row/col → 400; both carry an RFC 7807 exception body.
//! - Every tile response carries the render [`Trace`](swath_core::trace::Trace)
//!   as a response extension ([`TraceExtension`]) plus the `X-Swath-Trace`
//!   debug header (`bytes_read` + `total_ms`), and every render is
//!   published to the [`TraceBus`] feeding the `GET /traces` SSE stream
//!   (issue #28) — handlers never discard the Trace. The stream is
//!   best-effort telemetry with an API-layer envelope
//!   (`{"tile","layer","trace"}`); the [`traces`] module docs carry the
//!   full wire contract and slow-subscriber semantics.
//!
//! # Deferred (noted, not built)
//!
//! - **Bounded concurrency / backpressure** (ARCHITECTURE.md §11):
//!   `render_tile` runs inline on the handler task; `spawn_blocking`/rayon
//!   offload and admission control wait until a real server feels the
//!   latency (§16.7).
//! - **Catalog-backed layers**: the in-memory [`LayerRegistry`] is the
//!   walking-skeleton stand-in the pgstac catalog replaces in issue #30.
//! - **WebP / content negotiation beyond PNG**: PNG is the only encode
//!   format the render path emits today (`TileFormat`).

mod error;
mod model;
mod registry;
mod routes;
pub mod traces;

pub use error::ApiError;
pub use model::{Conformance, LandingPage, Link, TileSetItem, TileSetList, TileSetMetadata};
pub use registry::{Layer, LayerRegistry};
pub use routes::{ApiState, CONFORMANCE_CLASSES, TraceExtension, router};
pub use traces::{TraceBus, TraceEvent};
