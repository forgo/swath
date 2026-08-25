// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The executor's pinned failure taxonomy (issue #203): every ADR 0018
//! failure mode answers its own [`UdfError`] variant — a loud per-tile
//! error, never a hung worker and never UB.
//!
//! Pathological modules are hand-assembled in [`wasm`] (the same
//! no-`wat`-dependency posture as `engine_gate.rs`): the supply-chain
//! tree stays exactly wasmtime's, and each module is the *minimal* WASM
//! that exhibits one failure mode. Well-behaved modules come from the
//! committed fixtures.

use std::time::Instant;

use swath_render::WarpedBuffer;
use swath_render::udf::{UdfError, UdfExecutor, UdfStage};
use swath_udf_wasmtime::{MODULE_LRU_CAPACITY, WasmtimeUdf};

const NDVI: &[u8] = include_bytes!("fixtures/ndvi.wasm");
const TINYGO_NEGATE: &[u8] = include_bytes!("fixtures/tinygo-negate.wasm");

/// Minimal WASM binary assembly: just enough of the spec's binary format
/// to build one-failure-mode modules by hand.
mod wasm {
    /// Unsigned LEB128.
    pub(crate) fn uleb(mut v: u64) -> Vec<u8> {
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

    /// Signed LEB128.
    #[allow(
        clippy::cast_sign_loss,
        reason = "LEB128 emits the low 7 bits of each signed chunk by design"
    )]
    pub(crate) fn sleb(mut v: i64) -> Vec<u8> {
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

    /// `i32.const k` then `end`.
    pub(crate) fn ret_i32(k: i32) -> Vec<u8> {
        let mut body = vec![0x41];
        body.extend(sleb(i64::from(k)));
        body.push(0x0b);
        body
    }

    /// `i64.const k` then `end`.
    pub(crate) fn ret_i64(k: i64) -> Vec<u8> {
        let mut body = vec![0x42];
        body.extend(sleb(k));
        body.push(0x0b);
        body
    }

    /// `(loop (br 0))` — spins forever — then unreachable filler.
    pub(crate) fn spin_i64() -> Vec<u8> {
        vec![0x03, 0x40, 0x0c, 0x00, 0x0b, 0x42, 0x00, 0x0b]
    }

    /// `unreachable` then `end`.
    pub(crate) fn trap_i64() -> Vec<u8> {
        vec![0x00, 0x0b]
    }

    /// Grows linear memory to exactly the 64 MiB cap (1 + 1023 pages),
    /// then tries one page more and reports the second grow: answers
    /// `-1` as an i64 if the cap held, `0` if the runtime let it through.
    pub(crate) fn grow_past_cap_i64() -> Vec<u8> {
        let mut body = Vec::new();
        body.push(0x41); // i32.const 1023
        body.extend(sleb(1023));
        body.extend([0x40, 0x00, 0x1a]); // memory.grow, drop
        body.extend([0x41, 0x01, 0x40, 0x00]); // i32.const 1, memory.grow
        body.push(0x41); // i32.const -1
        body.extend(sleb(-1));
        body.push(0x46); // i32.eq
        body.extend([0x04, 0x7e]); // if (result i64)
        body.push(0x42); // i64.const -1
        body.extend(sleb(-1));
        body.push(0x05); // else
        body.extend([0x42, 0x00]); // i64.const 0
        body.extend([0x0b, 0x0b]); // end if, end func
        body
    }

    /// Options for [`abi_module`].
    pub(crate) struct AbiModule {
        /// What `swath_udf_abi` answers.
        pub abi: i32,
        /// `swath_udf_alloc`'s body (defaults elsewhere to `ret_i32(8)`).
        pub alloc: Vec<u8>,
        /// `swath_udf_run`'s body.
        pub run: Vec<u8>,
        /// Declared minimum memory pages.
        pub memory_pages: u32,
        /// Optional `(offset, bytes)` data segment.
        pub data: Option<(u32, Vec<u8>)>,
    }

    impl Default for AbiModule {
        fn default() -> Self {
            Self {
                abi: 1,
                alloc: ret_i32(8),
                run: ret_i64(0),
                memory_pages: 1,
                data: None,
            }
        }
    }

    /// Assembles a structurally conforming ABI v1 module: the four
    /// exports with the v1 signatures plus an exported linear memory —
    /// with each behavior injectable.
    pub(crate) fn abi_module(options: &AbiModule) -> Vec<u8> {
        let mut module = b"\0asm\x01\0\0\0".to_vec();
        // Types: 0 = () -> i32, 1 = (i32) -> i32, 2 = (i32, i32) -> i64.
        module.extend(section(
            1,
            &counted(&[
                vec![0x60, 0x00, 0x01, 0x7f],
                vec![0x60, 0x01, 0x7f, 0x01, 0x7f],
                vec![0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7e],
            ]),
        ));
        // Functions: abi, output_planes, alloc, run.
        module.extend(section(3, &counted(&[vec![0], vec![1], vec![1], vec![2]])));
        // Memory: min `memory_pages`, no max.
        let mut memory = vec![0x00];
        memory.extend(uleb(u64::from(options.memory_pages)));
        module.extend(section(5, &counted(&[memory])));
        // Exports.
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
        // Code: every body has zero locals.
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
                body(&ret_i32(options.abi)),
                body(&ret_i32(1)), // output_planes: always 1
                body(&options.alloc),
                body(&options.run),
            ]),
        ));
        if let Some((offset, bytes)) = &options.data {
            let mut segment = vec![0x00, 0x41]; // active, i32.const
            segment.extend(sleb(i64::from(*offset)));
            segment.push(0x0b);
            segment.extend(uleb(bytes.len() as u64));
            segment.extend_from_slice(bytes);
            module.extend(section(11, &counted(&[segment])));
        }
        module
    }

    /// A module importing one function — the zero-import rule's target.
    pub(crate) fn importing_module() -> Vec<u8> {
        let mut module = b"\0asm\x01\0\0\0".to_vec();
        module.extend(section(1, &counted(&[vec![0x60, 0x00, 0x00]])));
        let mut import = name("env");
        import.extend(name("clock"));
        import.extend([0x00, 0x00]); // func, type 0
        module.extend(section(2, &counted(&[import])));
        module
    }
}

