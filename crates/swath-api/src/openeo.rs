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

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use swath_core::catalog::stac::{STAC_VERSION, dataset_to_stac_collection};
use swath_core::catalog::{
    Catalog, Colormap as DomainColormap, Dataset, DatasetId, Layer as DomainLayer, PlanKind,
    Rescale as DomainRescale,
};
use swath_core::planner::Budget;
use swath_render::ir::{Colormap as IrColormap, PixelOp, RenderPlan};
use swath_render::{CompileContext, CompileError, NodataPolicy, Resampling, compile};

use crate::provider::{CatalogLayer, CatalogLayers};

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

/// Every openEO endpoint this surface serves, exactly as the capabilities
/// `endpoints` array declares it (only what exists; the spec says the
/// `GET /` entry itself is not listed). `/conformance` is the shared OGC
/// document.
pub const OPENEO_ENDPOINTS: &[(&str, &[&str])] = &[
    ("/collections", &["GET"]),
    ("/collections/{collection_id}", &["GET"]),
    ("/conformance", &["GET"]),
    ("/processes", &["GET"]),
    ("/service_types", &["GET"]),
    ("/services", &["GET", "POST"]),
    ("/services/{service_id}", &["GET", "DELETE"]),
];

/// An openEO-format error: HTTP status plus the standardized
/// `{"code","message"}` body. Codes come from the spec's `errors.json`
/// registry (pinned under `tests/data/openeo/`); the tests assert every
/// code this module emits exists there with a matching status.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct OpenEoError {
    /// HTTP status code (per the registry entry).
    pub status: StatusCode,
    /// Standardized openEO error code.
    pub code: &'static str,
    /// Human-readable diagnostic.
    pub message: String,
}

impl OpenEoError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    /// 500 `Internal` — a backend failure the client cannot fix.
    fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal", message)
    }
}

impl IntoResponse for OpenEoError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "code": self.code, "message": self.message })),
        )
            .into_response()
    }
}

/// Maps compiler diagnostics onto standardized openEO error codes — the
/// #32 diagnostics, spoken in the standard's vocabulary. The message is
/// the compiler's own (it names the offending node); the shapes are
/// pinned by snapshot tests.
impl From<CompileError> for OpenEoError {
    fn from(err: CompileError) -> Self {
        let (status, code) = match &err {
            CompileError::UnsupportedProcess { .. } => {
                (StatusCode::BAD_REQUEST, "ProcessUnsupported")
            }
            CompileError::UnknownCollection { .. } => (StatusCode::NOT_FOUND, "CollectionNotFound"),
            CompileError::MissingArgument { .. } => {
                (StatusCode::BAD_REQUEST, "ProcessParameterRequired")
            }
            CompileError::InvalidArgument { .. } | CompileError::UnknownBand { .. } => {
                (StatusCode::BAD_REQUEST, "ProcessParameterInvalid")
            }
            _ => (StatusCode::BAD_REQUEST, "ProcessGraphInvalid"),
        };
        Self::new(status, code, err.to_string())
    }
}

/// Everything the openEO handlers need: the same [`CatalogLayers`] the
/// tile handlers resolve through (clones share the layer set — a
/// `POST`ed service serves on the next tile request) and the base URL links and
/// service URLs are minted under.
#[derive(Debug)]
pub struct OpenEoState<C> {
    provider: CatalogLayers<C>,
    base_url: String,
}

impl<C> OpenEoState<C> {
    /// Wires the surface over the shared provider (trailing slashes of
    /// `base_url` trimmed, as in [`ApiState::new`](crate::ApiState::new)).
    pub fn new(provider: CatalogLayers<C>, base_url: impl Into<String>) -> Self {
        let mut base_url: String = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        Self { provider, base_url }
    }
}

