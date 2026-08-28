// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Two-source serving (ADR 0022, issue #296): a published `merge_cubes`
//! change layer renders one tile from two granules — one per branch,
//! each resolved within its own window — and the oracle's two-date
//! `compose` golden over the Park Fire pair pins the pixels. The cache
//! keys under the ordered granule pair (a new granule on either branch
//! is a new version), the trace carries one temporal record per branch,
//! a `datetime=` that empties a branch is a 404 naming it, and branches
//! in different CRSs are refused by the tiler's `MixedCrs`.

#[allow(
    dead_code,
    reason = "shared between the API test targets; not every helper is used in each"
)]
mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{Value, json};
use swath_api::TraceExtension;
use swath_core::catalog::{
    Bbox, Catalog as _, Dataset, DatasetId, Datetime, Extent, Granule, GranuleAsset, GranuleId,
    TimeRange,
};
use swath_core::trace::{Strategy, TemporalRule, Trace};
use swath_testkit::{DiffPolicy, diff, load_png};

/// The six Park Fire acquisitions (fixture README).
const FIRE_DAYS: [(&str, &str); 6] = [
    ("2024159", "2024-06-07T19:03:00Z"),
    ("2024204", "2024-07-22T19:03:00Z"),
    ("2024229", "2024-08-16T19:03:00Z"),
    ("2024249", "2024-09-05T19:03:00Z"),
    ("2024274", "2024-09-30T19:03:00Z"),
    ("2024289", "2024-10-15T19:03:00Z"),
];
/// The July (pre-fire) and August (fresh burn scar) windows, left-closed.
const JULY: [&str; 2] = ["2024-07-01T00:00:00Z", "2024-08-01T00:00:00Z"];
const AUGUST: [&str; 2] = ["2024-08-01T00:00:00Z", "2024-09-01T00:00:00Z"];
const PRE: &str = "hlss30-t10tfk-2024204";
const POST: &str = "hlss30-t10tfk-2024229";
const TILE: &str = "tiles/13/3100/1326";

fn fire_bbox() -> Bbox {
    Bbox {
        west: -121.7388,
        south: 39.9866,
        east: -121.6474,
        north: 40.0549,
    }
}

fn fire_dataset(id: &str) -> Dataset {
    Dataset {
        id: DatasetId::new(id),
        title: "HLS S30 Park Fire series".to_owned(),
        description: "Six T10TFK acquisitions across the 2024 Park Fire.".to_owned(),
        license: "CC0-1.0".to_owned(),
        extent: Extent {
            bbox: fire_bbox(),
            interval: TimeRange {
                start: Some(Datetime::new("2024-06-07T19:03:00Z").unwrap()),
                end: Some(Datetime::new("2024-10-15T19:03:00Z").unwrap()),
            },
        },
        bands: ["b04", "b8a"].map(str::to_owned).into_iter().collect(),
        layers: Vec::new(),
    }
}

/// A granule of `dataset` whose assets are the fixture files of `tile`
/// (`t10tfk` or `t13sdd`) at `day`.
fn granule(dataset: &str, id: &str, tile: &str, day: &str, datetime: &str) -> Granule {
    let asset = |band: &str| GranuleAsset::raster(format!("hlss30-{tile}-{day}-{band}.tif"));
    Granule {
        id: GranuleId::new(id),
        dataset: DatasetId::new(dataset),
        bbox: fire_bbox(),
        datetime: Datetime::new(datetime).unwrap(),
        assets: [
            ("b04".to_owned(), asset("b04")),
            ("b8a".to_owned(), asset("b8a")),
        ]
        .into(),
        ingested_at: Some(Datetime::new("2024-11-01T00:00:00Z").unwrap()),
    }
}

fn fire_granules() -> Vec<Granule> {
    FIRE_DAYS
        .iter()
        .map(|(day, datetime)| {
            granule(
                "park-fire",
                &format!("hlss30-t10tfk-{day}"),
                "t10tfk",
                day,
                datetime,
            )
        })
        .collect()
}

