// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Integration tests over the committed HLS fixture COGs (issue #183):
//! materialization is idempotent and resumable, the overlay advertises
//! and serves exactly the completed levels, embedded factors keep
//! delegating to the asset, and generated pixels equal an independently
//! computed decimation of the base grid.

use std::sync::Arc;

use swath_testsupport::paths::fixtures_dir;

use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt as _, PutPayload};
use swath_core::raster::{AssetRef, WindowRequest};
use swath_core::source::{BandSelection, PixelBuffer, RasterSource, ReadLevel};
use swath_pyramid_objectstore::{
    MaterializeError, MaterializeSpec, PyramidResampling, PyramidSource, layout,
};
use swath_source_cog::CogSource;

/// The committed single-date HLS fixture: 512x512, int16, nodata -9999,
/// one embedded x2 overview (average).
const B04: &str = "hlss30-t13sdd-2024158-b04.tif";
/// The categorical Fmask fixture: uint8, nodata 255, one embedded x2
/// overview (nearest).
const FMASK: &str = "hlss30-t13sdd-2024158-fmask.tif";

/// An in-memory store preloaded with the named fixtures, and the pyramid
/// overlay over a COG source reading from it.
async fn source(fixtures: &[&str]) -> (PyramidSource<CogSource>, Arc<dyn ObjectStore>) {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    for name in fixtures {
        let bytes = std::fs::read(fixtures_dir().join(name)).expect("fixture exists");
        store
            .put(&Path::from(*name), PutPayload::from(bytes))
            .await
            .expect("preload succeeds");
    }
    (
        PyramidSource::new(CogSource::new(Arc::clone(&store)), Arc::clone(&store)),
        store,
    )
}

/// A small spec that forces levels beyond the fixtures' embedded x2:
/// 512 -> ladder [2, 4, 8], minus embedded [2] = materialize [4, 8].
fn spec(resampling: PyramidResampling) -> MaterializeSpec {
    MaterializeSpec {
        min_dim: 64,
        ..MaterializeSpec::with_resampling(resampling)
    }
}

fn i16_pixels(pixels: &PixelBuffer) -> &[i16] {
    match pixels {
        PixelBuffer::Int16(v) => v,
        other => panic!("expected Int16 pixels, got {other:?}"),
    }
}

#[tokio::test]
async fn materialize_builds_missing_levels_and_is_idempotent() {
    let (source, _) = source(&[B04]).await;
    let asset = AssetRef::new(B04);

    let report = source
        .materialize(&asset, &spec(PyramidResampling::Average))
        .await
        .expect("materializes");
    assert_eq!(report.factors_completed, vec![4, 8]);
    assert!(report.factors_already_complete.is_empty());
    // Level 4 is 128x128 (one 256-chunk), level 8 is 64x64 (one chunk).
    assert_eq!(report.chunks_written, 2);
    assert_eq!(report.chunks_skipped, 0);

    // The rerun is a no-op: same ladder, everything already complete.
    let rerun = source
        .materialize(&asset, &spec(PyramidResampling::Average))
        .await
        .expect("rerun succeeds");
    assert!(rerun.factors_completed.is_empty());
    assert_eq!(rerun.factors_already_complete, vec![4, 8]);
    assert_eq!(rerun.chunks_written, 0);
    assert_eq!(rerun.chunks_skipped, 0, "complete levels are skipped whole");
}

#[tokio::test]
async fn describe_merges_materialized_factors_with_embedded() {
    let (source, _) = source(&[B04]).await;
    let asset = AssetRef::new(B04);
    let before = source.describe(&asset).await.expect("describe");
    assert_eq!(before.overview_levels, vec![2], "fixture embeds only x2");

    source
        .materialize(&asset, &spec(PyramidResampling::Average))
        .await
        .expect("materializes");
    let after = source.describe(&asset).await.expect("describe");
    assert_eq!(
        after.overview_levels,
        vec![2, 4, 8],
        "embedded and materialized factors merge, ascending"
    );
    // Everything else is untouched.
    assert_eq!(after.width, before.width);
    assert_eq!(after.transform, before.transform);
    assert_eq!(after.dtype, before.dtype);
}