/// The openEO router over `state`, to be merged with the OGC tiles router
/// (the two surfaces share `/` and `/conformance`, which live there).
pub fn openeo_router<C>(state: Arc<OpenEoState<C>>) -> axum::Router
where
    C: Catalog + 'static,
{
    axum::Router::new()
        .route("/.well-known/openeo", get(well_known))
        .route("/collections", get(collections))
        .route("/collections/{collection_id}", get(collection))
        .route("/processes", get(processes))
        .route("/service_types", get(service_types))
        .route("/services", get(list_services).post(create_service))
        .route(
            "/services/{service_id}",
            get(describe_service).delete(delete_service),
        )
        .with_state(state)
}

/// Merges the openEO capabilities vocabulary into the OGC landing page
/// document — `GET /` serves both standards' required fields from one
/// root. Called by the landing handler when the openEO surface is
/// enabled ([`ApiState::with_openeo`](crate::ApiState::with_openeo)).
pub(crate) fn extend_capabilities(landing: &mut Value, base: &str) {
    let endpoints: Vec<Value> = OPENEO_ENDPOINTS
        .iter()
        .map(|(path, methods)| json!({ "path": path, "methods": methods }))
        .collect();
    let doc = landing.as_object_mut().expect("landing page is an object");
    doc.insert("api_version".into(), json!(OPENEO_API_VERSION));
    doc.insert("backend_version".into(), json!(env!("CARGO_PKG_VERSION")));
    doc.insert("stac_version".into(), json!(STAC_VERSION));
    doc.insert("type".into(), json!("Catalog"));
    doc.insert("id".into(), json!("swath"));
    doc.insert("production".into(), json!(false));
    doc.insert("endpoints".into(), json!(endpoints));
    if let Some(links) = doc.get_mut("links").and_then(Value::as_array_mut) {
        links.push(json!({
            "rel": "data",
            "href": format!("{base}/collections"),
            "type": "application/json",
            "title": "Collections (openEO / STAC)",
        }));
    }
}

/// `GET /.well-known/openeo` — version discovery: this one instance.
async fn well_known<C>(State(app): State<Arc<OpenEoState<C>>>) -> Json<Value> {
    Json(json!({
        "versions": [{
            "url": format!("{base}/", base = app.base_url),
            "api_version": OPENEO_API_VERSION,
            "production": false,
        }],
    }))
}

// --- Collections (the R2-compatible read surface) ---

/// A [`Dataset`] as an openEO collection document: the #30 STAC converter
/// output with the swath-internal fields (`swath:bands`, `swath:layers`)
/// removed, datacube dimensions derived from the extent and band
/// vocabulary, and the required links minted. openEO collections are
/// STAC-based — STAC stays hidden from Swath's own control plane (R2),
/// but openEO clients speak STAC, and that is the standard.
fn collection_doc(dataset: &Dataset, base: &str) -> Value {
    let mut doc = dataset_to_stac_collection(dataset);
    let extent = dataset.extent.bbox;
    let bands: Vec<&String> = dataset.bands.iter().collect();
    let obj = doc.as_object_mut().expect("collection doc is an object");
    obj.remove("swath:bands");
    obj.remove("swath:layers");
    obj.insert(
        "stac_extensions".into(),
        json!(["https://stac-extensions.github.io/datacube/v2.2.0/schema.json"]),
    );
    obj.insert(
        "cube:dimensions".into(),
        json!({
            "x": {
                "type": "spatial",
                "axis": "x",
                "extent": [extent.west, extent.east],
                "reference_system": 4326,
            },
            "y": {
                "type": "spatial",
                "axis": "y",
                "extent": [extent.south, extent.north],
                "reference_system": 4326,
            },
            "t": {
                "type": "temporal",
                "extent": [
                    dataset.extent.interval.start.as_ref().map(ToString::to_string),
                    dataset.extent.interval.end.as_ref().map(ToString::to_string),
                ],
            },
            "bands": { "type": "bands", "values": bands },
        }),
    );
    obj.insert("summaries".into(), json!({}));
    obj.insert(
        "links".into(),
        json!([
            {
                "rel": "self",
                "href": format!("{base}/collections/{id}", id = dataset.id),
                "type": "application/json",
            },
            { "rel": "root", "href": format!("{base}/"), "type": "application/json" },
            { "rel": "parent", "href": format!("{base}/"), "type": "application/json" },
        ]),
    );
    doc
}

