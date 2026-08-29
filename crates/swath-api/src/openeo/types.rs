// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The openEO surface's documents and compile helpers (#354): the
//! collection and service documents, the pinned process definitions, the
//! service request, and the graph → layer lowering.

// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

use axum::http::StatusCode;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use swath_core::catalog::stac::dataset_to_stac_collection;
use swath_core::catalog::{Catalog, Dataset, Layer as DomainLayer};
use swath_core::planner::Budget;
use swath_render::{CompileContext, CompileError, NodataPolicy, Resampling, compile};

use super::handlers::fetch_dataset;
use super::{OpenEoError, OpenEoState, SERVICE_ID_PREFIX, SERVICE_TYPE, TILE_SIZES};
use crate::provider::CatalogLayer;
use crate::udf::{UdfModules, UdfPublish};

/// A [`Dataset`] as an openEO collection document: the #30 STAC converter
/// output with the swath-internal fields (`swath:bands`, `swath:layers`)
/// removed, datacube dimensions derived from the extent and band
/// vocabulary, and the required links minted. openEO collections are
/// STAC-based — STAC stays hidden from Swath's own control plane (R2),
/// but openEO clients speak STAC, and that is the standard.
pub(super) fn collection_doc(dataset: &Dataset, base: &str) -> Value {
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

/// The pinned official openeo-processes 1.2.0 definitions for the
/// compiler's supported subset (byte-identical to the compiler's oracle
/// copies — a test asserts it), plus the Swath-profile narrowing note
/// appended to each description where v0 narrows the spec.
pub(super) const PROCESS_DEFINITIONS: &[(&str, &str)] = &[
    (
        include_str!("../../data/openeo-processes/add.json"),
        "supported inside a `reduce_dimension` reducer or a `merge_cubes` overlap_resolver, over \
         band elements, numbers, resolver operands, and other results.",
    ),
    (
        include_str!("../../data/openeo-processes/array_element.json"),
        "supported inside a `reduce_dimension` reducer, over its band array \
         (`from_parameter: \"data\"`); exactly one of `index`/`label`.",
    ),
    (
        include_str!("../../data/openeo-processes/divide.json"),
        "supported inside a `reduce_dimension` reducer or a `merge_cubes` overlap_resolver; \
         division by zero makes the pixel no-data.",
    ),
    (
        include_str!("../../data/openeo-processes/filter_temporal.json"),
        "narrows the frame-selection resolution window (ADR 0015): the interval constrains \
         *which granule backs a frame* — the latest acquisition inside the window — never how \
         pixels combine; bounds must be UTC (`Z`) date-times, dates, or years, compared at \
         millisecond precision; `dimension` must be omitted, null, or `t`; an interval that \
         provably selects nothing is rejected at validation time.",
    ),
    (
        include_str!("../../data/openeo-processes/linear_scale_range.json"),
        "`outputMin`/`outputMax` must be exactly 0/255, spelled out explicitly (the render \
         path quantizes to 8-bit RGBA; the spec's defaults, 0/1, are rejected); at most one \
         scale per graph, applied after reduction/composition.",
    ),
    (
        include_str!("../../data/openeo-processes/load_collection.json"),
        "`id` must name the collection the graph is authored against; `bands` is required and \
         entries must be dataset band names; `temporal_extent` constrains the frame-selection \
         resolution window (ADR 0015): which granule backs a frame — the latest acquisition \
         inside the window — never how pixels combine (bounds must be UTC (`Z`) date-times, \
         dates, or years, compared at millisecond precision); `spatial_extent` and \
         `properties` are accepted and ignored (tile serving decides the spatial window).",
    ),
    (
        include_str!("../../data/openeo-processes/merge_cubes.json"),
        "the two-cube join at the bounded profile (ADR 0022): `cube1` and `cube2` must be gray \
         (one value per pixel — an `ndvi` or `reduce_dimension` result), unscaled, and load \
         this same collection through two different `load_collection` nodes, each with its \
         own `temporal_extent` (one granule per branch, frame-selected per ADR 0015; a tile's \
         `datetime=` is intersected with every branch's window); `overlap_resolver` is \
         required — a child graph over `x` (from `cube1`) and `y` (from `cube2`) producing one \
         value per pixel pair, e.g. `subtract` — since the spec's default (fail on overlap) \
         would reject every pixel; `context` is not accepted; the result is gray, over both \
         sources; band-wise merges, `mask`, cross-collection joins, and UDF results as inputs \
         are outside the profile.",
    ),
    (
        include_str!("../../data/openeo-processes/multiply.json"),
        "supported inside a `reduce_dimension` reducer or a `merge_cubes` overlap_resolver, over \
         band elements, numbers, resolver operands, and other results.",
    ),
    (
        include_str!("../../data/openeo-processes/ndvi.json"),
        "`target_band` must be omitted or null (the bands dimension is dropped; the result is gray).",
    ),
    (
        include_str!("../../data/openeo-processes/reduce_dimension.json"),
        "only `dimension: \"bands\"` is supported.",
    ),
    (
        include_str!("../../data/openeo-processes/run_udf.json"),
        "runs a sandboxed WASM module per tile (ADR 0018) over the loaded cube, one request \
         plane per loaded band in load order, producing 1 (gray) or 3 (RGB) output planes; \
         `runtime` must be \"wasm\" and `version` omitted, null, or \"1\"; `udf` is either \
         `data:application/wasm;base64,…` (at most 8 MiB) or an absolute http(s) URL fetched \
         exactly once when the graph is submitted — the module is thereafter addressed by \
         its content hash, so a changed remote never changes a published service; `context` \
         passes through to the module verbatim; the module must import nothing and export \
         the Swath UDF ABI v1 symbols (docs/udf-abi/v1.md); one `run_udf` per graph, over a \
         loaded (unreduced, unscaled) cube; its result accepts only `linear_scale_range` and \
         a colormap-less `save_result`. `POST /result` previews a `run_udf` graph under the \
         same per-tile fuel budget publishing would enforce (the validation loop before \
         publishing): a module that runs out of fuel or time answers ProcessGraphComplexity, \
         one that traps or answers malformed output ProcessParameterInvalid with the \
         executor's diagnosis. Listed only where this deployment wires a UDF executor and \
         module store.",
    ),
    (
        include_str!("../../data/openeo-processes/save_result.json"),
        "`format` must be \"png\" (case-insensitive); `options` accepts exactly one optional \
         key, `colormap` (\"grayscale\" | \"viridis\" | \"magma\" | \"rdylgn\"), the palette \
         applied to a gray result (rejected on a multi-band composite or a run_udf result; \
         absent, gray results default to \"grayscale\"); must be the graph's result node.",
    ),
    (
        include_str!("../../data/openeo-processes/subtract.json"),
        "supported inside a `reduce_dimension` reducer or a `merge_cubes` overlap_resolver, over \
         band elements, numbers, resolver operands, and other results.",
    ),
];

/// The served process list, built once: pinned definitions with the
/// narrowing note appended. `run_udf` is included only when `udf` is
/// wired — the list states what this deployment offers.
pub(super) fn process_list(udf: bool) -> Vec<&'static Value> {
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
    .iter()
    .filter(|doc| udf || doc["id"] != "run_udf")
    .collect()
}

