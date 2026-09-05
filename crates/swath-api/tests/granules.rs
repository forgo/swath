// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Granule browsing tests (issue #107): the typed page shape (snapshot-
//! pinned over the committed fixtures), bbox/datetime filter correctness,
//! limit/offset pagination, the uniform RFC 7807 error taxonomy (404
//! unknown dataset, 400 malformed parameters), and the no-host-paths
//! leakage check.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use swath_api::{GranulesState, granules_router};
use swath_core::catalog::{Bbox, DatasetId, Datetime, Granule, GranuleAsset, GranuleId};

use common::MemoryCatalog;

/// The granules router over a fresh in-memory catalog seeded with the
/// HLS fixture dataset and `granules`.
fn app(granules: Vec<Granule>) -> Router {
    let catalog = MemoryCatalog::default();
    catalog.seed(common::hls_catalog_dataset(), granules);
    granules_router(Arc::new(GranulesState::new(catalog, common::BASE_URL)))
}

/// A synthetic granule of the fixture dataset: distinct footprint and
/// acquisition time per id, one raster asset per band.
fn granule(id: &str, bbox: [f64; 4], datetime: &str) -> Granule {
    Granule {
        id: GranuleId::new(id),
        dataset: DatasetId::new("hls-s30"),
        bbox: Bbox::from_array(bbox),
        datetime: Datetime::new(datetime).unwrap(),
        assets: [
            (
                "b04".to_owned(),
                GranuleAsset::raster(format!("{id}-b04.tif")),
            ),
            (
                "b03".to_owned(),
                GranuleAsset::raster(format!("{id}-b03.tif")),
            ),
        ]
        .into(),
        ingested_at: None,
        properties: BTreeMap::new(),
    }
}

async fn get_json(app: &Router, path: &str) -> (StatusCode, serde_json::Value) {
    let response = common::request_on(app, "GET", path, None).await;
    let status = response.status();
    (status, common::body_json(response).await)
}

fn ids(body: &serde_json::Value) -> Vec<&str> {
    body["granules"]
        .as_array()
        .expect("granules is an array")
        .iter()
        .map(|g| g["id"].as_str().expect("id is a string"))
        .collect()
}

// --- The page shape ---

/// An existing dataset with nothing ingested is an empty 200 page, not
/// an error (mirrors the tileset resolution semantics: identity exists,
/// content doesn't yet).
#[tokio::test]
async fn empty_dataset_is_an_empty_page() {
    let app = app(Vec::new());
    let (status, body) = get_json(&app, "/datasets/hls-s30/granules").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["granules"], serde_json::json!([]));
    assert_eq!(body["numberMatched"], 0);
    assert_eq!(body["numberReturned"], 0);
    // A self link, no next.
    let rels: Vec<&str> = body["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["rel"].as_str().unwrap())
        .collect();
    assert_eq!(rels, ["self"]);
}

/// The fixtures-dataset response, pinned whole: this JSON is the
/// contract the UI consumes.
#[tokio::test]
async fn fixtures_response_snapshot() {
    let app = app(vec![common::hls_catalog_granule()]);
    let (status, body) = get_json(&app, "/datasets/hls-s30/granules").await;
    assert_eq!(status, StatusCode::OK);
    insta::assert_json_snapshot!("granules_fixtures", body);
}

/// Leakage check (issue #107 review box): the response carries store
/// keys, never absolute serving-host paths — even though the fixture
/// store root is an absolute local directory.
#[tokio::test]
async fn response_contains_no_absolute_host_paths() {
    let app = app(vec![common::hls_catalog_granule()]);
    let (status, body) = get_json(&app, "/datasets/hls-s30/granules").await;
    assert_eq!(status, StatusCode::OK);
    let text = serde_json::to_string(&body).unwrap();
    let fixtures = common::fixtures_dir()
        .canonicalize()
        .expect("fixture dir exists");
    assert!(
        !text.contains(&fixtures.display().to_string()),
        "response leaks the store root: {text}"
    );
    for item in body["granules"].as_array().unwrap() {
        for (band, asset) in item["assets"].as_object().unwrap() {
            let href = asset["href"].as_str().unwrap();
            assert!(
                !href.starts_with('/') && !href.contains(":\\"),
                "asset `{band}` href `{href}` is an absolute path"
            );
        }
    }
}

