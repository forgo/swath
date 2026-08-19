// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! NDVI as a UDF — the dual-implementation oracle (issue #202).
//!
//! The engine already expresses NDVI as built-in band math,
//! `(b1 - b2) / (b1 + b2)`; this module computes the identical IEEE
//! expression as a `run_udf` WASM module, so the adapter can pin that the
//! UDF path and the band-math path produce byte-identical tiles.
//!
//! Inputs: 2 planes (NIR, RED — header order). Output: 1 plane.
//!
//! Validity mirrors the band-math semantics end to end: where either
//! input is invalid the output is invalid; where the denominator is zero
//! the division yields a non-finite value, which the module reports as
//! invalid itself (the host would canonicalize it to invalid anyway —
//! ABI v1's non-finite post-condition).

#![no_std]

extern crate alloc;

use swath_udf_guest::{Plane, Request, Response, swath_udf};

swath_udf! {
    output_planes: |input_planes| if input_planes == 2 { 1 } else { 0 },
    run: ndvi,
}

fn ndvi(request: &Request) -> Option<Response> {
    let [nir, red] = request.planes.as_slice() else {
        return None;
    };
    let mut out = Plane::invalid(request.pixels());
    for i in 0..request.pixels() {
        if nir.validity[i] == 0 || red.validity[i] == 0 {
            continue;
        }
        let value = (nir.values[i] - red.values[i]) / (nir.values[i] + red.values[i]);
        if value.is_finite() {
            out.values[i] = value;
            out.validity[i] = 1;
        }
    }
    Some(Response {
        planes: alloc::vec![out],
    })
}
