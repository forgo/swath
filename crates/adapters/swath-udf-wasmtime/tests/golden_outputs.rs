// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Golden OUTPUTS for the reference UDFs (issue #209) — the seed of the
//! verification lattice: each reference module's answer over a committed
//! input tile is itself committed, and every run must reproduce it
//! **byte-identically** under the real deterministic engine.
//!
//! Per UDF, two fixture files (tests/fixtures/golden/):
//! - `<name>.request.bin` — the committed input tile as the exact ABI v1
//!   request buffer the host sends. A test pins it against the in-code
//!   construction below, so the committed input can't drift silently.
//! - `<name>.response.bin` — the pinned output planes, as the exact
//!   response buffer the module answered when the golden was captured.
//!
//! The byte-compare is the lattice claim (deterministic WASM, ADR 0018:
//! same module + same input = same bytes, on every host); the decoded
//! spot-checks alongside anchor the pinned bytes to hand-computed
//! meaning, so a golden can never be regenerated into frozen garbage.
//!
//! Regeneration — only after a deliberate change to a reference UDF or
//! its input tile: `just udf-goldens` (rebuilds the fixture modules,
//! then runs the ignored `golden_capture` test to rewrite both files).

mod common;

use std::path::PathBuf;

use common::Guest;
use swath_udf_guest::{Plane, decode_response, encode_request};

const SIZE: u32 = 8;
const PIXELS: usize = 64;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden")
}

fn read_golden(name: &str) -> Vec<u8> {
    let path = golden_dir().join(name);
    std::fs::read(&path).unwrap_or_else(|err| {
        panic!(
            "cannot read golden fixture {}: {err} (regenerate with `just udf-goldens`)",
            path.display()
        )
    })
}

// --- the committed input tiles, constructed exactly ----------------------

/// NDVI input: NIR ramps up, RED ramps down (both exact in f64), with an
/// invalid pixel in each band, a zero-denominator pixel (nir = -red), and
/// a 0/0 pixel.
#[allow(clippy::cast_precision_loss)] // pixel indices < 64 are exact in f64
fn ndvi_request() -> Vec<u8> {
    let mut nir = Plane {
        values: (0..PIXELS).map(|i| i as f64 * 0.0125).collect(),
        validity: vec![1; PIXELS],
    };
    let mut red = Plane {
        values: (0..PIXELS).map(|i| 0.4 - i as f64 * 0.005).collect(),
        validity: vec![1; PIXELS],
    };
    nir.validity[7] = 0;
    red.validity[14] = 0;
    // Pixel 30: valid inputs, nir + red == 0 — non-finite NDVI.
    nir.values[30] = 0.2;
    red.values[30] = -0.2;
    // Pixel 40: 0/0.
    nir.values[40] = 0.0;
    red.values[40] = 0.0;
    encode_request(SIZE, SIZE, &[nir, red]).expect("encodes")
}

/// Hillshade input: an exact paraboloid bowl centered between pixels
/// (integer-scaled, exact in f64) with one invalid interior pixel at
/// (4, 4) — its whole 3x3 neighborhood must come out invalid.
#[allow(clippy::cast_precision_loss)] // rows/cols < 8 are exact in f64
fn hillshade_request() -> Vec<u8> {
    let mut elevation = Plane {
        values: (0..PIXELS)
            .map(|i| {
                let (row, col) = ((i / 8) as f64, (i % 8) as f64);
                ((2.0 * row - 7.0) * (2.0 * row - 7.0) + (2.0 * col - 7.0) * (2.0 * col - 7.0))
                    / 4.0
            })
            .collect(),
        validity: vec![1; PIXELS],
    };
    elevation.validity[4 * 8 + 4] = 0;
    encode_request(SIZE, SIZE, std::slice::from_ref(&elevation)).expect("encodes")
}

/// QA-mask input: Fmask bytes cycling through clear/flagged words, plus
/// one pixel of every unrepresentable shape and one input-invalid pixel.
fn qamask_request() -> Vec<u8> {
    const WORDS: [f64; 8] = [0.0, 1.0, 2.0, 4.0, 8.0, 14.0, 32.0, 255.0];
    let mut qa = Plane {
        values: (0..PIXELS).map(|i| WORDS[i % WORDS.len()]).collect(),
        validity: vec![1; PIXELS],
    };
    // Row 7: the dishonest shapes.
    qa.values[56] = 3.5;
    qa.values[57] = -1.0;
    qa.values[58] = 256.0;
    qa.values[59] = f64::NAN;
    qa.values[60] = 1e300;
    qa.validity[63] = 0;
    encode_request(SIZE, SIZE, std::slice::from_ref(&qa)).expect("encodes")
}

type Case = (&'static str, &'static [u8], fn() -> Vec<u8>);

const CASES: [Case; 3] = [
    ("ndvi", include_bytes!("fixtures/ndvi.wasm"), ndvi_request),
    (
        "hillshade",
        include_bytes!("fixtures/hillshade.wasm"),
        hillshade_request,
    ),
    (
        "qamask",
        include_bytes!("fixtures/qamask.wasm"),
        qamask_request,
    ),
];