// --- Filters ---

/// Bbox filtering against fixtures: only granules whose footprint
/// intersects the query box appear (edges inclusive).
#[tokio::test]
async fn bbox_filter_selects_intersecting_footprints() {
    let app = app(vec![
        granule(
            "g-west",
            [-106.0, 39.0, -105.5, 39.5],
            "2024-06-01T00:00:00Z",
        ),
        granule(
            "g-east",
            [-105.4, 39.0, -104.9, 39.5],
            "2024-06-02T00:00:00Z",
        ),
        granule(
            "g-north",
            [-106.0, 40.0, -105.5, 40.5],
            "2024-06-03T00:00:00Z",
        ),
    ]);

    // A box over the western footprint only.
    let (status, body) = get_json(
        &app,
        "/datasets/hls-s30/granules?bbox=-105.9,39.1,-105.6,39.4",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&body), ["g-west"]);
    assert_eq!(body["numberMatched"], 1);

    // A box spanning west and east, but south of the northern granule.
    let (_, body) = get_json(
        &app,
        "/datasets/hls-s30/granules?bbox=-106.0,39.0,-105.0,39.5",
    )
    .await;
    assert_eq!(ids(&body), ["g-east", "g-west"], "newest first");

    // A box touching nothing.
    let (_, body) = get_json(&app, "/datasets/hls-s30/granules?bbox=0,0,1,1").await;
    assert_eq!(body["numberMatched"], 0);
}

/// Datetime filtering: inclusive bounds, open-ended forms.
#[tokio::test]
async fn datetime_filter_is_inclusive_and_open_endable() {
    let app = app(vec![
        granule("g-1", [-106.0, 39.0, -105.5, 39.5], "2024-06-01T00:00:00Z"),
        granule("g-2", [-106.0, 39.0, -105.5, 39.5], "2024-06-02T00:00:00Z"),
        granule("g-3", [-106.0, 39.0, -105.5, 39.5], "2024-06-03T00:00:00Z"),
    ]);
    let (_, body) = get_json(
        &app,
        "/datasets/hls-s30/granules?datetime=2024-06-01T00:00:00Z/2024-06-02T00:00:00Z",
    )
    .await;
    assert_eq!(ids(&body), ["g-2", "g-1"]);
    let (_, body) = get_json(
        &app,
        "/datasets/hls-s30/granules?datetime=../2024-06-01T00:00:00Z",
    )
    .await;
    assert_eq!(ids(&body), ["g-1"]);
    let (_, body) = get_json(
        &app,
        "/datasets/hls-s30/granules?datetime=2024-06-03T00:00:00Z/..",
    )
    .await;
    assert_eq!(ids(&body), ["g-3"]);
    let (_, body) = get_json(
        &app,
        "/datasets/hls-s30/granules?datetime=2024-06-02T00:00:00Z",
    )
    .await;
    assert_eq!(ids(&body), ["g-2"]);
}

// --- Pagination ---

