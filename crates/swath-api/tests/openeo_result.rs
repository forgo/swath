// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The preview-bounded `POST /result` (issue #169, ADR 0014): the
//! spec-shaped body compiles through the exact `POST /services` path and
//! answers one small overview-backed PNG — proven **byte-identical** to
//! the corresponding published-service tile (THE equivalence: preview
//! pixels are exactly what publishing serves, held against the same
//! oracle-golden service tiles `openeo_services.rs` pins). Plus the
//! honesty rules: no persistence of any kind, identical diagnostics for
//! identical graphs on either route, the budget refusal as the spec's
//! `ProcessGraphComplexity`, and error bodies schema-valid under the
//! pinned openEO 1.2.0 spec (the #27 pattern).
//!
//! And the preview as the `run_udf` validation loop (ADR 0018, #206):
//! upload a module, see it render — byte-identical to what publishing
//! serves — or see the exact fuel/trap diagnostic as a 400 in the
//! registry's vocabulary, before anything is published.

#[allow(
    dead_code,
    reason = "shared between the API test targets; not every helper is used in each"
)]
mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};

use common::{NDVI_WASM, NoFetch, wasm, wasm_data_url};

/// The built-in NDVI graph, exactly as `openeo_services.rs` authors it —
/// the same math as the built-in `ndvi` layer and the committed goldens.
fn ndvi_process() -> Value {
    ndvi_process_with_extent(&Value::Null)
}

/// [`ndvi_process`] with an explicit `load_collection` `spatial_extent`.
fn ndvi_process_with_extent(spatial_extent: &Value) -> Value {
    json!({ "process_graph": {
        "load": { "process_id": "load_collection", "arguments": {
            "id": "hls-s30", "spatial_extent": spatial_extent, "temporal_extent": null,
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
            "options": { "colormap": "rdylgn" },
        }, "result": true },
    }})
}

fn result_request(process: &Value) -> Value {
    json!({ "process": process })
}

/// The `run_udf` shape of [`ndvi_process`]: the same load and scale,
/// with the inline `module` in place of the built-in `ndvi` process.
fn udf_process(module: &[u8]) -> Value {
    json!({ "process_graph": {
        "load": { "process_id": "load_collection", "arguments": {
            "id": "hls-s30", "spatial_extent": null, "temporal_extent": null,
            "bands": ["b8a", "b04"],
        }},
        "udf": { "process_id": "run_udf", "arguments": {
            "data": { "from_node": "load" },
            "udf": wasm_data_url(module),
            "runtime": "wasm",
        }},
        "scale": { "process_id": "linear_scale_range", "arguments": {
            "x": { "from_node": "udf" },
            "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 255,
        }},
        "save": { "process_id": "save_result", "arguments": {
            "data": { "from_node": "scale" }, "format": "png",
        }, "result": true },
    }})
}

