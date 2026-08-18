// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The dataset-creation surface (#196): register a dataset and a granule
//! by API — headers validated through the serving source stack, extents
//! derived from what registered — then author an NDVI layer through the
//! openEO services surface and serve a tile from it: the full
//! register → author → serve loop, in process, over the committed
//! fixtures. Plus the refusal taxonomy: RFC 7807 on bad assets, unknown
//! bands, unknown datasets, ambiguous bodies, and duplicate ids.

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
use swath_api::{
    ApiState, CatalogLayers, DatasetsState, OpenEoState, datasets_router, openeo_router, router,
};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;

use common::{BASE_URL, MemoryCatalog};
use swath_core::catalog::Catalog as _;

/// The catalog-mode app with the openEO **and** dataset-creation surfaces
/// merged — the `swath serve` catalog wiring — over an EMPTY in-memory
/// catalog and the committed fixtures.
fn register_app() -> (Router, MemoryCatalog) {
    let catalog = MemoryCatalog::default();
    let provider = CatalogLayers::new(catalog.clone(), Vec::new());
    let store: Arc<dyn object_store::ObjectStore> = Arc::new(
        LocalFileSystem::new_with_prefix(common::fixtures_dir()).expect("fixture dir exists"),
    );
    let state = ApiState::new(
        provider.clone(),
        CogSource::new(Arc::clone(&store)),
        Proj4rsReproject,
        BASE_URL,
    )
    .with_openeo();
    let openeo_state = OpenEoState::new(
        provider.clone(),
        CogSource::new(Arc::clone(&store)),
        Proj4rsReproject,
        BASE_URL,
    );
    let datasets_state = DatasetsState::new(provider, CogSource::new(store), Proj4rsReproject);
    let app = router(Arc::new(state))
        .merge(openeo_router(Arc::new(openeo_state)))
        .merge(datasets_router(Arc::new(datasets_state)));
    (app, catalog)
}

fn dataset_body() -> Value {
    json!({
        "id": "api-hls",
        "title": "HLS, registered by API",
        "description": "Registered through POST /datasets (#196).",
        "bands": ["b04", "b8a"],
    })
}

