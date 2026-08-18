// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Read-only serving (#198): the write routes are **absent** (404, never
//! 403), the read surface and the ADR 0014 preview stay, and the
//! capabilities document says exactly what is mounted — asserted over the
//! same in-process wiring `swath serve --read-only` assembles.

#[allow(
    dead_code,
    reason = "this binary uses a subset of the shared test plumbing"
)]
mod common;

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use object_store::local::LocalFileSystem;
use serde_json::{Value, json};
use swath_api::{ApiState, CatalogLayers, OpenEoState, openeo_read_router, router};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;

use common::{BASE_URL, MemoryCatalog};

/// The catalog-mode app assembled READ-ONLY: the openEO read router only,
/// no dataset-creation router — `serve_catalog`'s `--read-only` branch.
fn read_only_app() -> Router {
    let catalog = MemoryCatalog::default();
    catalog.seed(
        common::hls_catalog_dataset(),
        vec![common::hls_catalog_granule()],
    );
    let provider = CatalogLayers::new(catalog, Vec::new());
    let store: Arc<dyn object_store::ObjectStore> = Arc::new(
        LocalFileSystem::new_with_prefix(common::fixtures_dir()).expect("fixture dir exists"),
    );
    let state = ApiState::new(
        provider.clone(),
        CogSource::new(Arc::clone(&store)),
        Proj4rsReproject,
        BASE_URL,
    )
    .with_openeo()
    .read_only();
    let openeo_state =
        OpenEoState::new(provider, CogSource::new(store), Proj4rsReproject, BASE_URL);
    router(Arc::new(state)).merge(openeo_read_router(Arc::new(openeo_state)))
}

#[tokio::test]
async fn write_routes_are_absent_not_403() {
    let app = read_only_app();

    // The write surface: absent means 404/405, never 403.
    for (method, path, body) in [
        ("POST", "/services", Some(json!({}))),
        ("DELETE", "/services/some-id", None),
        ("POST", "/datasets", Some(json!({}))),
        ("POST", "/datasets/hls-s30/granules", Some(json!({}))),
        ("PUT", "/uploads/scene.tif", Some(json!({}))),
    ] {
        let response = common::request_on(&app, method, path, body).await;
        assert!(
            response.status() == StatusCode::NOT_FOUND
                || response.status() == StatusCode::METHOD_NOT_ALLOWED,
            "{method} {path}: expected absent (404/405), got {}",
            response.status()
        );
    }

    // The read surface stays whole…
    for path in [
        "/collections",
        "/collections/hls-s30",
        "/processes",
        "/file_formats",
        "/service_types",
        "/services",
        "/.well-known/openeo",
    ] {
        let response = common::request_on(&app, "GET", path, None).await;
        assert_eq!(response.status(), StatusCode::OK, "GET {path}");
    }

    // …and POST /result REMAINS enabled (planner-budget-bounded by
    // design, ADR 0014 — the demo wow).
    let graph = json!({ "process": { "process_graph": {
        "load": { "process_id": "load_collection", "arguments": {
            "id": "hls-s30", "spatial_extent": null, "temporal_extent": null,
            "bands": ["b8a", "b04"] }},
        "ndvi": { "process_id": "ndvi", "arguments": {
            "data": { "from_node": "load" }, "nir": "b8a", "red": "b04" }},
        "save": { "process_id": "save_result", "arguments": {
            "data": { "from_node": "ndvi" }, "format": "png" }, "result": true },
    }}});
    let response = common::request_on(&app, "POST", "/result", Some(graph)).await;
    assert_eq!(response.status(), StatusCode::OK, "POST /result stays");
}

#[tokio::test]
async fn capabilities_document_reflects_what_is_mounted() {
    let app = read_only_app();
    let response = common::request_on(&app, "GET", "/", None).await;
    let doc = common::body_json(response).await;
    let endpoints = doc["endpoints"].as_array().expect("endpoints list");

    let methods_of = |path: &str| -> Vec<String> {
        endpoints
            .iter()
            .filter(|e| e["path"] == path)
            .flat_map(|e| {
                e["methods"]
                    .as_array()
                    .expect("methods")
                    .iter()
                    .map(|m| m.as_str().expect("method").to_owned())
            })
            .collect()
    };
    assert_eq!(
        methods_of("/services"),
        ["GET"],
        "services advertises listing only"
    );
    assert_eq!(methods_of("/services/{service_id}"), ["GET"]);
    assert_eq!(
        methods_of("/result"),
        ["POST"],
        "the preview stays declared"
    );
    // The dataset-creation surface (#196) and the upload route (#197):
    // POST-only /datasets disappears entirely, granule browsing keeps its
    // GET, and no upload route is claimed.
    assert!(
        methods_of("/datasets").is_empty(),
        "read-only advertises no dataset registration"
    );
    assert_eq!(methods_of("/datasets/{dataset_id}/granules"), ["GET"]);
    assert!(
        methods_of("/uploads/{filename}").is_empty(),
        "read-only advertises no upload route"
    );
    assert!(
        !doc["endpoints"].to_string().contains("DELETE"),
        "no write method advertised anywhere: {}",
        doc["endpoints"]
    );

    // The honest counterpart: the WRITABLE assembly advertises the full
    // surface (guards against the filter over-pruning).
    let writable: Value = {
        let response = common::request_on(&common::openeo_app().0, "GET", "/", None).await;
        common::body_json(response).await
    };
    assert!(
        writable["endpoints"].to_string().contains("DELETE"),
        "writable serving still advertises the write surface"
    );
    let registers = writable["endpoints"]
        .as_array()
        .expect("writable endpoints")
        .iter()
        .any(|e| e["path"] == "/datasets" && e["methods"] == json!(["POST"]));
    assert!(
        registers,
        "writable serving advertises dataset registration (#197's panel \
         gates on exactly this): {}",
        writable["endpoints"]
    );
}
