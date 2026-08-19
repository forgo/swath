// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! ABI v1 conformance over the committed fixture modules (issue #202) —
//! the seed of the #203 executor's test bed, run under the #200
//! deterministic engine today so the fixtures are proven before the
//! adapter that will consume them exists.
//!
//! Fixtures (tests/fixtures/):
//! - `ndvi.wasm`, `hillshade.wasm`, `qamask.wasm` — built from `examples/udf/` by
//!   `just udf-fixtures`; CI rebuilds and byte-compares
//!   (`just udf-fixtures-verify`), so committed bytes always match source.
//! - `assemblyscript-double.wasm`, `tinygo-negate.wasm` — prebuilt from
//!   the pinned toolchains documented in `examples/udf/README.md`
//!   (deliberately NOT CI-built: they pin language neutrality, and this
//!   suite is what keeps the committed bytes honest).
//!
//! Every fixture must satisfy the registration rules (zero imports, the
//! four exports with the v1 signatures, exported linear memory, <= 64 MiB)
//! and answer a real request correctly and deterministically.

mod common;

use common::{Guest, engine};
use swath_udf_guest::{Plane, decode_response, encode_request};
use wasmtime::{ExternType, Module};

const NDVI: &[u8] = include_bytes!("fixtures/ndvi.wasm");
const HILLSHADE: &[u8] = include_bytes!("fixtures/hillshade.wasm");
const QAMASK: &[u8] = include_bytes!("fixtures/qamask.wasm");
const AS_DOUBLE: &[u8] = include_bytes!("fixtures/assemblyscript-double.wasm");
const TINYGO_NEGATE: &[u8] = include_bytes!("fixtures/tinygo-negate.wasm");

const ALL_FIXTURES: [(&str, &[u8]); 5] = [
    ("ndvi", NDVI),
    ("hillshade", HILLSHADE),
    ("qamask", QAMASK),
    ("assemblyscript-double", AS_DOUBLE),
    ("tinygo-negate", TINYGO_NEGATE),
];

/// Registration-shaped checks: zero imports, the four v1 exports with the
/// v1 signatures, an exported linear memory within the 64 MiB cap.
#[test]
fn fixtures_meet_the_registration_rules() {
    let engine = engine();
    for (name, bytes) in ALL_FIXTURES {
        let module = Module::new(&engine, bytes).expect(name);
        assert_eq!(module.imports().len(), 0, "{name}: zero-import rule");
        let sig = |export: &str| -> (Vec<String>, Vec<String>) {
            match module.get_export(export) {
                Some(ExternType::Func(f)) => (
                    f.params().map(|t| t.to_string()).collect(),
                    f.results().map(|t| t.to_string()).collect(),
                ),
                other => panic!("{name}: export {export} is {other:?}, expected a func"),
            }
        };
        assert_eq!(sig("swath_udf_abi"), (vec![], vec!["i32".into()]));
        assert_eq!(
            sig("swath_udf_output_planes"),
            (vec!["i32".into()], vec!["i32".into()])
        );
        assert_eq!(
            sig("swath_udf_alloc"),
            (vec!["i32".into()], vec!["i32".into()])
        );
        assert_eq!(
            sig("swath_udf_run"),
            (vec!["i32".into(), "i32".into()], vec!["i64".into()])
        );
        let Some(ExternType::Memory(memory)) = module.get_export("memory") else {
            panic!("{name}: no exported linear memory — the host cannot pass buffers");
        };
        assert!(
            memory.minimum() * 65536 <= 64 * 1024 * 1024,
            "{name}: declared memory over the 64 MiB cap"
        );
    }
}