/// The committed fixture COGs as a direct-form granule; no bbox — it must
/// be derived from the asset header.
fn granule_body() -> Value {
    json!({
        "id": "hlss30-t13sdd-2024158",
        "datetime": "2024-06-06T17:54:00Z",
        "assets": {
            "b04": "hlss30-t13sdd-2024158-b04.tif",
            "b8a": "hlss30-t13sdd-2024158-b8a.tif",
        },
    })
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one linear scenario: register, refuse-duplicate, register \
              granule, discover, author, serve, persistence"
)]
async fn register_author_serve_loop() {
    let (app, catalog) = register_app();

    // Register the dataset…
    let response = common::request_on(&app, "POST", "/datasets", Some(dataset_body())).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    // …exactly once (register, don't manage).
    let response = common::request_on(&app, "POST", "/datasets", Some(dataset_body())).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    // Register the granule; the asset headers validate through the same
    // COG reader serving uses, and the bbox derives from them.
    let response = common::request_on(
        &app,
        "POST",
        "/datasets/api-hls/granules",
        Some(granule_body()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    // The collection document shows the DERIVED extents: the fixture
    // window's WGS84 footprint (~ -105.54..-105.35, 39.19..39.34) and the
    // granule's acquisition instant closing both interval ends.
    let response = common::request_on(&app, "GET", "/collections/api-hls", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let doc = common::body_json(response).await;
    let bbox = doc["extent"]["spatial"]["bbox"][0]
        .as_array()
        .expect("spatial extent")
        .iter()
        .map(|v| v.as_f64().expect("bbox number"))
        .collect::<Vec<_>>();
    assert!(
        (bbox[0] - -105.54).abs() < 0.02 && (bbox[3] - 39.34).abs() < 0.02,
        "derived bbox from the asset header, got {bbox:?}"
    );
    assert_eq!(
        doc["extent"]["temporal"]["interval"][0],
        json!(["2024-06-06T17:54:00Z", "2024-06-06T17:54:00Z"]),
        "derived temporal interval from the registered granule"
    );

    // Author an NDVI layer on the registered dataset via the EXISTING
    // openEO services surface — register-then-author is one flow.
    let service = json!({
        "type": "xyz",
        "title": "NDVI (API-registered dataset)",
        "process": { "process_graph": {
            "load": { "process_id": "load_collection", "arguments": {
                "id": "api-hls", "spatial_extent": null, "temporal_extent": null,
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
            }, "result": true },
        }},
    });
    let response = common::request_on(&app, "POST", "/services", Some(service)).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let sid = response.headers()["openeo-identifier"]
        .to_str()
        .expect("identifier")
        .to_owned();

    // …and serve a tile from it: the registered data, through the engine.
    let response = common::request_on(
        &app,
        "GET",
        &format!("/tilesets/{sid}/tiles/12/1561/848"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "registered data serves");
    assert!(
        response.headers().contains_key("x-swath-trace"),
        "the served tile is traced"
    );

    // The dataset persisted with its authored layer (restart material).
    let persisted = catalog
        .get_dataset(&swath_core::catalog::DatasetId::new("api-hls"))
        .await
        .expect("catalog reads")
        .expect("dataset persisted");
    assert!(
        persisted.layers.iter().any(|l| l.process.is_some()),
        "the authored service persisted on the dataset"
    );
}

#[tokio::test]
async fn refusal_taxonomy_is_rfc7807() {
    let (app, _catalog) = register_app();
    common::request_on(&app, "POST", "/datasets", Some(dataset_body())).await;

    // A nonexistent asset: 400, problem details naming the asset.
    let bad = json!({
        "id": "bad", "datetime": "2024-06-06T17:54:00Z",
        "assets": { "b04": "no-such-file.tif" },
    });
    let response = common::request_on(&app, "POST", "/datasets/api-hls/granules", Some(bad)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let problem = common::body_json(response).await;
    assert!(
        problem["detail"]
            .as_str()
            .expect("detail")
            .contains("no-such-file.tif"),
        "problem details name the failing asset: {problem}"
    );

    // An undeclared band: refused at the door.
    let bad = json!({
        "id": "bad", "datetime": "2024-06-06T17:54:00Z",
        "assets": { "fmask": "hlss30-t13sdd-2024158-fmask.tif" },
    });
    let response = common::request_on(&app, "POST", "/datasets/api-hls/granules", Some(bad)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // An unregistered dataset: 404.
    let response = common::request_on(
        &app,
        "POST",
        "/datasets/nope/granules",
        Some(granule_body()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Ambiguous body (both forms): 400.
    let mut both = granule_body();
    both["stac_item"] = json!({"type": "Feature"});
    let response = common::request_on(&app, "POST", "/datasets/api-hls/granules", Some(both)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // A non-URL-safe dataset id: 400.
    let response = common::request_on(
        &app,
        "POST",
        "/datasets",
        Some(json!({"id": "no/slashes", "title": "x", "bands": ["b04"]})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn inline_stac_item_registers_and_collection_must_match() {
    let (app, _catalog) = register_app();
    common::request_on(&app, "POST", "/datasets", Some(dataset_body())).await;

    // A STAC Item as a pipeline would hand it over — the #30 converter's
    // own output shape (built from the granule the direct form registers).
    let granule = swath_core::catalog::Granule {
        id: swath_core::catalog::GranuleId::new("stac-registered"),
        dataset: swath_core::catalog::DatasetId::new("api-hls"),
        bbox: swath_core::catalog::Bbox {
            west: -105.537,
            south: 39.1954,
            east: -105.3581,
            north: 39.3345,
        },
        datetime: swath_core::catalog::Datetime::new("2024-06-07T17:54:00Z").unwrap(),
        assets: [(
            "b04".to_owned(),
            swath_core::catalog::GranuleAsset::raster("hlss30-t13sdd-2024158-b04.tif"),
        )]
        .into(),
        ingested_at: None,
    };
    let item = swath_core::catalog::stac::granule_to_stac_item(&granule);
    let response = common::request_on(
        &app,
        "POST",
        "/datasets/api-hls/granules",
        Some(json!({ "stac_item": item })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    // The same item under the WRONG dataset path: refused.
    let item = swath_core::catalog::stac::granule_to_stac_item(&granule);
    let response = common::request_on(
        &app,
        "POST",
        "/datasets/other/granules",
        Some(json!({ "stac_item": item })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
