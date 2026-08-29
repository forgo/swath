// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// `mod common` is compiled once per test binary and each binary uses a
// subset of it — one allow here instead of one per binary (#348).
#![allow(
    dead_code,
    reason = "compiled once per test binary; each uses a subset"
)]

//! Shared plumbing for the golden tests: the full fixture → tile pipeline
//! (COG read → reproject → warp → oracle-identical grayscale encode).

use std::sync::Arc;

use object_store::local::LocalFileSystem;
use swath_core::crs::Crs;
use swath_core::raster::AssetRef;
use swath_core::reproject::Reproject as _;
use swath_core::source::{BandSelection, RasterSource as _, ReadLevel};
use swath_core::tile::TileCoord;
use swath_render::{Resampling, TargetGrid, WarpedBuffer, source_window, warp};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;
use swath_testsupport::RgbaImage;
#[allow(
    unused_imports,
    reason = "shared between the render test binaries; not every one uses each"
)]
pub(crate) use swath_testsupport::paths::{fixtures_dir, render_goldens_dir as goldens_dir};

/// Pixel margin used for source windows: the resampling support. Bilinear
/// needs 1 pixel at scale >= 1, and `ceil(1/scale) + 1` when the warp
/// decimates — 4 covers every tile in this suite (worst case: z11, Y scale
/// 0.5, radius 2).
pub(crate) const WINDOW_MARGIN: u32 = 4;

/// Renders one 256-px XYZ tile of a fixture band through the full Swath
/// pipeline and returns the warped buffer plus the warp wall time.
pub(crate) async fn render_warped(
    fixture: &str,
    tile: TileCoord,
    resampling: Resampling,
) -> (WarpedBuffer, Option<f64>, std::time::Duration) {
    let store = LocalFileSystem::new_with_prefix(fixtures_dir()).expect("fixture dir exists");
    let source = CogSource::new(Arc::new(store));
    let asset = AssetRef::new(fixture);
    let info = source.describe(&asset).await.expect("describe fixture");

    let to_source = Proj4rsReproject
        .transformer(&Crs::WEB_MERCATOR, &info.crs)
        .expect("3857 -> fixture UTM transform");

    let grid = TargetGrid::for_tile(tile, 256);
    let window = source_window(&grid, &info, to_source.as_ref(), WINDOW_MARGIN)
        .expect("window computation")
        .expect("fixture tiles intersect the raster");
    let data = source
        .read_window(&asset, window, BandSelection::Single(0), ReadLevel::FullRes)
        .await
        .expect("read window");

    let started = std::time::Instant::now();
    let warped = warp(&data, to_source.as_ref(), &grid, resampling).expect("warp");
    (warped, info.nodata, started.elapsed())
}

/// Encodes a warped buffer exactly the way the oracle encodes its PNG:
/// invalid pixels carry the nodata sentinel through the same pipeline
/// (GDAL initializes the warp destination with nodata), then everything is
/// optionally rescaled `in_range -> 0..255` with numpy's semantics
/// (clip, scale, truncate toward zero) and alpha is 255/0 by validity.
pub(crate) fn encode_like_oracle(
    warped: &WarpedBuffer,
    nodata: Option<f64>,
    rescale: Option<(f64, f64)>,
) -> RgbaImage {
    let fill = nodata.unwrap_or(0.0);
    let mut raw_rgba = Vec::with_capacity(warped.values.len() * 4);
    for (idx, &valid) in warped.valid.iter().enumerate() {
        let raw = if valid { warped.values[idx] } else { fill };
        let gray = match rescale {
            Some((lo, hi)) => (raw.clamp(lo, hi) - lo) / (hi - lo) * 255.0,
            None => raw,
        };
        // numpy `astype(uint8)` semantics: truncation toward zero.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped into 0..=255 before the cast"
        )]
        let gray = gray.clamp(0.0, 255.0) as u8;
        let alpha = if valid { 255 } else { 0 };
        raw_rgba.extend_from_slice(&[gray, gray, gray, alpha]);
    }
    RgbaImage::from_raw(warped.width, warped.height, raw_rgba)
        .expect("buffer length matches dimensions")
}
