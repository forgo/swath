// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Opt-in CORS (issue #103, ADR 0011).
//!
//! **Default: off.** The default story is same-origin — the production
//! bundle is served by the binary itself ([`crate::ui`]), and the dev
//! workflow proxies the API through Vite; neither needs CORS, and a tile
//! server should not advertise cross-origin access nobody asked for.
//!
//! When a deployment *does* serve browsers on another origin (a separately
//! hosted frontend, cross-origin `vite dev` without the proxy), the
//! operator opts in with an explicit origin list
//! (`--cors-allowed-origins` / `SWATH_CORS_ALLOWED_ORIGINS` /
//! `cors-allowed-origins` in the config file). The single value `*` allows
//! any origin — a dev convenience, deliberately spelled the same as the
//! header it produces. Methods and request headers mirror whatever the
//! request asks for (`tower-http`'s mirroring behavior): the origin list
//! is the policy; the API is a public read surface plus the openEO
//! authoring routes, with no cookies or credentials to protect
//! (`allow_credentials` stays off, which is also what makes the mirrored
//! wildcard forms safe).

use axum::http::HeaderValue;
use tower_http::cors::{Any, CorsLayer};

/// The CORS layer for an explicit allowlist. `["*"]` (anywhere in the
/// list) allows any origin; otherwise only the exact origins given are
/// echoed. Origins that don't parse as header values are dropped (an
/// origin is a `scheme://host[:port]` token; anything else could never
/// match a browser's `Origin` header anyway).
///
/// Returns `None` for an empty list — the caller then applies no layer at
/// all, keeping the no-CORS path byte-identical to before.
#[must_use]
pub fn cors_layer(allowed_origins: &[String]) -> Option<CorsLayer> {
    if allowed_origins.is_empty() {
        return None;
    }
    let layer = CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers(Any);
    Some(if allowed_origins.iter().any(|origin| origin == "*") {
        layer.allow_origin(Any)
    } else {
        let origins: Vec<HeaderValue> = allowed_origins
            .iter()
            .filter_map(|origin| origin.parse().ok())
            .collect();
        layer.allow_origin(origins)
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::header::{ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_METHOD, ORIGIN};
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt as _;

    use super::cors_layer;

    /// A minimal real router (the fixture registry needs no I/O for `/`).
    fn router() -> axum::Router {
        let state = crate::ApiState::new(
            crate::LayerRegistry::hls_fixtures(),
            swath_source_cog::CogSource::new(Arc::new(object_store::memory::InMemory::new())),
            swath_reproject_proj4rs::Proj4rsReproject,
            "http://localhost:8080",
        );
        crate::router(Arc::new(state))
    }

    fn preflight(origin: &str) -> Request<axum::body::Body> {
        Request::builder()
            .method(Method::OPTIONS)
            .uri("/tilesets")
            .header(ORIGIN, origin)
            .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .body(axum::body::Body::empty())
            .expect("request builds")
    }

    /// The issue #103 integration pair: preflight succeeds with the layer
    /// on, and no CORS headers exist with it off (the default).
    #[tokio::test]
    async fn preflight_allowed_when_enabled_and_absent_when_off() {
        // Off (the default): no layer, no CORS headers on any response.
        let plain = router();
        let response = plain
            .clone()
            .oneshot(preflight("http://localhost:5173"))
            .await
            .expect("infallible");
        assert!(
            response
                .headers()
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none(),
            "no allow-origin header when CORS is off"
        );
        let get = Request::builder()
            .uri("/")
            .header(ORIGIN, "http://localhost:5173")
            .body(axum::body::Body::empty())
            .expect("request builds");
        let response = plain.oneshot(get).await.expect("infallible");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response
                .headers()
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none()
        );

        // On, exact allowlist: the listed origin is echoed on preflight...
        let layer = cors_layer(&["http://localhost:5173".to_owned()]).expect("layer for a list");
        let app = router().layer(layer.clone());
        let response = app
            .oneshot(preflight("http://localhost:5173"))
            .await
            .expect("infallible");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .expect("allow-origin present"),
            "http://localhost:5173"
        );

        // ...an unlisted origin is not...
        let app = router().layer(layer);
        let response = app
            .oneshot(preflight("http://evil.example"))
            .await
            .expect("infallible");
        assert!(
            response
                .headers()
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none(),
            "unlisted origins get no allow-origin"
        );

        // ...and `*` allows any origin.
        let any = cors_layer(&["*".to_owned()]).expect("layer for *");
        let app = router().layer(any);
        let response = app
            .oneshot(preflight("http://anywhere.example"))
            .await
            .expect("infallible");
        assert_eq!(
            response
                .headers()
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .expect("allow-origin present"),
            "*"
        );
    }

    #[test]
    fn empty_list_means_no_layer() {
        assert!(cors_layer(&[]).is_none(), "default off: no layer at all");
    }
}
