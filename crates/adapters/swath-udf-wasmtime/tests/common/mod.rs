// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The host's side of one fixture run, shared by the conformance suite
//! (`abi_fixtures.rs`) and the golden-output suite (`golden_outputs.rs`):
//! instantiate a committed module under the real deterministic engine,
//! write a request at a guest-allocated pointer, call `swath_udf_run`,
//! read back the response bytes.

// Each test binary compiles its own copy of this module and uses only
// the helpers it needs.
#![allow(dead_code)]

use swath_udf_wasmtime::deterministic_engine;
use wasmtime::{Engine, Instance, Module, Store};

/// The real deterministic engine (#200) every fixture runs under.
pub(crate) fn engine() -> Engine {
    deterministic_engine().expect("engine builds on this host")
}

/// One instantiated fixture module plus its store.
pub(crate) struct Guest {
    store: Store<()>,
    instance: Instance,
}

impl Guest {
    pub(crate) fn new(bytes: &[u8]) -> Self {
        let engine = engine();
        let module = Module::new(&engine, bytes).expect("module compiles");
        let mut store = Store::new(&engine, ());
        store.set_fuel(1_000_000_000).expect("fuel on");
        store.set_epoch_deadline(1);
        let instance = Instance::new(&mut store, &module, &[]).expect("instantiates");
        Self { store, instance }
    }

    pub(crate) fn call_i32(&mut self, name: &str, arg: i32) -> i32 {
        self.instance
            .get_typed_func::<i32, i32>(&mut self.store, name)
            .expect("export")
            .call(&mut self.store, arg)
            .expect("call succeeds")
    }

    pub(crate) fn abi(&mut self) -> i32 {
        self.instance
            .get_typed_func::<(), i32>(&mut self.store, "swath_udf_abi")
            .expect("export")
            .call(&mut self.store, ())
            .expect("call succeeds")
    }

    /// The host's side of one run: write the request at a guest-allocated
    /// pointer, call `swath_udf_run`, read back the response bytes.
    pub(crate) fn run(&mut self, request: &[u8]) -> Option<Vec<u8>> {
        let len = i32::try_from(request.len()).expect("request fits i32");
        let ptr = self.call_i32("swath_udf_alloc", len);
        assert!(ptr > 0, "allocation failed");
        let memory = self
            .instance
            .get_memory(&mut self.store, "memory")
            .expect("exported memory");
        memory
            .write(&mut self.store, usize::try_from(ptr).unwrap(), request)
            .expect("request fits guest memory");
        let packed = self
            .instance
            .get_typed_func::<(i32, i32), i64>(&mut self.store, "swath_udf_run")
            .expect("export")
            .call(&mut self.store, (ptr, len))
            .expect("run does not trap");
        if packed == 0 {
            return None;
        }
        #[allow(clippy::cast_sign_loss)] // the packed value is a (u32, u32) pair by contract
        let (out_ptr, out_len) = (
            (packed as u64 >> 32) as usize,
            (packed as u64 & 0xFFFF_FFFF) as usize,
        );
        let mut out = vec![0u8; out_len];
        memory
            .read(&self.store, out_ptr, &mut out)
            .expect("response in bounds");
        Some(out)
    }
}
