// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `run_udf` served (ADR 0018, #205): **the** dual-implementation golden
//! through the HTTP tile path — the reference NDVI module
//! (`examples/udf/ndvi`) published as a service renders byte-identical
//! tiles to the built-in band-math `ndvi` published beside it, which
//! hands the UDF path the whole rio-tiler-validated pipeline as its
//! ground truth (and the tile is checked against that oracle's golden
//! directly, too). The cost is visible in the debug header, the Trace,
//! and the preview; a module that exhausts its fuel answers a pinned
//! RFC 7807 problem.

#[allow(
    dead_code,
    reason = "shared between the API test targets; not every helper is used in each"
)]
mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use object_store::local::LocalFileSystem;
use serde_json::{Value, json};
use swath_api::{ApiState, Layer, LayerRegistry, TraceExtension, router};
use swath_core::planner::Budget;
use swath_core::raster::AssetRef;
use swath_render::ir::{BandInput, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::udf::UdfStage;
use swath_render::{NodataPolicy, Resampling};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;
use swath_testkit::{DiffPolicy, diff, load_png};
use swath_udf_wasmtime::WasmtimeUdf;

use common::{NDVI_WASM as NDVI, NoFetch, wasm_data_url as data_url};

/// The golden-suite tile (z 12, col 848, row 1561) in OGC path order.
const TILE: &str = "tiles/12/1561/848";

/// load(b8a, b04) → `pixel` → linear_scale_range(-1..1 → 0..255) → save.
fn graph(pixel: &Value) -> Value {
    json!({ "process_graph": {
        "load": { "process_id": "load_collection", "arguments": {
            "id": "hls-s30", "spatial_extent": null, "temporal_extent": null,
            "bands": ["b8a", "b04"],
        }},
        "pixel": pixel,
        "scale": { "process_id": "linear_scale_range", "arguments": {
            "x": { "from_node": "pixel" },
            "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 255,
        }},
        "save": { "process_id": "save_result", "arguments": {
            "data": { "from_node": "scale" }, "format": "png",
        }, "result": true },
    }})
}

/// The UDF implementation: the reference module over the two bands.
fn udf_graph() -> Value {
    graph(&json!({ "process_id": "run_udf", "arguments": {
        "data": { "from_node": "load" },
        "udf": data_url(NDVI),
        "runtime": "wasm",
    }}))
}

/// The built-in implementation: the profile's `ndvi` process.
fn band_math_graph() -> Value {
    graph(&json!({ "process_id": "ndvi", "arguments": {
        "data": { "from_node": "load" }, "nir": "b8a", "red": "b04",
    }}))
}

async fn publish(app: &Router, process: Value) -> String {
    let body = json!({ "type": "xyz", "title": "ndvi", "process": process });
    let response = common::request_on(app, "POST", "/services", Some(body)).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    response.headers()["openeo-identifier"]
        .to_str()
        .expect("ascii id")
        .to_owned()
}

/// GET one tile: the PNG bytes, the parsed `X-Swath-Trace` header, and
/// the full Trace off the response extension.
async fn get_tile(app: &Router, layer: &str) -> (Vec<u8>, Value, Arc<swath_core::trace::Trace>) {
    let response = common::request_on(app, "GET", &format!("/tilesets/{layer}/{TILE}"), None).await;
    assert_eq!(response.status(), StatusCode::OK, "GET {layer}");
    assert_eq!(response.headers()["content-type"], "image/png");
    let header: Value = serde_json::from_str(
        response.headers()["x-swath-trace"]
            .to_str()
            .expect("header is ASCII"),
    )
    .expect("header is JSON");
    let trace = response
        .extensions()
        .get::<TraceExtension>()
        .expect("trace extension")
        .0
        .clone();
    (common::body_bytes(response).await, header, trace)
}

/// THE dual-implementation golden (#205): the UDF service and the
/// band-math service, published side by side, serve byte-identical
/// tiles — and the tile passes the rio-tiler oracle's golden, so the
/// UDF path inherits the validated pipeline whole. Cost is visible on
/// the UDF render only, and reproduces exactly across renders.
#[tokio::test]
async fn ndvi_udf_service_is_byte_identical_to_band_math_through_the_tile_path() {
    let udf_app = common::openeo_app_with_udf(NoFetch);
    let udf_id = publish(&udf_app.app, udf_graph()).await;
    let band_math_id = publish(&udf_app.app, band_math_graph()).await;
    assert_ne!(udf_id, band_math_id, "two distinct services");

    let (udf_bytes, udf_header, udf_trace) = get_tile(&udf_app.app, &udf_id).await;
    let (band_math_bytes, band_math_header, band_math_trace) =
        get_tile(&udf_app.app, &band_math_id).await;
    assert_eq!(udf_bytes, band_math_bytes, "dual implementation");

    // Not vacuous: real pixels, matching the oracle golden.
    let image = image::load_from_memory(&udf_bytes)
        .expect("PNG decodes")
        .into_rgba8();
    assert!(image.pixels().any(|p| p.0[3] == 255), "opaque data pixels");
    let golden =
        load_png(&common::render_goldens_dir().join("ndvi-12-848-1561.png")).expect("golden loads");
    let report = diff(&image, &golden).expect("dimensions match");
    assert!(
        report.passes(&DiffPolicy::default()),
        "UDF tile fails the oracle policy: max |diff| {}",
        report.max_abs_channel_diff,
    );

    // The cost, everywhere the platform explains itself.
    let fuel = udf_trace.udf_fuel_used.expect("the UDF render meters fuel");
    assert!(fuel > 0);
    assert!(
        fuel < Budget::default().max_udf_fuel_per_tile,
        "the reference module runs well inside the default budget: {fuel}"
    );
    assert_eq!(udf_header["udf_fuel_used"], json!(fuel));
    assert!(
        udf_trace.timings.udf_ms <= udf_trace.timings.pixel_ops_ms,
        "{:?}",
        udf_trace.timings
    );
    assert_eq!(band_math_trace.udf_fuel_used, None);
    assert!(band_math_header.get("udf_fuel_used").is_none());
    assert_eq!(band_math_trace.timings.udf_ms, 0);

    // Deterministic: same bytes, same fuel, on a second render.
    let (again, _, again_trace) = get_tile(&udf_app.app, &udf_id).await;
    assert_eq!(again, udf_bytes);
    assert_eq!(again_trace.udf_fuel_used, Some(fuel));
}

/// `POST /result` renders a `run_udf` graph through the same executor
/// the module was just registered with (the half PR #263 left to #205),
/// byte-identical to the band-math preview and with the fuel in its
/// debug header.
#[tokio::test]
async fn preview_renders_a_udf_graph_with_its_cost_visible() {
    let udf_app = common::openeo_app_with_udf(NoFetch);
    let preview = |process: Value| {
        let app = udf_app.app.clone();
        async move {
            let response =
                common::request_on(&app, "POST", "/result", Some(json!({ "process": process })))
                    .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], "image/png");
            let header: Value =
                serde_json::from_str(response.headers()["x-swath-trace"].to_str().expect("ASCII"))
                    .expect("JSON");
            (common::body_bytes(response).await, header)
        }
    };
    let (udf_png, udf_header) = preview(udf_graph()).await;
    let (band_math_png, band_math_header) = preview(band_math_graph()).await;
    assert_eq!(udf_png, band_math_png, "dual implementation, previewed");
    assert!(udf_header["udf_fuel_used"].as_u64().expect("fuel") > 0);
    assert!(band_math_header.get("udf_fuel_used").is_none());
}

