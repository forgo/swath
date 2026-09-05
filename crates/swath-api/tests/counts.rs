// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The counts endpoint (#410): `GET /datasets/{datasetId}/counts`.
//!
//! The timeline and the density overlay ask the same question here, so
//! the property that matters is agreement: a bucketed count must equal
//! what paging the full result set yields. The scale fixture (#414) is
//! what makes that assertable — 250 rows across a year, where seven
//! fixture granules could not tell a partition from a coincidence.
//!
//! Also pinned: the refusal path (a bucketing past the cap is named, not
//! answered slowly), the cell bucketing's honest `overlapping` flag, and
//! the shared error taxonomy.
//!
//! **Measured cost** (2026-09-04, this fixture, warm in-memory catalog):
//! at 250 rows a granule page is 361µs and a day bucketing 744µs; at 1460
//! rows they are 1.63ms and 4.24ms. Linear in matched rows, as a full
//! scan is — the bucketing itself is cheap, and rendering the calendar
//! boundaries is most of the difference from the page.

mod common;

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use swath_api::{GranulesState, granules_router};
use swath_testsupport::fixtures::{scale_dataset, scale_granules};

use common::MemoryCatalog;

/// The scale fixture's size: 250 granules, six hours apart, from
/// 2024-01-01 — so a day holds four and January holds 124.
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

fn buckets(body: &serde_json::Value) -> Vec<(String, String, u64)> {
    body["buckets"]
        .as_array()
        .expect("buckets is an array")
        .iter()
        .map(|bucket| {
            (
                bucket["start"].as_str().unwrap_or_default().to_owned(),
                bucket["end"].as_str().unwrap_or_default().to_owned(),
                bucket["count"].as_u64().expect("a count"),
            )
        })
        .collect()
}

fn sum(body: &serde_json::Value) -> u64 {
    body["buckets"]
        .as_array()
        .expect("buckets")
        .iter()
        .map(|bucket| bucket["count"].as_u64().expect("a count"))
        .sum()
}

/// The whole point: a time bucketing partitions the scope. Every granule
/// is in exactly one bucket, so the counts sum to `total` — and `total`
/// is the same number the granule page reports as `numberMatched`.
#[tokio::test]
async fn time_counts_agree_with_paging_the_full_set() {
    let app = app();
    let (_, page) = get_json(&app, &format!("/datasets/{DATASET}/granules?limit=1")).await;
    let matched = page["numberMatched"].as_u64().expect("numberMatched");
    assert_eq!(matched, COUNT as u64);

    for step in ["hour", "day", "week", "month", "year"] {
        let (status, body) =
            get_json(&app, &format!("/datasets/{DATASET}/counts?step={step}")).await;
        assert_eq!(status, StatusCode::OK, "{step}");
        assert_eq!(body["total"].as_u64(), Some(matched), "{step}: total");
        assert_eq!(sum(&body), matched, "{step}: the buckets partition the set");
        assert_eq!(body["overlapping"], false, "{step}");
        assert_eq!(body["by"], "time");
        assert_eq!(body["step"], step);
    }
}

/// Calendar buckets are calendar buckets: the fixture steps six hours, so
/// a day holds four and January holds 31 × 4. Months are not 30 days, and
/// this is where that shows.
#[tokio::test]
async fn calendar_buckets_are_calendar_shaped() {
    let app = app();
    let (_, daily) = get_json(&app, &format!("/datasets/{DATASET}/counts?step=day")).await;
    let days = buckets(&daily);
    // 250 six-hourly granules is 62 whole days and a remainder of two.
    assert_eq!(days.len(), COUNT.div_ceil(4));
    assert_eq!(days.last().expect("a last day").2, 2, "the partial day");
    assert_eq!(
        days[0],
        (
            "2024-01-01T00:00:00Z".to_owned(),
            "2024-01-02T00:00:00Z".to_owned(),
            4
        )
    );

    let (_, monthly) = get_json(&app, &format!("/datasets/{DATASET}/counts?step=month")).await;
    let months = buckets(&monthly);
    assert_eq!(
        months[0],
        (
            "2024-01-01T00:00:00Z".to_owned(),
            "2024-02-01T00:00:00Z".to_owned(),
            124
        ),
        "31 days at four a day"
    );
    // February 2024 has 29 days — a leap year, and a fixed 30-day step
    // would get this wrong.
    assert_eq!(months[1].1, "2024-03-01T00:00:00Z");
    assert_eq!(months[1].2, 29 * 4);
    assert_eq!(sum(&monthly), COUNT as u64);

    // Buckets ascend and never overlap.
    let mut ordered = months.clone();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(months, ordered);
    for pair in months.windows(2) {
        assert_eq!(
            pair[0].1, pair[1].0,
            "each bucket ends where the next begins"
        );
    }
}

