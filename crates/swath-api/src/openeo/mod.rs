// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The openEO authoring surface (ADR 0010): capabilities, collections,
//! processes, and XYZ secondary services over the process compiler.
//!
//! # The bounded profile
//!
//! This module implements the openEO API **1.2.0** at exactly the profile
//! ADR 0010 records — real openEO clients can discover Swath, read its
//! collections and processes, and publish a process graph as a live tiled
//! layer:
//!
//! | Path | Resource |
//! |------|----------|
//! | `GET /.well-known/openeo` | version discovery |
//! | `GET /` | capabilities (merged into the OGC landing page — one root, both vocabularies) |
//! | `GET /collections` | openEO collections, derived from catalog [`Dataset`]s |
//! | `GET /collections/{collection_id}` | one collection |
//! | `GET /processes` | the compiler's supported subset, as pinned official definitions |
//! | `POST /result` | preview-bounded synchronous subset (ADR 0014): one small PNG preview |
//! | `GET /service_types` | the single service type: `xyz` |
//! | `GET /services` · `POST /services` | list / create secondary services |
//! | `GET /services/{service_id}` · `DELETE …` | describe / delete one service |
//!
//! `POST /services` is the R3 wedge in one motion: the submitted process
//! graph is validated through the #32 compiler against the referenced
//! collection's bands, persisted as a [`Layer`](DomainLayer) on the
//! Dataset (`swath:layers`, carrying the graph verbatim in its `process`
//! field), inserted into the live [`CatalogLayers`] provider — and the 201
//! answers with the service's tile URL, which is the OGC tiles endpoint.
//! openEO graph in, live XYZ out.
//!
//! # Honesty notes (declared, not implied)
//!
//! - **No auth**: the openEO spec requires the authentication endpoints
//!   for conformance; they are absent (Phase-3 work per the charter), so
//!   the general openEO conformance class is **not** claimed anywhere —
//!   `/conformance` keeps listing only the OGC Tiles classes actually met,
//!   and the capabilities `endpoints` array lists only what exists.
//! - **`PATCH /services` is omitted** (delete + re-create covers v0), as
//!   are jobs, batch processing, user-defined processes, and files.
//! - **`POST /result` is preview-bounded, not general synchronous
//!   processing** (ADR 0014): the graph compiles through the same #32
//!   path as `POST /services` (identical diagnostics — same codes for
//!   the same graph on either route) and answers **one** small
//!   overview-backed `image/png` render covering the graph's
//!   `spatial_extent` (when null/absent: the footprint of the granule the
//!   preview renders — the collection's real coverage, not a
//!   config-declared placeholder box) — not the spec's full extent at
//!   native resolution. The render is admitted
//!   through the planner's `max_estimated_live_bytes` cost model; when
//!   the estimate exceeds the preview budget and nothing cheaper can
//!   serve, the server refuses with the spec's `ProcessGraphComplexity`
//!   — never a silent downgrade, never an unbounded read. Nothing is
//!   persisted: no service, no `swath:layers` write, no trace-bus event.
//!   The capabilities description states the narrowing; no sync-
//!   processing conformance class is claimed. A `run_udf` graph previews
//!   under the per-tile fuel budget publishing enforces, and a module's
//!   runtime failure is a user-fixable 400 in the registry vocabulary
//!   ([`preview_udf_error`]) — the ADR 0018 validation loop (#206).
//! - Process definitions are served verbatim from the pinned
//!   openeo-processes 1.2.0 documents, with Swath's parameter narrowing
//!   appended to the `description` (see `data/openeo-processes/README.md`).
//! - Errors on this surface use the **openEO error format**
//!   (`{"code","message"}`, codes from the spec's `errors.json` registry),
//!   not the OGC RFC 7807 shape the tiles routes use — each standard gets
//!   its own error vocabulary. [`CompileError`] variants map onto
//!   standardized codes ([`OpenEoError::from`]).
//! - Service ids are content-derived (`xyz-` + a hash of the process
//!   graph): re-POSTing an identical graph updates the same service
//!   rather than minting a duplicate — creation is idempotent.
//! - **`run_udf`** (ADR 0018, #204) is offered only where the deployment
//!   wires a [`UdfPublish`] (executor + module store + fetcher): the
//!   process list includes it exactly then, and the compile motion
//!   registers the module, fetches a remote `udf` URL **once**, and
//!   persists the bytes by content hash in the module store before the
//!   service is published. Rehydration resolves the hash from the store
//!   and never fetches (see [`crate::udf`]). Rejected modules are
//!   `ProcessParameterInvalid` naming the node and the `udf` argument;
//!   without the wiring, `run_udf` is `ProcessUnsupported` — the same
//!   answer as any process outside the served list.

// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use axum::routing::get;
use serde_json::{Value, json};
use swath_core::catalog::Catalog;
use swath_core::catalog::stac::STAC_VERSION;
use swath_core::planner::Budget;
use swath_core::reproject::Reproject;
use swath_core::source::RasterSource;

use crate::provider::CatalogLayers;
use crate::udf::UdfPublish;

mod errors;
mod handlers;
mod types;

use handlers::{
    collection, collections, create_service, delete_service, describe_service, file_formats,
    list_services, preview_result, processes, service_types, well_known,
};
use types::Lowering;

pub use crate::error::OpenEo as OpenEoError;
pub use types::compile_service_layer;

/// The openEO API version this surface implements against (the pinned
/// spec under `tests/data/openeo/`, ADR 0010).
pub const OPENEO_API_VERSION: &str = "1.2.0";

/// The single secondary-service type: slippy-map tiles served from the
/// OGC API - Tiles endpoint.
const SERVICE_TYPE: &str = "xyz";

/// Prefix of content-derived service ids.
const SERVICE_ID_PREFIX: &str = "xyz-";

/// Tile sizes a service `configuration` may request.
const TILE_SIZES: [u32; 2] = [256, 512];

/// Side length of the preview render (ADR 0014): one classic tile.
const PREVIEW_TILE_SIZE: u32 = 256;

/// Request-body limit on the graph-accepting routes (`POST /services`,
/// `POST /result`): an inline `run_udf` module is at most 8 MiB of WASM
/// (`swath_core::udf::MODULE_MAX_BYTES`), which base64 inflates by 4/3
/// — so a valid graph can be ~11.2 MiB, over axum's 2 MiB default. 16
/// MiB admits every module the compiler could accept and refuses the
/// rest at the transport (413) before any parsing.
const GRAPH_BODY_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// The preview budget's default `max_estimated_live_bytes` ceiling
/// (ADR 0014: "strict budget, refusal over degradation"). A documented
/// calibration point, not a fit: it admits any near-native live window a
/// single 256-px preview can honestly need (a few MB of samples — e.g.
/// two int16 bands of a ~1400 px window) while refusing the unbounded
/// full-resolution read of a large extent whose source has no overviews.
/// Overview-backed candidates are admitted by the planner regardless of
/// this ceiling — a preview is exactly the workload overviews exist for.
const PREVIEW_MAX_ESTIMATED_LIVE_BYTES: u64 = 8 * 1024 * 1024;

/// Deepest tile matrix a preview may select — the `WebMercatorQuad`
/// registered definition's deepest matrix, same bound the tiles routes
/// enforce on tile addresses.
const PREVIEW_MAX_ZOOM: u8 = 24;

/// Web Mercator's latitude domain bound; extents are clamped into it
/// before tile selection (the projection is undefined beyond it).
const WEB_MERCATOR_MAX_LAT: f64 = 85.051_128_779_806_59;

/// Every endpoint this instance serves beside the OGC tiles surface,
/// exactly as the capabilities `endpoints` array declares it (only what
/// exists; the spec says the `GET /` entry itself is not listed).
/// `/conformance` is the shared OGC document. The `/datasets` entries are
/// Swath's dataset-creation surface (#196) and granule browsing (#107) —
/// not openEO vocabulary, but the capabilities document is where a client
/// (the #197 add-data panel) learns what is mounted, and the read-only
/// filter (#198) prunes them with the other write methods.
pub const OPENEO_ENDPOINTS: &[(&str, &[&str])] = &[
    ("/collections", &["GET"]),
    ("/collections/{collection_id}", &["GET"]),
    ("/conformance", &["GET"]),
    ("/datasets", &["POST"]),
    ("/datasets/{dataset_id}/granules", &["GET", "POST"]),
    ("/file_formats", &["GET"]),
    ("/processes", &["GET"]),
    ("/result", &["POST"]),
    ("/service_types", &["GET"]),
    ("/services", &["GET", "POST"]),
    ("/services/{service_id}", &["GET", "DELETE"]),
];

