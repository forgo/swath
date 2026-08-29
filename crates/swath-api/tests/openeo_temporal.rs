// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Temporal conformance (issue #181, ADR 0015 frame selection): a graph's
//! `temporal_extent` / `filter_temporal` compile into the layer's
//! resolution window, and a windowed layer serves the granule *of its
//! window* — proven per timestamp against the GDAL/rio-tiler oracle
//! goldens over the Park Fire series (tests/fixtures/hlss30-t10tfk-*,
//! issue #179): pre-fire green, fresh burn scar, early post-fire are
//! visually and numerically distinct acquisitions, so a wrong granule
//! cannot pass. Diagnostics: a window excluding every granule is the
//! registry's `NotFound` at preview time; a window that can never select
//! anything is `ProcessParameterInvalid` at validation time.

mod common;

use swath_testsupport::fixtures::{FIRE_DAYS, park_fire};
use swath_testsupport::http::publish;

/// The served tile must match the committed oracle golden for `day`.
fn assert_matches_golden(tile: &[u8], day: &str) {
    let served = image::load_from_memory(tile)
        .expect("served PNG decodes")
        .into_rgba8();
    let golden = format!("ndvi-{day}-13-1326-3100.png");
    swath_testsupport::pdiff::assert_matches_golden(
        &golden,
        &served,
        &common::render_goldens_dir().join(&golden),
    );
}

use axum::http::StatusCode;
use serde_json::{Value, json};

/// The app over the full six-acquisition series.
fn fire_app() -> axum::Router {
    let (dataset, granules) = park_fire(&FIRE_DAYS);
    common::openeo_app_seeded(dataset, granules).0
}

