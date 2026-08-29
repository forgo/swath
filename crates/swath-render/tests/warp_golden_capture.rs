// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Regenerates `swath-warp`'s self-contained oracle goldens (#186) —
//! ignored by default; run explicitly after a deliberate kernel change:
//!
//! ```text
//! cargo test -p swath-render --test warp_golden_capture -- --ignored
//! ```
//!
//! The extracted kernel must stay provable **outside** this workspace,
//! where the COG reader and the proj4rs adapter don't exist. This capture
//! freezes everything a warp consumes into flat binary fixtures under
//! `crates/swath-warp/tests/data/`:
//!
//! * `tile-<z>-<x>-<y>.xf` — every point the kernel asks the
//!   `CoordTransform` for, **in call order**, as transformed by the real
//!   proj4rs 3857→UTM adapter (the same transform the PNG golden suite
//!   uses). One file per tile: the call sequence is grid-driven, so every
//!   band of a tile records the identical sequence (asserted here).
//! * `<band>-<z>-<x>-<y>.src` — the source window the COG reader returned
//!   (placement, raw samples, nodata, grid) plus the tile's target grid.
//! * `<band>-<z>-<x>-<y>.out` — the warped values + validity produced by
//!   this workspace's pipeline, which `tests/golden.rs` holds to the
//!   GDAL/rio-tiler oracle. Regenerate only while that suite is green:
//!   its passing is what makes these bytes oracle-anchored.
//!
//! `swath-warp`'s `tests/golden.rs` replays the `.xf` sequence through its
//! own transform port and requires **bit-identical** output — a stricter
//! bar than the perceptual-diff PNG suite, valid because both sides run
//! the same recorded inputs.

use std::io::Write as _;

use std::sync::{Arc, Mutex};
use swath_testsupport::paths::{fixtures_dir, warp_data_dir};

use object_store::local::LocalFileSystem;
use swath_core::crs::Crs;
use swath_core::raster::AssetRef;
use swath_core::reproject::{CoordTransform, Reproject as _, ReprojectError};
use swath_core::source::{BandSelection, PixelBuffer, RasterSource as _, ReadLevel};
use swath_core::tile::TileCoord;
use swath_render::{NodataPolicy, Resampling, TargetGrid, source_window, tile_grid, warp};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;

/// The margin the PNG golden suite uses (`tests/common/mod.rs`).
const WINDOW_MARGIN: u32 = 4;

const B04: &str = "hlss30-t13sdd-2024158-b04.tif";
const FMASK: &str = "hlss30-t13sdd-2024158-fmask.tif";

/// Records every successful transform output in call order — the exact
/// sequence `swath-warp`'s replay transform will play back.
struct Recording {
    inner: Box<dyn CoordTransform>,
    log: Mutex<Vec<(f64, f64)>>,
}

impl CoordTransform for Recording {
    fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
        let out = self.inner.transform(x, y)?;
        self.log.lock().expect("log lock").push(out);
        Ok(out)
    }

    fn transform_slice(&self, points: &mut [(f64, f64)]) -> Result<(), ReprojectError> {
        self.inner.transform_slice(points)?;
        self.log.lock().expect("log lock").extend_from_slice(points);
        Ok(())
    }
}