fn executor() -> WasmtimeUdf {
    WasmtimeUdf::new().expect("engine builds on this host")
}

fn stage(code_hash: &str, output_planes: u32) -> UdfStage {
    UdfStage::new(code_hash, output_planes, serde_json::Value::Null)
}

fn one_plane() -> Vec<WarpedBuffer> {
    vec![WarpedBuffer {
        width: 2,
        height: 2,
        values: vec![1.0, 2.0, 3.0, 4.0],
        valid: vec![true; 4],
    }]
}

// --- tile path -------------------------------------------------------------

#[test]
fn unknown_hash_is_refused_never_compiled_inline() {
    let executor = executor();
    let err = executor
        .run(&stage("cafe", 1), &one_plane())
        .expect_err("unknown module");
    assert_eq!(
        err,
        UdfError::UnknownModule {
            code_hash: "cafe".into()
        }
    );
}

#[test]
fn empty_inputs_are_a_typed_request_error() {
    let executor = executor();
    let hash = executor.compile(TINYGO_NEGATE).expect("fixture compiles");
    assert!(matches!(
        executor.run(&stage(&hash, 1), &[]),
        Err(UdfError::InvalidRequest { .. })
    ));
}

#[test]
fn fuel_exhaustion_is_its_own_variant() {
    let executor = executor().with_fuel_budget(10_000);
    let spin = wasm::abi_module(&wasm::AbiModule {
        run: wasm::spin_i64(),
        ..wasm::AbiModule::default()
    });
    let hash = executor.compile(&spin).expect("spin module compiles");
    let err = executor
        .run(&stage(&hash, 1), &one_plane())
        .expect_err("fuel must trip");
    assert_eq!(err, UdfError::FuelExhausted { budget: 10_000 });
}

#[test]
fn epoch_deadline_stops_a_runaway_module_within_bounds() {
    // Effectively unlimited fuel: only the 250 ms wall-clock backstop can
    // stop the spin — the ADR 0012 inline posture must survive it.
    let executor = executor().with_fuel_budget(u64::MAX / 2);
    let spin = wasm::abi_module(&wasm::AbiModule {
        run: wasm::spin_i64(),
        ..wasm::AbiModule::default()
    });
    let hash = executor.compile(&spin).expect("spin module compiles");
    let started = Instant::now();
    let err = executor
        .run(&stage(&hash, 1), &one_plane())
        .expect_err("deadline must trip");
    assert_eq!(err, UdfError::EpochDeadline { deadline_ms: 250 });
    assert!(
        started.elapsed().as_millis() < 2_000,
        "a runaway module must be stopped promptly, never a hung worker \
         (took {:?})",
        started.elapsed()
    );
}

#[test]
fn guest_allocation_failure_is_memory_limit() {
    let executor = executor();
    let alloc_zero = wasm::abi_module(&wasm::AbiModule {
        alloc: wasm::ret_i32(0),
        ..wasm::AbiModule::default()
    });
    let hash = executor.compile(&alloc_zero).expect("compiles");
    assert!(matches!(
        executor.run(&stage(&hash, 1), &one_plane()),
        Err(UdfError::MemoryLimit { .. })
    ));
}

