// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! ABI property tests over the real executor (issue #203): the wire
//! contract holds for arbitrary tiles, not just the fixtures' examples.
//!
//! - **Identity round-trip through linear memory**: negating twice is the
//!   identity, bit for bit — the request/response transfer never perturbs
//!   a value on its way through guest memory.
//! - **Validity-AND invariant**: a guest that claims *everything* valid
//!   (a hand-assembled module answering a canned all-valid response)
//!   cannot resurrect a single pixel the input marked invalid, and its
//!   non-finite "valid" values are canonicalized to invalid.

use proptest::prelude::*;
use swath_render::ir::{BandInput, OutputSpec, PixelOp, TileFormat};
use swath_render::udf::{UdfExecutor, UdfStage};
use swath_render::{NoUdf, RenderPlan, WarpedBuffer, eval};
use swath_udf_guest::{Plane, Response, encode_response};
use swath_udf_wasmtime::WasmtimeUdf;

const TINYGO_NEGATE: &[u8] = include_bytes!("fixtures/tinygo-negate.wasm");

fn stage(code_hash: &str) -> UdfStage {
    UdfStage::new(code_hash, 1, serde_json::Value::Null)
}

/// A strategy for one small tile: dimensions, finite values, arbitrary
/// validity.
fn tile() -> impl Strategy<Value = WarpedBuffer> {
    (1u32..=6, 1u32..=6)
        .prop_flat_map(|(width, height)| {
            let len = (width * height) as usize;
            (
                Just(width),
                Just(height),
                proptest::collection::vec(
                    any::<f64>().prop_filter("finite", |v| v.is_finite()),
                    len,
                ),
                proptest::collection::vec(any::<bool>(), len),
            )
        })
        .prop_map(|(width, height, values, valid)| WarpedBuffer {
            width,
            height,
            values,
            valid,
        })
}

proptest! {
    // Each case is a full instantiate-run cycle; keep the count sane.
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// fneg twice is the identity: arbitrary tiles come back from two
    /// trips through guest linear memory with every bit intact, validity
    /// passed through untouched.
    #[test]
    fn negate_twice_round_trips_bit_exactly(input in tile()) {
        static EXECUTOR: std::sync::LazyLock<(WasmtimeUdf, String)> =
            std::sync::LazyLock::new(|| {
                let executor = WasmtimeUdf::new().expect("engine builds");
                let hash = executor.compile(TINYGO_NEGATE).expect("fixture compiles");
                (executor, hash)
            });
        let (executor, hash) = &*EXECUTOR;
        let once = executor
            .run(&stage(hash), std::slice::from_ref(&input))
            .expect("first negate");
        let twice = executor.run(&stage(hash), &once).expect("second negate");
        prop_assert_eq!(twice.len(), 1);
        let round_tripped = &twice[0];
        prop_assert_eq!(&round_tripped.valid, &input.valid, "validity passthrough");
        for (index, (got, want)) in round_tripped
            .values
            .iter()
            .zip(&input.values)
            .enumerate()
        {
            prop_assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "pixel {}: bit-exact round trip",
                index
            );
        }
    }

    /// The guest cannot re-validate: a module claiming every pixel valid
    /// (including one non-finite value) still renders with the host's
    /// AND — output validity is a subset of input validity, always, and
    /// the NaN pixel is invalid unconditionally.
    #[test]
    fn an_all_valid_claim_cannot_resurrect_invalid_pixels(
        valid in proptest::collection::vec(any::<bool>(), 4),
        values in proptest::collection::vec(-1.0e6f64..1.0e6, 4),
    ) {
        static EXECUTOR: std::sync::LazyLock<(WasmtimeUdf, String)> =
            std::sync::LazyLock::new(|| {
                let executor = WasmtimeUdf::new().expect("engine builds");
                let hash = executor
                    .compile(&all_valid_claimer())
                    .expect("claimer compiles");
                (executor, hash)
            });
        let (executor, hash) = &*EXECUTOR;
        let input = WarpedBuffer {
            width: 2,
            height: 2,
            values,
            valid: valid.clone(),
        };
        let plan = RenderPlan::new(
            vec![BandInput::new("x")],
            vec![PixelOp::Udf(stage(hash))],
            OutputSpec::new(TileFormat::Png),
        );
        let tile = eval(&plan, std::slice::from_ref(&input), executor).expect("evaluates");
        // Sanity: without a wired executor the same plan refuses.
        prop_assert!(eval(&plan, std::slice::from_ref(&input), &NoUdf).is_err());
        for (pixel, &input_valid) in valid.iter().enumerate() {
            let alpha = tile.pixels[pixel * 4 + 3];
            // Pixel 1 of the canned response is NaN claimed valid: the
            // non-finite post-condition invalidates it regardless.
            let expect_valid = input_valid && pixel != 1;
            prop_assert_eq!(
                alpha != 0,
                expect_valid,
                "pixel {}: output validity must be input AND guest AND finite",
                pixel
            );
        }
    }
}