/// The grayscale NDVI service request (the oracle golden's math:
/// `(b8a - b04) / (b8a + b04)` rescaled -1..1), with the load node's
/// temporal arguments spliced in by the caller.
fn ndvi_service(temporal_extent: &Value) -> Value {
    json!({
        "type": "xyz",
        "title": "Fire NDVI",
        "process": { "process_graph": {
            "load": { "process_id": "load_collection", "arguments": {
                "id": "park-fire", "spatial_extent": null,
                "temporal_extent": temporal_extent,
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
    })
}

/// Serves the z13 tile fully inside the fixture window from `service`.
async fn fire_tile(app: &axum::Router, service: &str) -> Vec<u8> {
    let path = format!("/tilesets/{service}/tiles/13/3100/1326");
    let response = common::request_on(app, "GET", &path, None).await;
    assert_eq!(response.status(), StatusCode::OK, "GET {path}");
    common::body_bytes(response).await
}

// --- The correctly dated granule, per timestamp -------------------------

/// A `temporal_extent` window serves the granule that was current in it —
/// oracle-pinned per timestamp: the same graph over three windows serves
/// three visually distinct acquisitions (pre-fire green, fresh burn scar,
/// early post-fire), each matching the golden of *its* date.
#[tokio::test]
async fn temporal_extent_serves_the_granule_of_its_window() {
    let app = fire_app();
    let mut tiles = Vec::new();
    for (extent, day) in [
        (json!(["2024-06-01", "2024-06-08"]), "2024159"),
        (json!(["2024-08-01", "2024-09-01"]), "2024229"),
        (json!(["2024-10-01", null]), "2024289"),
    ] {
        let id = publish(&app, ndvi_service(&extent)).await;
        let tile = fire_tile(&app, &id).await;
        assert_matches_golden(&tile, day);
        tiles.push(tile);
    }
    // The three frames really are three different acquisitions.
    assert_ne!(tiles[0], tiles[1], "pre-fire vs burn scar must differ");
    assert_ne!(tiles[1], tiles[2], "burn scar vs post-fire must differ");
}

/// An interval resolves at its **end** (ADR 0015): a window holding three
/// acquisitions serves the latest of them, not the first.
#[tokio::test]
async fn a_window_holding_many_granules_resolves_to_its_latest() {
    let app = fire_app();
    // 2024159, 2024204, and 2024229 all fall in [June, September).
    let id = publish(&app, ndvi_service(&json!([null, "2024-09-01"]))).await;
    assert_matches_golden(&fire_tile(&app, &id).await, "2024229");
}

/// An unwindowed graph still resolves to latest — the parameter is purely
/// additive (today's behavior, byte for byte).
#[tokio::test]
async fn an_open_window_still_serves_the_latest_granule() {
    let app = fire_app();
    let id = publish(&app, ndvi_service(&Value::Null)).await;
    assert_matches_golden(&fire_tile(&app, &id).await, "2024289");
}

// --- filter_temporal joins the served set -------------------------------

/// `filter_temporal` narrows exactly like `temporal_extent`: the same
/// window expressed as a filter node serves byte-identical pixels.
#[tokio::test]
async fn filter_temporal_narrows_the_window_like_temporal_extent() {
    let app = fire_app();
    let mut filtered = ndvi_service(&Value::Null);
    filtered["process"]["process_graph"]["filter"] = json!({
        "process_id": "filter_temporal",
        "arguments": {
            "data": { "from_node": "load" },
            "extent": ["2024-08-01", "2024-09-01"],
            "dimension": "t",
        },
    });
    filtered["process"]["process_graph"]["ndvi"]["arguments"]["data"] =
        json!({ "from_node": "filter" });
    let filtered_id = publish(&app, filtered).await;
    let extent_id = publish(&app, ndvi_service(&json!(["2024-08-01", "2024-09-01"]))).await;
    let filtered_tile = fire_tile(&app, &filtered_id).await;
    assert_eq!(filtered_tile, fire_tile(&app, &extent_id).await);
    assert_matches_golden(&filtered_tile, "2024229");
}

// --- Diagnostics (pinned registry codes) --------------------------------

/// POST /result with the given process graph body.
async fn preview(app: &axum::Router, request: Value) -> axum::http::Response<axum::body::Body> {
    common::request_on(
        app,
        "POST",
        "/result",
        Some(json!({ "process": request["process"] })),
    )
    .await
}

/// Asserts an openEO error of `code`/`status` whose message contains
/// every `needle`, schema-valid under the pinned spec.
async fn assert_openeo_error(
    response: axum::http::Response<axum::body::Body>,
    status: StatusCode,
    code: &str,
    needles: &[&str],
) {
    assert_eq!(response.status(), status);
    let error = common::body_json(response).await;
    let schema = common::openeo_schema("/components/schemas/error");
    common::assert_openeo_valid(&schema, "error", &error);
    assert_eq!(error["code"], json!(code));
    let message = error["message"].as_str().expect("message");
    for needle in needles {
        assert!(
            message.contains(needle),
            "message must contain `{needle}`: {message}"
        );
    }
}

/// A window excluding every ingested granule: the preview has nothing to
/// render — the registry's `NotFound`, naming the window.
#[tokio::test]
async fn a_window_excluding_all_granules_is_the_registrys_not_found() {
    let app = fire_app();
    let request = ndvi_service(&json!(["2023-01-01", "2023-06-01"]));
    assert_openeo_error(
        preview(&app, request).await,
        StatusCode::NOT_FOUND,
        "NotFound",
        &["acquisition datetime within", "2023-01-01T00:00:00Z"],
    )
    .await;
}

/// An interval that can never select anything (empty, or disjoint from
/// the window already applied) is rejected at validation time with
/// `ProcessParameterInvalid` — before any granule is consulted.
#[tokio::test]
async fn provably_empty_windows_are_process_parameter_invalid() {
    let app = fire_app();
    // Empty: the left-closed [t, t) contains no instant.
    let empty = ndvi_service(&json!(["2024-08-01", "2024-08-01"]));
    assert_openeo_error(
        preview(&app, empty.clone()).await,
        StatusCode::BAD_REQUEST,
        "ProcessParameterInvalid",
        &["empty temporal window"],
    )
    .await;
    // The same graph is rejected identically at publish time.
    let response = common::request_on(&app, "POST", "/services", Some(empty)).await;
    assert_openeo_error(
        response,
        StatusCode::BAD_REQUEST,
        "ProcessParameterInvalid",
        &["empty temporal window"],
    )
    .await;
    // Disjoint: a filter that cannot overlap the loaded window.
    let mut disjoint = ndvi_service(&json!(["2024-06-01", "2024-07-01"]));
    disjoint["process"]["process_graph"]["filter"] = json!({
        "process_id": "filter_temporal",
        "arguments": {
            "data": { "from_node": "load" },
            "extent": ["2024-08-01", "2024-09-01"],
        },
    });
    disjoint["process"]["process_graph"]["ndvi"]["arguments"]["data"] =
        json!({ "from_node": "filter" });
    assert_openeo_error(
        preview(&app, disjoint).await,
        StatusCode::BAD_REQUEST,
        "ProcessParameterInvalid",
        &["does not overlap"],
    )
    .await;
    // A non-temporal dimension: the spec's DimensionNotAvailable
    // exception, mapped onto the registry's ProcessParameterInvalid.
    let mut wrong_dim = ndvi_service(&Value::Null);
    wrong_dim["process"]["process_graph"]["filter"] = json!({
        "process_id": "filter_temporal",
        "arguments": {
            "data": { "from_node": "load" },
            "extent": ["2024-08-01", "2024-09-01"],
            "dimension": "bands",
        },
    });
    wrong_dim["process"]["process_graph"]["ndvi"]["arguments"]["data"] =
        json!({ "from_node": "filter" });
    assert_openeo_error(
        preview(&app, wrong_dim).await,
        StatusCode::BAD_REQUEST,
        "ProcessParameterInvalid",
        &["DimensionNotAvailable"],
    )
    .await;
}
