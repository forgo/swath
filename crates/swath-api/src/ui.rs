// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Static UI assets served from the API router (issue #103).
//!
//! The production web bundle (`web/dist`, a Vite build of the demo app) is
//! embedded into the binary by `swath-cli` (cargo feature `embedded-ui`)
//! and handed to the router as a [`UiAssets`] set. Serving rules, chosen so
//! the UI can never shadow the API:
//!
//! - **API routes always win.** Assets are served from the router's
//!   *fallback*, which axum consults only when no registered route matches
//!   — the priority is structural, not name-based. A bundle that shipped a
//!   file literally named `tilesets` would still never shadow
//!   `GET /tilesets` (pinned by test).
//! - **`GET /` is content-negotiated.** Browsers (an `Accept` listing
//!   `text/html`) receive the UI's `index.html`; every other client keeps
//!   receiving the OGC/openEO landing page JSON exactly as before. OGC
//!   clients request JSON (or send no `Accept`), so the standards surface
//!   is unchanged.
//! - **No SPA fallback.** The app is a single page; only exact asset paths
//!   resolve (`/index.html`, `/assets/<hashed>`). Unknown paths stay plain
//!   404, byte-identical to the router's pre-UI behavior.
//! - **Caching follows Vite's contract**: hashed files under `assets/` are
//!   `immutable`; `index.html` (the un-hashed entry) is `no-cache`.

use std::borrow::Cow;
use std::collections::BTreeMap;

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// Path prefixes (first URL segment) registered by the API routers — the
/// OGC surface, the operational endpoints, and the openEO authoring
/// surface. The UI bundle must not ship a top-level path under any of
/// these: the router's fallback priority already guarantees the API wins,
/// and the `swath-cli` route-table test asserts the embedded bundle stays
/// disjoint so no asset is silently unreachable.
pub const API_ROUTE_PREFIXES: [&str; 10] = [
    "conformance",
    "tiles",
    "tilesets",
    "traces",
    "healthz",
    ".well-known",
    "collections",
    "processes",
    "service_types",
    "services",
];

/// True when `path` (relative, no leading slash) starts with a segment the
/// API routers own.
#[must_use]
pub fn collides_with_api_routes(path: &str) -> bool {
    let first = path.split('/').next().unwrap_or("");
    API_ROUTE_PREFIXES.contains(&first)
}

/// One embedded file: bytes plus the content type derived from its
/// extension at construction.
#[derive(Debug)]
struct UiFile {
    bytes: Cow<'static, [u8]>,
    content_type: &'static str,
    /// Hashed assets (under `assets/`) are immutable; the entry page is
    /// revalidated every load.
    cache_control: &'static str,
}

/// The embedded UI bundle: relative path (`index.html`,
/// `assets/index-<hash>.js`, …) → file. Construction is data-only — where
/// the bytes come from (compile-time embedding, a directory walk in tests)
/// is the caller's business.
#[derive(Debug, Default)]
pub struct UiAssets {
    files: BTreeMap<String, UiFile>,
}

impl UiAssets {
    /// Builds the set from `(relative path, bytes)` pairs. Paths are
    /// normalized to no leading slash.
    pub fn from_files<I, P, B>(files: I) -> Self
    where
        I: IntoIterator<Item = (P, B)>,
        P: Into<String>,
        B: Into<Cow<'static, [u8]>>,
    {
        let files = files
            .into_iter()
            .map(|(path, bytes)| {
                let path = path.into();
                let path = path.trim_start_matches('/').to_owned();
                let file = UiFile {
                    content_type: content_type_for(&path),
                    cache_control: if path.starts_with("assets/") {
                        "public, max-age=31536000, immutable"
                    } else {
                        "no-cache"
                    },
                    bytes: bytes.into(),
                };
                (path, file)
            })
            .collect();
        Self { files }
    }

    /// True when the bundle has no `index.html` — the router then serves
    /// exactly its pre-UI behavior (a build without `web/dist`, e.g. plain
    /// `cargo build` before `just build-full`, embeds an empty set).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.files.contains_key("index.html")
    }

    /// The embedded relative paths (route-table test surface).
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// The UI entry page as a response, if the bundle carries one.
    pub(crate) fn index_response(&self) -> Option<Response> {
        self.files.get("index.html").map(respond)
    }

    /// An exact asset lookup as a response; `None` when the path is not in
    /// the bundle (the fallback then answers plain 404).
    pub(crate) fn asset_response(&self, path: &str) -> Option<Response> {
        self.files.get(path.trim_start_matches('/')).map(respond)
    }
}

/// 200 with the file's bytes, content type, and cache policy.
fn respond(file: &UiFile) -> Response {
    (
        StatusCode::OK,
        [
            (CONTENT_TYPE, HeaderValue::from_static(file.content_type)),
            (CACHE_CONTROL, HeaderValue::from_static(file.cache_control)),
        ],
        file.bytes.clone().into_owned(),
    )
        .into_response()
}

/// Content type by extension — exactly the types a Vite bundle of this app
/// can contain, plus a safe default.
fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("map" | "json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::{UiAssets, collides_with_api_routes, content_type_for};

    #[test]
    fn collision_check_covers_every_registered_prefix() {
        for path in [
            "conformance",
            "tiles",
            "tilesets/x.js",
            "traces",
            "healthz",
            ".well-known/openeo",
            "collections",
            "processes",
            "service_types",
            "services/abc",
        ] {
            assert!(collides_with_api_routes(path), "{path} must collide");
        }
        for path in ["index.html", "assets/index-abc.js", "favicon.ico", ""] {
            assert!(!collides_with_api_routes(path), "{path} must not collide");
        }
    }

    #[test]
    fn content_types_and_cache_policy_follow_the_bundle_shape() {
        assert_eq!(content_type_for("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type_for("assets/index-abc.js"), "text/javascript");
        assert_eq!(content_type_for("assets/index-abc.css"), "text/css");
        assert_eq!(content_type_for("mystery"), "application/octet-stream");

        let ui = UiAssets::from_files([
            ("index.html", b"<!doctype html>".as_slice()),
            ("/assets/index-abc.js", b"js".as_slice()),
        ]);
        let index = ui.index_response().expect("index present");
        assert_eq!(index.status(), StatusCode::OK);
        assert_eq!(
            index.headers().get("cache-control").unwrap(),
            "no-cache",
            "the un-hashed entry page revalidates"
        );
        let asset = ui.asset_response("/assets/index-abc.js").expect("asset");
        assert_eq!(
            asset.headers().get("cache-control").unwrap(),
            "public, max-age=31536000, immutable",
            "hashed assets are immutable"
        );
    }

    #[test]
    fn empty_means_no_index() {
        assert!(UiAssets::default().is_empty());
        let no_index = UiAssets::from_files([("assets/x.js", b"js".as_slice())]);
        assert!(no_index.is_empty());
        let with_index = UiAssets::from_files([("index.html", b"<!doctype html>".as_slice())]);
        assert!(!with_index.is_empty());
        assert_eq!(
            with_index.paths().collect::<Vec<_>>(),
            ["index.html"],
            "paths lists the normalized relative paths"
        );
    }
}
