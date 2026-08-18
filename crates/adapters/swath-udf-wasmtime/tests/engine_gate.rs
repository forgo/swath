// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The #200 gate's executable half: the deterministic engine configuration
//! actually delivers ADR 0018's engine-level commitments, proven on a tiny
//! hand-assembled zero-import module (WAT equivalent in the comment below —
//! no `wat` dependency; the checkpoint's tree is exactly wasmtime's).

use swath_udf_wasmtime::deterministic_engine;
use wasmtime::{Instance, Module, Store, Trap};

/// ```wat
/// (module
///   (func (export "seven") (result i32) (i32.const 7))
///   (func (export "spin") (loop (br 0))))
/// ```
const MODULE: [u8; 57] = [
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x60, 0x00, 0x01, 0x7f, 0x60,
    0x00, 0x00, 0x03, 0x03, 0x02, 0x00, 0x01, 0x07, 0x10, 0x02, 0x05, 0x73, 0x65, 0x76, 0x65, 0x6e,
    0x00, 0x00, 0x04, 0x73, 0x70, 0x69, 0x6e, 0x00, 0x01, 0x0a, 0x0e, 0x02, 0x04, 0x00, 0x41, 0x07,
    0x0b, 0x07, 0x00, 0x03, 0x40, 0x0c, 0x00, 0x0b, 0x0b,
];

#[test]
fn zero_import_module_runs_with_reproducible_fuel() {
    let run = || {
        let engine = deterministic_engine().expect("engine builds on this host");
        let module = Module::new(&engine, MODULE).expect("module compiles");
        assert_eq!(
            module.imports().len(),
            0,
            "the gate module imports nothing (ADR 0018's zero-import rule)"
        );
        let mut store = Store::new(&engine, ());
        store.set_fuel(1_000_000).expect("fuel on");
        // Epoch interruption is ON (the 250 ms backstop's mechanism), so a
        // deadline must be armed; the host-side ticker arrives with #203.
        store.set_epoch_deadline(1);
        let instance =
            Instance::new(&mut store, &module, &[]).expect("pooled instantiation succeeds");
        let seven = instance
            .get_typed_func::<(), i32>(&mut store, "seven")
            .expect("export present");
        let got = seven.call(&mut store, ()).expect("call succeeds");
        assert_eq!(got, 7);
        1_000_000 - store.get_fuel().expect("fuel readable")
    };
    let (a, b) = (run(), run());
    assert!(a > 0, "fuel metering is on (consumption observed)");
    assert_eq!(
        a, b,
        "identical inputs consume identical fuel (deterministic budget)"
    );
}

#[test]
fn runaway_module_is_stopped_by_fuel_never_by_the_host_hanging() {
    let engine = deterministic_engine().expect("engine builds");
    let module = Module::new(&engine, MODULE).expect("module compiles");
    let mut store = Store::new(&engine, ());
    store.set_fuel(10_000).expect("fuel on");
    store.set_epoch_deadline(1);
    let instance = Instance::new(&mut store, &module, &[]).expect("instantiates");
    let spin = instance
        .get_typed_func::<(), ()>(&mut store, "spin")
        .expect("export present");
    let err = spin
        .call(&mut store, ())
        .expect_err("the infinite loop must trap");
    let trap = err
        .downcast_ref::<Trap>()
        .expect("a trap, not another error");
    assert_eq!(*trap, Trap::OutOfFuel, "fuel is the bound that tripped");
}
