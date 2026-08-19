// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `run_udf` executor adapter (ADR 0018, #200/#203): sandboxed,
//! deterministic WASM behind the [`UdfExecutor`] port.
//!
//! Every ADR 0018 commitment lives here, in one place, tested:
//!
//! - **NaN canonicalization** — the one WASM-spec nondeterminism, off the
//!   table platform-wide: identical inputs give byte-identical outputs;
//! - **fuel metering** — the deterministic primary budget (same inputs,
//!   same fuel consumed);
//! - **250 ms epoch deadline** — the wall-clock backstop that keeps ADR
//!   0012's inline-render posture alive under a pathological module;
//! - **pooling allocator, [`POOL_SLOTS`] slots** — per-request
//!   instantiation at tile rates, a **fresh `Store` per invocation**;
//! - **64 MiB memory cap** — declared caps checked at registration,
//!   growth bounded at run time by both the pooling allocator and a
//!   per-store [`wasmtime::StoreLimits`];
//! - **zero-import modules** — enforced at the compile motion
//!   ([`WasmtimeUdf::compile`]); the engine also provides no host
//!   functions for a module to import in the first place;
//! - **no parallel compilation** — rayon stays out of the serve path.
//!
//! Compilation is a **publish/preview motion**, never a tile-path one:
//! [`WasmtimeUdf::compile`] validates a module against the registration
//! rules (`docs/udf-abi/v1.md`) and caches it in an LRU of
//! [`MODULE_LRU_CAPACITY`] entries keyed by content hash. The tile path
//! ([`UdfExecutor::run`]) only looks hashes up — an unknown hash is
//! [`UdfError::UnknownModule`], not an inline compile.
//!
//! Every failure mode is a distinct [`UdfError`] variant (issue #203's
//! pinned taxonomy): a loud per-tile error, never a hung worker.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use sha2::{Digest, Sha256};
use swath_render::WarpedBuffer;
use swath_render::udf::{UdfError, UdfExecutor, UdfStage};
use swath_udf_guest::{AbiError, Plane, decode_response, encode_request};
use wasmtime::{
    Config, Engine, ExternType, Instance, InstanceAllocationStrategy, Memory, Module,
    PoolingAllocationConfig, Store, StoreLimits, StoreLimitsBuilder, Trap, TypedFunc, ValType,
};

/// The ADR 0018 per-instance linear-memory cap, in bytes (64 MiB).
pub const MEMORY_CAP_BYTES: usize = 64 * 1024 * 1024;

/// Pooling-allocator slots: concurrent instantiations the engine
/// pre-provisions (issue #203). Execution is synchronous per ADR 0012, so
/// slots bound concurrent *tiles*, not queued work.
pub const POOL_SLOTS: u32 = 8;

/// Compiled-module LRU capacity, keyed by content hash (issue #203).
pub const MODULE_LRU_CAPACITY: usize = 32;

/// The default per-call fuel budget — ADR 0018's deterministic primary
/// bound. Generous for real per-pixel math over a 256×256 tile; #205's
/// serve wiring makes it a configured budget axis.
pub const DEFAULT_FUEL_BUDGET: u64 = 1_000_000_000;

/// The wall-clock backstop deadline (ADR 0018): a call is interrupted at
/// most this long after it starts.
pub const EPOCH_DEADLINE_MS: u64 = 250;

/// Epoch tick interval. The deadline is armed as
/// `EPOCH_DEADLINE_MS / EPOCH_TICK_MS` ticks, so a call is interrupted
/// between `EPOCH_DEADLINE_MS - EPOCH_TICK_MS` and `EPOCH_DEADLINE_MS`
/// of wall clock — the backstop is a bound, not a stopwatch.
const EPOCH_TICK_MS: u64 = 25;

/// What can go wrong building the engine (a startup-time failure: the
/// configuration is static, so this only trips on an unsupported host).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EngineError {
    /// The runtime rejected the configuration on this host.
    #[error("wasmtime engine configuration rejected: {detail}")]
    Config {
        /// The runtime's explanation.
        detail: String,
    },
}

