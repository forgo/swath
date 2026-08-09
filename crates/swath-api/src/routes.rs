// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Route table and handlers: OGC API requests in, core calls out.
//!
//! Handlers only translate — path/header parsing on the way in, OGC JSON
//! (or PNG bytes) on the way out. Everything between is a call into
//! `swath-core`/`swath-render`. See the crate docs for the route table,
//! the honesty rules on `/conformance`, and the documented behavioral
//! choices (transparent 200 for off-data tiles, 404-vs-400 taxonomy).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::{ACCEPT, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use swath_core::cache::{NoCache, TileCache, TileKey, TileKeyInputs, layer_version};
use swath_core::crs::Crs;
use swath_core::reproject::Reproject;
use swath_core::source::RasterSource;
use swath_core::tile::{LonLatBounds, TileCoord};
use swath_core::trace::{Strategy, Trace};
use swath_render::{render_tile, render_tile_cached};

use crate::error::ApiError;
use crate::model::{
    BoundingBox2D, Conformance, LandingPage, Link, TileSetItem, TileSetList, TileSetMetadata,
};
use crate::provider::{LayerIdentity, LayerProvider};
use crate::registry::Layer;
use crate::traces::TraceBus;

/// The OGC API - Tiles 1.0 (OGC 20-057) conformance classes this surface
/// implements — exactly the set `/conformance` declares. Kept honest by
/// hand: a class is added here only when its requirements are met and
/// smoke-tested.
pub const CONFORMANCE_CLASSES: [&str; 5] = [
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/core",
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/tileset",
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/tilesets-list",
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/dataset-tilesets",
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/png",
];

/// Registered URI of the `WebMercatorQuad` tile matrix set (OGC-NA
/// definition server) — `tileMatrixSetURI` and the tiling-scheme link
/// target. Linking to the registry keeps the API from having to serve
/// TMS definitions itself.
const WEB_MERCATOR_QUAD_URI: &str =
    "http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad";

/// CRS URI of the tile grid (EPSG:3857).
const TILE_CRS_URI: &str = "http://www.opengis.net/def/crs/EPSG/0/3857";

/// CRS URI bounding boxes are expressed in (CRS84: lon/lat degrees).
const CRS84_URI: &str = "http://www.opengis.net/def/crs/OGC/1.3/CRS84";

/// Deepest tile matrix served: the registered `WebMercatorQuad`
/// definition enumerates matrices `"0"`..`"24"`, so deeper addresses are
/// out-of-range for the declared tiling scheme even though the tiler's
/// address space is wider.
const MAX_TILE_MATRIX: u8 = 24;

/// Points sampled per raster edge when deriving a layer's geographic
/// bounds: enough to catch the bulge a curved CRS edge develops under
/// reprojection (same idea as the tiler's window boundary sampling).
const BOUNDS_SAMPLES_PER_EDGE: u32 = 16;

/// Tile matrix set id hashed into cache keys (#36) — the one TMS served.
const TMS_ID: &str = "WebMercatorQuad";

/// Everything the handlers need, wired once at startup: the layer
/// provider (static registry or catalog-backed, issue #31), the two
/// ports the render path consumes, and — when configured — the tile
/// cache (#36). Generic exactly like [`render_tile`] — the binary (#29)
/// and the tests pick concrete adapters. The cache parameter defaults to
/// [`NoCache`] so cache-less construction ([`ApiState::new`]) names no
/// cache type; [`ApiState::with_cache`] swaps in a real one.
#[derive(Debug)]
pub struct ApiState<S, R, L, C = NoCache> {
    layers: L,
    source: S,
    reproject: R,
    /// The write-through tile cache; `None` serves exactly as before #36
    /// (the render path never consults it).
    cache: Option<C>,
    /// Base URL links are minted under (no trailing slash), e.g.
    /// `http://localhost:8080`.
    base_url: String,
    /// The trace bus: the tile handler publishes every render, the
    /// `GET /traces` SSE stream (issue #28) fans them out.
    traces: TraceBus,
}