async fn collections<C: Catalog>(
    State(app): State<Arc<OpenEoState<C>>>,
) -> Result<Json<Value>, OpenEoError> {
    let datasets = app
        .provider
        .catalog()
        .list_datasets()
        .await
        .map_err(|err| OpenEoError::internal(format!("catalog listing failed: {err}")))?;
    let collections: Vec<Value> = datasets
        .iter()
        .map(|dataset| collection_doc(dataset, &app.base_url))
        .collect();
    Ok(Json(json!({
        "collections": collections,
        "links": [{
            "rel": "self",
            "href": format!("{base}/collections", base = app.base_url),
            "type": "application/json",
        }],
    })))
}

async fn collection<C: Catalog>(
    State(app): State<Arc<OpenEoState<C>>>,
    Path(collection_id): Path<String>,
) -> Result<Json<Value>, OpenEoError> {
    let dataset = fetch_dataset(&app, &collection_id).await?;
    Ok(Json(collection_doc(&dataset, &app.base_url)))
}

/// The collection, or the standardized `CollectionNotFound`.
async fn fetch_dataset<C: Catalog>(app: &OpenEoState<C>, id: &str) -> Result<Dataset, OpenEoError> {
    app.provider
        .catalog()
        .get_dataset(&DatasetId::new(id))
        .await
        .map_err(|err| OpenEoError::internal(format!("catalog lookup failed: {err}")))?
        .ok_or_else(|| {
            OpenEoError::new(
                StatusCode::NOT_FOUND,
                "CollectionNotFound",
                format!("Collection '{id}' does not exist."),
            )
        })
}

// --- Processes ---

/// The pinned official openeo-processes 1.2.0 definitions for the
/// compiler's supported subset (byte-identical to the compiler's oracle
/// copies — a test asserts it), plus the Swath-profile narrowing note
/// appended to each description where v0 narrows the spec.
const PROCESS_DEFINITIONS: &[(&str, &str)] = &[
    (
        include_str!("../data/openeo-processes/add.json"),
        "supported inside a `reduce_dimension` reducer, over band elements, numbers, and other results.",
    ),
    (
        include_str!("../data/openeo-processes/array_element.json"),
        "supported inside a `reduce_dimension` reducer, over its band array \
         (`from_parameter: \"data\"`); exactly one of `index`/`label`.",
    ),
    (
        include_str!("../data/openeo-processes/divide.json"),
        "supported inside a `reduce_dimension` reducer; division by zero makes the pixel no-data.",
    ),
    (
        include_str!("../data/openeo-processes/linear_scale_range.json"),
        "`outputMin`/`outputMax` must be exactly 0/255 (the render path quantizes to 8-bit \
         RGBA); at most one scale per graph, applied after reduction/composition.",
    ),
    (
        include_str!("../data/openeo-processes/load_collection.json"),
        "`id` must name the collection the graph is authored against; `bands` is required and \
         entries must be dataset band names; `spatial_extent`, `temporal_extent`, and \
         `properties` are accepted and ignored (tile serving decides the window and the granule).",
    ),
    (
        include_str!("../data/openeo-processes/multiply.json"),
        "supported inside a `reduce_dimension` reducer, over band elements, numbers, and other results.",
    ),
    (
        include_str!("../data/openeo-processes/ndvi.json"),
        "`target_band` must be omitted or null (the bands dimension is dropped; the result is gray).",
    ),
    (
        include_str!("../data/openeo-processes/reduce_dimension.json"),
        "only `dimension: \"bands\"` is supported.",
    ),
    (
        include_str!("../data/openeo-processes/save_result.json"),
        "`format` must be \"png\" (case-insensitive) and `options` empty; must be the graph's \
         result node.",
    ),
    (
        include_str!("../data/openeo-processes/subtract.json"),
        "supported inside a `reduce_dimension` reducer, over band elements, numbers, and other results.",
    ),
];

