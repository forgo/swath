// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Property tests: forward∘inverse round-trips across the valid domain,
//! and batch/per-point equivalence on arbitrary inputs (issue #23).
//!
//! Round-trip tolerances are in the SOURCE CRS's units and are deliberately
//! looser than the truth-table agreement bar: a round trip composes two
//! transforms and its error is dominated by float cancellation near
//! projection extremes, not by proj4rs-vs-PROJ disagreement. `1e-9`° is
//! ~0.1 µm of ground distance; `1e-6` m is a micrometer — both far below
//! any pixel the tiler will ever address.

use proptest::prelude::*;
use swath_core::crs::Crs;
use swath_core::reproject::Reproject;
use swath_reproject_proj4rs::Proj4rsReproject;

/// Web Mercator's latitude cutoff, degrees (atan(sinh(pi))).
const MERC_LAT_LIMIT: f64 = 85.051_128_779_806_59;

/// Round-trips `(x, y)` through `from -> to -> from` and asserts the result
/// is within `tol` (source-CRS units) of the input.
fn roundtrip(from: u32, to: u32, x: f64, y: f64, tol: f64) {
    let r = Proj4rsReproject::new();
    let fwd = r
        .transformer(Crs::from_epsg(from), Crs::from_epsg(to))
        .expect("forward transformer");
    let inv = r
        .transformer(Crs::from_epsg(to), Crs::from_epsg(from))
        .expect("inverse transformer");
    let (fx, fy) = fwd.transform(x, y).expect("forward transform");
    let (bx, by) = inv.transform(fx, fy).expect("inverse transform");
    let dev = (bx - x).abs().max((by - y).abs());
    assert!(
        dev <= tol,
        "{from}->{to}->{from}: ({x}, {y}) came back as ({bx}, {by}), deviation {dev:e} > {tol:e}"
    );
}

proptest! {
    /// 4326 -> 3857 -> 4326 over Web Mercator's full validity square.
    #[test]
    fn wgs84_webmercator_roundtrip(
        lon in -180.0f64..180.0,
        lat in -MERC_LAT_LIMIT..MERC_LAT_LIMIT,
    ) {
        roundtrip(4326, 3857, lon, lat, 1e-9);
    }

    /// 4326 -> UTM 13N -> 4326 over the zone's official domain
    /// (102°W-108°W, equator to 84°N).
    #[test]
    fn wgs84_utm13n_roundtrip(
        lon in -108.0f64..-102.0,
        lat in 0.0f64..84.0,
    ) {
        roundtrip(4326, 32613, lon, lat, 1e-9);
    }

    /// 4326 -> UTM 55S -> 4326 over the southern zone's official domain
    /// (144°E-150°E, 80°S to equator).
    #[test]
    fn wgs84_utm55s_roundtrip(
        lon in 144.0f64..150.0,
        lat in -80.0f64..0.0,
    ) {
        roundtrip(4326, 32755, lon, lat, 1e-9);
    }

    /// UTM 13N -> 3857 -> UTM 13N (the HLS render path and back) across
    /// the fixture zone's easting/northing envelope.
    #[test]
    fn utm13n_webmercator_roundtrip(
        easting in 166_000.0f64..834_000.0,
        northing in 0.0f64..9_330_000.0,
    ) {
        roundtrip(32613, 3857, easting, northing, 1e-6);
    }

    /// The adapter's overridden batch path is bit-identical to its
    /// per-point path on arbitrary in-domain input.
    #[test]
    fn batch_equals_per_point_loop(
        pts in proptest::collection::vec(
            (-180.0f64..180.0, -MERC_LAT_LIMIT..MERC_LAT_LIMIT),
            0..64,
        ),
    ) {
        let t = Proj4rsReproject::new()
            .transformer(Crs::WGS84, Crs::WEB_MERCATOR)
            .expect("transformer");
        let mut batch = pts.clone();
        t.transform_slice(&mut batch).expect("batch transform");
        for (&(lon, lat), &(bx, by)) in pts.iter().zip(&batch) {
            let (px, py) = t.transform(lon, lat).expect("per-point transform");
            prop_assert!(
                px.to_bits() == bx.to_bits() && py.to_bits() == by.to_bits(),
                "batch ({bx}, {by}) != per-point ({px}, {py}) for ({lon}, {lat})"
            );
        }
    }
}
