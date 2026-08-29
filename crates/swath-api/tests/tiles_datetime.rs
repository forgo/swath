// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The time dimension on the tiles route (ADR 0015, issue #180), driven
//! through the full catalog-mode router over the committed multi-date
//! Park Fire fixtures (`tests/fixtures/hlss30-t10tfk-*`): `datetime=`
//! selects which granule backs the frame; requests differing only in
//! `datetime` render different oracle-pinned pixels under distinct cache
//! keys; the same resolved granule shares one cache entry however it was
//! asked for; the Trace records the temporal decision; malformed values
//! are RFC 7807 400s and empty windows the established 404 refusal.

mod common;

use swath_testsupport::fixtures::{FIRE_DAYS, fire_dataset, fire_granule};
use swath_testsupport::http::get;

/// The served PNG must match the committed oracle golden.
fn assert_matches_golden(served: &[u8], golden_name: &str) {
    let served = image::load_from_memory(served)
        .expect("served PNG decodes")
        .into_rgba8();
    swath_testsupport::pdiff::assert_matches_golden(
        golden_name,
        &served,
        &common::render_goldens_dir().join(golden_name),
    );
}

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use swath_api::{ApiState, CatalogLayer, CatalogLayers, TraceExtension, router};
use swath_core::catalog::DatasetId;
use swath_core::trace::{Strategy, TemporalRule};
use swath_render::ir::Colormap;
use swath_render::{NodataPolicy, PlanSpec, Resampling, ndvi_expr, plan_for};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;
use swath_store_objectstore::ObjectStoreTileCache;

/// The proven fire tile: z13 (col 1326, row 3100) sits fully inside the
/// fixture window (tests/fixtures/README.md), OGC path order z/row/col.
const TILE: &str = "/tilesets/fire-ndvi/tiles/13/3100/1326";

/// The catalog-mode app over the Park Fire dates: one grayscale-NDVI
/// layer (comparable to the oracle goldens), an in-memory catalog whose
/// `find_granules` honors the datetime filter, and the write-through
/// tile cache — the full binary wiring, three frames deep in time.
fn fire_app() -> Router {
    let catalog = common::MemoryCatalog::default();
    catalog.seed(
        fire_dataset("hls-s30-fire"),
        FIRE_DAYS[..3]
            .iter()
            .map(|(day, at)| fire_granule("hls-s30-fire", "t10tfk", day, at))
            .collect(),
    );
    let provider = CatalogLayers::new(
        catalog,
        vec![CatalogLayer {
            id: "fire-ndvi".to_owned(),
            title: "Park Fire NDVI".to_owned(),
            description: "(B8A - B04) / (B8A + B04), grayscale.".to_owned(),
            dataset: DatasetId::new("hls-s30-fire"),
            plan: plan_for(&PlanSpec::BandMath {
                expr: ndvi_expr("b8a", "b04"),
                rescale: Some((-1.0, 1.0)),
                colormap: Colormap::Grayscale,
            })
            .0,
            resampling: Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
            tile_size: 256,
            budget: swath_core::planner::Budget::default(),
            // Config-defined shape: no compiled graph window — only the
            // request's datetime= constrains resolution here.
            window: swath_core::catalog::TimeRange::default(),
            sources: Vec::new(),
        }],
    );
    let store = LocalFileSystem::new_with_prefix(common::fixtures_dir()).expect("fixture dir");
    let state = ApiState::new(
        provider,
        CogSource::new(Arc::new(store)),
        Proj4rsReproject,
        common::BASE_URL,
    )
    .with_cache(ObjectStoreTileCache::new(Arc::new(InMemory::new())));
    router(Arc::new(state))
}

/// GETs a tile expecting 200 PNG; returns (bytes, decision, temporal).
async fn get_frame(
    app: &Router,
    path: &str,
) -> (Vec<u8>, Strategy, Option<swath_core::trace::TemporalTrace>) {
    let response = get(app, path).await;
    assert_eq!(response.status(), StatusCode::OK, "GET {path}");
    assert_eq!(response.headers()["content-type"], "image/png", "{path}");
    let trace = Arc::clone(
        &response
            .extensions()
            .get::<TraceExtension>()
            .expect("trace extension attached")
            .0,
    );
    let bytes = common::body_bytes(response).await;
    (bytes, trace.decision.clone(), trace.temporal.clone())
}