#[test]
fn ndvi_matches_the_band_math_expression_exactly() {
    let mut guest = Guest::new(NDVI);
    assert_eq!(guest.abi(), 1);
    assert_eq!(guest.call_i32("swath_udf_output_planes", 2), 1);
    assert_eq!(guest.call_i32("swath_udf_output_planes", 1), 0);

    let nir = Plane {
        values: vec![0.8, 0.6, 0.5, 0.0, 0.9, 0.3],
        validity: vec![1, 1, 1, 1, 1, 0],
    };
    let red = Plane {
        values: vec![0.1, 0.2, -0.25, 0.0, 0.0, 0.3],
        validity: vec![1, 1, 1, 1, 0, 1],
    };
    let request = encode_request(3, 2, &[nir.clone(), red.clone()]).expect("encodes");
    let out = guest.run(&request).expect("run succeeds");
    let response = decode_response(3, 2, &out).expect("response decodes");
    assert_eq!(response.planes.len(), 1);
    let plane = &response.planes[0];
    // Pixels 0..3: both valid — exactly (nir - red) / (nir + red), the
    // engine's own band-math expression (the dual-implementation oracle).
    for i in 0..3 {
        assert_eq!(plane.validity[i], 1, "pixel {i}");
        assert_eq!(
            plane.values[i].to_bits(),
            ((nir.values[i] - red.values[i]) / (nir.values[i] + red.values[i])).to_bits(),
            "pixel {i}: bit-exact against the band-math expression"
        );
    }
    // Pixel 3: valid inputs, zero denominator -> non-finite -> invalid.
    // Pixels 4, 5: an invalid input -> invalid.
    for i in 3..6 {
        assert_eq!(plane.validity[i], 0, "pixel {i}");
        assert_eq!(
            plane.values[i].to_bits(),
            0.0f64.to_bits(),
            "invalid pixels hold 0.0"
        );
    }
}

#[test]
fn hillshade_computes_the_interior_and_leaves_the_seam_ring_invalid() {
    let mut guest = Guest::new(HILLSHADE);
    assert_eq!(guest.abi(), 1);
    assert_eq!(guest.call_i32("swath_udf_output_planes", 1), 1);
    assert_eq!(guest.call_i32("swath_udf_output_planes", 2), 0);

    // A 4x4 plane sloping up to the east: x + 10, all valid.
    let (width, height) = (4u32, 4u32);
    let values: Vec<f64> = (0..16).map(|i| f64::from(i % 4) + 10.0).collect();
    let elevation = Plane {
        values,
        validity: vec![1; 16],
    };
    let request = encode_request(width, height, &[elevation]).expect("encodes");
    let out = guest.run(&request).expect("run succeeds");
    let response = decode_response(width, height, &out).expect("response decodes");
    let plane = &response.planes[0];
    for row in 0..4usize {
        for col in 0..4usize {
            let idx = row * 4 + col;
            let interior = (1..3).contains(&row) && (1..3).contains(&col);
            assert_eq!(
                plane.validity[idx],
                u8::from(interior),
                "({row},{col}): only the full-neighborhood interior is valid — \
                 the outermost ring is the documented v1 tile seam (no halo)"
            );
            if interior {
                assert!(
                    (0.0..=1.0).contains(&plane.values[idx]),
                    "({row},{col}): shade in [0,1], got {}",
                    plane.values[idx]
                );
            }
        }
    }
    // Rising eastward means the surface faces west — toward the 315°
    // (northwest) sun: the two interior pixels agree (uniform gradient)
    // and are brighter than flat ground (whose shade is sin 45°).
    assert_eq!(plane.values[5].to_bits(), plane.values[6].to_bits());
    assert!(
        plane.values[5] > core::f64::consts::FRAC_1_SQRT_2 && plane.values[5] < 1.0,
        "west-facing shade {} should exceed flat ground's",
        plane.values[5]
    );

    // Determinism: an identical second run answers byte-identical output.
    let again = guest.run(&request).expect("second run succeeds");
    assert_eq!(out, again, "byte-identical across runs");
}