/// Builds the deterministic engine per ADR 0018's commitments (module
/// docs). One engine serves the process; stores/instances are per-run.
///
/// # Errors
///
/// [`EngineError::Config`] when the host cannot honor the configuration.
pub fn deterministic_engine() -> Result<Engine, EngineError> {
    let mut config = Config::new();
    config
        .cranelift_nan_canonicalization(true)
        .consume_fuel(true)
        .epoch_interruption(true)
        // No `.parallel_compilation(false)` / `.wasm_threads(false)` /
        // `.wasm_reference_types(false)` calls: those proposals'
        // features are compiled out entirely (workspace Cargo.toml
        // trims them — no gc, no threads), so the knobs do not even
        // exist — absence is the guarantee, not a setting.
        .wasm_simd(true) // deterministic under NaN canonicalization; guests may use it
        .memory_reservation(u64::try_from(MEMORY_CAP_BYTES).expect("constant fits"))
        .memory_guard_size(0);
    let mut pooling = PoolingAllocationConfig::default();
    pooling
        .max_memory_size(MEMORY_CAP_BYTES)
        .total_core_instances(POOL_SLOTS)
        .total_memories(POOL_SLOTS)
        .total_tables(POOL_SLOTS);
    config.allocation_strategy(InstanceAllocationStrategy::Pooling(pooling));
    Engine::new(&config).map_err(|err| EngineError::Config {
        detail: err.to_string(),
    })
}

/// The compiled-in runtime identity, for startup logs and the Trace
/// (`wasmtime <semver>`): operators see exactly which runtime executes
/// user code.
#[must_use]
pub fn runtime_version() -> String {
    format!("wasmtime {}", env!("CARGO_PKG_VERSION_MAJOR"))
}

/// Keeps the engine's epoch advancing so armed deadlines can trip: one
/// background thread bumps the epoch every [`EPOCH_TICK_MS`] and stops
/// when the executor drops. The tick only *observes* wall clock — it
/// never perturbs execution below the deadline, so determinism holds.
#[derive(Debug)]
struct EpochTicker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn start(engine: &Engine) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let engine = engine.clone();
        let handle = std::thread::Builder::new()
            .name("swath-udf-epoch".into())
            .spawn(move || {
                while !flag.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(EPOCH_TICK_MS));
                    engine.increment_epoch();
                }
            })
            .expect("spawning the epoch ticker thread");
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            // The thread sleeps at most one tick; joining is bounded.
            let _ = handle.join();
        }
    }
}

/// The wasmtime [`UdfExecutor`]: pooled, fueled, deterministic (ADR
/// 0018). Construct one per process; it is `Send + Sync` and cheap to
/// share — every invocation gets a fresh [`Store`] from the pooled
/// engine.
#[derive(Debug)]
pub struct WasmtimeUdf {
    engine: Engine,
    /// Front = most recently used. `Module` is internally reference
    /// counted, so cloning out of the lock is cheap.
    modules: Mutex<Vec<(String, Module)>>,
    fuel_budget: u64,
    _ticker: EpochTicker,
}

impl WasmtimeUdf {
    /// Builds the executor over a fresh deterministic engine and starts
    /// the epoch ticker.
    ///
    /// # Errors
    ///
    /// [`UdfError::NoRuntime`] when the host rejects the engine
    /// configuration.
    pub fn new() -> Result<Self, UdfError> {
        let engine = deterministic_engine().map_err(|err| UdfError::NoRuntime {
            detail: err.to_string(),
        })?;
        let ticker = EpochTicker::start(&engine);
        Ok(Self {
            engine,
            modules: Mutex::new(Vec::new()),
            fuel_budget: DEFAULT_FUEL_BUDGET,
            _ticker: ticker,
        })
    }

    /// Sets the per-call fuel budget (#205 wires this from serve
    /// configuration; the default is [`DEFAULT_FUEL_BUDGET`]).
    #[must_use]
    pub fn with_fuel_budget(mut self, fuel: u64) -> Self {
        self.fuel_budget = fuel;
        self
    }