#[tokio::test]
async fn materialized_level_matches_independent_decimation_of_its_base() {
    let (source, _) = source(&[B04]).await;
    let asset = AssetRef::new(B04);
    source
        .materialize(&asset, &spec(PyramidResampling::Average))
        .await
        .expect("materializes");

    // Read the whole materialized x4 level.
    let full = WindowRequest {
        col_off: 0,
        row_off: 0,
        width: 512,
        height: 512,
    };
    let level = source
        .read_window(
            &asset,
            full,
            BandSelection::Single(0),
            ReadLevel::Overview { factor: 4 },
        )
        .await
        .expect("level read");
    assert_eq!((level.grid.width, level.grid.height), (128, 128));
    assert_eq!(level.window.width, 128);
    assert_eq!(level.nodata, Some(-9999.0));
    assert!(
        level
            .provenance
            .iter()
            .all(|p| p.path.starts_with("pyramids/")),
        "materialized reads fetch pyramid chunks: {:?}",
        level.provenance
    );
    // The x4 transform is the full-res transform scaled by exactly 4.
    let info = source.describe(&asset).await.expect("describe");
    assert!((level.grid.transform.pixel_width - info.transform.pixel_width * 4.0).abs() < 1e-9);

    // Independent oracle: the x4 level was built from the embedded x2
    // overview (the coarsest available divisor); recompute the
    // nodata-aware 2x2 block mean from that grid and compare exactly.
    let base = source
        .read_window(
            &asset,
            full,
            BandSelection::Single(0),
            ReadLevel::Overview { factor: 2 },
        )
        .await
        .expect("base read");
    assert_eq!((base.window.width, base.window.height), (256, 256));
    let base_px = i16_pixels(&base.pixels);
    let level_px = i16_pixels(&level.pixels);
    for row in 0..128_usize {
        for col in 0..128_usize {
            let mut sum = 0.0_f64;
            let mut count = 0_u32;
            for dr in 0..2 {
                for dc in 0..2 {
                    let v = base_px[(row * 2 + dr) * 256 + (col * 2 + dc)];
                    if v != -9999 {
                        sum += f64::from(v);
                        count += 1;
                    }
                }
            }
            let expected = if count == 0 {
                -9999.0
            } else {
                (sum / f64::from(count)).round()
            };
            assert!(
                (f64::from(level_px[row * 128 + col]) - expected).abs() < 1.0,
                "pixel ({row},{col}): got {}, expected {expected}",
                level_px[row * 128 + col]
            );
        }
    }
}

#[tokio::test]
async fn embedded_factor_still_delegates_to_the_asset() {
    let (source, _) = source(&[B04]).await;
    let asset = AssetRef::new(B04);
    source
        .materialize(&asset, &spec(PyramidResampling::Average))
        .await
        .expect("materializes");

    let window = WindowRequest {
        col_off: 0,
        row_off: 0,
        width: 512,
        height: 512,
    };
    let data = source
        .read_window(
            &asset,
            window,
            BandSelection::Single(0),
            ReadLevel::Overview { factor: 2 },
        )
        .await
        .expect("embedded read");
    assert!(
        data.provenance.iter().all(|p| p.path == B04),
        "embedded factors read the asset itself: {:?}",
        data.provenance
    );
}

#[tokio::test]
async fn interrupted_run_resumes_writing_only_whats_missing() {
    let (source, store) = source(&[B04]).await;
    let asset = AssetRef::new(B04);
    source
        .materialize(&asset, &spec(PyramidResampling::Average))
        .await
        .expect("materializes");

    // Simulate a run killed mid-level: its chunk is gone and its factor
    // was never recorded as complete.
    let root = layout::pyramid_root(B04);
    store
        .delete(&Path::from(layout::chunk_path(&root, 8, 0, 0)))
        .await
        .expect("delete chunk");
    let attrs_path = Path::from(layout::zattrs_path(&root));
    let attrs = store
        .get(&attrs_path)
        .await
        .expect("attrs")
        .bytes()
        .await
        .expect("bytes");
    let mut doc: serde_json::Value = serde_json::from_slice(&attrs).expect("parses");
    doc["swath:pyramid"]["completed"] = serde_json::json!([4]);
    store
        .put(
            &attrs_path,
            PutPayload::from(serde_json::to_vec(&doc).expect("serializes")),
        )
        .await
        .expect("rewrite attrs");

    let resumed = source
        .materialize(&asset, &spec(PyramidResampling::Average))
        .await
        .expect("resumes");
    assert_eq!(resumed.factors_already_complete, vec![4]);
    assert_eq!(resumed.factors_completed, vec![8]);
    assert_eq!(resumed.chunks_written, 1, "exactly the missing chunk");

    // And the level serves again.
    let describe = source.describe(&asset).await.expect("describe");
    assert_eq!(describe.overview_levels, vec![2, 4, 8]);
}