/// The served process list, built once: pinned definitions with the
/// narrowing note appended.
fn process_list() -> &'static Vec<Value> {
    static LIST: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        PROCESS_DEFINITIONS
            .iter()
            .map(|(raw, note)| {
                let mut doc: Value =
                    serde_json::from_str(raw).expect("pinned process definition parses");
                let description = doc["description"]
                    .as_str()
                    .expect("pinned definition has a description");
                doc["description"] = json!(format!("{description}\n\n**Swath profile:** {note}"));
                doc
            })
            .collect()
    })
}

async fn processes<C>(State(app): State<Arc<OpenEoState<C>>>) -> Json<Value> {
    Json(json!({
        "processes": process_list(),
        "links": [{
            "rel": "self",
            "href": format!("{base}/processes", base = app.base_url),
            "type": "application/json",
        }],
    }))
}

// --- Secondary services (the authoring loop) ---

async fn service_types<C>(State(_): State<Arc<OpenEoState<C>>>) -> Json<Value> {
    Json(json!({
        SERVICE_TYPE: {
            "title": "XYZ tiled web map (slippy map)",
            "description": "The published process graph served as live map tiles from the \
                            OGC API - Tiles endpoint. The service URL is a tile template \
                            ({z}/{y}/{x} — OGC order: tileMatrix/tileRow/tileCol).",
            "configuration": {
                "tile_size": {
                    "description": "Tile side length in pixels.",
                    "type": "integer",
                    "default": 256,
                    "enum": TILE_SIZES,
                },
            },
            "process_parameters": [],
        },
    }))
}

/// The parsed, validated `POST /services` request body.
struct ServiceRequest {
    title: Option<String>,
    description: Option<String>,
    /// The full `process` object (`process_graph_with_metadata`), stored
    /// verbatim on the layer.
    process: Value,
    tile_size: u32,
}

impl ServiceRequest {
    /// Validates the store-service request: type `xyz` (case-insensitive,
    /// per the spec), a process graph present, only supported
    /// configuration settings, no disabling.
    fn parse(body: &Value) -> Result<Self, OpenEoError> {
        let bad =
            |code, message: String| Err(OpenEoError::new(StatusCode::BAD_REQUEST, code, message));

        let Some(service_type) = body.get("type").and_then(Value::as_str) else {
            return bad(
                "ServiceUnsupported",
                "Service type is required (this back-end supports: \"xyz\").".into(),
            );
        };
        if !service_type.eq_ignore_ascii_case(SERVICE_TYPE) {
            return bad(
                "ServiceUnsupported",
                format!("Service type '{service_type}' is not supported. Supported: \"xyz\"."),
            );
        }
        if body.get("enabled").and_then(Value::as_bool) == Some(false) {
            return Err(OpenEoError::new(
                StatusCode::NOT_IMPLEMENTED,
                "FeatureUnsupported",
                "Creating a disabled service is not supported: services are always enabled.",
            ));
        }

        let process = body.get("process").cloned().unwrap_or(Value::Null);
        if process.get("process_graph").is_none_or(|g| !g.is_object()) {
            return bad(
                "ProcessGraphMissing",
                "Invalid process specified. It doesn't contain a process graph.".into(),
            );
        }

        let mut tile_size = 256;
        if let Some(configuration) = body.get("configuration") {
            let Some(settings) = configuration.as_object() else {
                return bad(
                    "ServiceConfigInvalid",
                    format!(
                        "The value passed for the service configuration is invalid: expected an object, got {configuration}."
                    ),
                );
            };
            for (name, value) in settings {
                if name != "tile_size" {
                    return bad(
                        "ServiceConfigUnsupported",
                        format!("Service parameter '{name}' is not supported."),
                    );
                }
                let size = value.as_u64().and_then(|s| u32::try_from(s).ok());
                match size.filter(|s| TILE_SIZES.contains(s)) {
                    Some(size) => tile_size = size,
                    None => {
                        return bad(
                            "ServiceConfigInvalid",
                            format!(
                                "The value passed for the service parameter 'tile_size' is \
                                 invalid: expected one of {TILE_SIZES:?}, got {value}."
                            ),
                        );
                    }
                }
            }
        }

        let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_owned);
        Ok(Self {
            title: text("title"),
            description: text("description"),
            process,
            tile_size,
        })
    }
}

