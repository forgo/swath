// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Known-answer test against the tiny committed HDF5 fixture: the generator's
//! manifest must byte-range-match `tiny.expected.json`, whose offsets and
//! codecs were derived independently of this crate (straight from h5py's
//! chunk index — see `tests/data/make_tiny_fixture.py`). This is the PR-CI
//! half of the conformance suite; the real-VNP09GA half is the gated
//! `vnp09ga_real.rs` + `just test-referencer`.

use std::path::PathBuf;

use swath_core::ingest::{IngestReferencer as _, ReferencerError};
use swath_core::manifest::{VirtualManifest, compare};
use swath_referencer::SwathReferencer;

fn data(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(file)
}

fn generated() -> VirtualManifest {
    SwathReferencer::new()
        .generate(&data("tiny.h5"))
        .expect("tiny fixture generates")
}

fn expected() -> VirtualManifest {
    let text = std::fs::read_to_string(data("tiny.expected.json")).expect("expected json");
    VirtualManifest::from_json_str(&text).expect("expected json parses as schema v1")
}

#[test]
fn tiny_fixture_matches_the_h5py_truth_exactly() {
    let report = compare(&generated(), &expected());
    assert!(
        report.equivalent(),
        "generator disagrees with the h5py-derived truth: {report:#?}"
    );
    // The comparison is only as strong as the truth's coverage: pin it.
    // (#39 grew the fixture: an HDF-EOS StructMetadata.0 grid block plus
    // two georeferenced sinusoidal data fields — see make_tiny_fixture.py.)
    let refs: usize = expected().arrays.iter().map(|a| a.refs.len()).sum();
    assert_eq!(expected().arrays.len(), 9, "fixture array count");
    assert_eq!(refs, 24, "fixture chunk ref count");
}

#[test]
fn storage_layouts_map_as_documented() {
    let manifest = generated();
    let array = |name: &str| {
        manifest
            .arrays
            .iter()
            .find(|a| a.name == name)
            .unwrap_or_else(|| panic!("array `{name}`"))
    };

    // Chunked + shuffle + deflate, exact 2x2 grid, pipeline in filter order.
    let reflectance = array("grid/reflectance");
    assert_eq!(reflectance.codecs, ["shuffle", "zlib:4"]);
    assert_eq!(reflectance.chunks, [4, 3]);
    assert_eq!(reflectance.refs.len(), 4);

    // Ragged grid: 5x7 in 4x3 chunks -> 2x3 = 6 allocated chunks, keys up
    // to "1.2".
    let ragged = array("grid/ragged");
    assert_eq!(ragged.refs.len(), 6);
    assert!(ragged.refs.iter().any(|r| r.key == "1.2"));

    // Partially written: exactly the allocated chunk appears.
    let partial = array("grid/partial");
    assert_eq!(partial.refs.len(), 1);
    assert_eq!(partial.refs[0].key, "0.0");

    // Contiguous: whole-storage ref, chunk shape = shape.
    let contiguous = array("aux/contiguous");
    assert_eq!(contiguous.chunks, contiguous.shape);
    assert_eq!(contiguous.refs.len(), 1);
    assert_eq!(contiguous.refs[0].key, "0.0");

    // Never written: the array exists, its ref list is empty.
    assert!(array("aux/unallocated").refs.is_empty());

    // String scalar: empty key, |S<n> dtype.
    let meta = array("meta");
    assert_eq!(meta.dtype, "|S18");
    assert_eq!(meta.refs[0].key, "");

    // Georefs live exactly on the TinyGrid data fields (#39): the parsed
    // StructMetadata must reproduce the maker script's own constants.
    let nir = array("HDFEOS/GRIDS/TinyGrid/Data Fields/nir");
    let georef = nir.georef.as_ref().expect("nir is georeferenced");
    assert_eq!(
        georef.crs,
        swath_core::manifest::GeorefCrs::Proj4(
            "+proj=sinu +lon_0=0 +x_0=0 +y_0=0 +R=6371007.181 +units=m +no_defs".to_owned()
        )
    );
    assert!((georef.transform.origin_x - 16_679_257.795).abs() < 1e-6);
    assert!((georef.transform.origin_y - -3_335_851.559).abs() < 1e-6);
    assert!((georef.transform.pixel_width - 926.625_433_055_833).abs() < 1e-6);
    assert!((georef.transform.pixel_height + 926.625_433_055_833).abs() < 1e-6);
    assert_eq!(georef.nodata, Some(-28_672.0));
    assert_eq!(georef.band.as_deref(), Some("nir"));
    let red = array("HDFEOS/GRIDS/TinyGrid/Data Fields/red");
    assert!(red.georef.is_some(), "red is georeferenced");
    // Everything outside the grid stays bare.
    assert!(
        manifest
            .arrays
            .iter()
            .filter(|a| !a.name.starts_with("HDFEOS/GRIDS/"))
            .all(|a| a.georef.is_none())
    );
}

#[test]
fn non_hdf5_bytes_are_a_malformed_error() {
    let err = SwathReferencer::new()
        .generate(&data("tiny.expected.json").with_extension("h5json"))
        .unwrap_err();
    // Wrong extension entirely -> Unsupported.
    assert!(matches!(err, ReferencerError::Unsupported { .. }), "{err}");

    // Right extension, not an HDF5 container -> Malformed. (Copy the JSON
    // to a .h5 name in a temp dir.)
    let dir = swath_testsupport::TempDir::new("referencer-malformed");
    let fake = dir.join("fake.h5");
    std::fs::copy(data("tiny.expected.json"), &fake).unwrap();
    let err = SwathReferencer::new().generate(&fake).unwrap_err();
    assert!(matches!(err, ReferencerError::Malformed { .. }), "{err}");
}