/// The parsed, validated `POST /services` request body.
pub(super) struct ServiceRequest {
    pub(super) title: Option<String>,
    pub(super) description: Option<String>,
    /// The full `process` object (`process_graph_with_metadata`), stored
    /// verbatim on the layer.
    pub(super) process: Value,
    pub(super) tile_size: u32,
}

impl ServiceRequest {
    /// Validates the store-service request: type `xyz` (case-insensitive,
    /// per the spec), a process graph present, only supported
    /// configuration settings, no disabling.
    pub(super) fn parse(body: &Value) -> Result<Self, OpenEoError> {
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
pub(super) fn loaded_collection(graph: &Value) -> Option<&str> {
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
pub(super) fn compile_context(dataset: &Dataset) -> CompileContext {
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
/// so a restarted server serves exactly what was published. `udf` is the
/// graph's `run_udf` inputs ([`UdfPublish::rehydrate`] at startup —
/// resolved from the module store by hash, never fetched; `None` where
/// no UDF support is wired, which makes a `run_udf` graph
/// [`CompileError::UdfUnavailable`]). `budget` is the operator's resolved
/// global budget (#272) — the same value config-declared layers get, so
/// a published service serves under the `[budget]` table and the global
/// flags/env exactly as a declared layer does. Nothing about the budget
/// is persisted with the service: rehydration passes the *current*
/// config's value, so an operator tightening `[budget]` and restarting
/// tightens every published service too.
///
/// # Errors
///
/// Any [`CompileError`] from re-compiling the recorded graph against the
/// dataset's current band vocabulary (e.g. a band was removed from the
/// dataset config since the service was created).
pub fn compile_service_layer(
    dataset: &Dataset,
    layer: &DomainLayer,
    udf: Option<&UdfModules>,
    budget: &Budget,
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
    let ctx = match udf {
        Some(udf) => udf.apply(compile_context(dataset)),
        None => compile_context(dataset),
    };
    let product = compile(graph, &ctx)?;
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
        budget: budget.clone(),
        window: product.window,
        sources: product.sources,
    })
}

/// The content-derived service id: `xyz-` plus the first 12 hex digits of
/// the SHA-256 of the canonical (sorted-key) process-graph JSON. Identical
/// definition, identical id — creation is idempotent by construction.
pub(super) fn service_id(process_graph: &Value) -> String {
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
pub(super) fn service_doc(base: &str, layer: &DomainLayer, full: bool) -> Value {
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

/// The collection a graph is authored against, resolved from its
/// `load_collection` node — the shared pre-pass of `POST /services` and
/// `POST /result` (the compiler re-validates every `load_collection`
/// node against the compile context).
pub(super) async fn graph_dataset<S, R, C: Catalog>(
    app: &OpenEoState<S, R, C>,
    process: &Value,
) -> Result<Dataset, OpenEoError> {
    let collection = loaded_collection(process).ok_or_else(|| {
        OpenEoError::new(
            StatusCode::BAD_REQUEST,
            "ProcessGraphInvalid",
            "Invalid process graph specified: no load_collection node names a collection.",
        )
    })?;
    fetch_dataset(app, collection).await
}

/// What a graph lowering reads from the surface's state beyond the
/// request itself: the `run_udf` wiring (`None` = not offered) and the
/// operator's global budget (#272). [`OpenEoState::lowering`] hands it
/// out so `POST /services` and `POST /result` cannot lower differently.
#[derive(Clone, Copy)]
pub(super) struct Lowering<'a> {
    pub(super) udf: Option<&'a UdfPublish>,
    pub(super) budget: &'a Budget,
}