impl<S, R, L> ApiState<S, R, L> {
    /// Wires the API over a layer provider (a
    /// [`LayerRegistry`](crate::registry::LayerRegistry), or
    /// [`CatalogLayers`](crate::provider::CatalogLayers) for catalog-backed
    /// serving), the two ports, and the base URL links advertise (trailing
    /// slash trimmed). No cache: serving is byte-for-byte the pre-#36
    /// behavior until [`ApiState::with_cache`] adds one.
    pub fn new(layers: L, source: S, reproject: R, base_url: impl Into<String>) -> Self {
        let mut base_url: String = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        Self {
            layers,
            source,
            reproject,
            cache: None,
            base_url,
            traces: TraceBus::default(),
        }
    }
}

impl<S, R, L, C> ApiState<S, R, L, C> {
    /// Replaces the default trace bus — the seam tests use to shrink the
    /// subscriber buffer (forcing `lagged`) and the keepalive interval.
    #[must_use]
    pub fn with_trace_bus(mut self, traces: TraceBus) -> Self {
        self.traces = traces;
        self
    }

    /// Enables the write-through tile cache (#36): the tile handler
    /// consults it before rendering and writes fresh renders through.
    #[must_use]
    pub fn with_cache<C2>(self, cache: C2) -> ApiState<S, R, L, C2> {
        ApiState {
            layers: self.layers,
            source: self.source,
            reproject: self.reproject,
            cache: Some(cache),
            base_url: self.base_url,
            traces: self.traces,
        }
    }

    /// The trace bus renders are published to. Exposed so tests (and,
    /// later, non-tile render paths) can publish and observe directly.
    pub fn trace_bus(&self) -> &TraceBus {
        &self.traces
    }
}

/// The render [`Trace`] of a served tile, attached to the response as an
/// extension — the seam the Trace SSE stream (issue #28) consumes: a
/// middleware or stream fan-out reads it from the response without the
/// handler having to know who is listening. `Arc` because the Trace is
/// shared read-only once rendered.
#[derive(Debug, Clone)]
pub struct TraceExtension(pub Arc<Trace>);

/// The OGC API - Tiles router over `state`. Every route is `GET` (axum
/// answers `HEAD` from the same handlers).
pub fn router<S, R, L, C>(state: Arc<ApiState<S, R, L, C>>) -> axum::Router
where
    S: RasterSource + 'static,
    R: Reproject + 'static,
    L: LayerProvider + 'static,
    C: TileCache + 'static,
{
    axum::Router::new()
        .route("/", get(landing))
        .route("/conformance", get(conformance))
        // The tilesets list, twice: `/tiles` is the path OGC 20-057
        // `/req/dataset-tilesets/operation` and `/req/tilesets-list/
        // tileset-path` require on the dataset root; `/tilesets` is the
        // canonical resource collection self-links point into. Same
        // handler, same representation.
        .route("/tiles", get(tilesets))
        .route("/tilesets", get(tilesets))
        .route("/tilesets/{layerId}", get(tileset))
        .route(
            "/tilesets/{layerId}/tiles/{tileMatrix}/{tileRow}/{tileCol}",
            get(tile),
        )
        // The x-ray Trace stream (issue #28) — control-plane, not OGC.
        .route("/traces", get(traces))
        // Operational liveness probe (#29) — not an OGC resource; kept
        // dependency-free (no registry/source I/O) so orchestrator
        // healthchecks measure the process, not the data plane.
        .route("/healthz", get(healthz))
        .with_state(state)
}

/// `GET /healthz` — plain 200 `ok`. Liveness only: the process is up and
/// serving HTTP. Readiness of catalog/store dependencies is a later,
/// separate concern (issues #30/#31).
async fn healthz() -> &'static str {
    "ok"
}

// --- JSON document handlers ---

