// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Horn hillshade as a UDF — something band math cannot express
//! (issue #202): each output pixel reads a 3x3 neighborhood, which the
//! bounded profile's per-pixel arithmetic has no way to spell.
//!
//! Inputs: 1 plane (elevation; the slope is taken in pixel units — ABI v1
//! carries no georeferencing or parameters, so the sun and z-scale are
//! compile-time constants: azimuth 315°, altitude 45°, z-factor 1).
//! Output: 1 plane of shade values in `[0, 1]`.
//!
//! **Tile-seam caveat (v1, by design):** the module sees exactly one
//! tile — there is no halo of neighboring pixels (a v2 reopen condition
//! recorded in ADR 0018). A neighborhood op therefore cannot compute its
//! outermost ring honestly, and this module marks that ring invalid
//! rather than inventing values: rendered output shows 1-pixel seams at
//! tile borders. That visible, honest artifact is the documented v1
//! behavior for neighborhood UDFs (`docs/udf-abi/authoring.md`).
//!
//! `no_std` has no `f64::sqrt`; the module carries its own deterministic
//! Newton iteration (pure f64 arithmetic — reproducible everywhere under
//! the runtime's NaN canonicalization).

#![no_std]

extern crate alloc;

use swath_udf_guest::{Plane, Request, Response, swath_udf};

/// Light vector for azimuth 315°, altitude 45° (east, north, up),
/// precomputed: (cos alt * sin az, cos alt * cos az, sin alt).
const LIGHT: [f64; 3] = [-0.5, 0.5, core::f64::consts::FRAC_1_SQRT_2];

swath_udf! {
    output_planes: |input_planes| if input_planes == 1 { 1 } else { 0 },
    run: hillshade,
}

/// Deterministic square root: bit-level initial guess + Newton-Raphson.
/// Not ulp-exact against libm, but pure f64 arithmetic — identical bytes
/// on every host, which is the property that matters here.
fn sqrt(x: f64) -> f64 {
    if x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    // Halve the exponent for the initial guess.
    let mut y = f64::from_bits((x.to_bits() >> 1) + 0x1FF8_0000_0000_0000);
    for _ in 0..6 {
        y = 0.5 * (y + x / y);
    }
    y
}

fn hillshade(request: &Request) -> Option<Response> {
    let [elevation] = request.planes.as_slice() else {
        return None;
    };
    let (width, height) = (request.width as usize, request.height as usize);
    let mut out = Plane::invalid(request.pixels());
    // The outermost ring has no full 3x3 neighborhood inside this tile
    // (no halo in ABI v1) and stays invalid — the documented seam.
    for row in 1..height.saturating_sub(1) {
        'pixel: for col in 1..width.saturating_sub(1) {
            let mut z = [0.0f64; 9];
            for (k, zk) in z.iter_mut().enumerate() {
                let idx = (row + k / 3 - 1) * width + (col + k % 3 - 1);
                if elevation.validity[idx] == 0 {
                    continue 'pixel; // any invalid neighbor: output invalid
                }
                *zk = elevation.values[idx];
            }
            // Horn 1981; row 0 is the tile's top row, so +y in the grid
            // points south and the north component is negated.
            let dz_dx = ((z[2] + 2.0 * z[5] + z[8]) - (z[0] + 2.0 * z[3] + z[6])) / 8.0;
            let dz_dy = ((z[6] + 2.0 * z[7] + z[8]) - (z[0] + 2.0 * z[1] + z[2])) / 8.0;
            // Surface normal (unnormalized): (-p, q_north, 1).
            let normal = [-dz_dx, dz_dy, 1.0];
            let dot = normal[0] * LIGHT[0] + normal[1] * LIGHT[1] + normal[2] * LIGHT[2];
            let norm = sqrt(normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]);
            let shade = if dot > 0.0 { dot / norm } else { 0.0 };
            if shade.is_finite() {
                let idx = row * width + col;
                out.values[idx] = shade;
                out.validity[idx] = 1;
            }
        }
    }
    Some(Response {
        planes: alloc::vec![out],
    })
}