    /// The publish/preview compile motion: validates `bytes` against the
    /// registration rules (`docs/udf-abi/v1.md` — zero imports, the four
    /// v1 exports with the v1 signatures, an exported linear memory
    /// within the 64 MiB cap, `swath_udf_abi() == 1`), compiles, and
    /// caches the module in the LRU. Returns the content hash (lowercase
    /// sha256 hex) — the identity a [`UdfStage`] carries.
    ///
    /// Never called on the tile path: [`UdfExecutor::run`] only looks
    /// hashes up.
    ///
    /// # Errors
    ///
    /// [`UdfError::InvalidModule`], [`UdfError::ForbiddenImport`],
    /// [`UdfError::MissingExport`], [`UdfError::MemoryLimit`], or
    /// [`UdfError::UnsupportedAbiVersion`] per the failed rule;
    /// [`UdfError::Trap`]/[`UdfError::FuelExhausted`]/
    /// [`UdfError::EpochDeadline`] if the version probe itself
    /// misbehaves.
    pub fn compile(&self, bytes: &[u8]) -> Result<String, UdfError> {
        let code_hash = format!("{:x}", Sha256::digest(bytes));
        if self.lookup(&code_hash).is_some() {
            return Ok(code_hash);
        }
        check_declared_memory(bytes)?;
        let module = Module::new(&self.engine, bytes).map_err(|err| UdfError::InvalidModule {
            detail: err.to_string(),
        })?;
        validate_shape(&module)?;
        // Probe the ABI version in a bounded store — the registration
        // motion may instantiate; the tile path never compiles.
        let mut store = self.fresh_store();
        let instance = instantiate(&mut store, &module)?;
        let abi: TypedFunc<(), i32> = typed_export(&instance, &mut store, "swath_udf_abi")?;
        let got = abi
            .call(&mut store, ())
            .map_err(|err| self.map_call_error(&err))?;
        if got != swath_udf_guest::ABI_VERSION {
            return Err(UdfError::UnsupportedAbiVersion { got });
        }
        let mut modules = self.modules.lock().expect("module LRU lock");
        modules.retain(|(hash, _)| *hash != code_hash);
        modules.insert(0, (code_hash.clone(), module));
        modules.truncate(MODULE_LRU_CAPACITY);
        Ok(code_hash)
    }

    /// Looks `code_hash` up in the LRU, marking it most recently used.
    fn lookup(&self, code_hash: &str) -> Option<Module> {
        let mut modules = self.modules.lock().expect("module LRU lock");
        let index = modules.iter().position(|(hash, _)| hash == code_hash)?;
        let entry = modules.remove(index);
        let module = entry.1.clone();
        modules.insert(0, entry);
        Some(module)
    }

    /// A fresh per-invocation store: fuel budget, epoch deadline, and
    /// the 64 MiB [`StoreLimits`] armed.
    fn fresh_store(&self) -> Store<StoreLimits> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(MEMORY_CAP_BYTES)
            .memories(1)
            .instances(1)
            .build();
        let mut store = Store::new(&self.engine, limits);
        store.limiter(|limits| limits);
        store
            .set_fuel(self.fuel_budget)
            .expect("consume_fuel is on in the deterministic engine");
        store.set_epoch_deadline(EPOCH_DEADLINE_MS.div_ceil(EPOCH_TICK_MS));
        store
    }

    /// Maps a guest-call failure onto the pinned taxonomy: fuel and
    /// deadline traps get their own variants, everything else that
    /// trapped is [`UdfError::Trap`].
    fn map_call_error(&self, err: &wasmtime::Error) -> UdfError {
        match err.downcast_ref::<Trap>() {
            Some(Trap::OutOfFuel) => UdfError::FuelExhausted {
                budget: self.fuel_budget,
            },
            Some(Trap::Interrupt) => UdfError::EpochDeadline {
                deadline_ms: EPOCH_DEADLINE_MS,
            },
            Some(trap) => UdfError::Trap {
                detail: trap.to_string(),
            },
            None => UdfError::Trap {
                detail: format!("{err:#}"),
            },
        }
    }
}