async fn landing<S, R, L, C>(State(app): State<Arc<ApiState<S, R, L, C>>>) -> Json<LandingPage>
where
    S: RasterSource + 'static,
    R: Reproject + 'static,
    L: LayerProvider + 'static,
    C: TileCache + 'static,
{
    let base = &app.base_url;
    Json(LandingPage {
        title: "Swath".to_owned(),
        description: "Live satellite imagery tiles: OGC API - Tiles over the Swath tiler."
            .to_owned(),
        links: vec![
            Link::new(format!("{base}/"), "self")
                .media_type("application/json")
                .title("This landing page"),
            Link::new(format!("{base}/conformance"), "conformance")
                .media_type("application/json")
                .title("Conformance declaration"),
            Link::new(
                format!("{base}/conformance"),
                "http://www.opengis.net/def/rel/ogc/1.0/conformance",
            )
            .media_type("application/json")
            .title("Conformance declaration"),
            // Map tilesets of the dataset (OGC 20-057
            // /req/dataset-tilesets/landingpage).
            Link::new(
                format!("{base}/tiles"),
                "http://www.opengis.net/def/rel/ogc/1.0/tilesets-map",
            )
            .media_type("application/json")
            .title("Tilesets, one per layer"),
        ],
    })
}

async fn conformance() -> Json<Conformance> {
    Json(Conformance {
        conforms_to: CONFORMANCE_CLASSES.map(str::to_owned).to_vec(),
    })
}

/// The list-item subset of a layer's tileset metadata
/// (`/req/tilesets-list/tileset-links`: `dataType`, `crs`,
/// `tileMatrixSetURI`, self + tiling-scheme links).
fn tileset_item(base: &str, layer: &LayerIdentity) -> TileSetItem {
    TileSetItem {
        title: layer.title.clone(),
        data_type: "map".to_owned(),
        crs: TILE_CRS_URI.to_owned(),
        tile_matrix_set_uri: WEB_MERCATOR_QUAD_URI.to_owned(),
        links: vec![
            Link::new(format!("{base}/tilesets/{id}", id = layer.id), "self")
                .media_type("application/json")
                .title(format!("{} tileset metadata", layer.title)),
            Link::new(
                WEB_MERCATOR_QUAD_URI,
                "http://www.opengis.net/def/rel/ogc/1.0/tiling-scheme",
            )
            .media_type("application/json")
            .title("WebMercatorQuad tile matrix set definition"),
        ],
    }
}

async fn tilesets<S, R, L, C>(State(app): State<Arc<ApiState<S, R, L, C>>>) -> Json<TileSetList>
where
    S: RasterSource + 'static,
    R: Reproject + 'static,
    L: LayerProvider + 'static,
    C: TileCache + 'static,
{
    Json(TileSetList {
        tilesets: app
            .layers
            .identities()
            .iter()
            .map(|layer| tileset_item(&app.base_url, layer))
            .collect(),
    })
}

/// Tileset metadata. The bounding box derives from the layer's *resolved*
/// assets, so a catalog-backed layer whose dataset has no granules yet is
/// 404 here (its identity still appears in the tilesets list) — resolution
/// semantics live with [`LayerProvider::resolve`].
async fn tileset<S, R, L, C>(
    State(app): State<Arc<ApiState<S, R, L, C>>>,
    Path(layer_id): Path<String>,
) -> Result<Json<TileSetMetadata>, ApiError>
where
    S: RasterSource + 'static,
    R: Reproject + 'static,
    L: LayerProvider + 'static,
    C: TileCache + 'static,
{
    let identity = app
        .layers
        .identity(&layer_id)
        .ok_or_else(|| ApiError::not_found(format!("no layer `{layer_id}`")))?;
    let resolved = app.layers.resolve(&layer_id).await?;
    let bounds = layer_bounds(&app.source, &app.reproject, &resolved.layer).await?;
    let item = tileset_item(&app.base_url, &identity);

    let mut links = item.links;
    links.push(
        Link::new(
            format!(
                "{base}/tilesets/{id}/tiles/{{tileMatrix}}/{{tileRow}}/{{tileCol}}",
                base = app.base_url,
                id = identity.id,
            ),
            "item",
        )
        .media_type("image/png")
        .title(format!("{} tiles (PNG)", identity.title))
        .templated(),
    );

    Ok(Json(TileSetMetadata {
        title: item.title,
        description: identity.description.clone(),
        data_type: item.data_type,
        crs: item.crs,
        tile_matrix_set_uri: item.tile_matrix_set_uri,
        bounding_box: BoundingBox2D {
            lower_left: [bounds.west, bounds.south],
            upper_right: [bounds.east, bounds.north],
            crs: CRS84_URI.to_owned(),
            ordered_axes: ["Lon".to_owned(), "Lat".to_owned()],
        },
        links,
    }))
}