/// limit/offset walk the newest-first order without overlap, `next`
/// appears exactly while more remain, and the counts stay honest.
#[tokio::test]
async fn pagination_walks_the_total_order() {
    let app = app((1..=5)
        .map(|i| {
            granule(
                &format!("g-{i}"),
                [-106.0, 39.0, -105.5, 39.5],
                &format!("2024-06-0{i}T00:00:00Z"),
            )
        })
        .collect());

    let (_, page1) = get_json(&app, "/datasets/hls-s30/granules?limit=2").await;
    assert_eq!(ids(&page1), ["g-5", "g-4"]);
    assert_eq!(page1["numberMatched"], 5);
    assert_eq!(page1["numberReturned"], 2);
    let next = page1["links"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["rel"] == "next")
        .expect("a next link");
    assert_eq!(
        next["href"],
        "http://localhost/datasets/hls-s30/granules?limit=2&offset=2"
    );

    let (_, page2) = get_json(&app, "/datasets/hls-s30/granules?limit=2&offset=2").await;
    assert_eq!(ids(&page2), ["g-3", "g-2"]);
    let (_, page3) = get_json(&app, "/datasets/hls-s30/granules?limit=2&offset=4").await;
    assert_eq!(ids(&page3), ["g-1"]);
    assert!(
        !page3["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["rel"] == "next"),
        "the last page has no next link"
    );

    // Offset past the end: an empty page, honestly counted.
    let (status, past) = get_json(&app, "/datasets/hls-s30/granules?offset=99").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(past["numberMatched"], 5);
    assert_eq!(past["numberReturned"], 0);
}

// --- The error taxonomy (uniform RFC 7807, as the tiles routes) ---

/// An unknown dataset addresses a resource that does not exist: 404 with
/// a schema-valid exception body.
#[tokio::test]
async fn unknown_dataset_is_a_schema_valid_404_exception() {
    let app = app(Vec::new());
    let (status, exception) = get_json(&app, "/datasets/no-such/granules").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    common::assert_valid("common/exception.json", &exception);
    assert_eq!(exception["status"], 404);
}

/// Malformed parameters are 400s with schema-valid exception bodies that
/// name the offending parameter.
#[tokio::test]
async fn malformed_parameters_are_schema_valid_400_exceptions() {
    let app = app(vec![common::hls_catalog_granule()]);
    for (query, names) in [
        ("bbox=1,2,3", "bbox"),
        ("bbox=a,b,c,d", "bbox"),
        ("bbox=1,2,3,4,5", "bbox"),
        ("bbox=0,10,1,-10", "bbox"),
        ("datetime=yesterday", "datetime"),
        ("limit=0", "limit"),
        ("limit=x", "limit"),
        ("offset=-1", "offset"),
    ] {
        let (status, exception) =
            get_json(&app, &format!("/datasets/hls-s30/granules?{query}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "?{query}");
        common::assert_valid("common/exception.json", &exception);
        assert_eq!(exception["status"], 400, "?{query}");
        assert!(
            exception["detail"].as_str().unwrap().contains(names),
            "?{query} detail names `{names}`: {exception}"
        );
    }
}

/// Foreign STAC properties reach the granule row, verbatim (#408) — and a
/// granule without them serves exactly the bytes it did before the field
/// existed, so the shape change is additive.
#[tokio::test]
async fn properties_are_served_on_the_row_and_omitted_when_empty() {
    let mut carried = granule(
        "g-with",
        [-106.1, 39.2, -105.9, 39.4],
        "2024-06-06T17:54:00Z",
    );
    carried.properties = BTreeMap::from([
        ("eo:cloud_cover".to_owned(), serde_json::json!(12.5)),
        ("nested".to_owned(), serde_json::json!({ "a": [1, 2, 3] })),
    ]);
    let bare = granule(
        "g-bare",
        [-106.1, 39.2, -105.9, 39.4],
        "2024-06-05T17:54:00Z",
    );

    let app = app(vec![carried, bare]);
    let (status, body) = get_json(&app, "/datasets/hls-s30/granules").await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["granules"].as_array().expect("a granule array");

    let with = rows
        .iter()
        .find(|row| row["id"] == "g-with")
        .expect("the carried granule");
    assert_eq!(
        with["properties"]["eo:cloud_cover"],
        serde_json::json!(12.5)
    );
    assert_eq!(
        with["properties"]["nested"],
        serde_json::json!({ "a": [1, 2, 3] })
    );

    // Omitted, not `{}`: a client that never knew about the field sees the
    // same document it always did.
    let without = rows
        .iter()
        .find(|row| row["id"] == "g-bare")
        .expect("the bare granule");
    assert!(without.get("properties").is_none());
}