fn put_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn put_f64(buf: &mut Vec<u8>, v: f64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

#[allow(clippy::print_stdout, reason = "capture progress is the tool's report")]
fn write_file(name: &str, bytes: &[u8]) {
    let path = warp_data_dir().join(name);
    let mut f = std::fs::File::create(&path).expect("create golden file");
    f.write_all(bytes).expect("write golden file");
    println!("wrote {} ({} bytes)", path.display(), bytes.len());
}

struct Case {
    fixture: &'static str,
    band: &'static str,
    tile: (u8, u32, u32),
    resampling: Resampling,
}

async fn capture(case: &Case) -> Vec<(f64, f64)> {
    let (z, x, y) = case.tile;
    let store = LocalFileSystem::new_with_prefix(fixtures_dir()).expect("fixture dir exists");
    let source = CogSource::new(Arc::new(store));
    let asset = AssetRef::new(case.fixture);
    let info = source.describe(&asset).await.expect("describe fixture");

    let recording = Recording {
        inner: Proj4rsReproject
            .transformer(&Crs::WEB_MERCATOR, &info.crs)
            .expect("3857 -> fixture UTM transform"),
        log: Mutex::new(Vec::new()),
    };

    let tile = TileCoord::new(z, x, y).expect("valid tile");
    let grid = tile_grid(tile, 256);
    let window = source_window(&grid, &info, &recording, WINDOW_MARGIN)
        .expect("window computation")
        .expect("fixture tiles intersect the raster");
    let data = source
        .read_window(&asset, window, BandSelection::Single(0), ReadLevel::FullRes)
        .await
        .expect("read window");
    let warped = warp(&data, &recording, &grid, case.resampling).expect("warp");

    // .src — the source window + target grid, everything but the transform.
    let mut src = Vec::new();
    put_u64(&mut src, data.window.col_off);
    put_u64(&mut src, data.window.row_off);
    put_u64(&mut src, data.window.width);
    put_u64(&mut src, data.window.height);
    match &data.pixels {
        PixelBuffer::UInt8(v) => {
            src.push(1);
            src.push(u8::from(data.nodata.is_some()));
            put_f64(&mut src, data.nodata.unwrap_or(0.0));
            header_tail(&mut src, &data.grid, &grid);
            src.extend_from_slice(v);
        }
        PixelBuffer::Int16(v) => {
            src.push(2);
            src.push(u8::from(data.nodata.is_some()));
            put_f64(&mut src, data.nodata.unwrap_or(0.0));
            header_tail(&mut src, &data.grid, &grid);
            for s in v {
                src.extend_from_slice(&s.to_le_bytes());
            }
        }
        other => panic!("unhandled fixture dtype {:?}", other.dtype()),
    }
    write_file(&format!("{}-{z}-{x}-{y}.src", case.band), &src);

    // .out — the oracle-anchored expected warp result, bit for bit.
    let mut out = Vec::new();
    put_u32(&mut out, warped.width);
    put_u32(&mut out, warped.height);
    for v in &warped.values {
        put_f64(&mut out, *v);
    }
    out.extend(warped.valid.iter().map(|v| u8::from(*v)));
    write_file(&format!("{}-{z}-{x}-{y}.out", case.band), &out);

    recording.log.into_inner().expect("log lock")
}

/// Raster grid (dims + geotransform) and target grid (bounds + size).
fn header_tail(buf: &mut Vec<u8>, info: &swath_core::raster::RasterInfo, grid: &TargetGrid) {
    put_u64(buf, info.width);
    put_u64(buf, info.height);
    let gt = &info.transform;
    for v in [
        gt.origin_x,
        gt.pixel_width,
        gt.row_rotation,
        gt.origin_y,
        gt.col_rotation,
        gt.pixel_height,
    ] {
        put_f64(buf, v);
    }
    let b = grid.bounds();
    for v in [b.min_x, b.min_y, b.max_x, b.max_y] {
        put_f64(buf, v);
    }
    put_u32(buf, grid.width());
    put_u32(buf, grid.height());
}

#[tokio::test]
#[ignore = "regenerates crates/swath-warp/tests/data — run only while tests/golden.rs is green"]
async fn capture_swath_warp_goldens() {
    std::fs::create_dir_all(warp_data_dir()).expect("data dir");

    let bilinear = Resampling::Bilinear(NodataPolicy::ExcludeRenormalize);

    // z12 swath-edge tile (the real nodata test), both kernels; z11 parent
    // (decimation: the scaled-triangle path at scales 256/320 and 256/381).
    let groups: [&[Case]; 2] = [
        &[
            Case {
                fixture: B04,
                band: "b04",
                tile: (12, 848, 1562),
                resampling: bilinear,
            },
            Case {
                fixture: FMASK,
                band: "fmask",
                tile: (12, 848, 1562),
                resampling: Resampling::Nearest,
            },
        ],
        &[Case {
            fixture: B04,
            band: "b04",
            tile: (11, 424, 780),
            resampling: bilinear,
        }],
    ];

    for cases in groups {
        let mut first: Option<Vec<(f64, f64)>> = None;
        for case in cases {
            let log = capture(case).await;
            match &first {
                None => {
                    let (z, x, y) = case.tile;
                    let mut xf = Vec::new();
                    put_u64(&mut xf, log.len() as u64);
                    for (px, py) in &log {
                        put_f64(&mut xf, *px);
                        put_f64(&mut xf, *py);
                    }
                    write_file(&format!("tile-{z}-{x}-{y}.xf"), &xf);
                    first = Some(log);
                }
                Some(reference) => assert_eq!(
                    reference, &log,
                    "transform call sequence must be identical for every band of a tile"
                ),
            }
        }
    }
}