// --- The tile handler ---

async fn tile<S, R, L, C>(
    State(app): State<Arc<ApiState<S, R, L, C>>>,
    Path((layer_id, tile_matrix, tile_row, tile_col)): Path<(String, String, String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError>
where
    S: RasterSource + 'static,
    R: Reproject + 'static,
    L: LayerProvider + 'static,
    C: TileCache + 'static,
{
    let coord = parse_tile_path(&tile_matrix, &tile_row, &tile_col)?;
    check_accepts_png(&headers)?;
    let layer = app.layers.resolve(&layer_id).await?;

    let request = layer.tile_request(coord);
    let render = match &app.cache {
        // Cache configured (#36): consult it first, write fresh renders
        // through. The key is computed per request — resolution already
        // ran, so every input is at hand and no I/O is added; the layer
        // version is content-derived (granule id + plan hash), which is
        // the whole invalidation story (swath-core cache module docs).
        Some(cache) => {
            let plan_json = serde_json::to_string(&request.plan).map_err(|err| {
                ApiError::internal(format!("render plan failed to serialize: {err}"))
            })?;
            let version = layer_version(layer.granule_id.as_deref(), &plan_json);
            let key = TileKey::compute(&TileKeyInputs {
                layer: &layer_id,
                layer_version: &version,
                plan_json: &plan_json,
                tms: TMS_ID,
                coord,
                tile_size: request.tile_size,
            });
            render_tile_cached(&app.source, &app.reproject, cache, &key, &request).await
        }
        None => render_tile(&app.source, &app.reproject, &request).await,
    };
    let (encoded, trace) =
        render.map_err(|err| ApiError::internal(format!("tile render failed: {err}")))?;

    // 200 + PNG bytes, with the Trace both summarized in a debug header
    // and attached whole as a response extension (the #28 SSE seam — the
    // handler never discards the Trace). `ingest_to_pixel_ms` joins the
    // header when present: the north-star number must be readable from a
    // plain curl -D (the e2e gate does exactly that). `decision` joined
    // in #36 so a cache hit is readable the same way (the e2e asserts
    // `cache_hit` off exactly this header).
    let ingest_to_pixel = trace
        .ingest_to_pixel_ms
        .map_or_else(String::new, |ms| format!(",\"ingest_to_pixel_ms\":{ms}"));
    let decision = match &trace.decision {
        Strategy::Live => "live",
        Strategy::CacheHit { .. } => "cache_hit",
        Strategy::Overview { .. } => "overview",
    };
    let debug_header = format!(
        "{{\"decision\":\"{decision}\",\"bytes_read\":{},\"total_ms\":{}{ingest_to_pixel}}}",
        trace.bytes_read, trace.timings.total_ms,
    );
    let mut response = (
        StatusCode::OK,
        [(CONTENT_TYPE, HeaderValue::from_static("image/png"))],
        encoded.bytes,
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(&debug_header) {
        response.headers_mut().insert("x-swath-trace", value);
    }
    let trace = Arc::new(trace);
    response
        .extensions_mut()
        .insert(TraceExtension(Arc::clone(&trace)));
    // Published to the SSE bus (#28) only after the response is fully
    // assembled, so stream fan-out can never skew served-tile timing.
    // `publish` is non-blocking by construction — a slow x-ray subscriber
    // loses events (reported as `lagged`), never delays a tile.
    app.traces.publish(&layer.layer.id, coord, trace);
    Ok(response)
}

/// `GET /traces` — the x-ray SSE stream (issue #28): `text/event-stream`
/// of every render published from connection time on. Wire contract and
/// slow-subscriber semantics live in [`crate::traces`].
async fn traces<S, R, L, C>(State(app): State<Arc<ApiState<S, R, L, C>>>) -> impl IntoResponse
where
    S: RasterSource + 'static,
    R: Reproject + 'static,
    L: LayerProvider + 'static,
    C: TileCache + 'static,
{
    app.traces.sse()
}

// --- Translation helpers (parsing only — no domain logic) ---

/// Parses `{tileMatrix}/{tileRow}/{tileCol}` — the **OGC order**, z/y/x —
/// into a [`TileCoord`] (whose fields are XYZ-named: `y` = row, `x` =
/// col).
///
/// Taxonomy (OGC 20-057 `/req/core/tc-error` allows 404 or 400 for
/// out-of-range): an unknown tile-matrix identifier or an out-of-matrix
/// row/col addresses a tile that does not exist → 404; a row/col that is
/// not an integer at all is a malformed request → 400.
fn parse_tile_path(
    tile_matrix: &str,
    tile_row: &str,
    tile_col: &str,
) -> Result<TileCoord, ApiError> {
    let z: u8 = tile_matrix
        .parse()
        .ok()
        .filter(|z| *z <= MAX_TILE_MATRIX)
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "tileMatrix `{tile_matrix}` is not a WebMercatorQuad tile matrix (expected 0..={MAX_TILE_MATRIX})"
            ))
        })?;
    let row: u32 = tile_row
        .parse()
        .map_err(|_| ApiError::bad_request(format!("tileRow `{tile_row}` is not an integer")))?;
    let col: u32 = tile_col
        .parse()
        .map_err(|_| ApiError::bad_request(format!("tileCol `{tile_col}` is not an integer")))?;
    TileCoord::new(z, col, row).map_err(|_| {
        ApiError::not_found(format!(
            "tile {z}/{row}/{col} is outside tile matrix {z} (rows and columns run 0..{})",
            1u64 << z,
        ))
    })
}

