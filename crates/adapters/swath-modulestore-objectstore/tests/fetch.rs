// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The real HTTP fetch path (issue #204) against a local axum server:
//! bytes arrive intact, a 404 is `NotFound`, an over-limit body is
//! refused by its declared size, and non-http(s) URLs never leave the
//! process.

use std::net::SocketAddr;

use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use swath_core::udf::{MODULE_MAX_BYTES, ModuleFetchError, ModuleFetcher};
use swath_modulestore_objectstore::HttpModuleFetcher;

const MODULE: &[u8] = b"\0asm\x01\0\0\0 the module";

/// Serves `/m.wasm` (the module) and `/huge.wasm` (one byte over the
/// limit — hyper derives Content-Length from the real body, so the body
/// has to exist; the fetcher must refuse it by that header without
/// reading it) on an ephemeral port.
async fn serve() -> SocketAddr {
    let app = Router::new()
        .route("/m.wasm", get(|| async { MODULE }))
        .route(
            "/huge.wasm",
            get(|| async { (StatusCode::OK, vec![0u8; MODULE_MAX_BYTES + 1]).into_response() }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("ephemeral port");
    let addr = listener.local_addr().expect("bound address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("server runs");
    });
    addr
}

#[tokio::test]
async fn fetches_the_module_bytes_over_http() {
    let addr = serve().await;
    let fetcher = HttpModuleFetcher::new();
    let bytes = fetcher
        .fetch(&format!("http://{addr}/m.wasm"))
        .await
        .expect("fetch");
    assert_eq!(bytes, MODULE);
}

#[tokio::test]
async fn missing_and_oversized_remotes_are_typed_errors() {
    let addr = serve().await;
    let fetcher = HttpModuleFetcher::new();
    let missing = format!("http://{addr}/nope.wasm");
    assert_eq!(
        fetcher.fetch(&missing).await,
        Err(ModuleFetchError::NotFound { url: missing })
    );
    let huge = format!("http://{addr}/huge.wasm");
    assert_eq!(
        fetcher.fetch(&huge).await,
        Err(ModuleFetchError::TooLarge {
            url: huge,
            size: (MODULE_MAX_BYTES + 1) as u64,
        })
    );
}

#[tokio::test]
async fn only_http_schemes_are_fetched() {
    let fetcher = HttpModuleFetcher::new();
    for url in ["file:///etc/passwd", "s3://bucket/m.wasm", "not a url"] {
        assert_eq!(
            fetcher.fetch(url).await,
            Err(ModuleFetchError::Unsupported {
                url: url.to_owned()
            }),
            "{url}"
        );
    }
}
