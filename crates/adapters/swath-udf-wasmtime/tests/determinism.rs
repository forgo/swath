// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! ADR 0018's determinism commitment, pinned end to end (issue #203):
//! identical inputs give **byte-identical** outputs — across fresh
//! stores, across executor instances, and across time (the insta
//! snapshots hold each fixture module's exact output bits hostage).
//!
//! The NDVI fixture is the dual-implementation oracle: the same IEEE
//! expression exists as engine band math and as a WASM module, and the
//! rendered tiles must match byte for byte.

use swath_render::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, TileFormat};
use swath_render::udf::{UdfExecutor, UdfLimits, UdfStage};
use swath_render::{RenderPlan, WarpedBuffer, eval};
use swath_udf_wasmtime::WasmtimeUdf;

const NDVI: &[u8] = include_bytes!("fixtures/ndvi.wasm");
const HILLSHADE: &[u8] = include_bytes!("fixtures/hillshade.wasm");
const QAMASK: &[u8] = include_bytes!("fixtures/qamask.wasm");
const AS_DOUBLE: &[u8] = include_bytes!("fixtures/assemblyscript-double.wasm");
const TINYGO_NEGATE: &[u8] = include_bytes!("fixtures/tinygo-negate.wasm");

fn stage(code_hash: String, output_planes: u32) -> UdfStage {
    UdfStage::new(code_hash, output_planes, serde_json::Value::Null)
}

/// A deterministic pseudo-random tile: values in a plausible reflectance
/// range, a scattering of invalid pixels, no platform dependence.
fn synthetic_plane(width: u32, height: u32, seed: u64) -> WarpedBuffer {
    let len = (width as usize) * (height as usize);
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    let mut next = move || {
        // xorshift64*: stable everywhere, good enough for test data.
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let mut values = Vec::with_capacity(len);
    let mut valid = Vec::with_capacity(len);
    for _ in 0..len {
        let sample = next();
        #[allow(
            clippy::cast_precision_loss,
            reason = "synthetic test data; exactness of the mapping is irrelevant"
        )]
        let value = (sample >> 11) as f64 / (1u64 << 53) as f64;
        let ok = sample % 17 != 0;
        values.push(if ok { value } else { 0.0 });
        valid.push(ok);
    }
    WarpedBuffer {
        width,
        height,
        values,
        valid,
    }
}

fn bits(buffer: &WarpedBuffer) -> (Vec<u64>, Vec<bool>) {
    (
        buffer.values.iter().map(|v| v.to_bits()).collect(),
        buffer.valid.clone(),
    )
}

/// The core determinism harness: the same 256×256 tile, 16 runs, every
/// run a fresh `Store` — bit-identical outputs; and a second executor
/// instance (fresh engine, fresh compilation) agrees byte for byte.
#[test]
fn ndvi_renders_bit_identical_across_16_fresh_stores_and_executors() {
    let inputs = vec![
        synthetic_plane(256, 256, 7),
        synthetic_plane(256, 256, 1312),
    ];
    let mut baseline = None;
    for instance in 0..2 {
        let executor = WasmtimeUdf::new().expect("engine builds");
        let hash = executor.compile(NDVI).expect("fixture compiles");
        let stage = stage(hash, 1);
        for run in 0..8 {
            let out = executor
                .run(&stage, &inputs, &UdfLimits::default())
                .expect("ndvi runs");
            assert_eq!(out.planes.len(), 1);
            let got = bits(&out.planes[0]);
            match &baseline {
                None => baseline = Some(got),
                Some(want) => {
                    assert_eq!(
                        want, &got,
                        "executor {instance} run {run}: outputs must be bit-identical"
                    );
                }
            }
        }
    }
}

/// The dual-implementation oracle: the NDVI UDF tile and the band-math
/// NDVI tile are byte-identical RGBA — user code joins the goldens.
#[test]
fn ndvi_udf_tile_is_byte_identical_to_band_math() {
    let executor = WasmtimeUdf::new().expect("engine builds");
    let hash = executor.compile(NDVI).expect("fixture compiles");
    let inputs = vec![synthetic_plane(64, 64, 3), synthetic_plane(64, 64, 4)];
    let bands = vec![BandInput::new("nir"), BandInput::new("red")];
    let tail = [
        PixelOp::Rescale {
            min: -1.0,
            max: 1.0,
        },
        PixelOp::Colormap(Colormap::RdYlGn),
    ];

    let mut band_math_ops = vec![PixelOp::BandMath(
        (Expr::band("nir") - Expr::band("red")) / (Expr::band("nir") + Expr::band("red")),
    )];
    band_math_ops.extend(tail.clone());
    let band_math = RenderPlan::new(
        bands.clone(),
        band_math_ops,
        OutputSpec::new(TileFormat::Png),
    );

    let mut udf_ops = vec![PixelOp::Udf(stage(hash, 1))];
    udf_ops.extend(tail);
    let udf = RenderPlan::new(bands, udf_ops, OutputSpec::new(TileFormat::Png));

    let want = eval(&band_math, &inputs, &swath_render::NoUdf).expect("band math evaluates");
    let got = eval(&udf, &inputs, &executor).expect("udf evaluates");
    assert_eq!(want, got, "the two NDVI implementations must agree exactly");
}

/// One insta snapshot per example module: the exact output bit patterns
/// of a small fixed tile, held against drift across wasmtime upgrades,
/// fixture rebuilds, and adapter changes.
#[test]
fn fixture_outputs_are_snapshot_pinned() {
    /// Field order is declaration order for a derived struct — the
    /// snapshot shape cannot drift with `serde_json` feature unification
    /// (`preserve_order` on or off).
    #[derive(serde::Serialize)]
    struct PlaneBits {
        value_bits: Vec<String>,
        valid: Vec<bool>,
    }
    let executor = WasmtimeUdf::new().expect("engine builds");
    let tile_a = WarpedBuffer {
        width: 3,
        height: 2,
        values: vec![0.8, 0.6, 0.5, 0.0, 0.9, 0.0],
        valid: vec![true, true, true, true, true, false],
    };
    let tile_b = WarpedBuffer {
        width: 3,
        height: 2,
        values: vec![0.1, 0.2, -0.25, 0.0, 0.0, 0.3],
        valid: vec![true, true, true, true, false, true],
    };
    let cases: [(&str, &[u8], usize); 5] = [
        ("ndvi", NDVI, 2),
        ("hillshade", HILLSHADE, 1),
        ("qamask", QAMASK, 1),
        ("assemblyscript-double", AS_DOUBLE, 1),
        ("tinygo-negate", TINYGO_NEGATE, 1),
    ];
    for (name, bytes, arity) in cases {
        let hash = executor.compile(bytes).expect(name);
        let inputs: Vec<WarpedBuffer> = [tile_a.clone(), tile_b.clone()][..arity].to_vec();
        let out = executor
            .run(&stage(hash, 1), &inputs, &UdfLimits::default())
            .expect(name);
        let planes: Vec<PlaneBits> = out
            .planes
            .iter()
            .map(|plane| PlaneBits {
                value_bits: plane
                    .values
                    .iter()
                    .map(|v| format!("{:016x}", v.to_bits()))
                    .collect(),
                valid: plane.valid.clone(),
            })
            .collect();
        insta::assert_json_snapshot!(format!("{name}_3x2_output"), planes);
    }
}