/// Content negotiation, PNG edition: the only tile format today. Absent
/// `Accept`, `*/*`, `image/*`, and `image/png` are acceptable; anything
/// else is an honest 406 rather than a silently mismatched body.
fn check_accepts_png(headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(accept) = headers.get(ACCEPT) else {
        return Ok(());
    };
    let Ok(accept) = accept.to_str() else {
        return Err(ApiError::bad_request("Accept header is not valid text"));
    };
    let acceptable = accept.split(',').any(|range| {
        let media = range.split(';').next().unwrap_or("").trim();
        matches!(media, "*/*" | "image/*" | "image/png")
    });
    if acceptable {
        Ok(())
    } else {
        Err(ApiError::not_acceptable(format!(
            "no acceptable representation: tiles are available as image/png (Accept: {accept})"
        )))
    }
}

/// Geographic (CRS84) bounds of a layer: every distinct asset is
/// described, its raster boundary sampled in pixel space, projected to
/// the source CRS, transformed to WGS 84, and the union taken. Metadata
/// I/O only — no pixels are read.
async fn layer_bounds<S, R>(
    source: &S,
    reproject: &R,
    layer: &Layer,
) -> Result<LonLatBounds, ApiError>
where
    S: RasterSource,
    R: Reproject,
{
    let mut described: Vec<&swath_core::raster::AssetRef> = Vec::new();
    let mut bounds: Option<LonLatBounds> = None;

    for asset in layer.bands.values() {
        if described.contains(&asset) {
            continue;
        }
        described.push(asset);

        let info = source
            .describe(asset)
            .await
            .map_err(|err| ApiError::internal(format!("describe failed for `{asset}`: {err}")))?;
        let to_wgs84 = reproject.transformer(info.crs, Crs::WGS84).map_err(|err| {
            ApiError::internal(format!("no {} -> WGS84 transform: {err}", info.crs))
        })?;

        // Boundary of the raster in fractional pixel coordinates.
        #[allow(
            clippy::cast_precision_loss,
            reason = "raster dimensions are far below 2^52"
        )]
        let (width, height) = (info.width as f64, info.height as f64);
        let mut boundary: Vec<(f64, f64)> = Vec::new();
        for i in 0..=BOUNDS_SAMPLES_PER_EDGE {
            let t = f64::from(i) / f64::from(BOUNDS_SAMPLES_PER_EDGE);
            boundary.push((t * width, 0.0)); // top edge
            boundary.push((t * width, height)); // bottom edge
            boundary.push((0.0, t * height)); // left edge
            boundary.push((width, t * height)); // right edge
        }

        for (col, row) in boundary {
            let (x, y) = info.transform.pixel_to_crs(col, row);
            let (lon, lat) = to_wgs84.transform(x, y).map_err(|err| {
                ApiError::internal(format!(
                    "boundary point of `{asset}` failed to transform: {err}"
                ))
            })?;
            bounds = Some(match bounds {
                None => LonLatBounds {
                    west: lon,
                    south: lat,
                    east: lon,
                    north: lat,
                },
                Some(b) => LonLatBounds {
                    west: b.west.min(lon),
                    south: b.south.min(lat),
                    east: b.east.max(lon),
                    north: b.north.max(lat),
                },
            });
        }
    }

    bounds.ok_or_else(|| ApiError::internal("layer has no band assets to derive bounds from"))
}

