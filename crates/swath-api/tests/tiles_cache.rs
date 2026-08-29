// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The cached serve path through the full router (issue #36): first
//! request renders live and writes through, the repeat serves
//! byte-identical bytes with a `cache_hit` decision readable from both
//! the `X-Swath-Trace` header and the Trace extension.

mod common;

use swath_testsupport::http::get;

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use http_body_util::BodyExt as _;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use swath_api::{ApiState, LayerRegistry, TraceExtension, router};
use swath_core::trace::Strategy;
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;
use swath_store_objectstore::ObjectStoreTileCache;

/// The fixture app of `common::app()`, plus an in-memory write-through
/// tile cache. One router, reused across requests — the cache lives in
/// the shared state, exactly like the binary's wiring.
fn cached_app() -> Router {
    let store = LocalFileSystem::new_with_prefix(common::fixtures_dir()).expect("fixture dir");
    let state = ApiState::new(
        LayerRegistry::hls_fixtures(),
        CogSource::new(Arc::new(store)),
        Proj4rsReproject,
        common::BASE_URL,
    )
    .with_cache(ObjectStoreTileCache::new(Arc::new(InMemory::new())));
    router(Arc::new(state))
}

#[tokio::test]
async fn second_request_is_a_cache_hit_with_identical_bytes() {
    let app = cached_app();
    let path = "/tilesets/truecolor/tiles/12/1561/848";

    // First request: live render, write-through.
    let first = get(&app, path).await;
    assert_eq!(first.status(), StatusCode::OK);
    let first_summary: serde_json::Value =
        serde_json::from_str(first.headers()["x-swath-trace"].to_str().expect("ascii"))
            .expect("header is JSON");
    assert_eq!(first_summary["decision"], "live");
    let first_bytes = first
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();

    // Second request: served from the cache, byte-identical.
    let second = get(&app, path).await;
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(second.headers()["content-type"], "image/png");
    let second_summary: serde_json::Value =
        serde_json::from_str(second.headers()["x-swath-trace"].to_str().expect("ascii"))
            .expect("header is JSON");
    assert_eq!(second_summary["decision"], "cache_hit");
    assert_eq!(
        second_summary["bytes_read"], 0,
        "a hit reads no source bytes (documented Trace decision)"
    );

    // The full Trace extension carries the key.
    let trace = Arc::clone(
        &second
            .extensions()
            .get::<TraceExtension>()
            .expect("trace extension attached")
            .0,
    );
    let Strategy::CacheHit { key } = &trace.decision else {
        panic!("expected a CacheHit decision, got {:?}", trace.decision);
    };
    assert_eq!(key.len(), 64, "the key is the full sha256 hex");
    assert!(trace.provenance.is_empty());

    let second_bytes = second
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    assert_eq!(second_bytes, first_bytes, "hit must serve identical bytes");
}

/// Different layers (and different tiles) never collide: the ndvi tile at
/// the same coordinate still renders live on its first request even after
/// truecolor was cached.
#[tokio::test]
async fn cache_keys_are_layer_scoped() {
    let app = cached_app();
    let truecolor = "/tilesets/truecolor/tiles/12/1561/848";
    let ndvi = "/tilesets/ndvi/tiles/12/1561/848";

    let _ = get(&app, truecolor).await;
    let response = get(&app, ndvi).await;
    let summary: serde_json::Value =
        serde_json::from_str(response.headers()["x-swath-trace"].to_str().expect("ascii"))
            .expect("header is JSON");
    assert_eq!(summary["decision"], "live", "ndvi was never cached");
}