/// The change service: `NDVI(after) − NDVI(before)`, each branch its own
/// `load_collection` with a window, joined by a `subtract` resolver and
/// scaled −1..1 — the committed `change-detection.json` shape over the
/// fire collection.
fn change_service(collection: &str, after: [&str; 2], before: [&str; 2]) -> Value {
    let load = |extent: [&str; 2]| {
        json!({ "process_id": "load_collection", "arguments": {
            "id": collection, "spatial_extent": null,
            "temporal_extent": extent, "bands": ["b8a", "b04"],
        }})
    };
    let ndvi = |from: &str| {
        json!({ "process_id": "ndvi", "arguments": {
            "data": { "from_node": from }, "nir": "b8a", "red": "b04",
        }})
    };
    json!({
        "type": "xyz",
        "title": "Fire change",
        "process": { "process_graph": {
            "before": load(before),
            "after": load(after),
            "ndvi_before": ndvi("before"),
            "ndvi_after": ndvi("after"),
            "change": { "process_id": "merge_cubes", "arguments": {
                "cube1": { "from_node": "ndvi_after" },
                "cube2": { "from_node": "ndvi_before" },
                "overlap_resolver": { "process_graph": {
                    "diff": { "process_id": "subtract", "arguments": {
                        "x": { "from_parameter": "x" }, "y": { "from_parameter": "y" },
                    }, "result": true },
                }},
            }},
            "scale": { "process_id": "linear_scale_range", "arguments": {
                "x": { "from_node": "change" },
                "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 255,
            }},
            "save": { "process_id": "save_result", "arguments": {
                "data": { "from_node": "scale" }, "format": "png",
            }, "result": true },
        }},
    })
}

async fn publish(app: &axum::Router, request: Value) -> String {
    let response = common::request_on(app, "POST", "/services", Some(request)).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    response.headers()["openeo-identifier"]
        .to_str()
        .expect("identifier header")
        .to_owned()
}

async fn get_tile(
    app: &axum::Router,
    service: &str,
    query: &str,
) -> axum::http::Response<axum::body::Body> {
    common::request_on(
        app,
        "GET",
        &format!("/tilesets/{service}/{TILE}{query}"),
        None,
    )
    .await
}

/// The tile's bytes and its trace (200 expected).
async fn frame(app: &axum::Router, service: &str, query: &str) -> (Vec<u8>, Arc<Trace>) {
    let response = get_tile(app, service, query).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "GET {service}/{TILE}{query}"
    );
    let trace = Arc::clone(
        &response
            .extensions()
            .get::<TraceExtension>()
            .expect("trace extension attached")
            .0,
    );
    (common::body_bytes(response).await, trace)
}

/// `(node, granule id)` per branch of the trace's temporal decision.
fn branches(trace: &Trace) -> Vec<(String, String)> {
    trace
        .temporal
        .as_ref()
        .expect("catalog-backed render carries the temporal decision")
        .sources
        .iter()
        .map(|s| (s.node.clone(), s.granule_id.clone()))
        .collect()
}

fn pair(after: &str, before: &str) -> Vec<(String, String)> {
    vec![
        ("after".to_owned(), after.to_owned()),
        ("before".to_owned(), before.to_owned()),
    ]
}

// --- The pixels: the oracle's two-date golden ---------------------------

/// The served change tile is NDVI(August) − NDVI(July) per pixel — the
/// GDAL/rio-tiler `compose` golden over the two fixture dates — and its
/// trace names both granules, the `after` branch first.
#[tokio::test]
async fn a_change_layer_serves_the_two_date_oracle_golden() {
    let (app, _) = common::openeo_app_seeded(fire_dataset("park-fire"), fire_granules());
    let service = publish(&app, change_service("park-fire", AUGUST, JULY)).await;
    let (bytes, trace) = frame(&app, &service, "").await;
    let golden = load_png(
        &common::render_goldens_dir().join("fire-change-13-1326-3100-2024204-2024229.png"),
    )
    .expect("golden loads");
    let served = image::load_from_memory(&bytes)
        .expect("PNG decodes")
        .into_rgba8();
    let report = diff(&served, &golden).expect("dimensions match");
    assert!(
        report.passes(&DiffPolicy::default()),
        "served change tile fails the oracle policy: max |diff| {}",
        report.max_abs_channel_diff
    );
    let temporal = trace.temporal.as_ref().expect("temporal decision");
    assert_eq!(temporal.granule_id, POST, "the primary branch is cube1's");
    assert_eq!(temporal.rule, TemporalRule::Latest);
    assert_eq!(branches(&trace), pair(POST, PRE));
}

// --- `datetime=` composes with every branch ------------------------------

