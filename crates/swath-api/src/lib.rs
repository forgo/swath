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
//! Every mounted route, with captured examples, is `docs/ENDPOINTS.md`,
//! whose table the docs gate diffs against the routers in this crate; the
//! OGC API - Tiles routes live in [`router`], the x-ray `GET /traces` stream
//! and the `/healthz` probe beside them, and the embedded UI in the fallback
//! ([`ui`]).
//!
//! Catalog-backed deployments additionally merge in the **openEO
//! authoring surface** (ADR 0010, [`openeo`] module: capabilities,
//! collections, processes, XYZ secondary services, and the
//! preview-bounded `POST /result` of ADR 0014) — `GET /` then serves
//! the OGC landing page and the openEO capabilities from one root
//! ([`ApiState::with_openeo`]) — and the **granule browsing surface**
//! (issue #107, [`granules`] module): read-only
//! `GET /datasets/{datasetId}/granules` over `Catalog::find_granules`,
//! paginated, in the same RFC 7807 error taxonomy as the tiles routes.
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
//! - **The embedded UI** (issue #103, ADR 0011): the binary can embed the
//!   production web bundle ([`ApiState::with_ui`]). Browsers (an `Accept`
//!   listing `text/html`) get its `index.html` at `GET /`; every other
//!   client keeps the JSON landing page. Assets serve from the router
//!   *fallback*, so API routes structurally outrank any bundle file; no
//!   SPA rewrite — unknown paths stay plain 404. Full rules in [`ui`].
//!
//! # CORS (a decision, recorded — issue #103, ADR 0011)
//!
//! **Default off, opt-in by origin allowlist.** The default deployment
//! story is same-origin: the binary serves the UI itself (above), and the
//! dev workflow proxies API routes through Vite — no cross-origin
//! requests exist, and none are advertised. Deployments that serve a
//! browser frontend from another origin opt in with an explicit list
//! (`--cors-allowed-origins` / `SWATH_CORS_ALLOWED_ORIGINS` /
//! `cors-allowed-origins`; `*` = any origin, for cross-origin dev). The
//! layer is built by [`cors_layer`] (tower-http) and applied by the
//! binary over the whole merged router; with no origins configured no
//! layer exists at all and responses are byte-identical to before. No
//! credentials support — the surface is public reads plus openEO
//! authoring, cookie-less by design. See [`cors`] module docs.
//!
//! # Deferred (noted, not built)
//!
//! Bounded concurrency/backpressure for the inline render (ADR 0012's
//! reopen trigger) and encode formats beyond PNG (`docs/ROADMAP.md`, the
//! deferral inventory) — neither is built, both are recorded there.
//!
//! # Layer resolution (issue #31)
//!
//! Handlers resolve `{layerId}` through the [`LayerProvider`] seam: the
//! in-memory [`LayerRegistry`] (fixtures/config mode, unchanged) or the
//! catalog-backed [`CatalogLayers`], whose tiles render from the **latest
//! granule within the request's `datetime` window** (ADR 0015; absent =
//! plain latest) of each layer's dataset and whose Traces carry
//! `ingest_to_pixel_ms` (also surfaced in the `X-Swath-Trace` header) — the
//! north-star metric's serve half. See [`provider`](CatalogLayers) docs.

pub mod cors;
pub mod datasets;
mod error;
pub mod granules;
mod model;
pub mod openeo;
mod provider;
mod registry;
mod routes;
mod temporal;
pub mod traces;
pub mod udf;
pub mod ui;
pub mod uploads;

pub use cors::cors_layer;
pub use datasets::{DatasetsState, datasets_router};
pub use error::ApiError;
pub use granules::{GranuleList, GranulesState, granules_router};
pub use model::{Conformance, LandingPage, Link, TileSetItem, TileSetList, TileSetMetadata};
pub use openeo::{
    OPENEO_API_VERSION, OpenEoError, OpenEoState, compile_service_layer, openeo_read_router,
    openeo_router,
};
pub use provider::{CatalogLayer, CatalogLayers, LayerIdentity, LayerProvider, ResolvedLayer};
pub use registry::{Layer, LayerRegistry};
pub use routes::{ApiState, CONFORMANCE_CLASSES, TraceExtension, router};
pub use traces::{TraceBus, TraceEvent};
pub use udf::{RehydrateError, SharedUdfExecutor, UdfModules, UdfPublish};
pub use ui::UiAssets;
pub use uploads::{UploadsState, uploads_router};