#[test]
fn memory_growth_past_the_cap_is_denied() {
    let executor = executor();
    let grower = wasm::abi_module(&wasm::AbiModule {
        run: wasm::grow_past_cap_i64(),
        ..wasm::AbiModule::default()
    });
    let hash = executor.compile(&grower).expect("compiles");
    // The module reports the over-cap grow: `-1` (denied) becomes a
    // nonsense packed pointer, caught as MalformedOutput. If the runtime
    // ever let the grow through, the module would answer `0` and this
    // would fail as GuestFailure — the assertion pins the 64 MiB cap.
    assert!(matches!(
        executor.run(&stage(&hash, 1), &one_plane()),
        Err(UdfError::MalformedOutput { .. })
    ));
}

#[test]
fn a_trapping_module_is_a_typed_trap() {
    let executor = executor();
    let trapper = wasm::abi_module(&wasm::AbiModule {
        run: wasm::trap_i64(),
        ..wasm::AbiModule::default()
    });
    let hash = executor.compile(&trapper).expect("compiles");
    assert!(matches!(
        executor.run(&stage(&hash, 1), &one_plane()),
        Err(UdfError::Trap { .. })
    ));
}

#[test]
fn guest_declared_failure_is_its_own_variant() {
    // NDVI refuses arity 1 by answering 0 from swath_udf_run.
    let executor = executor();
    let hash = executor.compile(NDVI).expect("fixture compiles");
    let err = executor
        .run(&stage(&hash, 1), &one_plane())
        .expect_err("wrong arity");
    assert_eq!(err, UdfError::GuestFailure { code_hash: hash });
}

#[test]
fn an_out_of_bounds_response_pointer_is_malformed_output() {
    let executor = executor();
    // Claims a response at 3 GiB with a 1 GiB length: the bounds check
    // must refuse before allocating anything host-side.
    let liar = wasm::abi_module(&wasm::AbiModule {
        run: wasm::ret_i64((0xC000_0000_i64 << 32) | 0x4000_0000),
        ..wasm::AbiModule::default()
    });
    let hash = executor.compile(&liar).expect("compiles");
    assert!(matches!(
        executor.run(&stage(&hash, 1), &one_plane()),
        Err(UdfError::MalformedOutput { .. })
    ));
}

#[test]
fn output_planes_disagreement_is_pinned() {
    let executor = executor();
    let hash = executor.compile(TINYGO_NEGATE).expect("fixture compiles");
    let err = executor
        .run(&stage(&hash, 3), &one_plane())
        .expect_err("negate answers 1 plane, stage pins 3");
    assert_eq!(
        err,
        UdfError::OutputPlanes {
            declared: 3,
            actual: 1
        }
    );
}

// --- compile motion --------------------------------------------------------

#[test]
fn garbage_bytes_do_not_compile() {
    assert!(matches!(
        executor().compile(b"not wasm at all"),
        Err(UdfError::InvalidModule { .. })
    ));
}

#[test]
fn an_importing_module_is_rejected_by_name() {
    let err = executor()
        .compile(&wasm::importing_module())
        .expect_err("zero-import rule");
    assert_eq!(
        err,
        UdfError::ForbiddenImport {
            module: "env".into(),
            name: "clock".into()
        }
    );
}

#[test]
fn missing_and_mis_signed_exports_are_rejected() {
    // The 8-byte header alone is a valid, empty module: it compiles,
    // then fails the export check by name.
    let bare = executor()
        .compile(&wasm::abi_module(&wasm::AbiModule::default())[..8])
        .expect_err("empty module has no exports");
    assert!(
        matches!(&bare, UdfError::MissingExport { export, .. } if export == "swath_udf_abi"),
        "got {bare:?}"
    );

    // A module with the right names but a wrong signature: rebuild the
    // skeleton with `swath_udf_run` returning i32 instead of i64 by
    // pointing its function at type 1.
    let mut mis_signed = wasm::abi_module(&wasm::AbiModule::default());
    // Function section payload for [0, 1, 1, 2] assembled by abi_module:
    // section id 3, size 5, count 4, then the type indices.
    let signature = [0x03, 0x05, 0x04, 0x00, 0x01, 0x01, 0x02];
    let position = mis_signed
        .windows(signature.len())
        .position(|window| window == signature)
        .expect("function section present");
    mis_signed[position + 6] = 0x01; // run: type 2 -> type 1
    // The body still ends with an i64 const; strip it to an i32 const so
    // the module validates: swap the run body's 0x42 (i64.const) for 0x41.
    let body_position = mis_signed
        .iter()
        .rposition(|&byte| byte == 0x42)
        .expect("run body i64.const present");
    mis_signed[body_position] = 0x41;
    let err = executor()
        .compile(&mis_signed)
        .expect_err("mis-signed export");
    assert!(
        matches!(&err, UdfError::MissingExport { export, .. } if export == "swath_udf_run"),
        "got {err:?}"
    );
}