/// An instant inside both windows' reach resolves the same pair (and so
/// the same bytes) under latest-at-or-before; one that leaves a branch
/// without a granule is the tile route's 404, naming the branch.
#[tokio::test]
async fn datetime_intersects_every_branch() {
    let (app, _) = common::openeo_app_seeded(fire_dataset("park-fire"), fire_granules());
    let service = publish(&app, change_service("park-fire", AUGUST, JULY)).await;
    let (latest, _) = frame(&app, &service, "").await;
    let (dated, trace) = frame(&app, &service, "?datetime=2024-08-20T00:00:00Z").await;
    assert_eq!(dated, latest, "same resolved pair, same pixels");
    let temporal = trace.temporal.as_ref().expect("temporal decision");
    assert_eq!(temporal.rule, TemporalRule::LatestAtOrBefore);
    assert_eq!(temporal.requested.as_deref(), Some("2024-08-20T00:00:00Z"));
    assert_eq!(branches(&trace), pair(POST, PRE));

    // July 30th: the `after` branch's window (August) has nothing at or
    // before it, even though `before` would resolve.
    let response = get_tile(&app, &service, "?datetime=2024-07-30T00:00:00Z").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let problem: Value =
        serde_json::from_slice(&common::body_bytes(response).await).expect("problem JSON");
    let detail = problem["detail"].as_str().expect("detail");
    assert!(detail.contains("(branch `after`)"), "{detail}");
    assert!(
        detail.contains("no granule of dataset `park-fire`"),
        "{detail}"
    );
}

// --- Cache identity: the ordered granule pair ---------------------------

/// The cache keys under both branches' granules: a repeat is a hit, and a
/// granule newly ingested into the *second* branch's window is a new
/// version — the tile renders live again, from the new pair.
#[tokio::test]
async fn a_new_granule_on_the_second_branch_is_a_new_cache_version() {
    let (app, catalog) =
        common::openeo_app_seeded_cached(fire_dataset("park-fire"), fire_granules());
    let service = publish(&app, change_service("park-fire", AUGUST, JULY)).await;
    let (_, first) = frame(&app, &service, "").await;
    assert_eq!(first.decision, Strategy::Live);
    let (_, again) = frame(&app, &service, "").await;
    assert!(
        matches!(again.decision, Strategy::CacheHit { .. }),
        "same pair, same key: {:?}",
        again.decision
    );
    // A later July acquisition lands (same pixels as the 204 fixture, a
    // new granule id): the `before` branch now resolves to it.
    catalog
        .upsert_granules(&[granule(
            "park-fire",
            "hlss30-t10tfk-2024212",
            "t10tfk",
            "2024204",
            "2024-07-30T19:03:00Z",
        )])
        .await
        .expect("upsert");
    let (_, fresh) = frame(&app, &service, "").await;
    assert_eq!(
        fresh.decision,
        Strategy::Live,
        "a new second-branch granule is a new version"
    );
    assert_eq!(branches(&fresh), pair(POST, "hlss30-t10tfk-2024212"));
}

// --- Scope fence: cross-CRS branches -------------------------------------

/// Two branches whose granules sit in different UTM zones (T10TFK and
/// T13SDD fixtures) are refused by the tiler's `MixedCrs` — a 500 that
/// names both CRSs — never silently warped through one of them.
#[tokio::test]
async fn branches_in_different_crs_are_refused_as_mixed_crs() {
    let granules = vec![
        granule(
            "mixed",
            "t10-jul",
            "t10tfk",
            "2024204",
            "2024-07-22T19:03:00Z",
        ),
        granule(
            "mixed",
            "t13-jun",
            "t13sdd",
            "2024158",
            "2024-06-06T17:54:00Z",
        ),
    ];
    let (app, _) = common::openeo_app_seeded(fire_dataset("mixed"), granules);
    let service = publish(
        &app,
        change_service(
            "mixed",
            JULY,
            ["2024-06-01T00:00:00Z", "2024-07-01T00:00:00Z"],
        ),
    )
    .await;
    let response = get_tile(&app, &service, "").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let problem: Value =
        serde_json::from_slice(&common::body_bytes(response).await).expect("problem JSON");
    let detail = problem["detail"].as_str().expect("detail");
    assert!(
        detail.contains("mixed source CRSs are unsupported"),
        "{detail}"
    );
}
