// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The openEO surface's handlers: capabilities, collections,
//! processes, secondary services and the bounded preview (ADR 0014).

// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use swath_core::catalog::{Bbox, Catalog, Dataset, DatasetId, Layer as DomainLayer};
use swath_core::planner::Budget;
use swath_core::reproject::Reproject;
use swath_core::source::RasterSource;
use swath_core::tile::TileCoord;
use swath_render::{NoUdf, UdfExecutor, compile, plan_for, render_tile};

use super::errors::{preview_render_error, preview_resolution_error, service_not_found};
use super::types::{
    Lowering, ServiceRequest, collection_doc, compile_context, compile_service_layer,
    graph_dataset, process_list, service_doc, service_id,
};
use super::{
    OPENEO_API_VERSION, OpenEoError, OpenEoState, PREVIEW_MAX_ZOOM, PREVIEW_TILE_SIZE,
    SERVICE_TYPE, TILE_SIZES, WEB_MERCATOR_MAX_LAT,
};
use crate::provider::CatalogLayer;
use crate::udf::UdfPublish;

/// `GET /.well-known/openeo` — version discovery: this one instance.
pub(super) async fn well_known<S, R, C>(
    State(app): State<Arc<OpenEoState<S, R, C>>>,
) -> Json<Value> {
    Json(json!({
        "versions": [{
            "url": format!("{base}/", base = app.base_url),
            "api_version": OPENEO_API_VERSION,
            "production": false,
        }],
    }))
}