/// The collection id the graph loads: the `id` argument of its (first)
/// top-level `load_collection` node. A pre-pass only — the compiler
/// re-validates it against the compile context.
fn loaded_collection(graph: &Value) -> Option<&str> {
    let nodes = graph.get("process_graph").unwrap_or(graph).as_object()?;
    nodes.values().find_map(|node| {
        (node.get("process_id")?.as_str()? == "load_collection")
            .then(|| node.get("arguments")?.get("id")?.as_str())
            .flatten()
    })
}

/// The compile context of a dataset: every dataset band bound by its own
/// name (graphs name dataset bands directly; no alias vocabulary is
/// persisted in v0).
fn compile_context(dataset: &Dataset) -> CompileContext {
    dataset
        .bands
        .iter()
        .fold(CompileContext::new(dataset.id.as_str()), |ctx, band| {
            ctx.with_band(band, std::iter::empty::<String>())
        })
}

/// Compiles a persisted service layer (one carrying its authoring
/// `process`) back into the servable [`CatalogLayer`] template — the
/// single lowering both `POST /services` and serve-time rehydration use,
/// so a restarted server serves exactly what was published.
///
/// # Errors
///
/// Any [`CompileError`] from re-compiling the recorded graph against the
/// dataset's current band vocabulary (e.g. a band was removed from the
/// dataset config since the service was created).
pub fn compile_service_layer(
    dataset: &Dataset,
    layer: &DomainLayer,
) -> Result<CatalogLayer, CompileError> {
    let graph = layer
        .process
        .as_ref()
        .ok_or_else(|| CompileError::Malformed {
            detail: format!(
                "layer `{id}` carries no openEO process record",
                id = layer.id
            ),
        })?;
    let product = compile(graph, &compile_context(dataset))?;
    Ok(CatalogLayer {
        id: layer.id.clone(),
        title: layer.title.clone(),
        description: layer.description.clone(),
        dataset: dataset.id.clone(),
        plan: product.plan,
        resampling: match layer.resampling {
            swath_core::catalog::Resampling::Nearest => Resampling::Nearest,
            // Bilinear, and any future kernel until this lowering learns
            // it: the continuous-band default of the golden suites.
            _ => Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
        },
        tile_size: layer.tile_size,
        budget: Budget::default(),
    })
}

/// The persisted colormap vocabulary of a compiled plan's `Colormap` op
/// (`None` when the plan has none, i.e. a composite). The compiled graph
/// stays the source of truth — this is presentation metadata mirroring
/// exactly what the plan will render, variant for variant.
fn domain_colormap(plan: &RenderPlan) -> Option<DomainColormap> {
    plan.ops.iter().find_map(|op| match op {
        PixelOp::Colormap(map) => Some(match map {
            IrColormap::Viridis => DomainColormap::Viridis,
            IrColormap::Magma => DomainColormap::Magma,
            IrColormap::RdYlGn => DomainColormap::RdYlGn,
            // Grayscale, and any future palette until this lowering
            // learns it (the graph record still renders it faithfully).
            _ => DomainColormap::Grayscale,
        }),
        _ => None,
    })
}

/// The content-derived service id: `xyz-` plus the first 12 hex digits of
/// the SHA-256 of the canonical (sorted-key) process-graph JSON. Identical
/// definition, identical id — creation is idempotent by construction.
fn service_id(process_graph: &Value) -> String {
    use std::fmt::Write as _;
    let canonical = serde_json::to_string(process_graph).expect("process graph JSON re-serializes");
    let digest = Sha256::digest(canonical.as_bytes());
    let hex = digest
        .iter()
        .take(6)
        .fold(String::with_capacity(12), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        });
    format!("{SERVICE_ID_PREFIX}{hex}")
}