/// THE equivalence proof (ADR 0014: "same compiler path"): the preview
/// of the built-in NDVI graph is byte-identical to the tile the
/// published service serves at the very address the preview rendered —
/// the same oracle-golden-pinned serve path `openeo_services.rs` proves.
/// And previewing persists NOTHING: no service, no `swath:layers` write,
/// no tileset.
#[tokio::test]
async fn preview_is_byte_identical_to_the_published_service_tile() {
    let (app, catalog) = common::openeo_app();

    // Preview the draft over the committed fixture extent, named: a
    // named extent selects the deepest WebMercatorQuad tile containing
    // it whole — z7 (26, 48) — served from an overview: a preview is
    // exactly the workload overviews exist for.
    let fixture_extent =
        json!({ "west": -105.537, "south": 39.1954, "east": -105.3581, "north": 39.3345 });
    let ndvi_over_fixture = || ndvi_process_with_extent(&fixture_extent);
    let response = common::request_on(
        &app,
        "POST",
        "/result",
        Some(result_request(&ndvi_over_fixture())),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/png");
    assert_eq!(response.headers()["x-swath-preview-tile"], "7/48/26");
    let trace = response.headers()["x-swath-trace"]
        .to_str()
        .expect("trace header")
        .to_owned();
    assert!(
        trace.contains("\"decision\":\"overview\""),
        "the preview must be overview-backed, got {trace}"
    );
    let preview = common::body_bytes(response).await;
    assert!(!preview.is_empty());

    // Nothing persisted, nothing servable: the services list is empty,
    // the dataset's swath:layers are untouched, and no tileset appeared.
    let services =
        common::body_json(common::request_on(&app, "GET", "/services", None).await).await;
    assert_eq!(services["services"], json!([]));
    let dataset = catalog.stored_dataset("hls-s30").expect("dataset");
    assert!(
        dataset.layers.iter().all(|layer| layer.process.is_none()),
        "preview must not write swath:layers"
    );
    let tilesets =
        common::body_json(common::request_on(&app, "GET", "/tilesets", None).await).await;
    assert_eq!(
        tilesets["tilesets"].as_array().expect("tilesets").len(),
        2,
        "preview must not add a tileset"
    );

    // Publish the same graph; fetch the published service's tile at the
    // address the preview named. Byte-identical: same compiler, same
    // lowering, same render path, same pixels.
    let service = json!({ "type": "xyz", "process": ndvi_over_fixture() });
    let response = common::request_on(&app, "POST", "/services", Some(service)).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let id = response.headers()["openeo-identifier"]
        .to_str()
        .expect("identifier")
        .to_owned();
    let response =
        common::request_on(&app, "GET", &format!("/tilesets/{id}/tiles/7/48/26"), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let published = common::body_bytes(response).await;
    assert_eq!(
        preview, published,
        "the preview must be byte-identical to the published-service tile"
    );

    // Re-previewing after publishing changes nothing: still stateless.
    let response = common::request_on(
        &app,
        "POST",
        "/result",
        Some(result_request(&ndvi_over_fixture())),
    )
    .await;
    assert_eq!(common::body_bytes(response).await, published);
}

/// A `spatial_extent` narrows the preview window: the tiny box selects a
/// deeper tile than the collection extent, and — with no overview
/// eligible at that depth — the small live read is admitted under the
/// default preview budget (reasonable authoring-sized drafts are never
/// refused, the ADR's reopen trigger).
#[tokio::test]
async fn spatial_extent_selects_a_deeper_tile_and_small_live_previews_are_admitted() {
    let (app, _) = common::openeo_app();
    let extent = json!({ "west": -105.45, "south": 39.26, "east": -105.44, "north": 39.27 });
    let response = common::request_on(
        &app,
        "POST",
        "/result",
        Some(result_request(&ndvi_process_with_extent(&extent))),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tile = response.headers()["x-swath-preview-tile"]
        .to_str()
        .expect("preview tile header")
        .to_owned();
    let z: u8 = tile
        .split('/')
        .next()
        .expect("z")
        .parse()
        .expect("z parses");
    assert!(z > 7, "a tiny extent must select a deeper tile, got {tile}");
    let trace = response.headers()["x-swath-trace"]
        .to_str()
        .expect("trace header")
        .to_owned();
    assert!(
        trace.contains("\"decision\":\"live\""),
        "no overview is eligible this deep; the small live read serves: {trace}"
    );
}

/// No `spatial_extent` frames the granule the preview renders, not the
/// collection's advertised box: a config-declared dataset carries the
/// whole-world placeholder extent until a registration derives its real
/// one (ROADMAP row 15), and a preview tile of that box is the root tile
/// with the granule sub-pixel inside — a decoded, fully transparent PNG
/// (issue #270: the authoring canvas showed it as an empty
/// checkerboard). The preview names the granule's own tile instead.
#[tokio::test]
async fn null_extent_previews_the_resolved_granule_not_a_placeholder_extent() {
    let mut placeholder = common::hls_catalog_dataset();
    placeholder.extent.bbox = swath_core::catalog::Bbox {
        west: -180.0,
        south: -90.0,
        east: 180.0,
        north: 90.0,
    };
    placeholder.layers.clear();
    let (app, _) = common::openeo_app_seeded(placeholder, vec![common::hls_catalog_granule()]);

    // The collection still advertises the placeholder (deriving extents
    // is registration's job, not the preview's)…
    let collection =
        common::body_json(common::request_on(&app, "GET", "/collections/hls-s30", None).await)
            .await;
    assert_eq!(
        collection["extent"]["spatial"]["bbox"],
        json!([[-180.0, -90.0, 180.0, 90.0]])
    );

    // …but the preview frames the granule at its own scale: the z10 tile
    // around its center (the granule spans about half of it), with real
    // pixels — not the z7 tile that contains it whole as a sliver.
    let response = common::request_on(
        &app,
        "POST",
        "/result",
        Some(result_request(&ndvi_process())),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-swath-preview-tile"], "10/390/212");
    let preview = common::body_bytes(response).await;
    let decoded = image::load_from_memory(&preview)
        .expect("the preview decodes")
        .to_rgba8();
    let opaque = decoded.pixels().filter(|pixel| pixel[3] > 0).count();
    let total = decoded.pixels().count();
    // The fixture is a diagonal clip of its bbox (about a sixth of the
    // z10 tile paints); the z7 root-of-the-box tile painted ~1%, the
    // placeholder's z0 tile nothing at all.
    assert!(
        opaque * 8 >= total,
        "the granule-framed preview must be substantially painted: {opaque} of {total} px opaque"
    );

    // The granule-scale tile is exactly what publishing serves there —
    // the ADR 0014 equivalence holds for the footprint-framed preview too.
    let service = json!({ "type": "xyz", "process": ndvi_process() });
    let response = common::request_on(&app, "POST", "/services", Some(service)).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let id = response.headers()["openeo-identifier"]
        .to_str()
        .expect("identifier")
        .to_owned();
    let response = common::request_on(
        &app,
        "GET",
        &format!("/tilesets/{id}/tiles/10/390/212"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(common::body_bytes(response).await, preview);
}

/// The budget refusal (ADR 0014: refusal over degradation): when the
/// live estimate exceeds the preview ceiling and no overview can serve,
/// the answer is the spec's `ProcessGraphComplexity` — schema-valid,
/// status per the pinned registry — never a silent downgrade.
#[tokio::test]
async fn over_budget_previews_refuse_with_process_graph_complexity() {
    // A 1-byte ceiling forces the refusal; the extent is deep enough
    // that no overview factor is eligible (nothing cheaper can serve).
    let (app, _) = common::openeo_app_with_preview_ceiling(Some(1));
    let extent = json!({ "west": -105.45, "south": 39.26, "east": -105.44, "north": 39.27 });
    let response = common::request_on(
        &app,
        "POST",
        "/result",
        Some(result_request(&ndvi_process_with_extent(&extent))),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error = common::body_json(response).await;
    let schema = common::openeo_schema("/components/schemas/error");
    common::assert_openeo_valid(&schema, "budget refusal", &error);
    assert_eq!(error["code"], "ProcessGraphComplexity");
    let message = error["message"].as_str().expect("message");
    assert!(
        message.contains("budget") && message.contains("spatial extent"),
        "the refusal explains itself in budget terms: {message}"
    );
}

/// Identical graphs get identical diagnostics on either route (ADR 0014:
/// same compiler path, same codes) — plus the /result-specific shapes:
/// a missing graph, a malformed `spatial_extent`, a collection with no
/// granules yet. Every body is a schema-valid openEO error.
#[tokio::test]
async fn error_taxonomy_matches_the_services_route_and_stays_schema_valid() {
    let (app, catalog) = common::openeo_app();
    let error_schema = common::openeo_schema("/components/schemas/error");

    // A dataset with no granules: previewable graphs but no pixels yet.
    let mut empty = common::hls_catalog_dataset();
    empty.id = swath_core::catalog::DatasetId::new("hls-empty");
    empty.layers.clear();
    catalog.seed(empty, Vec::new());

    let unknown_band = json!({ "process_graph": {
        "load": { "process_id": "load_collection", "arguments": {
            "id": "hls-s30", "bands": ["b8a", "swir22"],
        }},
        "save": { "process_id": "save_result", "arguments": {
            "data": { "from_node": "load" }, "format": "png",
        }, "result": true },
    }});
    let unknown_collection = json!({ "process_graph": {
        "load": { "process_id": "load_collection", "arguments": {
            "id": "sentinel-99", "bands": ["b04"],
        }},
        "save": { "process_id": "save_result", "arguments": {
            "data": { "from_node": "load" }, "format": "png",
        }, "result": true },
    }});

    // Same graph, same code, whichever route it is submitted to.
    for (graph, code, status) in [
        (&unknown_band, "ProcessParameterInvalid", 400),
        (&unknown_collection, "CollectionNotFound", 404),
    ] {
        let preview =
            common::request_on(&app, "POST", "/result", Some(result_request(graph))).await;
        assert_eq!(preview.status().as_u16(), status, "{code} via /result");
        let preview_error = common::body_json(preview).await;
        common::assert_openeo_valid(&error_schema, code, &preview_error);
        assert_eq!(preview_error["code"], *code);

        let service = json!({ "type": "xyz", "process": graph });
        let published = common::request_on(&app, "POST", "/services", Some(service)).await;
        let published_error = common::body_json(published).await;
        assert_eq!(
            preview_error["code"], published_error["code"],
            "identical graphs must get identical codes on /result and /services"
        );
    }

    // No process graph at all.
    let response = common::request_on(&app, "POST", "/result", Some(json!({}))).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error = common::body_json(response).await;
    common::assert_openeo_valid(&error_schema, "missing graph", &error);
    assert_eq!(error["code"], "ProcessGraphMissing");

    // A malformed spatial_extent (the preview is the only consumer of
    // the argument; tiles ignore it).
    let bad_extent = json!({ "west": -105.5, "south": 39.2, "east": -105.6, "north": 39.3 });
    let response = common::request_on(
        &app,
        "POST",
        "/result",
        Some(result_request(&ndvi_process_with_extent(&bad_extent))),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error = common::body_json(response).await;
    common::assert_openeo_valid(&error_schema, "malformed extent", &error);
    assert_eq!(error["code"], "ProcessParameterInvalid");

    // A collection with no ingested granule: there is nothing to render.
    let no_granules = json!({ "process_graph": {
        "load": { "process_id": "load_collection", "arguments": {
            "id": "hls-empty", "bands": ["b8a", "b04"],
        }},
        "ndvi": { "process_id": "ndvi", "arguments": {
            "data": { "from_node": "load" }, "nir": "b8a", "red": "b04",
        }},
        "save": { "process_id": "save_result", "arguments": {
            "data": { "from_node": "ndvi" }, "format": "png",
        }, "result": true },
    }});
    let response =
        common::request_on(&app, "POST", "/result", Some(result_request(&no_granules))).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let error = common::body_json(response).await;
    common::assert_openeo_valid(&error_schema, "no granules", &error);
    assert_eq!(error["code"], "NotFound");
}

// --- The preview as the run_udf validation loop (ADR 0018, #206) ---

/// THE equivalence, for user code: the preview of the reference NDVI
/// module (`examples/udf/ndvi`, the #205 dual-implementation golden) is
/// byte-identical to the tile its published service serves at the
/// address the preview named — same compiler, same lowering, same
/// executor the module was registered with, same fuel budget. The fuel
/// is visible on the preview and reproduces on the published tile.
#[tokio::test]
async fn udf_preview_is_byte_identical_to_the_published_udf_service_tile() {
    let udf_app = common::openeo_app_with_udf(NoFetch);
    let app = &udf_app.app;
    let process = udf_process(NDVI_WASM);

    let response = common::request_on(app, "POST", "/result", Some(result_request(&process))).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/png");
    // No extent named: the preview frames the granule at its own scale.
    assert_eq!(response.headers()["x-swath-preview-tile"], "10/390/212");
    let header: Value =
        serde_json::from_str(response.headers()["x-swath-trace"].to_str().expect("ASCII"))
            .expect("JSON");
    let fuel = header["udf_fuel_used"]
        .as_u64()
        .expect("the UDF preview meters fuel");
    assert!(fuel > 0);
    let preview = common::body_bytes(response).await;

    // A preview persists nothing — no service, no module in the store.
    let services = common::body_json(common::request_on(app, "GET", "/services", None).await).await;
    assert_eq!(services["services"], json!([]));
    let code_hash = swath_core::udf::code_hash(NDVI_WASM);
    assert_eq!(
        swath_core::udf::ModuleStore::get(&udf_app.store, &code_hash)
            .await
            .expect("store answers"),
        None,
        "preview must not persist the module"
    );

    let service = json!({ "type": "xyz", "process": process });
    let response = common::request_on(app, "POST", "/services", Some(service)).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let id = response.headers()["openeo-identifier"]
        .to_str()
        .expect("identifier")
        .to_owned();
    let response = common::request_on(
        app,
        "GET",
        &format!("/tilesets/{id}/tiles/10/390/212"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let published_header: Value =
        serde_json::from_str(response.headers()["x-swath-trace"].to_str().expect("ASCII"))
            .expect("JSON");
    assert_eq!(
        published_header["udf_fuel_used"],
        json!(fuel),
        "fuel reproduces"
    );
    let published = common::body_bytes(response).await;
    assert_eq!(
        preview, published,
        "the UDF preview must be byte-identical to the published-service tile"
    );
}

/// A module's runtime failure on the preview is the author's to fix, in
/// the registry's vocabulary, always a 400 and never a 500: running out
/// of the per-tile fuel budget (or its wall-clock backstop) is a graph
/// too heavy for the bound — `ProcessGraphComplexity`, in plain words;
/// trapping or answering malformed output is a bad `udf` parameter —
/// `ProcessParameterInvalid` carrying the executor's diagnosis. Every
/// body is schema-valid, and nothing is published by the attempt.
#[tokio::test]
async fn udf_runtime_failures_preview_as_user_fixable_registry_errors() {
    let udf_app = common::openeo_app_with_udf(NoFetch);
    let app = &udf_app.app;
    let error_schema = common::openeo_schema("/components/schemas/error");

    let cases: [(&str, Vec<u8>, &str, &[&str]); 3] = [
        (
            "fuel bomb",
            wasm::fuel_bomb(),
            "ProcessGraphComplexity",
            &[
                "UDF exceeded the per-tile fuel budget",
                "simplify or narrow",
            ],
        ),
        (
            "trap",
            wasm::trapper(),
            "ProcessParameterInvalid",
            &["parameter 'udf' in process 'run_udf'", "UDF trapped"],
        ),
        (
            "malformed output",
            wasm::malformed_output(),
            "ProcessParameterInvalid",
            &[
                "parameter 'udf' in process 'run_udf'",
                "malformed UDF response",
            ],
        ),
    ];
    for (label, module, code, phrases) in cases {
        let response = common::request_on(
            app,
            "POST",
            "/result",
            Some(result_request(&udf_process(&module))),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{label}");
        let error = common::body_json(response).await;
        common::assert_openeo_valid(&error_schema, label, &error);
        assert_eq!(error["code"], code, "{label}: {error}");
        let message = error["message"].as_str().expect("message");
        for phrase in phrases {
            assert!(
                message.contains(phrase),
                "{label}: the diagnosis must say `{phrase}`, got: {message}"
            );
        }
    }

    let services = common::body_json(common::request_on(app, "GET", "/services", None).await).await;
    assert_eq!(
        services["services"],
        json!([]),
        "failed previews publish nothing"
    );
}

/// The operator's byte ceiling layers UNDER the preview's own (#272,
/// ADR 0014): a `[budget] max-estimated-live-bytes` tighter than the
/// preview ceiling refuses the preview; a looser one cannot widen it.
#[tokio::test]
async fn operator_byte_ceiling_narrows_previews_and_never_widens_them() {
    use swath_core::planner::Budget;

    let extent = json!({ "west": -105.45, "south": 39.26, "east": -105.44, "north": 39.27 });
    let refused = |app: axum::Router, label: &'static str| {
        let request = result_request(&ndvi_process_with_extent(&extent));
        async move {
            let response = common::request_on(&app, "POST", "/result", Some(request)).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{label}");
            let error = common::body_json(response).await;
            assert_eq!(error["code"], "ProcessGraphComplexity", "{label}");
        }
    };

    // Tighter than the (default) preview ceiling: the operator's cap binds.
    let (app, _) = common::openeo_app_with_budget(
        None,
        Budget {
            max_estimated_live_bytes: Some(1),
            ..Budget::default()
        },
    );
    refused(app, "operator cap under the preview ceiling").await;

    // Looser than a 1-byte preview ceiling: the preview's own bound holds.
    let (app, _) = common::openeo_app_with_budget(
        Some(1),
        Budget {
            max_estimated_live_bytes: Some(u64::MAX),
            ..Budget::default()
        },
    );
    refused(app, "operator cap above the preview ceiling").await;
}