/// A static registry serving a UDF plan under an explicit budget, over
/// the given executor — the fuel-axis harness.
fn udf_layer_app(max_udf_fuel_per_tile: u64, executor: Option<Arc<WasmtimeUdf>>) -> Router {
    let code_hash = swath_core::udf::code_hash(NDVI);
    let layer = Layer {
        id: "ndvi-udf".to_owned(),
        title: "NDVI (UDF)".to_owned(),
        description: String::new(),
        bands: BTreeMap::from([
            (
                "b8a".to_owned(),
                AssetRef::new("hlss30-t13sdd-2024158-b8a.tif"),
            ),
            (
                "b04".to_owned(),
                AssetRef::new("hlss30-t13sdd-2024158-b04.tif"),
            ),
        ]),
        plan: RenderPlan::new(
            vec![BandInput::new("b8a"), BandInput::new("b04")],
            vec![
                PixelOp::Udf(UdfStage::new(code_hash, 1, Value::Null)),
                PixelOp::Rescale {
                    min: -1.0,
                    max: 1.0,
                },
            ],
            OutputSpec::new(TileFormat::Png),
        ),
        resampling: Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
        tile_size: 256,
        budget: Budget {
            max_udf_fuel_per_tile,
            ..Budget::default()
        },
    };
    let store = LocalFileSystem::new_with_prefix(common::fixtures_dir()).expect("fixture dir");
    let mut state = ApiState::new(
        LayerRegistry::new([layer]),
        CogSource::new(Arc::new(store)),
        Proj4rsReproject,
        common::BASE_URL,
    );
    if let Some(executor) = executor {
        state = state.with_udf_executor(executor);
    }
    router(Arc::new(state))
}

/// The fuel-exhaustion tile error, and its unwired sibling, as RFC 7807
/// problems — shape pinned: the OGC exception schema holds, the status
/// is a 500, and `detail` carries the executor's own diagnosis (the
/// budget, the hint) rather than the outer `pixel ops failed`.
#[tokio::test]
async fn fuel_exhaustion_is_a_pinned_rfc7807_problem() {
    let executor = Arc::new(WasmtimeUdf::new().expect("engine builds on this host"));
    executor.compile(NDVI).expect("fixture compiles");
    let shape = |app: Router| async move {
        let response =
            common::request_on(&app, "GET", &format!("/tilesets/ndvi-udf/{TILE}"), None).await;
        let status = response.status().as_u16();
        assert_eq!(response.headers()["content-type"], "application/json");
        let body = common::body_json(response).await;
        common::assert_valid("common/exception.json", &body);
        json!({ "status": status, "body": body })
    };
    let starved = shape(udf_layer_app(1_000, Some(Arc::clone(&executor)))).await;
    let unwired = shape(udf_layer_app(1_000, None)).await;
    // A budget the module fits in renders — the ceiling is the only
    // difference between the failing and the serving layer.
    let generous = udf_layer_app(Budget::default().max_udf_fuel_per_tile, Some(executor));
    let (_, header, _) = get_tile(&generous, "ndvi-udf").await;
    assert!(header["udf_fuel_used"].as_u64().expect("fuel") > 1_000);

    insta::assert_json_snapshot!(
        "udf_tile_error_shapes",
        json!({
            "udf exhausts the layer's max_udf_fuel_per_tile": starved,
            "udf plan on a deployment without an executor": unwired,
        })
    );
}