#[test]
fn qamask_tests_the_fmask_bits_and_rejects_unrepresentable_words() {
    let mut guest = Guest::new(QAMASK);
    assert_eq!(guest.abi(), 1);
    assert_eq!(guest.call_i32("swath_udf_output_planes", 1), 1);
    assert_eq!(guest.call_i32("swath_udf_output_planes", 2), 0);

    // Words 0..8: real Fmask bytes (bit 1 cloud, bit 2 adjacent, bit 3
    // shadow are masked; cirrus/snow/water are not). 8..12: values with
    // no honest u8 bit pattern — fractional, negative, out of range,
    // NaN — plus an input-invalid pixel.
    let qa = Plane {
        values: vec![
            0.0,
            1.0,
            2.0,
            4.0,
            8.0,
            14.0,
            16.0,
            32.0, // bytes
            3.5,
            -1.0,
            256.0,
            f64::NAN, // unrepresentable
            255.0,
            0.0,
            64.0,
            0.0, // last one input-invalid
        ],
        validity: vec![1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0],
    };
    let request = encode_request(4, 4, std::slice::from_ref(&qa)).expect("encodes");
    let out = guest.run(&request).expect("run succeeds");
    let response = decode_response(4, 4, &out).expect("response decodes");
    assert_eq!(response.planes.len(), 1);
    let plane = &response.planes[0];
    // (expected validity, expected value) per pixel.
    let expected: [(u8, f64); 16] = [
        (1, 1.0), // 0: clear
        (1, 1.0), // 1: cirrus only — not masked
        (1, 0.0), // 2: cloud
        (1, 0.0), // 4: adjacent to cloud
        (1, 0.0), // 8: cloud shadow
        (1, 0.0), // 14: all three masked bits
        (1, 1.0), // 16: snow/ice — not masked
        (1, 1.0), // 32: water — not masked
        (0, 0.0), // 3.5: fractional
        (0, 0.0), // -1.0: negative
        (0, 0.0), // 256.0: out of u8 range
        (0, 0.0), // NaN
        (1, 0.0), // 255: everything set, cloud included
        (1, 1.0), // 0: clear
        (1, 1.0), // 64: bit 6 (aerosol) — not masked
        (0, 0.0), // input-invalid passes through as invalid
    ];
    for (i, (validity, value)) in expected.iter().enumerate() {
        assert_eq!(plane.validity[i], *validity, "pixel {i} validity");
        assert_eq!(
            plane.values[i].to_bits(),
            value.to_bits(),
            "pixel {i} value"
        );
    }
}

/// The cross-language fixtures: same request, language-specific expected
/// values (`assemblyscript-double` doubles, `tinygo-negate` negates),
/// validity passthrough.
#[test]
fn cross_language_fixtures_conform() {
    type Case = (&'static str, &'static [u8], fn(f64) -> f64);
    let input = Plane {
        values: vec![1.5, -2.25, 0.0, 1e300],
        validity: vec![1, 0, 1, 1],
    };
    let request = encode_request(2, 2, std::slice::from_ref(&input)).expect("encodes");
    let cases: [Case; 2] = [
        ("assemblyscript-double", AS_DOUBLE, |v| v * 2.0),
        ("tinygo-negate", TINYGO_NEGATE, |v| -v),
    ];
    for (name, bytes, expected) in cases {
        let mut guest = Guest::new(bytes);
        assert_eq!(guest.abi(), 1, "{name}");
        assert_eq!(guest.call_i32("swath_udf_output_planes", 1), 1, "{name}");
        assert_eq!(guest.call_i32("swath_udf_output_planes", 3), 0, "{name}");
        let out = guest
            .run(&request)
            .unwrap_or_else(|| panic!("{name}: run succeeds"));
        let response = decode_response(2, 2, &out).expect("response decodes");
        assert_eq!(response.planes.len(), 1, "{name}");
        let plane = &response.planes[0];
        assert_eq!(
            plane.validity, input.validity,
            "{name}: validity passthrough"
        );
        for (i, value) in input.values.iter().enumerate() {
            assert_eq!(
                plane.values[i].to_bits(),
                expected(*value).to_bits(),
                "{name}: pixel {i}"
            );
        }
    }
}

/// Malformed input is a guest-declared failure (`0`), never a trap or a
/// garbage answer — for every fixture, in every language.
#[test]
fn malformed_requests_answer_zero() {
    for (name, bytes) in ALL_FIXTURES {
        // NDVI takes 2 input planes; every other fixture takes 1.
        let arity = if name == "ndvi" { 2 } else { 1 };
        let plane = Plane {
            values: vec![1.0; 9],
            validity: vec![1; 9],
        };
        let good = encode_request(3, 3, &vec![plane; arity]).expect("encodes");
        let mut guest = Guest::new(bytes);
        // Wrong abi version in the header.
        let mut wrong_abi = good.clone();
        let header_start = 4 + wrong_abi[4..].iter().position(|&b| b == b':').unwrap() + 1;
        wrong_abi[header_start] = b'2';
        assert_eq!(guest.run(&wrong_abi), None, "{name}: abi 2 refused");
        // Payload shorter than the header claims.
        let truncated = &good[..good.len() - 1];
        assert_eq!(guest.run(truncated), None, "{name}: truncated refused");
        // And the good request still succeeds afterwards.
        assert!(guest.run(&good).is_some(), "{name}: recovers");
    }
}
