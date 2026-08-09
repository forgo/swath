// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `describe()` against the fixture's georef truth, and the addressing /
//! error taxonomy of the `<manifest-key>#<array-name>` convention.
//!
//! The truth is `tiny.expected.json` (swath-referencer's test data),
//! whose georef entries are derived by the fixture maker from its OWN
//! `StructMetadata` constants — independent of both the Rust parser and
//! this adapter.

mod common;

use swath_core::crs::Crs;
use swath_core::raster::{AssetRef, DType};
use swath_core::source::{RasterSource, SourceError};
use swath_source_virtual::VirtualSource;

/// The georef truth of one expected-manifest array, straight from the
/// committed JSON.
fn expected_georef(array: &str) -> swath_core::manifest::Georef {
    let path = common::fixture_path().with_file_name("tiny.expected.json");
    let text = std::fs::read_to_string(path).expect("expected json readable");
    let manifest =
        swath_core::manifest::VirtualManifest::from_json_str(&text).expect("expected json parses");
    manifest
        .arrays
        .iter()
        .find(|a| a.name == array)
        .unwrap_or_else(|| panic!("array `{array}`"))
        .georef
        .clone()
        .expect("array is georeferenced")
}

#[tokio::test]
async fn describe_matches_the_h5py_derived_georef_truth() {
    let source = common::memory_source().await;
    for array in [common::NIR, common::RED] {
        let info = source.describe(&common::asset(array)).await.unwrap();
        let truth = expected_georef(array);

        assert_eq!(info.crs, Crs::from(&truth.crs), "{array}: crs");
        assert!(
            matches!(&info.crs, Crs::Proj4(s) if s.starts_with("+proj=sinu")),
            "{array}: sinusoidal proj-string CRS"
        );
        assert_eq!(info.transform, truth.transform, "{array}: geotransform");
        assert_eq!(info.nodata, truth.nodata, "{array}: nodata");
        assert_eq!((info.width, info.height), (7, 8), "{array}: dims");
        assert_eq!(info.dtype, DType::Int16, "{array}: dtype");
        assert_eq!(info.band_count, 1, "{array}: one band per addressed array");
        // Virtual cubes have no overview pyramids — reported honestly.
        assert!(info.overview_levels.is_empty(), "{array}: overviews");
    }
}

#[tokio::test]
async fn fragmentless_and_unknown_addressing_fail_loudly() {
    let source = common::memory_source().await;

    // No `#<array>` fragment: refused with the convention named.
    let err = source
        .describe(&AssetRef::new(common::MANIFEST_KEY))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, SourceError::Format { detail, .. } if detail.contains("#<array-name>")),
        "{err}"
    );

    // Unknown array name.
    let err = source
        .describe(&common::asset("HDFEOS/GRIDS/TinyGrid/Data Fields/nope"))
        .await
        .unwrap_err();
    assert!(matches!(err, SourceError::Format { .. }), "{err}");

    // A real array with no georef is unsupported, not guessed.
    let err = source
        .describe(&common::asset("grid/reflectance"))
        .await
        .unwrap_err();
    assert!(matches!(err, SourceError::Unsupported { .. }), "{err}");

    // A missing manifest object is NotFound.
    let err = source
        .describe(&AssetRef::new("nowhere.vmanifest.json#x"))
        .await
        .unwrap_err();
    assert!(matches!(err, SourceError::NotFound { .. }), "{err}");
}

#[tokio::test]
async fn handles_recognizes_the_ingest_convention() {
    assert!(VirtualSource::handles(&common::asset(common::NIR)));
    assert!(!VirtualSource::handles(&AssetRef::new(
        "hlss30-t13sdd-2024158-b04.tif"
    )));
}
