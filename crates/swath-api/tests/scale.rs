// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The scale fixture (#414). Seven committed rows cannot tell us whether
//! paging, counting and ordering hold, so this suite seeds a deterministic
//! series of synthetic granules over the **already committed** T10TFK COGs
//! and asserts the properties every scale claim in M16 rests on: the count
//! the server reports equals the rows a full walk yields, the newest-first
//! order survives page boundaries, `next` stops exactly at the end, and a
//! filter's count is the filter's count and not the dataset's.
//!
//! Cost: the fixture is rows in memory and no bytes on disk, so the whole
//! suite runs in well under a second and stays on the default path.

mod common;

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use swath_api::{GranulesState, granules_router};
use swath_testsupport::fixtures::{scale_dataset, scale_granules};

use common::MemoryCatalog;

/// How many rows the suite seeds: past any page size it asks for, and past
/// the 100-row default both web clients stop at, so a first-page-only bug
/// cannot pass.
const COUNT: usize = 250;

const DATASET: &str = "scale";

fn app() -> Router {
    let catalog = MemoryCatalog::default();
    catalog.seed(
        scale_dataset(DATASET, COUNT),
        scale_granules(DATASET, COUNT),
    );
    granules_router(Arc::new(GranulesState::new(catalog, common::BASE_URL)))
}

async fn get_json(app: &Router, path: &str) -> (StatusCode, serde_json::Value) {
    let response = common::request_on(app, "GET", path, None).await;
    let status = response.status();
    (status, common::body_json(response).await)
}

fn ids(body: &serde_json::Value) -> Vec<String> {
    body["granules"]
        .as_array()
        .expect("granules is an array")
        .iter()
        .map(|g| g["id"].as_str().expect("id is a string").to_owned())
        .collect()
}

fn next_link(body: &serde_json::Value) -> Option<String> {
    body["links"]
        .as_array()
        .expect("links is an array")
        .iter()
        .find(|link| link["rel"] == "next")
        .map(|link| link["href"].as_str().expect("href is a string").to_owned())
}

/// Walk every page of `path` by following `next`, returning the ids in the
/// order the walk saw them and the count each page reported.
async fn walk(app: &Router, path: &str) -> (Vec<String>, Vec<u64>) {
    let (mut seen, mut counts) = (Vec::new(), Vec::new());
    let mut next = Some(path.to_owned());
    while let Some(href) = next {
        // The links are absolute; the in-process helper wants the path.
        let relative = href.strip_prefix(common::BASE_URL).unwrap_or(&href);
        let (status, body) = get_json(app, relative).await;
        assert_eq!(status, StatusCode::OK, "page {relative}");
        counts.push(body["numberMatched"].as_u64().expect("numberMatched"));
        seen.extend(ids(&body));
        next = next_link(&body);
        assert!(seen.len() <= COUNT + 1, "the walk is not terminating");
    }
    (seen, counts)
}

/// The fixture is a pure function of its index: the same call twice is the
/// same bytes. Nothing in it reads a clock or a random number, which is
/// what lets every assertion below be an equality.
#[test]
fn the_fixture_is_deterministic() {
    let first = scale_granules(DATASET, COUNT);
    let second = scale_granules(DATASET, COUNT);
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    assert_eq!(first.len(), COUNT);
    // Distinct ids, distinct instants — a fixture that collided on either
    // would make the ordering assertions vacuous.
    let ids: std::collections::BTreeSet<_> = first.iter().map(|g| g.id.as_str()).collect();
    let times: std::collections::BTreeSet<_> = first.iter().map(|g| g.datetime.as_str()).collect();
    assert_eq!(ids.len(), COUNT);
    assert_eq!(times.len(), COUNT);
}

/// `numberMatched` is the whole set, on every page — not the page's own
/// length, and not the first page's view of the world.
#[tokio::test]
async fn the_count_is_the_full_set_on_every_page() {
    let app = app();
    let (seen, counts) = walk(&app, &format!("/datasets/{DATASET}/granules?limit=40")).await;
    assert_eq!(seen.len(), COUNT, "the walk yields every row");
    assert!(
        counts.iter().all(|&count| count == COUNT as u64),
        "every page reports the full count: {counts:?}"
    );
    assert_eq!(counts.len(), COUNT.div_ceil(40), "page count");
}

/// Paging is a partition: no row is seen twice, none is skipped, and the
/// newest-first order holds *across* the boundaries, not only inside a
/// page. This is the assertion seven rows could never make.
#[tokio::test]
async fn paging_partitions_the_set_in_newest_first_order() {
    let app = app();
    let (seen, _) = walk(&app, &format!("/datasets/{DATASET}/granules?limit=37")).await;
    let unique: std::collections::BTreeSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), seen.len(), "no row is served twice");

    // Ids are zero-padded and the series steps forward in time, so
    // newest-first is the ids descending.
    let mut expected = seen.clone();
    expected.sort_by(|a, b| b.cmp(a));
    assert_eq!(seen, expected, "newest first, across page boundaries");
    assert_eq!(seen.first().unwrap(), &format!("scale-{:05}", COUNT - 1));
    assert_eq!(seen.last().unwrap(), "scale-00000");
}

/// A limit that divides the set exactly must not hand out an empty last
/// page: `next` stops when the rows do.
#[tokio::test]
async fn next_stops_when_the_rows_do() {
    let app = app();
    let (_, body) = get_json(
        &app,
        &format!("/datasets/{DATASET}/granules?limit=50&offset=200"),
    )
    .await;
    assert_eq!(ids(&body).len(), 50);
    assert_eq!(next_link(&body), None, "the last exact page has no next");

    let (_, walked) = walk(&app, &format!("/datasets/{DATASET}/granules?limit=50")).await;
    assert_eq!(walked.len(), 5, "250 rows in pages of 50");
}

/// A filtered count is the filter's count. `numberMatched` under a
/// datetime window must describe the window, and paging that window must
/// yield exactly that many rows.
#[tokio::test]
async fn a_filtered_count_matches_a_filtered_walk() {
    let app = app();
    // The series steps six hours, so January (31 days) holds 124 rows.
    let window = "2024-01-01T00:00:00Z/2024-01-31T23:59:59Z";
    let path = format!("/datasets/{DATASET}/granules?datetime={window}&limit=30");
    let (seen, counts) = walk(&app, &path).await;
    assert_eq!(seen.len(), 124, "31 days at four a day");
    assert!(
        counts.iter().all(|&count| count == 124),
        "the count is the window's, not the dataset's: {counts:?}"
    );
}

/// The bbox filter narrows by footprint, and its count agrees with its
/// rows. The lattice puts sixteen granules in a column-step, so a box
/// around the origin cell selects a known few.
#[tokio::test]
async fn a_bbox_count_matches_its_rows() {
    let app = app();
    // The first footprint, grown by half a lattice step in each direction.
    let first = &scale_granules(DATASET, 1)[0].bbox;
    let bbox = format!(
        "{},{},{},{}",
        first.west - 0.005,
        first.south - 0.005,
        first.east + 0.005,
        first.north + 0.005
    );
    let path = format!("/datasets/{DATASET}/granules?bbox={bbox}&limit=20");
    let (seen, counts) = walk(&app, &path).await;
    assert!(!seen.is_empty(), "the box selects its own granule at least");
    assert!(
        seen.len() < COUNT,
        "the box narrows — a filter that selected everything would make \
         the agreement below vacuous"
    );
    assert!(
        counts.iter().all(|&count| count == seen.len() as u64),
        "the count is the rows: {counts:?} vs {}",
        seen.len()
    );
}
