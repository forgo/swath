// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Pooled-instantiation criterion bench (issue #207): the fixed
//! per-invocation overhead the tile path pays before a single guest
//! instruction runs — a fresh fueled `Store` out of the pooling allocator,
//! `Instance::new` on the compiled reference module, the store's drop
//! back into the pool. This is the cost the `UdfExecutor::run` call
//! carries on top of the guest loop itself (`eval_udf_ndvi` in
//! `crates/swath-render/benches/render.rs` measures the whole stage);
//! the two together are the evidence ADR 0018's "per-request
//! instantiation at tile rates" commitment rests on.
//!
//! The store is built exactly as the executor builds it — same engine
//! configuration, fuel on, epoch deadline armed, the 64 MiB limiter —
//! so the number is the serve path's, not a stripped-down twin's.

// criterion's group/main macros generate undocumented items.
#![allow(missing_docs)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use swath_udf_wasmtime::{EPOCH_DEADLINE_MS, MEMORY_CAP_BYTES, deterministic_engine};
use wasmtime::{Engine, Instance, Module, Store, StoreLimits, StoreLimitsBuilder};

/// The reference NDVI module (`examples/udf/ndvi`).
const NDVI: &[u8] = include_bytes!("../tests/fixtures/ndvi.wasm");

/// One warm instantiation, the executor's way: a fresh limited store,
/// fueled, deadline armed, then `Instance::new` in the pooled allocator.
fn instantiate_warm(engine: &Engine, module: &Module) -> Instance {
    let limits = StoreLimitsBuilder::new()
        .memory_size(MEMORY_CAP_BYTES)
        .memories(1)
        .instances(1)
        .build();
    let mut store: Store<StoreLimits> = Store::new(engine, limits);
    store.limiter(|limits| limits);
    store.set_fuel(1_000_000).expect("fuel is on");
    store.set_epoch_deadline(EPOCH_DEADLINE_MS / 25);
    Instance::new(&mut store, module, &[]).expect("the reference module instantiates")
}

fn bench_instantiate(c: &mut Criterion) {
    let engine = deterministic_engine().expect("deterministic engine builds on this host");
    let module = Module::new(&engine, NDVI).expect("reference module compiles");
    c.bench_function("udf_instantiate_warm", |b| {
        b.iter(|| instantiate_warm(black_box(&engine), black_box(&module)));
    });
}

criterion_group!(benches, bench_instantiate);
criterion_main!(benches);