/// The keystone (issue #180 AC): the same tile at two dates renders
/// *different*, oracle-pinned pixels; the temporal decision is on the
/// Trace; and cache identity is granule-scoped — same resolved granule
/// shares an entry however it was asked for, different granules never
/// collide.
#[tokio::test]
async fn same_tile_at_two_dates_renders_different_oracle_pinned_pixels() {
    let app = fire_app();

    // Pre-fire, via an instant between the July and August acquisitions:
    // latest-at-or-before resolves the 2024-07-22 granule.
    let pre_path = format!("{TILE}?datetime=2024-08-01T00:00:00Z");
    let (pre, decision, temporal) = get_frame(&app, &pre_path).await;
    assert_eq!(decision, Strategy::Live, "first render of the pre frame");
    let temporal = temporal.expect("catalog-backed render carries the temporal decision");
    assert_eq!(temporal.granule_id, "hlss30-t10tfk-2024204");
    assert_eq!(temporal.granule_datetime, "2024-07-22T19:03:00Z");
    assert_eq!(temporal.requested.as_deref(), Some("2024-08-01T00:00:00Z"));
    assert_eq!(temporal.rule, TemporalRule::LatestAtOrBefore);
    assert_matches_golden(&pre, "fire-ndvi-13-1326-3100-2024204.png");

    // Post-fire (fresh burn scar), same tile, later instant.
    let post_path = format!("{TILE}?datetime=2024-08-20T00:00:00Z");
    let (post, decision, temporal) = get_frame(&app, &post_path).await;
    assert_eq!(
        decision,
        Strategy::Live,
        "a different resolved granule is a different cache key — never a hit on the pre entry"
    );
    let temporal = temporal.expect("temporal decision present");
    assert_eq!(temporal.granule_id, "hlss30-t10tfk-2024229");
    assert_matches_golden(&post, "fire-ndvi-13-1326-3100-2024229.png");

    assert_ne!(pre, post, "the burn scar must change the pixels");

    // Same date asked again: same granule → same key → cache hit with
    // identical bytes.
    let (pre_again, decision, _) = get_frame(&app, &pre_path).await;
    assert!(
        matches!(decision, Strategy::CacheHit { .. }),
        "repeat of the same frame must hit, got {decision:?}"
    );
    assert_eq!(pre_again, pre);
}

/// Absent `datetime` is plain latest — byte-identical to what the layer
/// served before the parameter existed, and cache-shared with every
/// spelling that resolves to the same (latest) granule.
#[tokio::test]
async fn absent_datetime_is_latest_and_shares_the_granules_cache_entry() {
    let app = fire_app();

    let (latest, decision, temporal) = get_frame(&app, TILE).await;
    assert_eq!(decision, Strategy::Live);
    let temporal = temporal.expect("temporal decision present");
    assert_eq!(temporal.granule_id, "hlss30-t10tfk-2024229");
    assert_eq!(temporal.requested, None);
    assert_eq!(temporal.rule, TemporalRule::Latest);
    assert_matches_golden(&latest, "fire-ndvi-13-1326-3100-2024229.png");

    // An instant after the last acquisition, an interval containing it,
    // and an open-ended interval all resolve to the same granule — and
    // therefore hit the entry the absent-parameter render just wrote:
    // the datetime string is provably not part of the cache key.
    for spelling in [
        "datetime=2030-01-01T00:00:00Z",
        "datetime=2024-08-01T00:00:00Z/2024-12-31T23:59:59Z",
        "datetime=2024-08-01T00:00:00Z/..",
    ] {
        let (bytes, decision, temporal) = get_frame(&app, &format!("{TILE}?{spelling}")).await;
        assert!(
            matches!(decision, Strategy::CacheHit { .. }),
            "{spelling}: same resolved granule must share the cache entry, got {decision:?}"
        );
        assert_eq!(bytes, latest, "{spelling}: byte-identical to plain latest");
        assert_eq!(
            temporal.expect("temporal decision present").granule_id,
            "hlss30-t10tfk-2024229",
            "{spelling}"
        );
    }
}