/// A service's openEO metadata document. The `url` is the live tile
/// template — the OGC tiles endpoint (`{z}/{y}/{x}` is the OGC
/// tileMatrix/tileRow/tileCol order). `full` adds the fields
/// `GET /services/{id}` requires beyond the list representation:
/// the verbatim authoring process, configuration, attributes.
fn service_doc(base: &str, layer: &DomainLayer, full: bool) -> Value {
    let mut doc = json!({
        "id": layer.id,
        "title": layer.title,
        "description": layer.description,
        "type": SERVICE_TYPE,
        "enabled": true,
        "url": format!("{base}/tilesets/{id}/tiles/{{z}}/{{y}}/{{x}}", id = layer.id),
    });
    if full {
        doc["process"] = layer.process.clone().unwrap_or(Value::Null);
        doc["configuration"] = json!({ "tile_size": layer.tile_size });
        doc["attributes"] = json!({});
    }
    doc
}

/// Every persisted service layer (those carrying a `process` record),
/// with its dataset, across all datasets — the catalog is the source of
/// truth for what services exist.
async fn service_layers<C: Catalog>(
    app: &OpenEoState<C>,
) -> Result<Vec<(Dataset, DomainLayer)>, OpenEoError> {
    let datasets = app
        .provider
        .catalog()
        .list_datasets()
        .await
        .map_err(|err| OpenEoError::internal(format!("catalog listing failed: {err}")))?;
    Ok(datasets
        .into_iter()
        .flat_map(|dataset| {
            let services: Vec<DomainLayer> = dataset
                .layers
                .iter()
                .filter(|layer| layer.process.is_some())
                .cloned()
                .collect();
            services
                .into_iter()
                .map(move |layer| (dataset.clone(), layer))
        })
        .collect())
}

async fn list_services<C: Catalog>(
    State(app): State<Arc<OpenEoState<C>>>,
) -> Result<Json<Value>, OpenEoError> {
    let services: Vec<Value> = service_layers(&app)
        .await?
        .iter()
        .map(|(_, layer)| service_doc(&app.base_url, layer, false))
        .collect();
    Ok(Json(json!({
        "services": services,
        "links": [{
            "rel": "self",
            "href": format!("{base}/services", base = app.base_url),
            "type": "application/json",
        }],
    })))
}

async fn describe_service<C: Catalog>(
    State(app): State<Arc<OpenEoState<C>>>,
    Path(service_id): Path<String>,
) -> Result<Json<Value>, OpenEoError> {
    let services = service_layers(&app).await?;
    let (_, layer) = services
        .iter()
        .find(|(_, layer)| layer.id == service_id)
        .ok_or_else(|| service_not_found(&service_id))?;
    Ok(Json(service_doc(&app.base_url, layer, true)))
}

fn service_not_found(id: &str) -> OpenEoError {
    OpenEoError::new(
        StatusCode::NOT_FOUND,
        "ServiceNotFound",
        format!("Service '{id}' does not exist."),
    )
}