/// Hand-assembles a module whose `swath_udf_run` ignores its input and
/// answers a canned 2×2 response claiming all four pixels valid — with
/// pixel 1 a NaN. Built from the same minimal-assembly helpers as
/// `executor.rs`, with the canned bytes produced by the guest kit's own
/// encoder (so the *frame* is well-formed; only the claim is hostile).
fn all_valid_claimer() -> Vec<u8> {
    const DATA_OFFSET: u32 = 4096;
    let canned = encode_response(
        2,
        2,
        &Response {
            planes: vec![Plane {
                values: vec![42.0, f64::NAN, 7.0, 200.0],
                validity: vec![1, 1, 1, 1],
            }],
        },
    )
    .expect("canned response encodes");
    let len = u32::try_from(canned.len()).expect("tiny response");
    let packed = (i64::from(DATA_OFFSET) << 32) | i64::from(len);
    minimal_wasm::abi_module_with_data(packed, DATA_OFFSET, &canned)
}

/// The subset of `executor.rs`'s assembler this file needs (integration
/// tests cannot share modules without a common crate; the duplication is
/// ~60 lines of spec bytes and deliberate — each file stays runnable
/// alone).
mod minimal_wasm {
    fn uleb(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    #[allow(
        clippy::cast_sign_loss,
        reason = "LEB128 emits the low 7 bits of each signed chunk by design"
    )]
    fn sleb(mut v: i64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            let sign = byte & 0x40 != 0;
            if (v == 0 && !sign) || (v == -1 && sign) {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    fn section(id: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![id];
        out.extend(uleb(payload.len() as u64));
        out.extend_from_slice(payload);
        out
    }

    fn counted(items: &[Vec<u8>]) -> Vec<u8> {
        let mut out = uleb(items.len() as u64);
        for item in items {
            out.extend_from_slice(item);
        }
        out
    }

    fn name(s: &str) -> Vec<u8> {
        let mut out = uleb(s.len() as u64);
        out.extend_from_slice(s.as_bytes());
        out
    }

    /// A conforming ABI v1 skeleton whose `run` answers the constant
    /// `packed`, with `data` planted at `data_offset`.
    pub(crate) fn abi_module_with_data(packed: i64, data_offset: u32, data: &[u8]) -> Vec<u8> {
        let mut module = b"\0asm\x01\0\0\0".to_vec();
        module.extend(section(
            1,
            &counted(&[
                vec![0x60, 0x00, 0x01, 0x7f],
                vec![0x60, 0x01, 0x7f, 0x01, 0x7f],
                vec![0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7e],
            ]),
        ));
        module.extend(section(3, &counted(&[vec![0], vec![1], vec![1], vec![2]])));
        module.extend(section(5, &counted(&[vec![0x00, 0x01]])));
        let export = |n: &str, kind: u8, index: u64| {
            let mut out = name(n);
            out.push(kind);
            out.extend(uleb(index));
            out
        };
        module.extend(section(
            7,
            &counted(&[
                export("memory", 0x02, 0),
                export("swath_udf_abi", 0x00, 0),
                export("swath_udf_output_planes", 0x00, 1),
                export("swath_udf_alloc", 0x00, 2),
                export("swath_udf_run", 0x00, 3),
            ]),
        ));
        let ret_i32 = |k: i64| {
            let mut code = vec![0x41];
            code.extend(sleb(k));
            code.push(0x0b);
            code
        };
        let ret_i64 = |k: i64| {
            let mut code = vec![0x42];
            code.extend(sleb(k));
            code.push(0x0b);
            code
        };
        let body = |code: &[u8]| {
            let mut entry = vec![0x00];
            entry.extend_from_slice(code);
            let mut out = uleb(entry.len() as u64);
            out.extend(entry);
            out
        };
        module.extend(section(
            10,
            &counted(&[
                body(&ret_i32(1)),
                body(&ret_i32(1)),
                body(&ret_i32(8)),
                body(&ret_i64(packed)),
            ]),
        ));
        let mut segment = vec![0x00, 0x41];
        segment.extend(sleb(i64::from(data_offset)));
        segment.push(0x0b);
        segment.extend(uleb(data.len() as u64));
        segment.extend_from_slice(data);
        module.extend(section(11, &counted(&[segment])));
        module
    }
}
