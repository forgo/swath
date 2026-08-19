// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Render-stage criterion suites (issue #100; ENGINEERING.md §2's criterion
//! mandate for the benchmarks that gate the north-star latency budget).
//!
//! Every input is a committed HLS fixture (`tests/fixtures`, ADR 0004) —
//! nothing is fetched. Stage benches (`warp`, `eval`, `encode_png`,
//! `source_window`) run over real pixel data prepared once from the
//! interior z12 golden tile (12/848/1561), so the measured inner loops see
//! honest value distributions, not synthetic flat buffers. The composite
//! bench awaits the real `render_tile` future through criterion's tokio
//! support (see [`bench_composite`] for why async, not sync composition)
//! over an in-memory `object_store` preloaded with the fixture bytes — the
//! adapters' test pattern — so no filesystem I/O rides the measurement.
//!
//! Baselines: `just bench` runs these; `just bench-baseline` distills the
//! criterion estimates into `docs/perf/bench-baseline.json`.

// criterion's group/main macros generate undocumented items.
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use object_store::ObjectStoreExt as _;
use object_store::memory::InMemory;
use object_store::path::Path as StorePath;
use swath_core::crs::Crs;
use swath_core::raster::{AssetRef, RasterInfo};
use swath_core::reproject::{CoordTransform, Reproject as _};
use swath_core::source::{BandSelection, RasterSource as _, ReadLevel, WindowData};
use swath_core::tile::TileCoord;
use swath_render::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::{
    NoUdf, NodataPolicy, Resampling, RgbaTile, TargetGrid, TileRequest, WarpedBuffer, encode_png,
    eval, render_tile, source_window, warp,
};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;
use tokio::runtime::Runtime;

const B04: &str = "hlss30-t13sdd-2024158-b04.tif";
const B8A: &str = "hlss30-t13sdd-2024158-b8a.tif";

/// The interior z12 golden tile — the north-star serve shape: a live
/// full-resolution render of a 256 px tile.
const TILE: (u8, u32, u32) = (12, 848, 1561);

/// The reflectance-band kernel, as in the golden suites.
const BILINEAR: Resampling = Resampling::Bilinear(NodataPolicy::ExcludeRenormalize);

/// Source-window margin covering the resampling support (the golden
/// suites' constant).
const WINDOW_MARGIN: u32 = 4;

/// The committed HLS fixture directory (tests/fixtures/README.md, ADR 0004).
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// A `CogSource` over an in-memory store preloaded with the fixture bytes,
/// so benched renders never touch the filesystem (the adapters' pattern).
async fn memory_source() -> CogSource {
    let store = InMemory::new();
    for name in [B04, B8A] {
        let bytes = std::fs::read(fixtures_dir().join(name)).expect("fixture readable");
        store
            .put(&StorePath::from(name), bytes.into())
            .await
            .expect("put fixture into memory store");
    }
    CogSource::new(Arc::new(store))
}

/// Inputs shared by the stage benches, prepared once outside measurement:
/// the tile grid, the target→source transform, and the full-resolution
/// fixture windows the z12 tile reads.
struct Prepared {
    grid: TargetGrid,
    info: RasterInfo,
    to_source: Box<dyn CoordTransform>,
    b04: WindowData,
    b8a: WindowData,
}

fn prepare(rt: &Runtime) -> Prepared {
    rt.block_on(async {
        let source = memory_source().await;
        let asset = AssetRef::new(B04);
        let info = source.describe(&asset).await.expect("describe fixture");
        let to_source = Proj4rsReproject
            .transformer(&Crs::WEB_MERCATOR, &info.crs)
            .expect("3857 -> fixture UTM transform");
        let (z, x, y) = TILE;
        let grid = TargetGrid::for_tile(TileCoord::new(z, x, y).expect("valid tile"), 256);
        let window = source_window(&grid, &info, to_source.as_ref(), WINDOW_MARGIN)
            .expect("window computation")
            .expect("fixture tile intersects the raster");
        let read = |name: &'static str| {
            let source = &source;
            async move {
                source
                    .read_window(
                        &AssetRef::new(name),
                        window,
                        BandSelection::Single(0),
                        ReadLevel::FullRes,
                    )
                    .await
                    .expect("read window")
            }
        };
        let b04 = read(B04).await;
        let b8a = read(B8A).await;
        Prepared {
            grid,
            info,
            to_source,
            b04,
            b8a,
        }
    })
}

/// The NDVI plan of the golden suites — band math, rescale, and the given
/// colormap (post-#94 the `RdYlGn` LUT path is part of the serve shape).
fn ndvi_plan(colormap: Colormap) -> RenderPlan {
    RenderPlan::new(
        vec![BandInput::new("b8a"), BandInput::new("b04")],
        vec![
            PixelOp::BandMath(
                (Expr::band("b8a") - Expr::band("b04")) / (Expr::band("b8a") + Expr::band("b04")),
            ),
            PixelOp::Rescale {
                min: -1.0,
                max: 1.0,
            },
            PixelOp::Colormap(colormap),
        ],
        OutputSpec::new(TileFormat::Png),
    )
}

