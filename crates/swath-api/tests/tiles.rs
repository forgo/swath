// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Tile endpoint tests: the z/y/x ordering proof, byte-identity with the
//! direct render path, perceptual identity with the committed #25/#26
//! oracle goldens, the Trace exposure seam (#28), and the documented
//! error taxonomy.

#[allow(
    dead_code,
    reason = "shared between the API test targets; not every helper is used in each"
)]
mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use object_store::local::LocalFileSystem;
use swath_api::{LayerRegistry, TraceExtension};
use swath_core::tile::TileCoord;
use swath_render::render_tile;
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;
use swath_testkit::{DiffPolicy, RgbaImage, diff, load_png};

/// Renders a layer tile directly through `render_tile` — the reference
/// the API-served bytes must equal. `x`/`y` are XYZ-named (col/row).
async fn direct_render(layer_id: &str, z: u8, x: u32, y: u32) -> Vec<u8> {
    let registry = LayerRegistry::hls_fixtures();
    let layer = registry.get(layer_id).expect("fixture layer");
    let store = LocalFileSystem::new_with_prefix(common::fixtures_dir()).expect("fixture dir");
    let source = CogSource::new(Arc::new(store));
    let request = layer.tile_request(TileCoord::new(z, x, y).expect("valid tile"));
    let (encoded, _) = render_tile(&source, &Proj4rsReproject, &request)
        .await
        .expect("direct render succeeds");
    encoded.bytes
}

async fn get_tile_ok(path: &str) -> Vec<u8> {
    let response = common::get(path).await;
    assert_eq!(response.status(), StatusCode::OK, "GET {path}");
    assert_eq!(
        response.headers()["content-type"],
        "image/png",
        "GET {path} content type"
    );
    common::body_bytes(response).await
}

fn decode(png: &[u8]) -> RgbaImage {
    image::load_from_memory(png)
        .expect("PNG decodes")
        .into_rgba8()
}

// --- The ordering proof: OGC {tileMatrix}/{tileRow}/{tileCol} = z/y/x ---

/// The keystone: tile (z=12, col x=848, row y=1561) is served at
/// `/tiles/12/1561/848` — row before column — and is byte-identical to
/// the direct `render_tile` output for that coordinate, and perceptually
/// identical to the committed rio-tiler/GDAL golden from the #25/#26
/// suites. Wiring AND ordering, proven in one motion.
#[tokio::test]
async fn ogc_tile_path_is_row_then_col_and_matches_the_direct_render() {
    let served = get_tile_ok("/tilesets/truecolor/tiles/12/1561/848").await;

    assert_eq!(
        served,
        direct_render("truecolor", 12, 848, 1561).await,
        "served tile must be byte-identical to the direct render of (z 12, x 848, y 1561)"
    );

    let golden = load_png(&common::render_goldens_dir().join("truecolor-12-848-1561.png"))
        .expect("golden loads");
    let report = diff(&decode(&served), &golden).expect("dimensions match");
    assert!(
        report.passes(&DiffPolicy::default()),
        "served tile fails the oracle policy: max |diff| {}",
        report.max_abs_channel_diff,
    );
}

/// The counter-proof: reading the same path segments in the XYZ z/x/y
/// habit would address (col 1561, row 848) — a valid z12 tile ~80 km off
/// the fixture swath. Served correctly it is fully transparent, so any
/// row/col swap in the handler would light this up.
#[tokio::test]
async fn swapping_row_and_col_addresses_an_empty_tile_not_the_swath() {
    let served = get_tile_ok("/tilesets/truecolor/tiles/12/848/1561").await;
    let image = decode(&served);
    assert_eq!(image.dimensions(), (256, 256));
    assert!(
        image.pixels().all(|p| p.0 == [0, 0, 0, 0]),
        "tile (row 848, col 1561) must be transparent — data there would mean z/x/y ordering"
    );
}

#[tokio::test]
async fn ndvi_layer_serves_the_same_bytes_as_its_direct_render() {
    let served = get_tile_ok("/tilesets/ndvi/tiles/12/1561/848").await;
    assert_eq!(served, direct_render("ndvi", 12, 848, 1561).await);
}

// --- Documented choice: in-matrix, off-data tiles are 200 + transparent ---

#[tokio::test]
async fn tile_outside_layer_bounds_is_200_transparent_png() {
    // Col 840 at z12 is ~78 km west of the fixture footprint (the same
    // coordinate the render suite pins): in-matrix, no data.
    let served = get_tile_ok("/tilesets/truecolor/tiles/12/1561/840").await;
    let image = decode(&served);
    assert!(
        image.pixels().all(|p| p.0 == [0, 0, 0, 0]),
        "off-data tile must be fully transparent"
    );
}

// --- Trace exposure: the debug header and the #28 extension seam ---

#[tokio::test]
async fn tile_responses_expose_the_trace_as_header_and_extension() {
    let response = common::get("/tilesets/truecolor/tiles/12/1561/848").await;
    assert_eq!(response.status(), StatusCode::OK);

    let header = response.headers()["x-swath-trace"]
        .to_str()
        .expect("header is ASCII");
    let summary: serde_json::Value = serde_json::from_str(header).expect("header is JSON");
    let bytes_read = summary["bytes_read"].as_u64().expect("bytes_read");
    assert!(bytes_read > 0, "a live render reads bytes");
    assert!(summary["total_ms"].is_u64(), "total_ms present");

    // The full Trace rides the response as an extension — the seam the
    // Trace SSE stream (#28) consumes.
    let trace = &response
        .extensions()
        .get::<TraceExtension>()
        .expect("trace extension attached")
        .0;
    assert_eq!(
        trace.bytes_read, bytes_read,
        "header summarizes the same trace"
    );
    assert!(!trace.provenance.is_empty());
}

// --- Error taxonomy (OGC 20-057 /req/core/tc-error: 404/400) ---

#[tokio::test]
async fn out_of_matrix_and_unknown_matrix_tiles_are_404() {
    for path in [
        "/tilesets/truecolor/tiles/12/4096/0", // row = matrix height
        "/tilesets/truecolor/tiles/12/0/4096", // col = matrix width
        "/tilesets/truecolor/tiles/25/0/0",    // beyond WebMercatorQuad's matrices
        "/tilesets/truecolor/tiles/abc/0/0",   // no such tile matrix id
    ] {
        let response = common::get(path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "GET {path}");
    }
}

#[tokio::test]
async fn non_integer_row_or_col_is_400() {
    for path in [
        "/tilesets/truecolor/tiles/12/1.5/848",
        "/tilesets/truecolor/tiles/12/1561/-1",
    ] {
        let response = common::get(path).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "GET {path}");
    }
}

// --- Content negotiation: PNG is the only tile format today ---

#[tokio::test]
async fn accept_negotiation_serves_png_or_406() {
    let path = "/tilesets/truecolor/tiles/12/1561/848";
    for accept in ["image/png", "image/*", "*/*", "text/html, image/png;q=0.5"] {
        let response = common::get_with_accept(path, Some(accept)).await;
        assert_eq!(response.status(), StatusCode::OK, "Accept: {accept}");
    }
    let response = common::get_with_accept(path, Some("application/json")).await;
    assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
    let exception = common::body_json(response).await;
    assert_eq!(exception["status"], 406);
}