/// Interval resolution: latest-within, inclusive bounds, open sides.
#[tokio::test]
async fn intervals_resolve_to_the_latest_granule_within() {
    let app = fire_app();
    for (spelling, expected) in [
        // Latest within June..July is the July acquisition.
        (
            "2024-06-01T00:00:00Z/2024-07-31T00:00:00Z",
            "hlss30-t10tfk-2024204",
        ),
        // Inclusive end: the bound exactly at an acquisition selects it.
        (
            "2024-06-01T00:00:00Z/2024-06-07T19:03:00Z",
            "hlss30-t10tfk-2024159",
        ),
        // Open start.
        ("../2024-06-30T00:00:00Z", "hlss30-t10tfk-2024159"),
    ] {
        let (_, _, temporal) = get_frame(&app, &format!("{TILE}?datetime={spelling}")).await;
        let temporal = temporal.expect("temporal decision present");
        assert_eq!(temporal.granule_id, expected, "datetime={spelling}");
        assert_eq!(temporal.rule, TemporalRule::LatestInInterval);
    }
}

/// The error taxonomy: malformed `datetime` → 400 RFC 7807 naming the
/// grammar; a window selecting no granule → the established 404 refusal
/// shape (same as "no granule ingested yet").
#[tokio::test]
async fn malformed_is_rfc7807_400_and_empty_window_the_404_refusal() {
    let app = fire_app();

    for bad in [
        "yesterday",
        "2024-06-06",
        "../..",
        "2024-06-06T17:54:00+00:00",
    ] {
        let response = get(&app, &format!("{TILE}?datetime={bad}")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "datetime={bad}");
        let body = common::body_json(response).await;
        assert_eq!(body["status"], 400, "datetime={bad}");
        assert_eq!(body["title"], "Bad Request");
        assert!(
            body["detail"].as_str().unwrap().contains("datetime"),
            "the problem names the parameter: {body}"
        );
    }

    // Before the first acquisition: an honest 404, window named.
    let response = get(&app, &format!("{TILE}?datetime=2024-01-01T00:00:00Z")).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = common::body_json(response).await;
    assert_eq!(body["status"], 404);
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .contains("acquisition datetime within"),
        "the refusal names the empty window: {body}"
    );

    // A static (fixtures-mode) layer still validates the grammar...
    let response = common::get("/tilesets/ndvi/tiles/12/1561/848?datetime=nope").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    // ...and serves its single timeless frame for any valid instant,
    // byte-identical to the parameterless request.
    let with = common::get("/tilesets/ndvi/tiles/12/1561/848?datetime=2030-01-01T00:00:00Z").await;
    assert_eq!(with.status(), StatusCode::OK);
    let with = common::body_bytes(with).await;
    let without = common::body_bytes(common::get("/tilesets/ndvi/tiles/12/1561/848").await).await;
    assert_eq!(with, without);
}

/// Temporal-domain discovery (the time-slider seam): a catalog-backed
/// layer's tileset metadata links its dataset's granule listing — the
/// acquisition datetimes there are the frames `datetime=` can select.
/// Static layers are a single timeless frame and carry no such link.
#[tokio::test]
async fn tileset_metadata_links_the_backing_datasets_granules() {
    let app = fire_app();
    let response = get(&app, "/tilesets/fire-ndvi").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = common::body_json(response).await;
    let granules = body["links"]
        .as_array()
        .expect("links array")
        .iter()
        .find(|link| link["rel"] == "granules")
        .expect("catalog-backed tileset metadata links its granules");
    assert_eq!(
        granules["href"],
        format!("{}/datasets/hls-s30-fire/granules", common::BASE_URL)
    );
    assert_eq!(granules["type"], "application/json");
    // The frames a client may offer (#301): the compiled window (open on
    // both sides for this config-defined layer) and one branch.
    assert_eq!(body["swath:window"], serde_json::json!([null, null]));
    assert_eq!(body["swath:sources"], serde_json::json!(1));

    // The static fixtures app: no dataset behind the layer, no link.
    let response = common::get("/tilesets/ndvi").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = common::body_json(response).await;
    assert!(body.get("swath:window").is_none() && body.get("swath:sources").is_none());
    assert!(
        !body["links"]
            .as_array()
            .expect("links array")
            .iter()
            .any(|link| link["rel"] == "granules"),
        "a static layer has no time dimension to advertise"
    );
}