#[test]
fn a_wrong_abi_version_is_rejected_at_registration() {
    let abi2 = wasm::abi_module(&wasm::AbiModule {
        abi: 2,
        ..wasm::AbiModule::default()
    });
    assert_eq!(
        executor().compile(&abi2).expect_err("abi 2 refused"),
        UdfError::UnsupportedAbiVersion { got: 2 }
    );
}

#[test]
fn a_module_declaring_over_the_cap_is_rejected() {
    let hog = wasm::abi_module(&wasm::AbiModule {
        memory_pages: 1025, // 64 MiB + one page
        ..wasm::AbiModule::default()
    });
    let err = executor().compile(&hog).expect_err("over-cap module");
    assert!(matches!(err, UdfError::MemoryLimit { .. }), "got {err:?}");
}

#[test]
fn compile_is_idempotent_and_content_addressed() {
    let executor = executor();
    let first = executor.compile(TINYGO_NEGATE).expect("compiles");
    let second = executor.compile(TINYGO_NEGATE).expect("recompiles");
    assert_eq!(first, second, "same bytes, same identity");
    assert_eq!(first.len(), 64, "lowercase sha256 hex");
    assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
}

#[test]
fn the_module_lru_holds_32_and_evicts_the_least_recently_used() {
    let executor = executor();
    let module_variant = |seed: u8| {
        wasm::abi_module(&wasm::AbiModule {
            data: Some((0, vec![seed])),
            ..wasm::AbiModule::default()
        })
    };
    let first = executor.compile(&module_variant(0)).expect("compiles");
    // Fill the cache: the first module stays warm until capacity + 1.
    for seed in 1..u8::try_from(MODULE_LRU_CAPACITY).unwrap() {
        executor.compile(&module_variant(seed)).expect("compiles");
    }
    // Still resident: running it fails past the lookup (GuestFailure —
    // the skeleton's run answers 0), never UnknownModule.
    assert!(matches!(
        executor.run(&stage(&first, 1), &one_plane()),
        Err(UdfError::GuestFailure { .. })
    ));
    // A 33rd distinct module evicts the *least recently used*. The run
    // above touched `first`, so it survives — under FIFO it would be the
    // victim, which is exactly what this pins.
    executor.compile(&module_variant(32)).expect("compiles");
    assert!(matches!(
        executor.run(&stage(&first, 1), &one_plane()),
        Err(UdfError::GuestFailure { .. })
    ));
}

// --- registration motion (#204): the UdfRegistrar port -------------------

/// The compiler's seam: one call registers the module and pins its
/// output arity for the plan's input count — the hash is the core's
/// `code_hash`, so the module store, the persisted layer, and the LRU
/// name the same bytes the same way.
#[test]
fn register_pins_the_content_hash_and_the_output_arity() {
    use swath_render::udf::{UdfRegistrar, UdfRegistration};
    let executor = executor();
    let registration = executor
        .register(NDVI, 2)
        .expect("ndvi registers over 2 planes");
    assert_eq!(
        registration,
        UdfRegistration::new(swath_core::udf::code_hash(NDVI), 1)
    );
    // The registered module is runnable by the same executor: a
    // registration is a promise the tile path keeps.
    let stage = stage(&registration.code_hash, registration.output_planes);
    let nir = one_plane().remove(0);
    let red = one_plane().remove(0);
    let out = executor.run(&stage, &[nir, red]).expect("runs");
    assert_eq!(out.len(), 1);
}

/// `swath_udf_output_planes` answering `<= 0` is the module refusing the
/// arity (`docs/udf-abi/v1.md`): rejected at registration, its own
/// variant.
#[test]
fn a_refused_input_arity_is_unsupported_arity() {
    use swath_render::udf::UdfRegistrar;
    let executor = executor();
    let err = executor
        .register(NDVI, 3)
        .expect_err("ndvi wants exactly 2");
    assert_eq!(
        err,
        UdfError::UnsupportedArity {
            input_planes: 3,
            output_planes: 0,
        }
    );
    // Registration failures leave the module compiled (content-addressed
    // and valid); only the arity probe refused.
    assert_eq!(
        executor.output_planes(&swath_core::udf::code_hash(NDVI), 2),
        Ok(1)
    );
}