// --- the golden gate ------------------------------------------------------

/// The committed request buffers are exactly the in-code constructions —
/// the input side of the pin cannot drift.
#[test]
fn golden_requests_match_their_construction() {
    for (name, _, request) in CASES {
        assert_eq!(
            read_golden(&format!("{name}.request.bin")),
            request(),
            "{name}: committed request drifted from its construction"
        );
    }
}

/// THE golden claim: each reference module, run over its committed
/// request under the deterministic engine, answers the committed
/// response byte-identically.
#[test]
fn reference_udfs_reproduce_their_golden_outputs_byte_identically() {
    for (name, module, _) in CASES {
        let request = read_golden(&format!("{name}.request.bin"));
        let expected = read_golden(&format!("{name}.response.bin"));
        let out = Guest::new(module)
            .run(&request)
            .unwrap_or_else(|| panic!("{name}: golden run succeeds"));
        assert_eq!(
            out, expected,
            "{name}: output bytes diverge from the committed golden"
        );
    }
}

/// Semantic anchors: decode the pinned responses and check hand-computed
/// pixels, so the goldens stay tied to meaning across regenerations.
#[test]
fn golden_outputs_decode_to_the_hand_computed_values() {
    let decode = |name: &str| {
        decode_response(SIZE, SIZE, &read_golden(&format!("{name}.response.bin")))
            .expect("golden response decodes")
    };

    // NDVI: pixel 0 is exactly the band-math expression over the
    // constructed inputs; the poisoned pixels are invalid.
    let ndvi = decode("ndvi");
    let plane = &ndvi.planes[0];
    assert_eq!(
        plane.values[0].to_bits(),
        ((0.0f64 - 0.4) / (0.0 + 0.4)).to_bits()
    );
    assert_eq!(plane.validity[0], 1);
    let nir_9 = 9.0f64 * 0.0125;
    let red_9 = 0.4f64 - 9.0 * 0.005;
    assert_eq!(
        plane.values[9].to_bits(),
        ((nir_9 - red_9) / (nir_9 + red_9)).to_bits()
    );
    for poisoned in [7, 14, 30, 40] {
        assert_eq!(plane.validity[poisoned], 0, "pixel {poisoned}");
        assert_eq!(plane.values[poisoned].to_bits(), 0.0f64.to_bits());
    }

    // Hillshade: seam ring invalid, the (4,4) hole's 3x3 neighborhood
    // invalid, everything else valid and in [0, 1] — and the bowl's
    // geometry holds in the pinned bytes: on the far rim (6,6) the
    // surface tilts up to the southeast, so its normal faces the 315°
    // (northwest) light and it comes out brighter than (1,1), whose
    // slope faces away.
    let hillshade = decode("hillshade");
    let plane = &hillshade.planes[0];
    for row in 0..8usize {
        for col in 0..8usize {
            let seam = !((1..7).contains(&row) && (1..7).contains(&col));
            let hole = (3..=5).contains(&row) && (3..=5).contains(&col);
            let idx = row * 8 + col;
            assert_eq!(
                plane.validity[idx],
                u8::from(!(seam || hole)),
                "({row},{col})"
            );
            if !(seam || hole) {
                assert!((0.0..=1.0).contains(&plane.values[idx]), "({row},{col})");
            }
        }
    }
    assert!(
        plane.values[6 * 8 + 6] > plane.values[9],
        "(6,6) faces the 315-degree light"
    );

    // QA mask: clear/flagged per the Fmask bits; every dishonest shape
    // and the input-invalid pixel are invalid.
    let qamask = decode("qamask");
    let plane = &qamask.planes[0];
    assert_eq!((plane.validity[0], plane.values[0]), (1, 1.0)); // word 0: clear
    assert_eq!((plane.validity[2], plane.values[2]), (1, 0.0)); // word 2: cloud
    assert_eq!((plane.validity[5], plane.values[5]), (1, 0.0)); // word 14: all masked bits
    assert_eq!((plane.validity[6], plane.values[6]), (1, 1.0)); // word 32: water, clear
    for invalid in 56..=60 {
        assert_eq!(plane.validity[invalid], 0, "pixel {invalid}");
    }
    assert_eq!(plane.validity[63], 0, "input-invalid passthrough");
}

/// Regenerates the committed goldens — run only deliberately, via
/// `just udf-goldens`, after a change to a reference UDF or its input.
#[test]
#[ignore = "rewrites the committed goldens; run via `just udf-goldens`"]
fn golden_capture() {
    let dir = golden_dir();
    std::fs::create_dir_all(&dir).expect("golden dir");
    for (name, module, request) in CASES {
        let request = request();
        let response = Guest::new(module)
            .run(&request)
            .unwrap_or_else(|| panic!("{name}: capture run succeeds"));
        std::fs::write(dir.join(format!("{name}.request.bin")), &request).expect("write request");
        std::fs::write(dir.join(format!("{name}.response.bin")), &response)
            .expect("write response");
    }
}
