// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `run_udf` executor adapter's foundation (ADR 0018, #200): the
//! **deterministic engine configuration** every later piece (#203's
//! executor, #204's module store) builds on, landed with the supply-chain
//! gate so the configuration itself is what the checkpoint reviews.
//!
//! What #200 deliberately does NOT contain: module loading, the ABI,
//! execution wiring — those arrive with #201–#205 behind this engine.
//!
//! Every ADR 0018 commitment that is an engine property is set here, in
//! one place, tested:
//!
//! - **NaN canonicalization** — the one WASM-spec nondeterminism, off the
//!   table platform-wide;
//! - **fuel metering** — the deterministic budget (same inputs, same fuel);
//! - **epoch interruption** — the 250 ms wall-clock backstop's mechanism;
//! - **pooling allocator** — per-request instantiation at tile rates,
//!   with the 64 MiB per-instance memory cap enforced by configuration;
//! - **no parallel compilation** — rayon stays out of the serve path.
//!
//! Zero-import enforcement is a *loader* rule (#204 rejects importing
//! modules at registration); the engine here provides no host functions
//! for a module to import in the first place.

use wasmtime::{Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig};

/// The ADR 0018 per-instance linear-memory cap, in bytes (64 MiB).
pub const MEMORY_CAP_BYTES: usize = 64 * 1024 * 1024;

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
    pooling.max_memory_size(MEMORY_CAP_BYTES);
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
