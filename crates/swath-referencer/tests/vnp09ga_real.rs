// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The real-granule half of the conformance suite: structural + georef
//! assertions against an actual VNP09GA granule (ADR 0008). Gated: the
//! granule is ~8 MB from LP DAAC (NASA Earthdata credentials), so it is not
//! committed; `just test-referencer` fetches/reuses one, exports
//! `SWATH_VNP09GA`, and runs this with `--ignored`. Without the variable the
//! test is skipped (the tiny committed fixture in `known_answer.rs` keeps PR
//! CI covering the same code paths).

// A gated test's skip notice legitimately goes to stderr.
#![allow(clippy::print_stderr)]

use std::path::PathBuf;

use swath_core::ingest::IngestReferencer as _;
use swath_core::manifest::GeorefCrs;
use swath_referencer::SwathReferencer;

/// The granule under test, when the harness provides one.
fn granule() -> Option<PathBuf> {
    std::env::var_os("SWATH_VNP09GA").map(PathBuf::from)
}

#[test]
#[ignore = "needs a real VNP09GA granule (run via `just test-referencer`)"]
fn vnp09ga_manifest_structure_and_georef() {
    let Some(path) = granule() else {
        // Belt and braces: even under --ignored, absent creds skip cleanly.
        eprintln!("SWATH_VNP09GA not set; skipping");
        return;
    };
    let manifest = SwathReferencer::new().generate(&path).expect("generates");

    // The bake-off's structural truth for this product (prototype 0001 §7):
    // 67 arrays, 1,551 chunk refs, deflate-8 on the compressed arrays.
    assert_eq!(manifest.arrays.len(), 67, "array count");
    let refs: usize = manifest.arrays.iter().map(|a| a.refs.len()).sum();
    assert_eq!(refs, 1551, "chunk ref count");

    // Every 2-D grid data field is georeferenced with the sinusoidal proj
    // string; both product grids appear.
    let georeferenced: Vec<_> = manifest
        .arrays
        .iter()
        .filter(|a| a.georef.is_some())
        .collect();
    assert_eq!(georeferenced.len(), 30, "georeferenced data fields");
    for array in &georeferenced {
        let georef = array.georef.as_ref().unwrap();
        let GeorefCrs::Proj4(proj) = &georef.crs else {
            panic!("{}: expected a proj4 CRS", array.name);
        };
        assert!(proj.contains("+proj=sinu"), "{}: {proj}", array.name);
        assert!(proj.contains("+R=6371007.181"), "{}: {proj}", array.name);
        assert!(georef.transform.pixel_height < 0.0, "{}", array.name);
        assert!(georef.band.is_some(), "{}", array.name);
    }
    // The two grids resolve to their documented cell sizes (h33v12 tile,
    // 1200×1200 at ~926.6 m and 2400×2400 at ~463.3 m).
    let cell = |name_part: &str| {
        georeferenced
            .iter()
            .find(|a| a.name.contains(name_part))
            .and_then(|a| a.georef.as_ref())
            .map(|g| g.transform.pixel_width)
            .expect(name_part)
    };
    assert!((cell("SurfReflect_M1_1") - 926.625).abs() < 0.01);
    assert!((cell("SurfReflect_I1_1") - 463.312).abs() < 0.01);

    // Numeric fills surface as nodata; the QF bands' b"N/A" honestly do not.
    let nodata = |name_part: &str| {
        georeferenced
            .iter()
            .find(|a| a.name.contains(name_part))
            .and_then(|a| a.georef.as_ref())
            .expect(name_part)
            .nodata
    };
    assert_eq!(nodata("SurfReflect_M1_1"), Some(-28672.0));
    assert_eq!(nodata("SurfReflect_QF1_1"), None);
}