#[cfg(test)]
mod tests {
    use super::{check_accepts_png, parse_tile_path};
    use axum::http::{HeaderMap, HeaderValue, StatusCode};

    #[test]
    fn tile_path_is_z_row_col() {
        // OGC order: {tileMatrix}/{tileRow}/{tileCol} = z/y/x.
        let coord = parse_tile_path("12", "1561", "848").unwrap();
        assert_eq!((coord.z, coord.x, coord.y), (12, 848, 1561));
    }

    #[test]
    fn tile_path_taxonomy() {
        // Unknown matrix (non-numeric or beyond the TMS definition): 404.
        assert_eq!(
            parse_tile_path("abc", "0", "0").unwrap_err().status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            parse_tile_path("25", "0", "0").unwrap_err().status,
            StatusCode::NOT_FOUND
        );
        // Malformed row/col: 400.
        assert_eq!(
            parse_tile_path("12", "x", "0").unwrap_err().status,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            parse_tile_path("12", "0", "-1").unwrap_err().status,
            StatusCode::BAD_REQUEST
        );
        // Out-of-matrix row/col: 404.
        assert_eq!(
            parse_tile_path("12", "4096", "0").unwrap_err().status,
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn accept_negotiation_only_admits_png_shapes() {
        let accepts = |value: Option<&str>| {
            let mut headers = HeaderMap::new();
            if let Some(value) = value {
                headers.insert("accept", HeaderValue::from_str(value).unwrap());
            }
            check_accepts_png(&headers)
        };
        assert!(accepts(None).is_ok());
        assert!(accepts(Some("*/*")).is_ok());
        assert!(accepts(Some("image/*")).is_ok());
        assert!(accepts(Some("image/png")).is_ok());
        assert!(accepts(Some("text/html, image/png;q=0.8")).is_ok());
        assert_eq!(
            accepts(Some("application/json")).unwrap_err().status,
            StatusCode::NOT_ACCEPTABLE
        );
    }
}