pub(super) async fn collections<S, R, C: Catalog>(
    State(app): State<Arc<OpenEoState<S, R, C>>>,
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

pub(super) async fn collection<S, R, C: Catalog>(
    State(app): State<Arc<OpenEoState<S, R, C>>>,
    Path(collection_id): Path<String>,
) -> Result<Json<Value>, OpenEoError> {
    let dataset = fetch_dataset(&app, &collection_id).await?;
    Ok(Json(collection_doc(&dataset, &app.base_url)))
}

/// The collection, or the standardized `CollectionNotFound`.
pub(super) async fn fetch_dataset<S, R, C: Catalog>(
    app: &OpenEoState<S, R, C>,
    id: &str,
) -> Result<Dataset, OpenEoError> {
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

pub(super) async fn processes<S, R, C>(
    State(app): State<Arc<OpenEoState<S, R, C>>>,
) -> Json<Value> {
    Json(json!({
        "processes": process_list(app.udf.is_some()),
        "links": [{
            "rel": "self",
            "href": format!("{base}/processes", base = app.base_url),
            "type": "application/json",
        }],
    }))
}

/// `GET /file_formats` — the honest single-format answer: PNG out (the
/// ADR 0014 preview), nothing in (`load_collection` is the only source).
/// Standard clients (openeo-python-client's `save_result`) validate the
/// requested format against this document before the POST.
pub(super) async fn file_formats<S, R, C>(
    State(_): State<Arc<OpenEoState<S, R, C>>>,
) -> Json<Value> {
    Json(json!({
        "input": {},
        "output": {
            "PNG": {
                "title": "Portable Network Graphics",
                "gis_data_types": ["raster"],
                "parameters": {},
            },
        },
    }))
}

pub(super) async fn service_types<S, R, C>(
    State(_): State<Arc<OpenEoState<S, R, C>>>,
) -> Json<Value> {
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

/// Every persisted service layer (those carrying a `process` record),
/// with its dataset, across all datasets — the catalog is the source of
/// truth for what services exist.
pub(super) async fn service_layers<S, R, C: Catalog>(
    app: &OpenEoState<S, R, C>,
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

pub(super) async fn list_services<S, R, C: Catalog>(
    State(app): State<Arc<OpenEoState<S, R, C>>>,
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

pub(super) async fn describe_service<S, R, C: Catalog>(
    State(app): State<Arc<OpenEoState<S, R, C>>>,
    Path(service_id): Path<String>,
) -> Result<Json<Value>, OpenEoError> {
    let services = service_layers(&app).await?;
    let (_, layer) = services
        .iter()
        .find(|(_, layer)| layer.id == service_id)
        .ok_or_else(|| service_not_found(&service_id))?;
    Ok(Json(service_doc(&app.base_url, layer, true)))
}

/// **The** graph lowering (single construction site): compiles `process`
/// through the whole process compiler against the dataset's bands, lowers
/// the compiled product to the persisted layer vocabulary via the one
/// constructor, and derives the servable [`CatalogLayer`] template from
/// the persisted form — exactly the `POST /services` motion, shared with
/// `POST /result` so a preview renders precisely what publishing the
/// same graph would serve. A `run_udf` graph resolves its remote module
/// **once**, here (ADR 0018, #204); the module bytes come back for the
/// publish path to persist (a preview persists nothing). The template
/// carries the operator's global budget, from `lowering`.
pub(super) async fn lower_graph(
    lowering: Lowering<'_>,
    dataset: &Dataset,
    process: &Value,
    id: String,
    title: Option<String>,
    description: Option<String>,
    tile_size: u32,
) -> Result<(DomainLayer, CatalogLayer, Option<Vec<u8>>), OpenEoError> {
    let Lowering { udf, budget } = lowering;
    // The compile motion's UDF inputs: remote modules fetched now, once.
    let modules = match udf {
        Some(udf) => Some(udf.resolve(process).await?),
        None => None,
    };
    let ctx = match &modules {
        Some(modules) => modules.apply(compile_context(dataset)),
        None => compile_context(dataset),
    };
    // Validate: the whole process compiler, against the collection's bands.
    let product = compile(process, &ctx)?;

    // Lower the compiled product to the persisted layer vocabulary: the
    // single constructor derives the metadata from the same spec the
    // plan was built from, so the two representations cannot disagree.
    let (_, meta) = plan_for(&product.spec);
    let layer = DomainLayer {
        id: id.clone(),
        title: title.unwrap_or(id),
        description: description.unwrap_or_default(),
        plan: meta.kind,
        rescale: meta.rescale,
        colormap: meta.colormap,
        resampling: swath_core::catalog::Resampling::Bilinear,
        tile_size,
        process: Some(process.clone()),
    };
    // One lowering for the live insert, every future rehydration, and
    // the preview render.
    let template = compile_service_layer(dataset, &layer, modules.as_ref(), budget)
        .map_err(|err| OpenEoError::internal(format!("service template compile failed: {err}")))?;
    Ok((layer, template, product.udf_module))
}

/// `POST /services` — the authoring loop in one motion (R3):
/// validate the graph through the compiler against the referenced
/// collection's bands, persist the derived layer on the dataset
/// (`swath:layers`), make it servable immediately, answer 201 with the
/// service's location and identifier.
pub(super) async fn create_service<S, R, C: Catalog>(
    State(app): State<Arc<OpenEoState<S, R, C>>>,
    Json(body): Json<Value>,
) -> Result<Response, OpenEoError> {
    let request = ServiceRequest::parse(&body)?;
    let mut dataset = graph_dataset(&app, &request.process).await?;

    let id = service_id(&request.process);
    let (layer, template, module) = lower_graph(
        app.lowering(),
        &dataset,
        &request.process,
        id.clone(),
        request.title,
        request.description,
        request.tile_size,
    )
    .await?;

    // The module persists by content hash BEFORE the service does: a
    // published `PlanKind::Udf { code_hash }` must always resolve
    // (ADR 0018). Idempotent — re-publishing the same bytes is a
    // no-op put.
    if let (Some(udf), Some(bytes)) = (app.udf.as_ref(), module.as_deref()) {
        let stored = udf.persist(bytes).await.map_err(|err| {
            OpenEoError::internal(format!("persisting the UDF module failed: {err}"))
        })?;
        debug_assert!(
            matches!(&layer.plan, swath_core::catalog::PlanKind::Udf { code_hash } if *code_hash == stored),
            "the store and the compiler disagree on the module's content hash"
        );
    }

    // Persist on the dataset (replace-or-append: identical graph, same id
    // — idempotent), then make it servable.
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
pub(super) async fn delete_service<S, R, C: Catalog>(
    State(app): State<Arc<OpenEoState<S, R, C>>>,
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

/// `POST /result` — the preview-bounded synchronous subset:
/// the spec-shaped body (`{"process": {"process_graph": …}}`) compiles
/// through [`lower_graph`], the exact `POST /services` path — same
/// narrowing, same typed diagnostics — and answers **one** small
/// overview-backed `image/png` render of [`preview_tile`] over the
/// graph's `spatial_extent` — or, when that is null, over what the
/// collection actually covers: the footprint of the granule the preview
/// resolves to (a config-declared dataset advertises a whole-world
/// placeholder box until a registration derives its extent, ROADMAP
/// row 15, and a preview of the placeholder is one blank root tile with
/// the granule sub-pixel inside it). Rendered under a
/// budget whose `max_estimated_live_bytes` ceiling refuses over-budget
/// live reads with the spec's `ProcessGraphComplexity`. Nothing is
/// persisted and nothing is published to the trace bus: a preview has no
/// side effects of any kind.
///
/// Debug headers (not part of the openEO contract, same instrument as
/// the tile handler): `X-Swath-Trace` summarizes the render decision,
/// and `X-Swath-Preview-Tile` names the rendered tile as
/// `{tileMatrix}/{tileRow}/{tileCol}` — the address a published service
/// would serve the identical bytes under.
pub(super) async fn preview_result<S, R, C>(
    State(app): State<Arc<OpenEoState<S, R, C>>>,
    Json(body): Json<Value>,
) -> Result<Response, OpenEoError>
where
    S: RasterSource,
    R: Reproject,
    C: Catalog,
{
    let process = body.get("process").cloned().unwrap_or(Value::Null);
    if process.get("process_graph").is_none_or(|g| !g.is_object()) {
        return Err(OpenEoError::new(
            StatusCode::BAD_REQUEST,
            "ProcessGraphMissing",
            "Invalid process specified. It doesn't contain a process graph.",
        ));
    }
    let dataset = graph_dataset(&app, &process).await?;

    // The single construction site: identical compile + lowering to
    // `POST /services`, so the preview pixels are exactly what
    // publishing this graph would serve. The template stays ephemeral —
    // never persisted, never inserted into the provider.
    let (_, mut template, _) = lower_graph(
        app.lowering(),
        &dataset,
        &process,
        "preview".to_owned(),
        None,
        None,
        PREVIEW_TILE_SIZE,
    )
    .await?;
    // The preview budget: the operator's global budget — so a
    // preview of a UDF graph runs the module under exactly the fuel its
    // published service would — with the byte ceiling the tighter of
    // the preview's own (ADR 0014) and the operator's. The operator's
    // cap layers UNDER the preview ceiling, never above it: a generous
    // `max-estimated-live-bytes` cannot widen what a preview may read.
    template.budget = Budget {
        max_estimated_live_bytes: Some(
            app.budget
                .max_estimated_live_bytes
                .map_or(app.preview_ceiling, |cap| cap.min(app.preview_ceiling)),
        ),
        ..template.budget
    };

    // A malformed extent refuses before the catalog is asked (the
    // argument's own diagnostic, not a resolution error).
    let extent = preview_extent(&process)?;
    let resolved = app
        .provider
        // Previews render the latest granule (fully open window): the
        // graph's `temporal_extent` stays accepted-and-ignored until the
        // compiler grows resolution windows (`docs/ROADMAP.md`).
        .resolve_template(&template, None)
        .await
        .map_err(preview_resolution_error)?;
    // A named extent is shown whole; with none named, the frame fits the
    // granule(s) this preview renders — every branch's footprint, joined
    // — the collection's real coverage at preview time; the
    // advertised extent stands in only for a resolution that carries no
    // footprint.
    let footprint = resolved
        .granules
        .iter()
        .map(|granule| granule.bbox)
        .reduce(union_bbox)
        .or(resolved.granule_bbox);
    let coord = match (extent, footprint) {
        (Some(bbox), _) => preview_tile(&bbox),
        (None, Some(footprint)) => preview_footprint_tile(&footprint),
        (None, None) => preview_tile(&dataset.extent.bbox),
    };
    let request = resolved.tile_request(coord);
    // The same executor the graph's module was just registered with
    // ; a deployment without UDF wiring could not have compiled a
    // UDF graph above, so `NoUdf` is never reached by one.
    let executor = app.udf.as_ref().map(UdfPublish::executor);
    let udf: &dyn UdfExecutor = executor
        .as_deref()
        .map_or(&NoUdf, |executor| executor as &dyn UdfExecutor);
    let (encoded, trace) = render_tile(&app.source, &app.reproject, udf, &request)
        .await
        .map_err(preview_render_error)?;

    let mut response = (
        StatusCode::OK,
        [(CONTENT_TYPE, HeaderValue::from_static("image/png"))],
        encoded.bytes,
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(&crate::routes::trace_debug_header(&trace)) {
        response.headers_mut().insert("x-swath-trace", value);
    }
    let tile = format!("{z}/{row}/{col}", z = coord.z, row = coord.y, col = coord.x);
    if let Ok(value) = HeaderValue::from_str(&tile) {
        response.headers_mut().insert("x-swath-preview-tile", value);
    }
    Ok(response)
}

/// The preview window: the graph's `spatial_extent` — read
/// from its (first) `load_collection` node, the same node
/// [`loaded_collection`] reads — validated: `Ok(Some)` for a well-formed
/// box, `Ok(None)` when the node names none (null or absent — the caller
/// then frames the resolved granule), and the standardized
/// `ProcessParameterInvalid` for a malformed one (the tile path ignores
/// the argument, so only the preview validates it).
pub(super) fn preview_extent(process: &Value) -> Result<Option<Bbox>, OpenEoError> {
    let nodes = process.get("process_graph").unwrap_or(process).as_object();
    let extent = nodes.and_then(|nodes| {
        nodes.values().find_map(|node| {
            (node.get("process_id")?.as_str()? == "load_collection")
                .then(|| node.get("arguments")?.get("spatial_extent"))
                .flatten()
        })
    });
    let Some(extent) = extent.filter(|extent| !extent.is_null()) else {
        return Ok(None);
    };
    let invalid = |detail: String| {
        OpenEoError::new(StatusCode::BAD_REQUEST, "ProcessParameterInvalid", detail)
    };
    let side = |name: &str| -> Result<f64, OpenEoError> {
        extent
            .get(name)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .ok_or_else(|| {
                invalid(format!(
                    "invalid argument `spatial_extent`: `{name}` must be a finite number, \
                     got {got}",
                    got = extent.get(name).unwrap_or(&Value::Null),
                ))
            })
    };
    let (west, south) = (side("west")?, side("south")?);
    let (east, north) = (side("east")?, side("north")?);
    if west > east || south > north {
        return Err(invalid(format!(
            "invalid argument `spatial_extent`: west ≤ east and south ≤ north required, \
             got west..east {west}..{east}, south..north {south}..{north}",
        )));
    }
    Ok(Some(Bbox {
        west,
        south,
        east,
        north,
    }))
}

/// Web Mercator unit-square fractions of a lon/lat point, clamped into
/// the projection's domain.
pub(super) fn mercator_fraction(lon: f64, lat: f64) -> (f64, f64) {
    let lon = lon.clamp(-180.0, 180.0);
    let lat = lat.clamp(-WEB_MERCATOR_MAX_LAT, WEB_MERCATOR_MAX_LAT);
    let x = (lon + 180.0) / 360.0;
    let y = (1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0;
    (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))
}

/// The `WebMercatorQuad` matrix index of a unit-square fraction at `z`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the fraction is clamped into [0, 1] and z ≤ 24, so \
              fraction × 2^z is a non-negative integer ≤ 2^24"
)]
pub(super) fn matrix_index(fraction: f64, z: u8) -> u32 {
    let scale = f64::from(1_u32 << z);
    ((fraction * scale) as u32).min((1_u32 << z) - 1)
}

/// The preview's target for a *named* `spatial_extent`: the **deepest**
/// `WebMercatorQuad` tile that fully contains the (Web-Mercator-clamped)
/// bbox — one small render, never a mosaic, and the whole box the author
/// asked for. An extent straddling a tile boundary is served by the
/// parent tile that contains it whole; descent stops at
/// [`PREVIEW_MAX_ZOOM`], the tiling scheme's deepest matrix.
pub(super) fn preview_tile(bbox: &Bbox) -> TileCoord {
    let (fraction, index) = (mercator_fraction, matrix_index);
    let (min_x, min_y) = fraction(bbox.west, bbox.north); // NW corner
    let (max_x, max_y) = fraction(bbox.east, bbox.south); // SE corner
    let mut chosen = TileCoord::new(0, 0, 0).expect("the z0 root tile is addressable");
    for z in 1..=PREVIEW_MAX_ZOOM {
        let (x, y) = (index(min_x, z), index(min_y, z));
        if (x, y) != (index(max_x, z), index(max_y, z)) {
            break;
        }
        chosen = TileCoord::new(z, x, y).expect("indices are within the matrix by construction");
    }
    chosen
}

/// The preview's target when the graph names *no* `spatial_extent`: the
/// footprint of the granule the preview renders, at its own scale — the
/// **deepest** `WebMercatorQuad` tile at least as large as the footprint
/// (side for side, in Mercator fractions), the one containing the
/// footprint's center. The containing-tile rule of [`preview_tile`]
/// serves a named box whole, but a footprint straddling a boundary at
/// every deep zoom would climb to a tile where it is a few pixels
/// (the fixture granule is a sliver of z7, invisible at
/// thumbnail size); with nothing named, the author asked to see the
/// data, so the frame fits the data and a straddling edge is cropped.
/// The smallest box holding both — what a two-source preview frames.
pub(super) fn union_bbox(a: Bbox, b: Bbox) -> Bbox {
    Bbox {
        west: a.west.min(b.west),
        south: a.south.min(b.south),
        east: a.east.max(b.east),
        north: a.north.max(b.north),
    }
}

pub(super) fn preview_footprint_tile(bbox: &Bbox) -> TileCoord {
    let (min_x, min_y) = mercator_fraction(bbox.west, bbox.north); // NW corner
    let (max_x, max_y) = mercator_fraction(bbox.east, bbox.south); // SE corner
    let side = (max_x - min_x).max(max_y - min_y);
    let (center_x, center_y) = (f64::midpoint(min_x, max_x), f64::midpoint(min_y, max_y));
    let mut chosen = TileCoord::new(0, 0, 0).expect("the z0 root tile is addressable");
    for z in 1..=PREVIEW_MAX_ZOOM {
        if 1.0 / f64::from(1_u32 << z) < side {
            break;
        }
        chosen = TileCoord::new(z, matrix_index(center_x, z), matrix_index(center_y, z))
            .expect("indices are within the matrix by construction");
    }
    chosen
}