/// Everything the openEO handlers need: the same [`CatalogLayers`] the
/// tile handlers resolve through (clones share the layer set — a
/// `POST`ed service serves on the next tile request), the two render
/// ports the preview endpoint consumes (ADR 0014 — `POST /result`
/// renders inline, exactly like the tile handler), and the base URL
/// links and service URLs are minted under.
#[derive(Debug)]
pub struct OpenEoState<S, R, C> {
    provider: CatalogLayers<C>,
    source: S,
    reproject: R,
    base_url: String,
    /// The preview budget's `max_estimated_live_bytes` ceiling
    /// ([`PREVIEW_MAX_ESTIMATED_LIVE_BYTES`] unless overridden).
    preview_ceiling: u64,
    /// The operator's resolved global budget (#272): every published
    /// service serves under it, and a preview runs under it narrowed by
    /// [`Self::preview_ceiling`]. `Budget::default()` unless the binary
    /// hands over its resolved `[budget]` → flags/env layering.
    budget: Budget,
    /// The `run_udf` publish wiring (ADR 0018, #204); `None` = the
    /// process is not offered here.
    udf: Option<UdfPublish>,
}

impl<S, R, C> OpenEoState<S, R, C> {
    /// Wires the surface over the shared provider and the two render
    /// ports (trailing slashes of `base_url` trimmed, as in
    /// [`ApiState::new`](crate::ApiState::new)).
    pub fn new(
        provider: CatalogLayers<C>,
        source: S,
        reproject: R,
        base_url: impl Into<String>,
    ) -> Self {
        let mut base_url: String = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        Self {
            provider,
            source,
            reproject,
            base_url,
            preview_ceiling: PREVIEW_MAX_ESTIMATED_LIVE_BYTES,
            budget: Budget::default(),
            udf: None,
        }
    }