/// `POST /services` — the authoring loop in one motion (R3, ADR 0010):
/// validate the graph through the compiler against the referenced
/// collection's bands, persist the derived layer on the dataset
/// (`swath:layers`), make it servable immediately, answer 201 with the
/// service's location and identifier.
async fn create_service<C: Catalog>(
    State(app): State<Arc<OpenEoState<C>>>,
    Json(body): Json<Value>,
) -> Result<Response, OpenEoError> {
    let request = ServiceRequest::parse(&body)?;

    // Which collection is the graph authored against? (The compiler
    // re-validates every load_collection node against this context.)
    let collection = loaded_collection(&request.process).ok_or_else(|| {
        OpenEoError::new(
            StatusCode::BAD_REQUEST,
            "ProcessGraphInvalid",
            "Invalid process graph specified: no load_collection node names a collection.",
        )
    })?;
    let mut dataset = fetch_dataset(&app, collection).await?;

    // Validate: the whole #32 compiler, against the collection's bands.
    let product = compile(&request.process, &compile_context(&dataset))?;

    // Lower the compiled plan to the persisted layer vocabulary. The ops
    // are exactly what the compiler emits: band math or a composite,
    // optionally rescaled, gray results colormapped.
    let id = service_id(&request.process);
    let expression = product.plan.ops.iter().find_map(|op| match op {
        PixelOp::BandMath(expr) => Some(expr.to_string()),
        _ => None,
    });
    let plan = match expression {
        Some(expression) => PlanKind::BandMath { expression },
        None => match product.plan.ops.first() {
            Some(PixelOp::Composite { r, g, b }) => PlanKind::Composite {
                r: r.clone(),
                g: g.clone(),
                b: b.clone(),
            },
            _ => {
                return Err(OpenEoError::internal(
                    "compiled plan has no band math and no composite (compiler invariant broken)",
                ));
            }
        },
    };
    let rescale = product
        .plan
        .ops
        .iter()
        .find_map(|op| match op {
            PixelOp::Rescale { min, max } => Some(DomainRescale {
                min: *min,
                max: *max,
            }),
            _ => None,
        })
        // No linear_scale_range in the graph: the identity mapping of the
        // 8-bit output range (exactly what the plan's absent Rescale op
        // renders).
        .unwrap_or(DomainRescale {
            min: 0.0,
            max: 255.0,
        });
    let colormap = domain_colormap(&product.plan);
    let layer = DomainLayer {
        id: id.clone(),
        title: request.title.unwrap_or_else(|| id.clone()),
        description: request.description.unwrap_or_default(),
        plan,
        rescale,
        colormap,
        resampling: swath_core::catalog::Resampling::Bilinear,
        tile_size: request.tile_size,
        process: Some(request.process.clone()),
    };

    // Persist on the dataset (replace-or-append: identical graph, same id
    // — idempotent), then make it servable. One lowering for both the
    // live insert and every future rehydration.
    let template = compile_service_layer(&dataset, &layer)
        .map_err(|err| OpenEoError::internal(format!("service template compile failed: {err}")))?;
    dataset.layers.retain(|existing| existing.id != id);
    dataset.layers.push(layer);
    app.provider
        .catalog()
        .upsert_dataset(&dataset)
        .await
        .map_err(|err| OpenEoError::internal(format!("persisting the service failed: {err}")))?;
    app.provider.insert(template);

    Ok((
        StatusCode::CREATED,
        [
            (
                axum::http::header::LOCATION,
                format!("{base}/services/{id}", base = app.base_url),
            ),
            (axum::http::HeaderName::from_static("openeo-identifier"), id),
        ],
    )
        .into_response())
}

/// `DELETE /services/{id}` — removes the persisted layer and stops
/// serving it. Only service-authored layers are deletable here; config
/// layers are not services.
async fn delete_service<C: Catalog>(
    State(app): State<Arc<OpenEoState<C>>>,
    Path(service_id): Path<String>,
) -> Result<StatusCode, OpenEoError> {
    let services = service_layers(&app).await?;
    let (dataset, _) = services
        .iter()
        .find(|(_, layer)| layer.id == service_id)
        .ok_or_else(|| service_not_found(&service_id))?;

    let mut dataset = dataset.clone();
    dataset.layers.retain(|layer| layer.id != service_id);
    app.provider
        .catalog()
        .upsert_dataset(&dataset)
        .await
        .map_err(|err| OpenEoError::internal(format!("removing the service failed: {err}")))?;
    app.provider.remove(&service_id);
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use swath_core::catalog::{
        Bbox, Colormap as DomainColormap, Dataset, DatasetId, Extent, PlanKind,
        Rescale as DomainRescale, TimeRange,
    };
    use swath_render::ir::{Colormap as IrColormap, PixelOp};

    use super::{
        DomainLayer, compile_context, compile_service_layer, domain_colormap, loaded_collection,
        service_id,
    };

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
            // Plan -> persisted vocabulary, variant for variant.
            assert_eq!(domain_colormap(&product.plan), Some(domain));
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
                colormap: domain_colormap(&product.plan),
                resampling: swath_core::catalog::Resampling::Bilinear,
                tile_size: 256,
                process: Some(graph),
            };
            let template =
                compile_service_layer(&dataset, &layer).expect("persisted layer recompiles");
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