impl UdfExecutor for WasmtimeUdf {
    fn run(
        &self,
        stage: &UdfStage,
        inputs: &[WarpedBuffer],
    ) -> Result<Vec<WarpedBuffer>, UdfError> {
        let module = self
            .lookup(&stage.code_hash)
            .ok_or_else(|| UdfError::UnknownModule {
                code_hash: stage.code_hash.clone(),
            })?;
        let request = encode_inputs(inputs)?;
        let (width, height) = (inputs[0].width, inputs[0].height);

        let mut store = self.fresh_store();
        let instance = instantiate(&mut store, &module)?;
        let memory =
            instance
                .get_memory(&mut store, "memory")
                .ok_or_else(|| UdfError::MissingExport {
                    export: "memory".into(),
                    detail: "no exported linear memory".into(),
                })?;

        // Transfer in: guest-allocate, then one bulk copy.
        let alloc: TypedFunc<i32, i32> = typed_export(&instance, &mut store, "swath_udf_alloc")?;
        let len = i32::try_from(request.len()).map_err(|_| UdfError::InvalidRequest {
            detail: format!(
                "request of {} bytes exceeds the i32 wire limit",
                request.len()
            ),
        })?;
        let ptr = alloc
            .call(&mut store, len)
            .map_err(|err| self.map_call_error(&err))?;
        if ptr <= 0 {
            return Err(UdfError::MemoryLimit {
                detail: format!("guest could not allocate the {len}-byte request buffer"),
            });
        }
        write_guest(&mut store, &memory, ptr, &request)?;

        // Execute, bounded by fuel and the epoch deadline.
        let run: TypedFunc<(i32, i32), i64> = typed_export(&instance, &mut store, "swath_udf_run")?;
        let packed = run
            .call(&mut store, (ptr, len))
            .map_err(|err| self.map_call_error(&err))?;
        if packed == 0 {
            return Err(UdfError::GuestFailure {
                code_hash: stage.code_hash.clone(),
            });
        }

        // Transfer out: bounds-check the guest's claim before touching it.
        let response = read_guest(&store, &memory, packed)?;
        decode_outputs(width, height, stage.output_planes, &response)
    }
}

/// Encodes the plan's warped planes as one ABI v1 request buffer.
fn encode_inputs(inputs: &[WarpedBuffer]) -> Result<Vec<u8>, UdfError> {
    let first = inputs.first().ok_or_else(|| UdfError::InvalidRequest {
        detail: "no input planes".into(),
    })?;
    let planes: Vec<Plane> = inputs
        .iter()
        .map(|buffer| Plane {
            values: buffer.values.clone(),
            validity: buffer.valid.iter().map(|&ok| u8::from(ok)).collect(),
        })
        .collect();
    encode_request(first.width, first.height, &planes).map_err(|err| UdfError::InvalidRequest {
        detail: err.to_string(),
    })
}

/// Instantiates in the pooled allocator; failures are resource-bound
/// failures (the 64 MiB cap, pool slots) by construction.
fn instantiate(store: &mut Store<StoreLimits>, module: &Module) -> Result<Instance, UdfError> {
    Instance::new(&mut *store, module, &[]).map_err(|err| UdfError::MemoryLimit {
        detail: format!("instantiation failed: {err:#}"),
    })
}

/// Resolves a typed export, mapping absence/mistyping onto
/// [`UdfError::MissingExport`].
fn typed_export<P, R>(
    instance: &Instance,
    store: &mut Store<StoreLimits>,
    name: &str,
) -> Result<TypedFunc<P, R>, UdfError>
where
    P: wasmtime::WasmParams,
    R: wasmtime::WasmResults,
{
    instance
        .get_typed_func::<P, R>(store, name)
        .map_err(|err| UdfError::MissingExport {
            export: name.into(),
            detail: err.to_string(),
        })
}

/// Writes the request into guest memory, bounds-checked: a bogus
/// allocation pointer is a typed error, never UB.
fn write_guest(
    store: &mut Store<StoreLimits>,
    memory: &Memory,
    ptr: i32,
    request: &[u8],
) -> Result<(), UdfError> {
    let offset = usize::try_from(ptr).expect("ptr > 0 checked");
    memory
        .write(store, offset, request)
        .map_err(|_| UdfError::MalformedOutput {
            detail: format!(
                "swath_udf_alloc answered pointer {ptr}, out of bounds for a \
                 {}-byte request",
                request.len()
            ),
        })
}