    /// Sets the operator's global budget (#272): the resolved `[budget]`
    /// → `--max-udf-fuel-per-tile` / `--max-estimated-live-bytes` layering
    /// the binary already applies to config-declared layers. Published
    /// services (`POST /services`, and every restart's rehydration through
    /// [`compile_service_layer`] with the same value) serve under it;
    /// `POST /result` runs under it with the byte ceiling narrowed to
    /// [`Self::with_preview_ceiling`]'s bound — an operator can cap user
    /// code below the built-in default on a public instance, never widen
    /// the preview above ADR 0014's ceiling.
    #[must_use]
    pub fn with_budget(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    /// Enables `run_udf` on this surface (ADR 0018, #204): graphs compile
    /// their module through `udf`'s registrar, remote modules are fetched
    /// once through its fetcher, and published modules persist in its
    /// store. `GET /processes` lists `run_udf` exactly when this is set.
    #[must_use]
    pub fn with_udf(mut self, udf: UdfPublish) -> Self {
        self.udf = Some(udf);
        self
    }

    /// Overrides the preview budget's byte ceiling — a calibration seam
    /// (tests pin the refusal path with a tiny ceiling; the default is
    /// [`PREVIEW_MAX_ESTIMATED_LIVE_BYTES`]).
    #[must_use]
    pub fn with_preview_ceiling(mut self, max_estimated_live_bytes: u64) -> Self {
        self.preview_ceiling = max_estimated_live_bytes;
        self
    }
}

/// The openEO router over `state`, to be merged with the OGC tiles router
/// (the two surfaces share `/` and `/conformance`, which live there).
pub fn openeo_router<S, R, C>(state: Arc<OpenEoState<S, R, C>>) -> axum::Router
where
    S: RasterSource + 'static,
    R: Reproject + 'static,
    C: Catalog + 'static,
{
    let writes = axum::Router::new()
        .route("/services", axum::routing::post(create_service))
        .route(
            "/services/{service_id}",
            axum::routing::delete(delete_service),
        )
        .layer(axum::extract::DefaultBodyLimit::max(GRAPH_BODY_LIMIT_BYTES))
        .with_state(Arc::clone(&state));
    writes.merge(openeo_read_router(state))
}

/// The read half of the openEO surface alone — what `--read-only` serving
/// mounts (#198): discovery, collections, processes, service listings,
/// and `POST /result` (deliberately: the ADR 0014 preview is
/// planner-budget-bounded by design and stays enabled — the demo wow).
/// The write routes (`POST /services`, `DELETE /services/{id}`) are
/// simply absent, not 403'd.
pub fn openeo_read_router<S, R, C>(state: Arc<OpenEoState<S, R, C>>) -> axum::Router
where
    S: RasterSource + 'static,
    R: Reproject + 'static,
    C: Catalog + 'static,
{
    axum::Router::new()
        .route("/.well-known/openeo", get(well_known))
        .route("/collections", get(collections))
        .route("/collections/{collection_id}", get(collection))
        .route("/file_formats", get(file_formats))
        .route("/processes", get(processes))
        .route("/result", axum::routing::post(preview_result))
        .route("/service_types", get(service_types))
        .route("/services", get(list_services))
        .route("/services/{service_id}", get(describe_service))
        .layer(axum::extract::DefaultBodyLimit::max(GRAPH_BODY_LIMIT_BYTES))
        .with_state(state)
}

/// Merges the openEO capabilities vocabulary into the OGC landing page
/// document — `GET /` serves both standards' required fields from one
/// root. Called by the landing handler when the openEO surface is
/// enabled ([`ApiState::with_openeo`](crate::ApiState::with_openeo)).
/// `uploads` additionally declares the local-mode upload route (#197) —
/// mounted, like everything here, only where it is true.
pub(crate) fn extend_capabilities(landing: &mut Value, base: &str, read_only: bool, uploads: bool) {
    // The capabilities document states what is MOUNTED: read-only serving
    // (#198) filters the write methods out rather than advertising routes
    // that do not exist.
    let mut endpoints: Vec<Value> = OPENEO_ENDPOINTS
        .iter()
        .filter_map(|(path, methods)| {
            let methods: Vec<&str> = methods
                .iter()
                .copied()
                .filter(|m| !read_only || *m == "GET" || (*m == "POST" && *path == "/result"))
                .collect();
            (!methods.is_empty()).then(|| json!({ "path": path, "methods": methods }))
        })
        .collect();
    if uploads && !read_only {
        endpoints.push(json!({ "path": "/uploads/{filename}", "methods": ["PUT"] }));
    }
    let doc = landing.as_object_mut().expect("landing page is an object");
    doc.insert("api_version".into(), json!(OPENEO_API_VERSION));
    doc.insert("backend_version".into(), json!(env!("CARGO_PKG_VERSION")));
    doc.insert("stac_version".into(), json!(STAC_VERSION));
    doc.insert("type".into(), json!("Catalog"));
    doc.insert("id".into(), json!("swath"));
    doc.insert("production".into(), json!(false));
    doc.insert("endpoints".into(), json!(endpoints));
    // The honest narrowing of `POST /result` (ADR 0014), declared where
    // clients look for it: the capabilities document's description.
    if let Some(Value::String(description)) = doc.get_mut("description") {
        description.push_str(
            " POST /result is a preview-bounded synchronous subset (not general synchronous \
             processing): it renders one small overview-backed PNG preview of the process \
             graph's spatial extent, and refuses requests over its preview budget with \
             ProcessGraphComplexity.",
        );
    }
    if let Some(links) = doc.get_mut("links").and_then(Value::as_array_mut) {
        links.push(json!({
            "rel": "data",
            "href": format!("{base}/collections"),
            "type": "application/json",
            "title": "Collections (openEO / STAC)",
        }));
    }
}

impl<S, R, C> OpenEoState<S, R, C> {
    /// The lowering inputs of this surface.
    fn lowering(&self) -> Lowering<'_> {
        Lowering {
            udf: self.udf.as_ref(),
            budget: &self.budget,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use swath_core::catalog::{
        Bbox, Colormap as DomainColormap, Dataset, DatasetId, Extent, PlanKind,
        Rescale as DomainRescale, TimeRange,
    };
    use swath_render::ir::{Colormap as IrColormap, PixelOp};
    use swath_render::plan_for;

    use swath_core::catalog::Bbox as DomainBbox;

    use super::handlers::{preview_extent, preview_footprint_tile, preview_tile};
    use super::types::{compile_context, compile_service_layer, loaded_collection, service_id};
    use swath_core::catalog::Layer as DomainLayer;

    #[test]
    fn loaded_collection_reads_the_load_node_wrapped_or_bare() {
        let wrapped = json!({ "process_graph": {
            "load": { "process_id": "load_collection", "arguments": { "id": "hls-s30" } },
        }});
        assert_eq!(loaded_collection(&wrapped), Some("hls-s30"));
        let bare = json!({
            "load": { "process_id": "load_collection", "arguments": { "id": "hls-s30" } },
        });
        assert_eq!(loaded_collection(&bare), Some("hls-s30"));
        assert_eq!(loaded_collection(&json!({ "process_graph": {} })), None);
    }

    /// The preview tile is the deepest `WebMercatorQuad` tile fully
    /// containing the bbox (ADR 0014's "one small render"): pinned on
    /// the committed HLS fixture extent, the world, boundary-straddling
    /// extents, and a degenerate point.
    #[test]
    fn preview_tile_is_the_deepest_containing_tile() {
        let bbox = |west, south, east, north| DomainBbox {
            west,
            south,
            east,
            north,
        };
        // The HLS fixture extent: z8 splits it across columns 52/53, so
        // z7 (26, 48) is the deepest single containing tile.
        let coord = preview_tile(&bbox(-105.537, 39.1954, -105.3581, 39.3345));
        assert_eq!((coord.z, coord.x, coord.y), (7, 26, 48));
        // The whole world only fits the root.
        let coord = preview_tile(&bbox(-180.0, -90.0, 180.0, 90.0));
        assert_eq!((coord.z, coord.x, coord.y), (0, 0, 0));
        // An extent straddling the antimeridian tile boundary of every
        // zoom (the prime-meridian column split) stays at the root.
        let coord = preview_tile(&bbox(-1.0, 10.0, 1.0, 12.0));
        assert_eq!((coord.z, coord.x, coord.y), (0, 0, 0));
        // A degenerate point descends to the deepest matrix served.
        let coord = preview_tile(&bbox(-105.4, 39.3, -105.4, 39.3));
        assert_eq!(coord.z, super::PREVIEW_MAX_ZOOM);
    }

    /// With no extent named, the frame fits the granule: the deepest tile
    /// at least as large as the footprint, around its center — the
    /// fixture granule fills about half of z10 (391 px at z10 ≈ 0.35°),
    /// where the containing-tile rule left it a sliver of z7.
    #[test]
    fn preview_footprint_tile_fits_the_granule_at_its_own_scale() {
        let bbox = |west, south, east, north| DomainBbox {
            west,
            south,
            east,
            north,
        };
        let coord = preview_footprint_tile(&bbox(-105.537, 39.1954, -105.3581, 39.3345));
        assert_eq!((coord.z, coord.x, coord.y), (10, 212, 390));
        // The whole world still only fits the root…
        let coord = preview_footprint_tile(&bbox(-180.0, -90.0, 180.0, 90.0));
        assert_eq!((coord.z, coord.x, coord.y), (0, 0, 0));
        // …while a box straddling the prime meridian no longer climbs to
        // it: z7 tiles are 2.8° wide, the last at least as large as 2°.
        let coord = preview_footprint_tile(&bbox(-1.0, 10.0, 1.0, 12.0));
        assert_eq!(coord.z, 7);
        // A degenerate point descends to the deepest matrix served.
        let coord = preview_footprint_tile(&bbox(-105.4, 39.3, -105.4, 39.3));
        assert_eq!(coord.z, super::PREVIEW_MAX_ZOOM);
    }

    /// `spatial_extent` selects the preview window; null/absent names
    /// none (the handler then frames the resolved granule); malformed
    /// extents refuse with the standardized code.
    #[test]
    fn preview_extent_reads_the_spatial_extent_or_names_none() {
        let graph = |extent: Value| {
            json!({ "process_graph": {
                "load": { "process_id": "load_collection", "arguments": {
                    "id": "hls-s30", "bands": ["b8a"], "spatial_extent": extent,
                }},
            }})
        };
        // Explicit extent wins.
        let explicit = preview_extent(&graph(
            json!({ "west": -105.5, "south": 39.2, "east": -105.4, "north": 39.3 }),
        ))
        .expect("explicit extent parses")
        .expect("explicit extent is named");
        assert_eq!((explicit.west, explicit.north), (-105.5, 39.3));
        // Null and absent name no extent.
        for process in [
            graph(Value::Null),
            json!({ "process_graph": { "load": {
                "process_id": "load_collection",
                "arguments": { "id": "hls-s30", "bands": ["b8a"] },
            }}}),
        ] {
            assert_eq!(preview_extent(&process).expect("no extent parses"), None);
        }
        // Malformed: a missing side, a non-numeric side, an inverted box.
        for extent in [
            json!({ "west": -105.5, "south": 39.2, "east": -105.4 }),
            json!({ "west": "far", "south": 39.2, "east": -105.4, "north": 39.3 }),
            json!({ "west": -105.4, "south": 39.2, "east": -105.5, "north": 39.3 }),
        ] {
            let err = preview_extent(&graph(extent)).expect_err("malformed refuses");
            assert_eq!(err.0.code, "ProcessParameterInvalid");
        }
    }

    #[test]
    fn service_ids_are_content_derived_and_stable() {
        let a = json!({ "process_graph": { "n": { "process_id": "x" } } });
        let b = json!({ "process_graph": { "n": { "process_id": "x" } } });
        let c = json!({ "process_graph": { "n": { "process_id": "y" } } });
        assert_eq!(service_id(&a), service_id(&b));
        assert_ne!(service_id(&a), service_id(&c));
        assert!(service_id(&a).starts_with("xyz-"));
        assert_eq!(service_id(&a).len(), 4 + 12);
    }

    /// A minimal HLS-shaped dataset for compiling graphs against.
    fn hls_dataset() -> Dataset {
        Dataset {
            id: DatasetId::new("hls-s30"),
            title: "HLS S30".to_owned(),
            description: String::new(),
            license: "CC0-1.0".to_owned(),
            extent: Extent {
                bbox: Bbox {
                    west: -180.0,
                    south: -90.0,
                    east: 180.0,
                    north: 90.0,
                },
                interval: TimeRange::default(),
            },
            bands: ["b04".to_owned(), "b8a".to_owned()].into(),
            layers: Vec::new(),
        }
    }

    /// An NDVI graph whose `save_result` carries the given `options`.
    fn ndvi_graph(options: &Value) -> Value {
        json!({ "process_graph": {
            "load": { "process_id": "load_collection", "arguments": {
                "id": "hls-s30", "bands": ["b8a", "b04"],
            }},
            "ndvi": { "process_id": "ndvi", "arguments": {
                "data": { "from_node": "load" }, "nir": "b8a", "red": "b04",
            }},
            "scale": { "process_id": "linear_scale_range", "arguments": {
                "x": { "from_node": "ndvi" },
                "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 255,
            }},
            "save": { "process_id": "save_result", "arguments": {
                "data": { "from_node": "scale" }, "format": "png", "options": options,
            }, "result": true },
        }})
    }

    /// The colormap AC's round trip (issue #94, unit-level until the M4
    /// round-trip proptest): a graph naming a colormap compiles to a plan
    /// carrying that variant; the plan lowers to the persisted
    /// `swath:layers` colormap vocabulary; and recompiling the persisted
    /// layer (what `swath serve` does at startup) reproduces the same
    /// executable plan, colormap included.
    #[test]
    fn colormap_round_trips_through_the_openeo_graph_representation() {
        let dataset = hls_dataset();
        for (name, ir, domain) in [
            (
                "grayscale",
                IrColormap::Grayscale,
                DomainColormap::Grayscale,
            ),
            ("viridis", IrColormap::Viridis, DomainColormap::Viridis),
            ("magma", IrColormap::Magma, DomainColormap::Magma),
            ("rdylgn", IrColormap::RdYlGn, DomainColormap::RdYlGn),
        ] {
            let graph = ndvi_graph(&json!({ "colormap": name }));
            // Graph -> plan: the option becomes the plan's Colormap op.
            let product =
                swath_render::compile(&graph, &compile_context(&dataset)).expect("graph compiles");
            assert_eq!(
                product.plan.ops.last(),
                Some(&PixelOp::Colormap(ir)),
                "{name}: compiled plan must end in its colormap"
            );
            // Plan -> persisted vocabulary, variant for variant (the #95
            // constructor derives it from the compiled spec).
            let meta = plan_for(&product.spec).1;
            assert_eq!(meta.colormap, Some(domain));
            // Persisted layer -> plan again (serve-time rehydration).
            let layer = DomainLayer {
                id: format!("xyz-{name}"),
                title: name.to_owned(),
                description: String::new(),
                plan: PlanKind::BandMath {
                    expression: "(b8a - b04) / (b8a + b04)".to_owned(),
                },
                rescale: DomainRescale {
                    min: -1.0,
                    max: 1.0,
                },
                colormap: plan_for(&product.spec).1.colormap,
                resampling: swath_core::catalog::Resampling::Bilinear,
                tile_size: 256,
                process: Some(graph),
            };
            let template = compile_service_layer(&dataset, &layer, None, &super::Budget::default())
                .expect("persisted layer recompiles");
            assert_eq!(
                template.plan, product.plan,
                "{name}: rehydrated plan must equal the originally compiled plan"
            );
        }
        // No colormap option at all: gray results default to grayscale.
        let bare = ndvi_graph(&json!({}));
        let product =
            swath_render::compile(&bare, &compile_context(&dataset)).expect("graph compiles");
        assert_eq!(
            product.plan.ops.last(),
            Some(&PixelOp::Colormap(IrColormap::Grayscale))
        );
    }
}
