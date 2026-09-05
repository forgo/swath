// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Facet discovery (#409): `GET /datasets/{datasetId}/facets`.
//!
//! The rule under test is an honesty rule — a facet exists here only
//! because a granule in scope carries the key, so a control rendered from
//! this response always has data behind it. What the suite pins: keys are
//! discovered from the items and never from a list, coverage keeps "no
//! value" distinguishable from "the value is zero", a mixed-type key
//! claims nothing beyond its coverage, the scope parameters narrow the
//! discovery, and a collection carrying nothing yields no facets at all.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use swath_api::{GranulesState, granules_router};
use swath_core::catalog::{Bbox, DatasetId, Datetime, Granule, GranuleAsset, GranuleId};

use common::MemoryCatalog;

fn app(granules: Vec<Granule>) -> Router {
    let catalog = MemoryCatalog::default();
    catalog.seed(common::hls_catalog_dataset(), granules);
    granules_router(Arc::new(GranulesState::new(catalog, common::BASE_URL)))
}

/// A granule of the fixture dataset with `properties` and nothing else
/// interesting.
fn granule(
    id: &str,
    bbox: [f64; 4],
    datetime: &str,
    properties: &[(&str, serde_json::Value)],
) -> Granule {
    Granule {
        id: GranuleId::new(id),
        dataset: DatasetId::new("hls-s30"),
        bbox: Bbox::from_array(bbox),
        datetime: Datetime::new(datetime).unwrap(),
        assets: [(
            "b04".to_owned(),
            GranuleAsset::raster(format!("{id}-b04.tif")),
        )]
        .into(),
        ingested_at: None,
        properties: properties
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    }
}

const WEST: [f64; 4] = [-106.0, 39.0, -105.0, 40.0];
const EAST: [f64; 4] = [-100.0, 39.0, -99.0, 40.0];

async fn get_json(app: &Router, path: &str) -> (StatusCode, serde_json::Value) {
    let response = common::request_on(app, "GET", path, None).await;
    let status = response.status();
    (status, common::body_json(response).await)
}

/// The facets of `body`, by key.
fn by_key(body: &serde_json::Value) -> BTreeMap<String, serde_json::Value> {
    body["facets"]
        .as_array()
        .expect("facets is an array")
        .iter()
        .map(|facet| {
            (
                facet["key"].as_str().expect("a key").to_owned(),
                facet.clone(),
            )
        })
        .collect()
}

/// A collection whose items carry nothing offers nothing. This is the
/// whole point: no key, no control — never a cloud slider that can only
/// ever do nothing.
#[tokio::test]
async fn a_collection_carrying_nothing_has_no_facets() {
    let app = app(vec![granule("g1", WEST, "2024-06-01T00:00:00Z", &[])]);
    let (status, body) = get_json(&app, "/datasets/hls-s30/facets").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 1);
    assert_eq!(body["facets"], serde_json::json!([]));
}

/// Keys come from the items. Numbers report a range, strings report their
/// distinct values with counts, booleans likewise — and each kind is
/// named, because the kind is the UI's only licence to render one control
/// rather than another.
#[tokio::test]
async fn keys_are_discovered_from_the_items_with_their_kinds() {
    let app = app(vec![
        granule(
            "g1",
            WEST,
            "2024-06-01T00:00:00Z",
            &[
                ("eo:cloud_cover", serde_json::json!(12.5)),
                ("platform", serde_json::json!("sentinel-2a")),
                ("swath:reprocessed", serde_json::json!(true)),
            ],
        ),
        granule(
            "g2",
            WEST,
            "2024-06-02T00:00:00Z",
            &[
                ("eo:cloud_cover", serde_json::json!(80.0)),
                ("platform", serde_json::json!("sentinel-2b")),
                ("swath:reprocessed", serde_json::json!(true)),
            ],
        ),
        granule(
            "g3",
            WEST,
            "2024-06-03T00:00:00Z",
            &[
                ("eo:cloud_cover", serde_json::json!(0.0)),
                ("platform", serde_json::json!("sentinel-2a")),
                ("swath:reprocessed", serde_json::json!(false)),
            ],
        ),
    ]);
    let (status, body) = get_json(&app, "/datasets/hls-s30/facets").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 3);
    let facets = by_key(&body);
    assert_eq!(
        facets.keys().collect::<Vec<_>>(),
        ["eo:cloud_cover", "platform", "swath:reprocessed"],
        "every key some granule carries, and no other"
    );

    let cloud = &facets["eo:cloud_cover"];
    assert_eq!(cloud["kind"], "number");
    assert_eq!(cloud["coverage"], 3);
    assert_eq!(cloud["min"], 0.0);
    assert_eq!(cloud["max"], 80.0);
    assert!(cloud.get("values").is_none(), "a range, not an enumeration");

    let platform = &facets["platform"];
    assert_eq!(platform["kind"], "string");
    assert_eq!(
        platform["values"],
        serde_json::json!([
            { "value": "sentinel-2a", "count": 2 },
            { "value": "sentinel-2b", "count": 1 },
        ]),
        "most common first"
    );
    assert!(platform.get("truncated").is_none(), "nothing left out");

    assert_eq!(facets["swath:reprocessed"]["kind"], "boolean");
    assert_eq!(
        facets["swath:reprocessed"]["values"],
        serde_json::json!([
            { "value": true, "count": 2 },
            { "value": false, "count": 1 },
        ])
    );
}

