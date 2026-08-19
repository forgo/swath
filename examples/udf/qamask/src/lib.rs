// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! QA cloud mask as a UDF — bitwise logic band math cannot express
//! (issue #209): the input samples are QA bitfield *words* (HLS
//! Fmask-style bytes riding in `f64` planes), and the mask tests
//! individual bits, which the bounded profile's arithmetic has no way
//! to spell.
//!
//! Inputs: 1 plane of QA words. Output: 1 plane — `1.0` where the pixel
//! is clear, `0.0` where any masked bit is set. ABI v1 carries no
//! parameters, so the bit selection is a compile-time constant of the
//! module (register variants for other policies — the documented v1
//! stance).
//!
//! Validity is strict representability: a QA word must be an exact
//! non-negative integer in `0..=255` (an Fmask byte). Anything else —
//! an invalid input pixel, a fractional value, a negative, an
//! out-of-range value, a non-finite — has no honest bit pattern and is
//! marked invalid rather than guessed at.

#![no_std]

extern crate alloc;

use swath_udf_guest::{Plane, Request, Response, swath_udf};

/// HLS v2 Fmask bit layout: bit 1 cloud, bit 2 adjacent-to-cloud,
/// bit 3 cloud shadow. A pixel with any of these set is not clear.
const MASK: u64 = (1 << 1) | (1 << 2) | (1 << 3);

swath_udf! {
    output_planes: |input_planes| if input_planes == 1 { 1 } else { 0 },
    run: qamask,
}

fn qamask(request: &Request) -> Option<Response> {
    let [qa] = request.planes.as_slice() else {
        return None;
    };
    let mut out = Plane::invalid(request.pixels());
    for i in 0..request.pixels() {
        if qa.validity[i] == 0 {
            continue;
        }
        let value = qa.values[i];
        // Exact-integer gate: the saturating cast then round-trip compare
        // rejects fractions, negatives, out-of-range, and non-finites in
        // one pure-f64 comparison (NaN compares unequal to everything).
        let word = value as u64;
        if word > 255 || word as f64 != value {
            continue;
        }
        out.values[i] = if word & MASK == 0 { 1.0 } else { 0.0 };
        out.validity[i] = 1;
    }
    Some(Response {
        planes: alloc::vec![out],
    })
}
