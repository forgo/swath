// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The authoring loop, end to end (issue #41, ADR 0010, R3): POST an
//! openEO process graph as an XYZ service → 201 with the tile URL →
//! fetch a tile from it → **byte-identical** to the built-in layer
//! compiled from the same math (same compiler, same serve path), and
//! held to the two-level NDVI golden scheme of issue #94: colormapped
//! bytes pinned against the committed self-golden, values still proven
//! against the grayscale GDAL/rio-tiler oracle golden. Plus
//! the persistence contract (`swath:layers` carries the graph verbatim),
//! idempotent re-creation, deletion, and the snapshot-pinned openEO error
//! shapes for every documented failure path.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};
use swath_core::catalog::PlanKind;
use swath_testsupport::{DiffPolicy, diff, load_png};

/// The NDVI process graph, authored the way an openEO client would:
/// dataset band names, the `ndvi` convenience process, the -1..1 → 0..255
/// scale of the built-in layer, and — matching the built-in layer since
/// issue #94 — the `RdYlGn` colormap named as a `save_result` option.
fn ndvi_request() -> Value {
    ndvi_request_with_save_options(&json!({ "colormap": "rdylgn" }))
}

/// [`ndvi_request`] with explicit `save_result` `options`.
fn ndvi_request_with_save_options(options: &Value) -> Value {
    json!({
        "type": "xyz",
        "title": "NDVI (authored)",
        "description": "NDVI published through POST /services.",
        "process": { "process_graph": {
            "load": { "process_id": "load_collection", "arguments": {
                "id": "hls-s30", "spatial_extent": null, "temporal_extent": null,
                "bands": ["b8a", "b04"],
            }},
            "ndvi": { "process_id": "ndvi", "arguments": {
                "data": { "from_node": "load" }, "nir": "b8a", "red": "b04",
            }},
            "scale": { "process_id": "linear_scale_range", "arguments": {
                "x": { "from_node": "ndvi" },
                "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 255,
            }},
            "save": { "process_id": "save_result", "arguments": {
                "data": { "from_node": "scale" }, "format": "png",
                "options": options,
            }, "result": true },
        }},
    })
}

async fn tile_bytes(app: &axum::Router, path: &str) -> Vec<u8> {
    let response = common::request_on(app, "GET", path, None).await;
    assert_eq!(response.status(), StatusCode::OK, "GET {path}");
    assert_eq!(response.headers()["content-type"], "image/png");
    common::body_bytes(response).await
}

