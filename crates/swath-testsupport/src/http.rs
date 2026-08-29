// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! In-process requests against an axum [`Router`] (#348): the `oneshot`
//! plumbing every API test binary used to redefine — one GET, one JSON
//! request, the body collectors, and the openEO service publish handshake.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use http_body_util::BodyExt as _;
use tower::ServiceExt as _;

/// One in-process request on `app` with an optional JSON body (which
/// sets `content-type: application/json`); the response is returned with
/// its body unread.
pub async fn request_on(
    app: &Router,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Response<Body> {
    let mut request = Request::builder().uri(path).method(method);
    let body = match body {
        Some(json) => {
            request = request.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&json).expect("JSON body serializes"))
        }
        None => Body::empty(),
    };
    app.clone()
        .oneshot(request.body(body).expect("request builds"))
        .await
        .expect("infallible service")
}

/// One in-process GET on `app`, with an optional `Accept` header.
pub async fn get_with_accept(app: &Router, path: &str, accept: Option<&str>) -> Response<Body> {
    let mut request = Request::builder().uri(path).method("GET");
    if let Some(accept) = accept {
        request = request.header("accept", accept);
    }
    app.clone()
        .oneshot(request.body(Body::empty()).expect("request builds"))
        .await
        .expect("infallible service")
}

/// One in-process GET on `app`.
pub async fn get(app: &Router, path: &str) -> Response<Body> {
    get_with_accept(app, path, None).await
}

/// Collects a response body to bytes.
pub async fn body_bytes(response: Response<Body>) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes()
        .to_vec()
}

/// Collects a response body as JSON.
pub async fn body_json(response: Response<Body>) -> serde_json::Value {
    serde_json::from_slice(&body_bytes(response).await).expect("body is JSON")
}

/// GET `path`, asserting a 200 with a JSON body, and returns it.
pub async fn json_ok(app: &Router, path: &str) -> serde_json::Value {
    let response = get(app, path).await;
    assert_eq!(response.status(), StatusCode::OK, "GET {path}");
    body_json(response).await
}

/// `POST /services` with `request` (an openEO service request) and
/// returns the status plus the `OpenEO-Identifier` the surface answered
/// with (empty when it did not) — the non-asserting form for failure
/// paths.
pub async fn try_publish(app: &Router, request: serde_json::Value) -> (StatusCode, String) {
    let response = request_on(app, "POST", "/services", Some(request)).await;
    let id = response
        .headers()
        .get("openeo-identifier")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_default();
    (response.status(), id)
}

/// `POST /services` with `request`, asserting `201 Created`, and returns
/// the new service id.
pub async fn publish(app: &Router, request: serde_json::Value) -> String {
    let (status, id) = try_publish(app, request).await;
    assert_eq!(status, StatusCode::CREATED, "publish");
    assert!(
        !id.is_empty(),
        "publish answered without an OpenEO-Identifier"
    );
    id
}