/// Reads the response the guest named with its packed `(ptr << 32) | len`
/// answer, bounds-checking against the memory's actual size *before*
/// allocating host-side — a hostile length claim cannot balloon the host.
fn read_guest(
    store: &Store<StoreLimits>,
    memory: &Memory,
    packed: i64,
) -> Result<Vec<u8>, UdfError> {
    #[allow(
        clippy::cast_sign_loss,
        reason = "the packed value is a (u32, u32) pair by ABI contract"
    )]
    let (out_ptr, out_len) = (
        (packed as u64 >> 32) as usize,
        (packed as u64 & 0xFFFF_FFFF) as usize,
    );
    let end = out_ptr.checked_add(out_len);
    if end.is_none_or(|end| end > memory.data_size(store)) {
        return Err(UdfError::MalformedOutput {
            detail: format!("response {out_ptr}..+{out_len} is out of guest memory bounds"),
        });
    }
    let mut out = vec![0u8; out_len];
    memory
        .read(store, out_ptr, &mut out)
        .map_err(|_| UdfError::MalformedOutput {
            detail: format!("response {out_ptr}..+{out_len} is out of guest memory bounds"),
        })?;
    Ok(out)
}

/// Decodes a response buffer into tile-shaped [`WarpedBuffer`]s, mapping
/// every wire violation onto the pinned taxonomy. Strict parsing, all
/// lengths checked: malformed bytes are typed errors, never UB.
fn decode_outputs(
    width: u32,
    height: u32,
    declared_planes: u32,
    response: &[u8],
) -> Result<Vec<WarpedBuffer>, UdfError> {
    let decoded = decode_response(width, height, response).map_err(|err| match err {
        AbiError::AbiVersion => UdfError::MalformedOutput {
            detail: "response header claims an ABI other than 1".into(),
        },
        other => UdfError::MalformedOutput {
            detail: other.to_string(),
        },
    })?;
    let actual = u32::try_from(decoded.planes.len()).unwrap_or(u32::MAX);
    if actual != declared_planes {
        return Err(UdfError::OutputPlanes {
            declared: declared_planes,
            actual,
        });
    }
    Ok(decoded
        .planes
        .into_iter()
        .map(|plane| WarpedBuffer {
            width,
            height,
            values: plane.values,
            valid: plane.validity.into_iter().map(|flag| flag != 0).collect(),
        })
        .collect())
}

/// Pre-checks the module's *declared* memory minimum against the 64 MiB
/// cap by scanning the binary's memory section directly, so an over-cap
/// declaration is [`UdfError::MemoryLimit`] — the taxonomy's name for it —
/// rather than whatever the pooled engine's compile error happens to say.
/// Anything irregular is left for `Module::new` to diagnose properly.
fn check_declared_memory(bytes: &[u8]) -> Result<(), UdfError> {
    let cap_pages = (MEMORY_CAP_BYTES / 65536) as u64;
    // uleb128 decode at `pos`, advancing it; None on any irregularity.
    let uleb = |bytes: &[u8], pos: &mut usize| -> Option<u64> {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = *bytes.get(*pos)?;
            *pos += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    };
    let scan = |bytes: &[u8]| -> Option<u64> {
        let mut pos = bytes.len().checked_sub(8).is_some().then_some(8)?;
        while pos < bytes.len() {
            let id = *bytes.get(pos)?;
            pos += 1;
            let size = usize::try_from(uleb(bytes, &mut pos)?).ok()?;
            let end = pos.checked_add(size)?;
            if id != 5 {
                pos = end;
                continue;
            }
            // Memory section: count, then per memory `flags` + uleb min.
            let count = uleb(bytes, &mut pos)?;
            if count == 0 {
                return None;
            }
            pos += 1; // limits flags; the minimum follows in every form
            return uleb(bytes, &mut pos);
        }
        None
    };
    match scan(bytes) {
        Some(minimum) if minimum > cap_pages => Err(UdfError::MemoryLimit {
            detail: format!(
                "module declares {minimum} pages of linear memory, cap is {cap_pages} (64 MiB)"
            ),
        }),
        _ => Ok(()),
    }
}