/// The scope narrows the count exactly as it narrows the page: a window's
/// buckets sum to the window's `numberMatched`, not the dataset's.
#[tokio::test]
async fn the_scope_narrows_the_count() {
    let app = app();
    let window = "2024-01-01T00:00:00Z/2024-01-31T23:59:59Z";
    let (_, page) = get_json(
        &app,
        &format!("/datasets/{DATASET}/granules?datetime={window}&limit=1"),
    )
    .await;
    let matched = page["numberMatched"].as_u64().expect("numberMatched");
    assert_eq!(matched, 124);

    let (_, body) = get_json(
        &app,
        &format!("/datasets/{DATASET}/counts?datetime={window}&step=day"),
    )
    .await;
    assert_eq!(body["total"].as_u64(), Some(matched));
    assert_eq!(sum(&body), matched);
    assert_eq!(buckets(&body).len(), 31);
    assert!(
        body["links"][0]["href"]
            .as_str()
            .expect("a self href")
            .contains("datetime=2024-01"),
        "the self link carries the scope"
    );
}

/// A footprint can fall in more than one cell, so the cell counts sum to
/// at least the total — and the answer says so rather than letting a
/// caller read the sum as a population.
#[tokio::test]
async fn cell_counts_say_they_overlap() {
    let app = app();
    let (status, body) =
        get_json(&app, &format!("/datasets/{DATASET}/counts?by=cell&size=1")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["by"], "cell");
    assert_eq!(body["size"], 1.0);
    assert_eq!(body["overlapping"], true);
    assert_eq!(body["total"].as_u64(), Some(COUNT as u64));
    assert!(
        sum(&body) >= COUNT as u64,
        "a footprint counts in every cell it touches"
    );
    assert!(body["step"].is_null(), "a cell bucketing has no step");

    // Every cell is a `size` square on the lattice anchored at (-180,-90),
    // so its corners are whole multiples of the size.
    for bucket in body["buckets"].as_array().expect("buckets") {
        let bbox = bucket["bbox"].as_array().expect("a cell bbox");
        let west = bbox[0].as_f64().expect("west");
        let south = bbox[1].as_f64().expect("south");
        assert!(
            (west - west.round()).abs() < 1e-9,
            "{west} is on the lattice"
        );
        assert_eq!(bbox[2].as_f64(), Some(west + 1.0));
        assert_eq!(bbox[3].as_f64(), Some(south + 1.0));
        assert!(bucket["start"].is_null(), "a cell bucket has no instant");
    }

    // A coarser lattice cannot have more cells than a finer one.
    let (_, coarse) = get_json(&app, &format!("/datasets/{DATASET}/counts?by=cell&size=10")).await;
    assert!(
        coarse["buckets"].as_array().expect("buckets").len()
            <= body["buckets"].as_array().expect("buckets").len()
    );
}

/// Refusal over a slow silent answer: a bucketing that would return more
/// buckets than anyone can read is a 400 that names the number, so the
/// caller can pick a coarser one.
#[tokio::test]
async fn an_unreasonable_bucketing_is_refused_with_a_reason() {
    let app = app();
    let (status, body) = get_json(
        &app,
        &format!("/datasets/{DATASET}/counts?by=cell&size=0.0001"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let detail = body["detail"].as_str().unwrap_or_default();
    assert!(detail.contains("2000"), "the cap is named: {detail}");
    assert!(
        detail.contains("coarser"),
        "and the way out is named: {detail}"
    );
}

/// The taxonomy is the granules route's, and a bucketing the route cannot
/// do is a malformed request rather than a silent default.
#[tokio::test]
async fn the_error_taxonomy_matches_the_granules_route() {
    let app = app();
    for (path, expected) in [
        ("/datasets/nope/counts".to_owned(), StatusCode::NOT_FOUND),
        (
            format!("/datasets/{DATASET}/counts?step=fortnight"),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("/datasets/{DATASET}/counts?by=galaxy"),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("/datasets/{DATASET}/counts?by=cell&size=0"),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("/datasets/{DATASET}/counts?by=cell&size=nope"),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("/datasets/{DATASET}/counts?bbox=1,2,3"),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (status, _) = get_json(&app, &path).await;
        assert_eq!(status, expected, "{path}");
    }
}

/// An empty scope is an empty answer, not an error and not a fabricated
/// zero-filled timeline.
#[tokio::test]
async fn an_empty_scope_counts_nothing() {
    let app = app();
    let (status, body) = get_json(
        &app,
        &format!("/datasets/{DATASET}/counts?datetime=2019-01-01T00:00:00Z/2019-12-31T00:00:00Z"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 0);
    assert_eq!(body["buckets"], serde_json::json!([]));
}
