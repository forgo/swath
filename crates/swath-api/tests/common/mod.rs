// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Shared plumbing for the API tests: the fixture-wired app (COG source +
//! proj4rs over the committed HLS fixtures), an in-process request
//! helper (`tower::ServiceExt::oneshot` — no network), and the OGC
//! schema validator over the committed official schemas.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response};
use http_body_util::BodyExt as _;
use jsonschema::{Retrieve, Uri, Validator};
use object_store::local::LocalFileSystem;
use swath_api::{ApiState, LayerRegistry, router};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;
use tower::ServiceExt as _;

/// Base URL the test app mints links under.
pub(crate) const BASE_URL: &str = "http://localhost";

/// The committed HLS fixture directory (tests/fixtures/README.md, ADR 0004).
pub(crate) fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// The committed official OGC schemas (tests/data/ogc/README.md).
pub(crate) fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/ogc")
}

/// swath-render's committed oracle goldens (the #25/#26 suite) — the API
/// tile tests compare served tiles against the very same references.
pub(crate) fn render_goldens_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../swath-render/tests/data")
}

/// The API over the fixture registry, wired to the concrete Phase-1
/// adapters — the same wiring the binary (#29) will do.
pub(crate) fn app() -> Router {
    let store = LocalFileSystem::new_with_prefix(fixtures_dir()).expect("fixture dir exists");
    let state = ApiState::new(
        LayerRegistry::hls_fixtures(),
        CogSource::new(Arc::new(store)),
        Proj4rsReproject,
        BASE_URL,
    );
    router(Arc::new(state))
}

/// One in-process GET; returns the full response (status, headers,
/// extensions) with the body still unread.
pub(crate) async fn get(path: &str) -> Response<Body> {
    get_with_accept(path, None).await
}

/// One in-process GET with an optional `Accept` header.
pub(crate) async fn get_with_accept(path: &str, accept: Option<&str>) -> Response<Body> {
    let mut request = Request::builder().uri(path).method("GET");
    if let Some(accept) = accept {
        request = request.header("accept", accept);
    }
    app()
        .oneshot(request.body(Body::empty()).expect("request builds"))
        .await
        .expect("infallible service")
}

/// Collects a response body to bytes.
pub(crate) async fn body_bytes(response: Response<Body>) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes()
        .to_vec()
}

/// Collects a response body as JSON.
pub(crate) async fn body_json(response: Response<Body>) -> serde_json::Value {
    serde_json::from_slice(&body_bytes(response).await).expect("body is JSON")
}

/// Resolves external `$ref`s against the committed schema files: every
/// reference in the OGC schemas is a relative sibling file name, so the
/// URI's last path segment names the file (looked up in `tms/`, then
/// `common/`).
struct CommittedSchemas;

impl Retrieve for CommittedSchemas {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        let name = uri
            .path()
            .as_str()
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("no file name in $ref URI `{uri}`"))?;
        let dir = schemas_dir();
        let path = [dir.join("tms").join(name), dir.join("common").join(name)]
            .into_iter()
            .find(|p| p.exists())
            .ok_or_else(|| format!("`{name}` is not a committed OGC schema (from `{uri}`)"))?;
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }
}

/// Compiles a committed OGC schema (path relative to `tests/data/ogc/`,
/// e.g. `"tms/tileSet.json"`).
pub(crate) fn schema(relative: &str) -> Validator {
    let raw = std::fs::read_to_string(schemas_dir().join(relative)).expect("schema file exists");
    let schema: serde_json::Value = serde_json::from_str(&raw).expect("schema parses");
    jsonschema::options()
        .with_retriever(CommittedSchemas)
        .build(&schema)
        .expect("schema compiles")
}

/// Asserts `instance` is valid under the committed schema, with a
/// readable failure listing every violation.
pub(crate) fn assert_valid(relative: &str, instance: &serde_json::Value) {
    let validator = schema(relative);
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|err| format!("  {} at {}", err, err.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "instance violates {relative}:\n{}\ninstance: {}",
        errors.join("\n"),
        serde_json::to_string_pretty(instance).expect("instance pretty-prints"),
    );
}