/// Registration shape checks: zero imports, the four v1 exports with the
/// v1 signatures, an exported linear memory within the 64 MiB cap.
fn validate_shape(module: &Module) -> Result<(), UdfError> {
    if let Some(import) = module.imports().next() {
        return Err(UdfError::ForbiddenImport {
            module: import.module().to_owned(),
            name: import.name().to_owned(),
        });
    }
    expect_func(module, "swath_udf_abi", &[], &[ValType::I32])?;
    expect_func(
        module,
        "swath_udf_output_planes",
        &[ValType::I32],
        &[ValType::I32],
    )?;
    expect_func(module, "swath_udf_alloc", &[ValType::I32], &[ValType::I32])?;
    expect_func(
        module,
        "swath_udf_run",
        &[ValType::I32, ValType::I32],
        &[ValType::I64],
    )?;
    let Some(ExternType::Memory(memory)) = module.get_export("memory") else {
        return Err(UdfError::MissingExport {
            export: "memory".into(),
            detail: "no exported linear memory".into(),
        });
    };
    let cap_pages = (MEMORY_CAP_BYTES / 65536) as u64;
    if memory.minimum() > cap_pages {
        return Err(UdfError::MemoryLimit {
            detail: format!(
                "module declares {} pages of linear memory, cap is {cap_pages} (64 MiB)",
                memory.minimum()
            ),
        });
    }
    Ok(())
}

/// Checks one exported function's exact v1 signature.
fn expect_func(
    module: &Module,
    name: &str,
    params: &[ValType],
    results: &[ValType],
) -> Result<(), UdfError> {
    let missing = |detail: String| UdfError::MissingExport {
        export: name.to_owned(),
        detail,
    };
    match module.get_export(name) {
        Some(ExternType::Func(func)) => {
            let same = |have: &mut dyn Iterator<Item = ValType>, want: &[ValType]| {
                let have: Vec<ValType> = have.collect();
                have.len() == want.len()
                    && have
                        .iter()
                        .zip(want)
                        .all(|(a, b)| a.matches(b) && b.matches(a))
            };
            if same(&mut func.params(), params) && same(&mut func.results(), results) {
                Ok(())
            } else {
                Err(missing(format!(
                    "signature is {func}, not the v1 signature"
                )))
            }
        }
        Some(other) => Err(missing(format!("export is {other:?}, expected a func"))),
        None => Err(missing("export absent".into())),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use swath_render::udf::UdfError;
    use swath_udf_guest::{Plane, Response, encode_response};

    use super::decode_outputs;

    /// A valid one-plane 2×2 response to mutate.
    fn good_response() -> Vec<u8> {
        let plane = Plane {
            values: vec![1.0, 2.0, 3.0, 4.0],
            validity: vec![1, 1, 0, 1],
        };
        encode_response(
            2,
            2,
            &Response {
                planes: vec![plane],
            },
        )
        .expect("encodes")
    }

    #[test]
    fn plane_count_disagreement_is_output_planes() {
        let err = decode_outputs(2, 2, 3, &good_response()).expect_err("count pinned");
        assert_eq!(
            err,
            UdfError::OutputPlanes {
                declared: 3,
                actual: 1
            }
        );
    }

    #[test]
    fn good_response_decodes_to_tile_shaped_buffers() {
        let out = decode_outputs(2, 2, 1, &good_response()).expect("decodes");
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].width, out[0].height), (2, 2));
        assert_eq!(out[0].values, vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(out[0].valid, vec![true, true, false, true]);
    }

    proptest! {
        /// Issue #203: malformed output headers/buffers are typed errors,
        /// never UB, never a panic — for arbitrary corruptions of a valid
        /// response and for arbitrary garbage.
        #[test]
        fn corrupted_responses_are_typed_errors(
            index in 0usize..100,
            byte in any::<u8>(),
            truncate in 0usize..100,
        ) {
            let mut buf = good_response();
            let index = index % buf.len();
            buf[index] = byte;
            buf.truncate(buf.len().saturating_sub(truncate % buf.len()));
            // Every outcome is a typed Result — the call must not panic,
            // and any error must be a taxonomy variant.
            if let Err(err) = decode_outputs(2, 2, 1, &buf) {
                let typed = matches!(
                    err,
                    UdfError::MalformedOutput { .. } | UdfError::OutputPlanes { .. }
                );
                prop_assert!(typed, "unexpected variant: {err:?}");
            }
        }

        #[test]
        fn arbitrary_garbage_is_a_typed_error(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
            if let Err(err) = decode_outputs(2, 2, 1, &bytes) {
                let typed = matches!(
                    err,
                    UdfError::MalformedOutput { .. } | UdfError::OutputPlanes { .. }
                );
                prop_assert!(typed, "unexpected variant: {err:?}");
            }
        }
    }
}