/// Warped NDVI input planes (b8a, b04 — the plan's declaration order).
fn ndvi_inputs(p: &Prepared) -> Vec<WarpedBuffer> {
    [&p.b8a, &p.b04]
        .into_iter()
        .map(|w| warp(w, p.to_source.as_ref(), &p.grid, BILINEAR).expect("warp"))
        .collect()
}

/// A realistic encoded tile input: the fully rendered colormapped NDVI
/// tile, so the deflate stage sees real spatial structure, not flat color.
fn rendered_ndvi_tile(p: &Prepared) -> RgbaTile {
    eval(&ndvi_plan(Colormap::RdYlGn), &ndvi_inputs(p), &NoUdf).expect("eval")
}

/// Stage 1 — the warp inner loop (`warp.rs`): a representative full-res
/// window resampled onto the 256 px tile grid, bilinear and nearest.
fn bench_warp(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let p = prepare(&rt);
    c.bench_function("warp_bilinear_fullres_z12", |b| {
        b.iter(|| warp(black_box(&p.b04), p.to_source.as_ref(), &p.grid, BILINEAR));
    });
    c.bench_function("warp_nearest_fullres_z12", |b| {
        b.iter(|| {
            warp(
                black_box(&p.b04),
                p.to_source.as_ref(),
                &p.grid,
                Resampling::Nearest,
            )
        });
    });
}

/// Stage 2 — IR eval (`ir.rs`): NDVI band math + rescale over the warped
/// planes, with the grayscale identity map and with the `RdYlGn` LUT (#94),
/// so the palette lookup cost is visible next to the gray path.
fn bench_eval(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let p = prepare(&rt);
    let inputs = ndvi_inputs(&p);
    let gray = ndvi_plan(Colormap::Grayscale);
    let rdylgn = ndvi_plan(Colormap::RdYlGn);
    c.bench_function("eval_ndvi_grayscale", |b| {
        b.iter(|| eval(black_box(&gray), black_box(&inputs), &NoUdf));
    });
    c.bench_function("eval_ndvi_rdylgn", |b| {
        b.iter(|| eval(black_box(&rdylgn), black_box(&inputs), &NoUdf));
    });
}

/// Stage 3 — PNG encode (`encode.rs`) of the real rendered 256×256 NDVI
/// tile: honest deflate timing over real spatial structure.
fn bench_encode(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let p = prepare(&rt);
    let tile = rendered_ndvi_tile(&p);
    c.bench_function("encode_png_ndvi_256", |b| {
        b.iter(|| encode_png(black_box(&tile)));
    });
}

/// Stage 4 — source-window computation (`window.rs`): the densified
/// boundary sampling that turns a tile grid into a read request.
fn bench_window(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let p = prepare(&rt);
    c.bench_function("source_window_z12", |b| {
        b.iter(|| {
            source_window(
                black_box(&p.grid),
                black_box(&p.info),
                p.to_source.as_ref(),
                WINDOW_MARGIN,
            )
        });
    });
}

/// Stage 5 — the end-to-end composite: a full `render_tile` of the fixture
/// NDVI tile (describe → window → read → warp → eval → encode).
///
/// Benched **async** via criterion's tokio support rather than composing
/// the sync stages: `render_tile` is the serve path's actual entry point,
/// and a sync re-composition would silently diverge from its orchestration
/// (concurrent band reads, trace assembly) and rot as the tiler evolves.
/// The runtime is the multi-thread scheduler the API server uses; source
/// bytes come from the in-memory store, so the measurement is CPU + async
/// orchestration, never disk.
fn bench_composite(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let source = rt.block_on(memory_source());
    let (z, x, y) = TILE;
    let request = TileRequest::new(
        BTreeMap::from([
            ("b8a".to_owned(), AssetRef::new(B8A)),
            ("b04".to_owned(), AssetRef::new(B04)),
        ]),
        ndvi_plan(Colormap::RdYlGn),
        TileCoord::new(z, x, y).expect("valid tile"),
        256,
        BILINEAR,
    );
    let mut group = c.benchmark_group("composite");
    // A full render is tens of milliseconds; fewer samples keep the suite
    // fast without drowning the median in noise.
    group.sample_size(30);
    group.bench_function("render_tile_ndvi_z12", |b| {
        b.to_async(&rt).iter(|| async {
            render_tile(
                black_box(&source),
                black_box(&Proj4rsReproject),
                black_box(&request),
            )
            .await
            .expect("render_tile succeeds")
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_warp,
    bench_eval,
    bench_encode,
    bench_window,
    bench_composite
);
criterion_main!(benches);
