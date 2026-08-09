// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `describe()` against the fixtures' committed manifest (issue #22): every
//! field the adapter reports must match what GDAL wrote and recorded.

mod common;

use std::collections::BTreeMap;

use swath_core::raster::AssetRef;
use swath_core::source::{RasterSource, SourceError};

/// One fixture's entry in tests/fixtures/manifest.json.
#[derive(serde::Deserialize)]
struct ManifestEntry {
    count: u32,
    crs: String,
    dtype: String,
    height: u64,
    nodata: f64,
    /// Rasterio affine order: a, b, c, d, e, f =
    /// `pixel_width`, `row_rotation`, `origin_x`, `col_rotation`,
    /// `pixel_height`, `origin_y`.
    transform: [f64; 6],
    width: u64,
}

fn manifest() -> BTreeMap<String, ManifestEntry> {
    let raw = std::fs::read_to_string(common::fixtures_dir().join("manifest.json"))
        .expect("manifest.json readable");
    serde_json::from_str(&raw).expect("manifest.json parses")
}

#[tokio::test]
async fn describe_matches_fixture_manifest() {
    let source = common::local_source();
    let entries = manifest();
    assert_eq!(entries.len(), 5, "expected the five HLS fixtures");
    for (file, want) in entries {
        let info = source
            .describe(&AssetRef::new(&file))
            .await
            .unwrap_or_else(|e| panic!("describe({file}) failed: {e}"));
        assert_eq!(info.width, want.width, "{file}: width");
        assert_eq!(info.height, want.height, "{file}: height");
        assert_eq!(info.band_count, want.count, "{file}: band count");
        assert_eq!(
            info.crs.to_string(),
            want.crs,
            "{file}: CRS (manifest is EPSG-prefixed)"
        );
        assert_eq!(
            info.dtype,
            common::dtype_from_str(&want.dtype),
            "{file}: dtype"
        );
        assert_eq!(info.nodata, Some(want.nodata), "{file}: nodata");
        // Exact float equality is the point: the manifest records the values
        // GDAL wrote into the file, and the adapter must reproduce them
        // bit-for-bit (they are exact in IEEE 754: 30.0, 453720.0, ...).
        #[allow(clippy::float_cmp, reason = "geotransform round-trip must be exact")]
        {
            let [pw, rr, ox, cr, ph, oy] = want.transform;
            assert_eq!(info.transform.pixel_width, pw, "{file}: pixel_width");
            assert_eq!(info.transform.row_rotation, rr, "{file}: row_rotation");
            assert_eq!(info.transform.origin_x, ox, "{file}: origin_x");
            assert_eq!(info.transform.col_rotation, cr, "{file}: col_rotation");
            assert_eq!(info.transform.pixel_height, ph, "{file}: pixel_height");
            assert_eq!(info.transform.origin_y, oy, "{file}: origin_y");
        }
        // Every fixture is a proper COG with exactly one overview level
        // (256x256 = decimation 2; tests/fixtures/README.md).
        assert_eq!(info.overview_levels, vec![2], "{file}: overview levels");
    }
}

#[tokio::test]
async fn describe_agrees_between_local_and_memory_stores() {
    let local = common::local_source();
    let memory = common::memory_source().await;
    for file in manifest().keys() {
        let asset = AssetRef::new(file);
        let a = local.describe(&asset).await.expect("local describe");
        let b = memory.describe(&asset).await.expect("memory describe");
        assert_eq!(a, b, "{file}: local and in-memory describe disagree");
    }
}

#[tokio::test]
async fn describe_missing_asset_is_not_found() {
    let source = common::local_source();
    let err = source
        .describe(&AssetRef::new("does-not-exist.tif"))
        .await
        .expect_err("missing asset must fail");
    assert!(
        matches!(err, SourceError::NotFound { .. }),
        "expected NotFound, got {err:?}"
    );
}