/// The acceptance criterion that matters most: coverage below `total`
/// says the key is absent from some granules, so "no value" never reads
/// as "the value is zero".
#[tokio::test]
async fn coverage_distinguishes_absent_from_zero() {
    let app = app(vec![
        granule(
            "g1",
            WEST,
            "2024-06-01T00:00:00Z",
            &[("eo:cloud_cover", serde_json::json!(0.0))],
        ),
        granule("g2", WEST, "2024-06-02T00:00:00Z", &[]),
        granule("g3", WEST, "2024-06-03T00:00:00Z", &[]),
    ]);
    let (_, body) = get_json(&app, "/datasets/hls-s30/facets").await;
    assert_eq!(body["total"], 3);
    let cloud = &by_key(&body)["eo:cloud_cover"];
    assert_eq!(cloud["coverage"], 1, "one granule carries the key");
    assert_eq!(cloud["min"], 0.0, "and its value is zero");
}

/// A key whose values are objects, or a mix of kinds, is not a filter.
/// It reports coverage and claims nothing else — no range to slide, no
/// values to pick.
#[tokio::test]
async fn a_mixed_or_structured_key_claims_only_its_coverage() {
    let app = app(vec![
        granule(
            "g1",
            WEST,
            "2024-06-01T00:00:00Z",
            &[
                ("mixed", serde_json::json!(3)),
                ("proj:transform", serde_json::json!({ "a": [1, 2] })),
            ],
        ),
        granule(
            "g2",
            WEST,
            "2024-06-02T00:00:00Z",
            &[("mixed", serde_json::json!("three"))],
        ),
    ]);
    let (_, body) = get_json(&app, "/datasets/hls-s30/facets").await;
    let facets = by_key(&body);
    for key in ["mixed", "proj:transform"] {
        let facet = &facets[key];
        assert_eq!(facet["kind"], "other", "{key}");
        assert!(facet.get("values").is_none(), "{key} enumerates nothing");
        assert!(facet.get("min").is_none(), "{key} has no range");
        assert!(facet.get("max").is_none(), "{key} has no range");
    }
    assert_eq!(facets["mixed"]["coverage"], 2);
    assert_eq!(facets["proj:transform"]["coverage"], 1);
}

/// `bbox` and `datetime` scope the discovery exactly as they scope the
/// granule page: the facets describe the set the user is looking at, so a
/// key that only exists outside the scope is not offered inside it.
#[tokio::test]
async fn the_scope_narrows_the_discovery() {
    let app = app(vec![
        granule(
            "west",
            WEST,
            "2024-06-01T00:00:00Z",
            &[("platform", serde_json::json!("sentinel-2a"))],
        ),
        granule(
            "east",
            EAST,
            "2024-07-01T00:00:00Z",
            &[("landsat:collection", serde_json::json!("c2"))],
        ),
    ]);

    let (_, all) = get_json(&app, "/datasets/hls-s30/facets").await;
    assert_eq!(all["total"], 2);
    assert_eq!(by_key(&all).len(), 2, "unscoped, both keys are offered");

    let (_, scoped) = get_json(&app, "/datasets/hls-s30/facets?bbox=-106,39,-105,40").await;
    assert_eq!(scoped["total"], 1);
    assert_eq!(
        by_key(&scoped).keys().collect::<Vec<_>>(),
        ["platform"],
        "the eastern granule's key is not offered in a western box"
    );

    let (_, windowed) = get_json(
        &app,
        "/datasets/hls-s30/facets?datetime=2024-07-01T00:00:00Z/2024-07-31T00:00:00Z",
    )
    .await;
    assert_eq!(
        by_key(&windowed).keys().collect::<Vec<_>>(),
        ["landsat:collection"]
    );
    // The self link echoes the scope, so the response says what it
    // describes.
    assert!(
        windowed["links"][0]["href"]
            .as_str()
            .expect("a self href")
            .contains("datetime=2024-07"),
        "the self link carries the scope: {}",
        windowed["links"][0]["href"]
    );
}

/// A key with more distinct values than a person can pick from says so
/// rather than pretending the prefix is the set.
#[tokio::test]
async fn too_many_values_are_truncated_and_admitted() {
    let granules: Vec<Granule> = (0..40)
        .map(|i| {
            granule(
                &format!("g{i:03}"),
                WEST,
                &format!("2024-06-01T{:02}:00:00Z", i % 24),
                &[("scene_id", serde_json::json!(format!("scene-{i:03}")))],
            )
        })
        .collect();
    let app = app(granules);
    let (_, body) = get_json(&app, "/datasets/hls-s30/facets").await;
    let scene = &by_key(&body)["scene_id"];
    assert_eq!(scene["coverage"], 40);
    assert_eq!(scene["truncated"], true);
    assert_eq!(
        scene["values"].as_array().expect("values").len(),
        25,
        "the cap, and the flag says the rest exist"
    );
}

/// The taxonomy is the granules route's: an unknown dataset is a resource
/// that does not exist, a malformed scope is a bad request.
#[tokio::test]
async fn the_error_taxonomy_matches_the_granules_route() {
    let app = app(Vec::new());
    let (status, _) = get_json(&app, "/datasets/nope/facets").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = get_json(&app, "/datasets/hls-s30/facets?bbox=1,2,3").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