/// THE loop (R3 proven end-to-end): graph in, live XYZ out, pixels
/// byte-identical to the built-in NDVI layer — same compiler, same path.
#[allow(
    clippy::too_many_lines,
    reason = "one linear story: publish, serve, two-level goldens, persist, rehydrate"
)]
#[tokio::test]
async fn post_service_serves_tiles_byte_identical_to_the_builtin_ndvi() {
    let (app, catalog) = common::openeo_app();

    // Publish.
    let response = common::request_on(&app, "POST", "/services", Some(ndvi_request())).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let id = response.headers()["openeo-identifier"]
        .to_str()
        .expect("identifier")
        .to_owned();
    assert_eq!(
        response.headers()["location"].to_str().expect("location"),
        format!("http://localhost/services/{id}")
    );

    // The service URL is live immediately: same tile, three oracles.
    let service_tile = tile_bytes(&app, &format!("/tilesets/{id}/tiles/12/1561/848")).await;
    let builtin_tile = tile_bytes(&app, "/tilesets/ndvi/tiles/12/1561/848").await;
    assert_eq!(
        service_tile, builtin_tile,
        "authored NDVI must be byte-identical to the built-in NDVI layer"
    );
    // Two-level golden scheme (issue #94, see golden_ir.rs): the GDAL
    // oracle is grayscale, so (2) the colormapped bytes are pinned
    // against the committed self-golden our own pipeline produced...
    let colormapped_golden =
        std::fs::read(common::render_goldens_dir().join("ndvi-rdylgn-12-848-1561.png"))
            .expect("colormapped golden loads");
    assert_eq!(
        service_tile, colormapped_golden,
        "authored NDVI must serve exactly the committed colormapped golden bytes"
    );
    // ...and (1) the *values* stay oracle-validated: the same graph with
    // a grayscale colormap must still match the GDAL/rio-tiler golden.
    let gray_request = ndvi_request_with_save_options(&json!({ "colormap": "grayscale" }));
    let response = common::request_on(&app, "POST", "/services", Some(gray_request)).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let gray_id = response.headers()["openeo-identifier"]
        .to_str()
        .expect("identifier")
        .to_owned();
    let gray_tile = tile_bytes(&app, &format!("/tilesets/{gray_id}/tiles/12/1561/848")).await;
    let golden =
        load_png(&common::render_goldens_dir().join("ndvi-12-848-1561.png")).expect("golden loads");
    let served = image::load_from_memory(&gray_tile)
        .expect("PNG decodes")
        .into_rgba8();
    let report = diff(&served, &golden).expect("dimensions match");
    assert!(
        report.passes(&DiffPolicy::default()),
        "authored grayscale NDVI fails the oracle policy: max |diff| {}",
        report.max_abs_channel_diff
    );

    // Discoverable on the OGC surface too: the service is a tileset.
    let tilesets =
        common::body_json(common::request_on(&app, "GET", "/tilesets", None).await).await;
    let titles: Vec<&str> = tilesets["tilesets"]
        .as_array()
        .expect("tilesets")
        .iter()
        .map(|item| item["title"].as_str().expect("title"))
        .collect();
    assert!(titles.contains(&"NDVI (authored)"), "got {titles:?}");

    // Persistence contract: the dataset's swath:layers carries the layer
    // with the graph verbatim and the lowered plan vocabulary.
    let dataset = catalog.stored_dataset("hls-s30").expect("dataset persists");
    let layer = dataset
        .layers
        .iter()
        .find(|layer| layer.id == id)
        .expect("service layer persisted");
    assert_eq!(
        layer.process.as_ref().expect("process stored")["process_graph"],
        ndvi_request()["process"]["process_graph"]
    );
    assert_eq!(
        layer.plan,
        PlanKind::BandMath {
            expression: "(b8a - b04) / (b8a + b04)".to_owned()
        }
    );
    assert_eq!((layer.rescale.min, layer.rescale.max), (-1.0, 1.0));
    assert_eq!(
        layer.colormap,
        Some(swath_core::catalog::Colormap::RdYlGn),
        "the graph's colormap option persists variant-for-variant"
    );

    // Rehydration parity: recompiling the persisted layer (what `swath
    // serve` does at startup) reproduces the built-in NDVI plan exactly
    // — under whatever budget the CURRENT config resolves (#272): nothing
    // budget-shaped is persisted with the service, so a restart after
    // tightening `[budget]` tightens the service.
    let tightened = swath_core::planner::Budget {
        max_estimated_live_bytes: Some(1),
        max_udf_fuel_per_tile: 10_000_000,
        ..swath_core::planner::Budget::default()
    };
    let template =
        swath_api::compile_service_layer(&dataset, layer, None, &tightened).expect("recompiles");
    let registry = swath_api::LayerRegistry::hls_fixtures();
    assert_eq!(template.plan, registry.get("ndvi").expect("ndvi").plan);
    assert_eq!(template.budget, tightened);

    // Idempotent creation: the same definition maps to the same service.
    let response = common::request_on(&app, "POST", "/services", Some(ndvi_request())).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.headers()["openeo-identifier"].to_str().unwrap(),
        id
    );
    let dataset = catalog.stored_dataset("hls-s30").expect("dataset");
    assert_eq!(
        dataset.layers.iter().filter(|layer| layer.id == id).count(),
        1,
        "re-POSTing an identical graph must not duplicate the layer"
    );

    // Delete: gone from the services surface, the catalog, and serving.
    let response = common::request_on(&app, "DELETE", &format!("/services/{id}"), None).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = common::request_on(&app, "GET", &format!("/services/{id}"), None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = common::request_on(
        &app,
        "GET",
        &format!("/tilesets/{id}/tiles/12/1561/848"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let dataset = catalog.stored_dataset("hls-s30").expect("dataset");
    assert!(dataset.layers.iter().all(|layer| layer.id != id));
}

/// A three-band composite graph publishes the built-in truecolor math —
/// byte-identical, proving the composite path of the lowering too.
#[tokio::test]
async fn composite_service_matches_the_builtin_truecolor() {
    let (app, _) = common::openeo_app();
    let request = json!({
        "type": "XYZ", // case-insensitive per the spec
        "process": { "process_graph": {
            "load": { "process_id": "load_collection", "arguments": {
                "id": "hls-s30", "spatial_extent": null, "temporal_extent": null,
                "bands": ["b04", "b03", "b02"],
            }},
            "scale": { "process_id": "linear_scale_range", "arguments": {
                "x": { "from_node": "load" },
                "inputMin": 0, "inputMax": 3000, "outputMin": 0, "outputMax": 255,
            }},
            "save": { "process_id": "save_result", "arguments": {
                "data": { "from_node": "scale" }, "format": "png",
            }, "result": true },
        }},
    });
    let response = common::request_on(&app, "POST", "/services", Some(request)).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let id = response.headers()["openeo-identifier"]
        .to_str()
        .unwrap()
        .to_owned();

    let service_tile = tile_bytes(&app, &format!("/tilesets/{id}/tiles/12/1561/848")).await;
    let builtin_tile = tile_bytes(&app, "/tilesets/truecolor/tiles/12/1561/848").await;
    assert_eq!(service_tile, builtin_tile);
}

// --- Error paths: openEO error format, snapshot-pinned shapes ---

/// POSTs `body` and snapshots `(status, error body)` — the #32
/// diagnostics through the services surface, in the standard's format.
async fn error_shape(app: &axum::Router, body: Value) -> Value {
    let response = common::request_on(app, "POST", "/services", Some(body)).await;
    let status = response.status().as_u16();
    let error = common::body_json(response).await;
    json!({ "status": status, "error": error })
}

/// The change graph (ADR 0022) with `cube1`/`cube2` wired to the named
/// nodes and an optional resolver — the join's rejection cases each pick
/// a wrong input or drop the resolver.
fn merge_cubes_graph(cube1: &str, cube2: &str, resolver: Option<Value>) -> Value {
    let mut change = json!({ "process_id": "merge_cubes", "arguments": {
        "cube1": { "from_node": cube1 }, "cube2": { "from_node": cube2 },
    }});
    if let Some(resolver) = resolver {
        change["arguments"]["overlap_resolver"] = resolver;
    }
    let load = |extent: [&str; 2]| {
        json!({ "process_id": "load_collection", "arguments": {
            "id": "hls-s30", "bands": ["b8a", "b04"], "temporal_extent": extent,
        }})
    };
    let ndvi = |from: &str| {
        json!({ "process_id": "ndvi", "arguments": {
            "data": { "from_node": from }, "nir": "b8a", "red": "b04",
        }})
    };
    json!({
        "before": load(["2024-05-01T00:00:00Z", "2024-06-01T00:00:00Z"]),
        "after": load(["2024-06-01T00:00:00Z", "2024-07-01T00:00:00Z"]),
        "ndvi_before": ndvi("before"),
        "ndvi_after": ndvi("after"),
        "scaled_before": { "process_id": "linear_scale_range", "arguments": {
            "x": { "from_node": "ndvi_before" },
            "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 255,
        }},
        "change": change,
        "save": { "process_id": "save_result", "arguments": {
            "data": { "from_node": "change" }, "format": "png",
        }, "result": true },
    })
}

/// Wraps a process graph in a store-service request.
fn service_request(process_graph: &Value) -> Value {
    json!({ "type": "xyz", "process": { "process_graph": process_graph } })
}

#[tokio::test]
async fn error_paths_speak_the_openeo_error_format() {
    let (app, _) = common::openeo_app();
    let error_schema = common::openeo_schema("/components/schemas/error");

    let wrong_type = error_shape(&app, json!({ "type": "wms", "process": {} })).await;
    let missing_graph = error_shape(&app, json!({ "type": "xyz" })).await;
    let unknown_collection = error_shape(
        &app,
        service_request(&json!({
            "load": { "process_id": "load_collection", "arguments": {
                "id": "sentinel-99", "bands": ["b04"],
            }},
            "save": { "process_id": "save_result", "arguments": {
                "data": { "from_node": "load" }, "format": "png",
            }, "result": true },
        })),
    )
    .await;
    let unknown_process = error_shape(
        &app,
        service_request(&json!({
            "load": { "process_id": "load_collection", "arguments": {
                "id": "hls-s30", "bands": ["b8a", "b04"],
            }},
            "smooth": { "process_id": "apply_kernel", "arguments": {
                "data": { "from_node": "load" },
            }},
            "save": { "process_id": "save_result", "arguments": {
                "data": { "from_node": "smooth" }, "format": "png",
            }, "result": true },
        })),
    )
    .await;
    let unknown_band = error_shape(
        &app,
        service_request(&json!({
            "load": { "process_id": "load_collection", "arguments": {
                "id": "hls-s30", "bands": ["b8a", "swir22"],
            }},
            "save": { "process_id": "save_result", "arguments": {
                "data": { "from_node": "load" }, "format": "png",
            }, "result": true },
        })),
    )
    .await;
    let no_result_node = error_shape(
        &app,
        service_request(&json!({
            "load": { "process_id": "load_collection", "arguments": {
                "id": "hls-s30", "bands": ["b8a", "b04"],
            }},
        })),
    )
    .await;
    let bad_scale_range = error_shape(
        &app,
        service_request(&json!({
            "load": { "process_id": "load_collection", "arguments": {
                "id": "hls-s30", "bands": ["b8a", "b04"],
            }},
            "ndvi": { "process_id": "ndvi", "arguments": {
                "data": { "from_node": "load" }, "nir": "b8a", "red": "b04",
            }},
            "scale": { "process_id": "linear_scale_range", "arguments": {
                "x": { "from_node": "ndvi" },
                "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 1,
            }},
            "save": { "process_id": "save_result", "arguments": {
                "data": { "from_node": "scale" }, "format": "png",
            }, "result": true },
        })),
    )
    .await;
    let bad_configuration = error_shape(
        &app,
        json!({
            "type": "xyz",
            "configuration": { "antialias": true },
            "process": ndvi_request()["process"],
        }),
    )
    .await;

    let shapes = json!({
        "wrong service type": wrong_type,
        "missing process graph": missing_graph,
        "unknown collection": unknown_collection,
        "unknown process id": unknown_process,
        "unknown band": unknown_band,
        "no result node": no_result_node,
        "unsupported output range": bad_scale_range,
        "unsupported configuration setting": bad_configuration,
    });
    // Every body is a schema-valid openEO error.
    for (name, shape) in shapes.as_object().expect("object") {
        common::assert_openeo_valid(&error_schema, name, &shape["error"]);
    }
    insta::assert_json_snapshot!("openeo_error_shapes", shapes);
}

/// The join's rejections (ADR 0022) speak the registry's codes: the
/// input kinds are `ProcessParameterInvalid` on the argument they name,
/// a missing resolver is `ProcessParameterRequired`, a resolver that
/// yields a cube is `ProcessGraphInvalid`. Shapes pinned by snapshot.
#[tokio::test]
async fn merge_cubes_rejections_speak_the_openeo_error_format() {
    let (app, _) = common::openeo_app();
    let error_schema = common::openeo_schema("/components/schemas/error");
    let subtract = json!({ "process_graph": { "diff": { "process_id": "subtract", "arguments": {
        "x": { "from_parameter": "x" }, "y": { "from_parameter": "y" },
    }, "result": true }}});
    let cube_resolver = json!({ "process_graph": { "again": { "process_id": "load_collection",
        "arguments": { "id": "hls-s30", "bands": ["b04"] }, "result": true }}});
    let merge_multi = error_shape(
        &app,
        service_request(&merge_cubes_graph(
            "after",
            "before",
            Some(subtract.clone()),
        )),
    )
    .await;
    let merge_scaled = error_shape(
        &app,
        service_request(&merge_cubes_graph(
            "ndvi_after",
            "scaled_before",
            Some(subtract.clone()),
        )),
    )
    .await;
    let merge_no_resolver = error_shape(
        &app,
        service_request(&merge_cubes_graph("ndvi_after", "ndvi_before", None)),
    )
    .await;
    let merge_cube_resolver = error_shape(
        &app,
        service_request(&merge_cubes_graph(
            "ndvi_after",
            "ndvi_before",
            Some(cube_resolver),
        )),
    )
    .await;
    let shapes = json!({
        "merge_cubes over multi-band cubes": merge_multi,
        "merge_cubes over a scaled cube": merge_scaled,
        "merge_cubes without a resolver": merge_no_resolver,
        "merge_cubes resolver returning a cube": merge_cube_resolver,
    });
    for (name, shape) in shapes.as_object().expect("object") {
        common::assert_openeo_valid(&error_schema, name, &shape["error"]);
    }
    insta::assert_json_snapshot!("openeo_merge_cubes_error_shapes", shapes);
}

/// Unknown service id: 404 `ServiceNotFound` on describe and delete.
#[tokio::test]
async fn unknown_service_is_service_not_found() {
    let (app, _) = common::openeo_app();
    for method in ["GET", "DELETE"] {
        let response = common::request_on(&app, method, "/services/xyz-000000000000", None).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method}");
        let error = common::body_json(response).await;
        assert_eq!(error["code"], "ServiceNotFound");
    }
}

/// A published service serves under the operator's `[budget]` (#272): a
/// global `max-estimated-live-bytes` ceiling refuses its tile exactly as
/// it would a declared layer's — the same graph serves under the
/// default, so the ceiling is the only difference.
#[tokio::test]
async fn published_service_inherits_the_operator_byte_ceiling() {
    let tile = |id: &str| format!("/tilesets/{id}/tiles/12/1561/848");

    let (capped, _) = common::openeo_app_with_budget(
        None,
        swath_core::planner::Budget {
            max_estimated_live_bytes: Some(1),
            ..swath_core::planner::Budget::default()
        },
    );
    let response = common::request_on(&capped, "POST", "/services", Some(ndvi_request())).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let id = response.headers()["openeo-identifier"]
        .to_str()
        .expect("ascii id")
        .to_owned();
    let response = common::request_on(&capped, "GET", &tile(&id), None).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.headers()["content-type"], "application/json");
    let body = common::body_json(response).await;
    common::assert_valid("common/exception.json", &body);
    let detail = body["detail"].as_str().expect("detail");
    assert!(
        detail.contains("materialization budget exceeded") && detail.contains("1-byte ceiling"),
        "the refusal names the operator's ceiling: {detail}"
    );

    let (permissive, _) = common::openeo_app();
    let response = common::request_on(&permissive, "POST", "/services", Some(ndvi_request())).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    tile_bytes(&permissive, &tile(&id)).await;
}
