// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Error-path behavior: unsupported CRSs fail at resolution, out-of-domain
//! points fail per point — and nothing ever panics or leaks NaN (issue #23).

use swath_core::crs::Crs;
use swath_core::reproject::{Reproject, ReprojectError};
use swath_reproject_proj4rs::Proj4rsReproject;

#[test]
fn unsupported_epsg_is_unknown_crs_on_either_side() {
    let r = Proj4rsReproject::new();
    for (from, to) in [(2154_u32, 4326_u32), (4326, 2154), (999_999, 3857)] {
        let Err(err) = r.transformer(&Crs::from_epsg(from), &Crs::from_epsg(to)) else {
            panic!("{from}->{to} unexpectedly resolved");
        };
        assert!(
            matches!(err, ReprojectError::UnknownCrs { .. }),
            "{from}->{to}: expected UnknownCrs, got {err:?}"
        );
    }
}

#[test]
fn out_of_domain_latitude_errors_without_panicking() {
    let t = Proj4rsReproject::new()
        .transformer(&Crs::WGS84, &Crs::WEB_MERCATOR)
        .expect("transformer");
    for (lon, lat) in [(0.0, 95.0), (0.0, -90.5), (10.0, 400.0)] {
        let err = t.transform(lon, lat).expect_err("|lat| > 90 must error");
        assert_eq!(
            err,
            ReprojectError::OutOfDomain { x: lon, y: lat },
            "error echoes the offending input in port units (degrees)"
        );
    }
}

#[test]
fn non_finite_input_errors_without_panicking() {
    let t = Proj4rsReproject::new()
        .transformer(&Crs::WGS84, &Crs::from_epsg(32613))
        .expect("transformer");
    for (x, y) in [
        (f64::NAN, 0.0),
        (0.0, f64::NAN),
        (f64::INFINITY, 10.0),
        (-105.0, f64::NEG_INFINITY),
    ] {
        assert!(
            t.transform(x, y).is_err(),
            "non-finite input ({x}, {y}) must error, never propagate"
        );
    }
}

#[test]
fn absurd_projected_input_never_panics_or_leaks_nonfinite() {
    // Nonsense eastings/northings fed to an inverse UTM transform: the
    // contract is error-or-finite, never panic, never NaN through the port.
    let t = Proj4rsReproject::new()
        .transformer(&Crs::from_epsg(32613), &Crs::WGS84)
        .expect("transformer");
    for (x, y) in [
        (1e12, 1e12),
        (-1e12, 4.35e6),
        (5e5, -1e300),
        (f64::MAX, f64::MAX),
    ] {
        if let Ok((lon, lat)) = t.transform(x, y) {
            assert!(
                lon.is_finite() && lat.is_finite(),
                "({x}, {y}) produced non-finite output ({lon}, {lat})"
            );
        }
    }
}

#[test]
fn batch_with_bad_point_errors_and_names_it() {
    let t = Proj4rsReproject::new()
        .transformer(&Crs::WGS84, &Crs::WEB_MERCATOR)
        .expect("transformer");
    let mut pts = [(0.0, 10.0), (5.0, 95.0), (10.0, 20.0)];
    let err = t
        .transform_slice(&mut pts)
        .expect_err("batch containing |lat| > 90 must error");
    assert_eq!(err, ReprojectError::OutOfDomain { x: 5.0, y: 95.0 });
}
