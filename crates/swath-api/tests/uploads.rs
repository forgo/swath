// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The local-mode upload surface (#197): `PUT /uploads/{filename}` lands
//! bytes in the serving store, and the returned `href` registers through
//! the dataset-creation surface (#196) with its headers validated against
//! exactly those bytes — upload-then-register is one loop, proven here
//! over an in-memory store seeded from the committed fixture COG.

#[allow(
    dead_code,
    reason = "this binary uses a subset of the shared test plumbing"
)]
mod common;

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use object_store::memory::InMemory;
use serde_json::json;
use swath_api::{CatalogLayers, DatasetsState, UploadsState, datasets_router, uploads_router};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;
use tower::ServiceExt as _;

use common::MemoryCatalog;

/// The upload + dataset-creation routers over ONE in-memory store: what
/// uploads is what registration's header validation reads.
fn upload_app() -> Router {
    let store: Arc<dyn object_store::ObjectStore> = Arc::new(InMemory::new());
    let catalog = MemoryCatalog::default();
    let provider = CatalogLayers::new(catalog, Vec::new());
    let datasets = datasets_router(Arc::new(DatasetsState::new(
        provider,
        CogSource::new(Arc::clone(&store)),
        Proj4rsReproject,
    )));
    datasets.merge(uploads_router(Arc::new(UploadsState::new(store))))
}

async fn put_bytes(app: &Router, path: &str, bytes: Vec<u8>) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(path)
                .body(Body::from(bytes))
                .expect("request builds"),
        )
        .await
        .expect("infallible service")
}

#[tokio::test]
async fn upload_then_register_loop() {
    let app = upload_app();
    let cog = std::fs::read(common::fixtures_dir().join("hlss30-t13sdd-2024158-b04.tif"))
        .expect("fixture COG reads");

    // Upload: 201 with the store key registration will name.
    let response = put_bytes(&app, "/uploads/dropped-b04.tif", cog).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = common::body_json(response).await;
    assert_eq!(body["href"], "uploads/dropped-b04.tif");

    // Register a dataset, then the uploaded file as its granule — the
    // header validation reads the very bytes the upload stored.
    let response = common::request_on(
        &app,
        "POST",
        "/datasets",
        Some(json!({"id": "dropped", "title": "Dropped file", "bands": ["b04"]})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let response = common::request_on(
        &app,
        "POST",
        "/datasets/dropped/granules",
        Some(json!({
            "id": "dropped-1", "datetime": "2024-06-06T17:54:00Z",
            "assets": { "b04": "uploads/dropped-b04.tif" },
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "the uploaded bytes register (bbox derived from their header)"
    );
}

#[tokio::test]
async fn upload_refusals_are_rfc7807() {
    let app = upload_app();

    // Unsafe names: refused before anything is stored.
    for bad in ["/uploads/.hidden", "/uploads/sp%20ace"] {
        let response = put_bytes(&app, bad, b"x".to_vec()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{bad}");
        let problem = common::body_json(response).await;
        assert_eq!(problem["title"], "Bad Request");
        assert_eq!(problem["type"], "about:blank");
    }

    // An empty body is a mistake worth refusing loudly.
    let response = put_bytes(&app, "/uploads/empty.tif", Vec::new()).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Garbage uploads but honestly refuses to REGISTER (the door where
    // servability is decided).
    let response = put_bytes(&app, "/uploads/not-a-cog.tif", b"not a tiff".to_vec()).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let response = common::request_on(
        &app,
        "POST",
        "/datasets",
        Some(json!({"id": "junk", "title": "Junk", "bands": ["b04"]})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let response = common::request_on(
        &app,
        "POST",
        "/datasets/junk/granules",
        Some(json!({
            "id": "junk-1", "datetime": "2024-06-06T17:54:00Z",
            "assets": { "b04": "uploads/not-a-cog.tif" },
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let problem = common::body_json(response).await;
    assert!(
        problem["detail"]
            .as_str()
            .expect("detail")
            .contains("uploads/not-a-cog.tif"),
        "the refusal names the unreadable upload: {problem}"
    );
}