#[tokio::test]
async fn stale_pyramid_is_not_advertised_and_refuses_resume() {
    let (source, store) = source(&[B04]).await;
    let asset = AssetRef::new(B04);
    source
        .materialize(&asset, &spec(PyramidResampling::Average))
        .await
        .expect("materializes");

    // Corrupt the identity: pretend the pyramid was built from a
    // different grid (the asset was swapped under its URI).
    let root = layout::pyramid_root(B04);
    let attrs_path = Path::from(layout::zattrs_path(&root));
    let attrs = store
        .get(&attrs_path)
        .await
        .expect("attrs")
        .bytes()
        .await
        .expect("bytes");
    let mut doc: serde_json::Value = serde_json::from_slice(&attrs).expect("parses");
    doc["swath:pyramid"]["width"] = serde_json::json!(9999);
    store
        .put(
            &attrs_path,
            PutPayload::from(serde_json::to_vec(&doc).expect("serializes")),
        )
        .await
        .expect("rewrite attrs");

    // Serving degrades to the inner source: only the embedded factor.
    let info = source.describe(&asset).await.expect("describe");
    assert_eq!(info.overview_levels, vec![2]);

    // Materialization refuses loudly rather than mixing generations.
    let err = source
        .materialize(&asset, &spec(PyramidResampling::Average))
        .await
        .expect_err("conflict");
    assert!(
        matches!(err, MaterializeError::Conflict { .. }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn nearest_resampling_decimates_categories_without_inventing_values() {
    let (source, _) = source(&[FMASK]).await;
    let asset = AssetRef::new(FMASK);
    let report = source
        .materialize(&asset, &spec(PyramidResampling::Nearest))
        .await
        .expect("materializes");
    assert_eq!(report.factors_completed, vec![4, 8]);

    let full = WindowRequest {
        col_off: 0,
        row_off: 0,
        width: 512,
        height: 512,
    };
    let level = source
        .read_window(
            &asset,
            full,
            BandSelection::Single(0),
            ReadLevel::Overview { factor: 4 },
        )
        .await
        .expect("level read");
    let base = source
        .read_window(
            &asset,
            full,
            BandSelection::Single(0),
            ReadLevel::Overview { factor: 2 },
        )
        .await
        .expect("base read");
    let (PixelBuffer::UInt8(level_px), PixelBuffer::UInt8(base_px)) = (&level.pixels, &base.pixels)
    else {
        panic!("fmask is uint8");
    };
    // Nearest: every level sample is the top-left of its 2x2 base block —
    // never an averaged (invented) class value.
    for row in 0..128_usize {
        for col in 0..128_usize {
            assert_eq!(
                level_px[row * 128 + col],
                base_px[(row * 2) * 256 + col * 2],
                "pixel ({row},{col})"
            );
        }
    }
}

#[tokio::test]
async fn full_res_reads_pass_through_untouched() {
    let (source, _) = source(&[B04]).await;
    let asset = AssetRef::new(B04);
    source
        .materialize(&asset, &spec(PyramidResampling::Average))
        .await
        .expect("materializes");
    let window = WindowRequest {
        col_off: 10,
        row_off: 10,
        width: 32,
        height: 32,
    };
    let data = source
        .read_window(&asset, window, BandSelection::Single(0), ReadLevel::FullRes)
        .await
        .expect("full-res read");
    assert_eq!(data.window, window);
    assert!(data.provenance.iter().all(|p| p.path == B04));
}
