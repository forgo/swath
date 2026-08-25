// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `run_udf` executor **port** (ADR 0018, issue #201): the seam
//! between the Render IR's [`PixelOp::Udf`](crate::ir::PixelOp::Udf)
//! stage and whatever actually runs the sandboxed WASM module.
//!
//! This crate defines only the trait; the wasmtime adapter (#203) lives in
//! `swath-udf-wasmtime` behind it, so **swath-render never depends on
//! wasmtime** — the same port/adapter posture every other boundary in the
//! engine keeps (ARCHITECTURE.md, ADR 0013). The seam is also ADR 0018's
//! rollback lever: moving UDF execution onto a bounded worker pool, or
//! withdrawing it from the live tile path entirely, is a wiring change at
//! this trait, not an IR redesign.
//!
//! A [`UdfStage`] names the module by **content hash** — the module bytes
//! never enter the IR; the module store (#204) resolves the hash at
//! execution time. [`NoUdf`] is the default executor for deployments with
//! no UDF support wired: it refuses every stage, and since
//! [`eval`](crate::ir::eval) consults the executor only when it reaches a
//! `Udf` op, plans without UDF stages never touch it.

use serde::{Deserialize, Serialize};

use crate::warp::WarpedBuffer;

/// One `run_udf` stage of a [`RenderPlan`](crate::ir::RenderPlan): the
/// sandboxed module (by content hash), its pinned output arity, and its
/// opaque parameters.
///
/// This is IR **data** — serde round-trippable, snapshot-pinned — and the
/// argument [`UdfExecutor::run`] receives. The module bytes themselves are
/// deliberately absent (plans stay small, cacheable, and hash-addressed);
/// the module store (#204) owns hash → bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UdfStage {
    /// Lowercase sha256 hex of the registered module bytes — the
    /// content-addressed module identity (ADR 0018). Registration (#204)
    /// computes it; serving resolves it; the IR only carries it.
    pub code_hash: String,
    /// How many planes the module produces per tile — the
    /// `swath_udf_output_planes` answer pinned at registration
    /// (`docs/udf-abi/v1.md`). [`eval`](crate::ir::eval) renders 1 plane
    /// as gray and 3 as RGB; other counts are a plan error.
    pub output_planes: u32,
    /// Opaque UDF parameters (openEO `run_udf`'s `context` argument),
    /// carried verbatim for the executor. `Null` when the caller passed
    /// none. Part of the plan — and therefore of cache identity (#205).
    pub params: serde_json::Value,
}

impl UdfStage {
    /// A stage running the module named by `code_hash`, producing
    /// `output_planes` planes, with `params` as its opaque parameters.
    #[must_use]
    pub fn new(
        code_hash: impl Into<String>,
        output_planes: u32,
        params: serde_json::Value,
    ) -> Self {
        Self {
            code_hash: code_hash.into(),
            output_planes,
            params,
        }
    }
}

/// Why a UDF stage could not be executed. Distinct from
/// [`PlanError`](crate::ir::PlanError)'s structural variants: these are
/// the executor port's failures, wrapped into the plan taxonomy as
/// [`PlanError::Udf`](crate::ir::PlanError::Udf).
///
/// The taxonomy is pinned by issue #203 and implemented by the wasmtime
/// adapter (`swath-udf-wasmtime`); every ADR 0018 failure mode is a
/// distinct variant — a loud per-tile error, never a hung worker and
/// never a stringly-typed catch-all. Registration-motion failures
/// ([`InvalidModule`](Self::InvalidModule) through
/// [`UnsupportedAbiVersion`](Self::UnsupportedAbiVersion)) and tile-path
/// failures ([`UnknownModule`](Self::UnknownModule) onward) share the one
/// enum because the port has one error channel. `#[non_exhaustive]`: a
/// future adapter may still add variants without breaking consumers.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum UdfError {
    /// No UDF executor is wired into this deployment ([`NoUdf`]): the
    /// plan names a module, but nothing can run it. Serve wiring arrives
    /// with #205; until then every UDF plan refuses loudly here.
    #[error("no UDF executor is configured: plan names module `{code_hash}` (ADR 0018)")]
    NotConfigured {
        /// The module the plan asked for.
        code_hash: String,
    },
    /// The WASM runtime is unavailable on this host: the deterministic
    /// engine configuration (ADR 0018) was rejected at executor
    /// construction. A startup-time failure — the configuration is
    /// static, so this only trips on an unsupported host.
    #[error("WASM runtime unavailable: {detail}")]
    NoRuntime {
        /// The runtime's explanation.
        detail: String,
    },
    /// Registration: the bytes are not a valid WASM module the engine
    /// can compile.
    #[error("module does not compile: {detail}")]
    InvalidModule {
        /// The compiler's explanation.
        detail: String,
    },
    /// Registration: the module imports something. Zero-import modules
    /// are ADR 0018's structural determinism guarantee — with no imports
    /// there is nothing nondeterministic to call.
    #[error("module imports `{module}`.`{name}`: zero-import rule (ADR 0018)")]
    ForbiddenImport {
        /// The import's module namespace.
        module: String,
        /// The imported symbol.
        name: String,
    },
    /// Registration: a required ABI v1 export (`swath_udf_abi`,
    /// `swath_udf_output_planes`, `swath_udf_alloc`, `swath_udf_run`, or
    /// the linear `memory`) is absent or has the wrong signature.
    #[error("module export `{export}` missing or mis-typed: {detail}")]
    MissingExport {
        /// The export that failed the check.
        export: String,
        /// What was found instead.
        detail: String,
    },
    /// Registration: `swath_udf_abi` answered something other than `1`
    /// (`docs/udf-abi/v1.md`: the next incompatible contract is a new
    /// version, never a silent blend).
    #[error("module speaks UDF ABI {got}, this host speaks 1")]
    UnsupportedAbiVersion {
        /// The version the module claimed.
        got: i32,
    },
    /// Tile path: the stage names a module hash the executor has not
    /// compiled. Compilation happens at the publish/preview motion, never
    /// the tile path — an unknown hash is refused, not compiled inline.
    #[error("module `{code_hash}` is not registered with this executor")]
    UnknownModule {
        /// The hash the plan asked for.
        code_hash: String,
    },
    /// Tile path: the input planes cannot be encoded as an ABI v1
    /// request (no planes, zero dimensions, mismatched plane shapes, or
    /// a request too large for the wire). Host-side and unreachable
    /// through a validated plan — kept loud rather than panicking.
    #[error("input planes cannot form a v1 request: {detail}")]
    InvalidRequest {
        /// Which precondition failed.
        detail: String,
    },
    /// The deterministic fuel budget — ADR 0018's primary bound — ran
    /// out. Reproducible: identical inputs consume identical fuel, so
    /// this either always trips for a given tile or never does.
    #[error("UDF exhausted its fuel budget of {budget}")]
    FuelExhausted {
        /// The budget the call was given.
        budget: u64,
    },
    /// The wall-clock epoch deadline — the backstop that keeps ADR
    /// 0012's inline-render posture alive under a pathological module —
    /// interrupted the call.
    #[error("UDF exceeded the {deadline_ms} ms epoch deadline")]
    EpochDeadline {
        /// The deadline, in milliseconds.
        deadline_ms: u64,
    },
    /// The 64 MiB per-instance memory cap (ADR 0018): the module
    /// declares more than the cap, instantiation failed, or the guest
    /// could not allocate the request buffer (`swath_udf_alloc` answered
    /// `0` — growth past the cap is denied, so allocation failure is the
    /// shape a memory overrun takes inside a conforming guest).
    #[error("UDF memory limit: {detail}")]
    MemoryLimit {
        /// Which allocation failed, and how.
        detail: String,
    },
    /// The module trapped for any reason other than fuel or the epoch
    /// deadline (unreachable, out-of-bounds access, stack overflow, a
    /// guest panic — the guest kit's panic handler traps deliberately).
    #[error("UDF trapped: {detail}")]
    Trap {
        /// The runtime's trap description.
        detail: String,
    },
    /// The guest declared failure: `swath_udf_run` answered `0` (the
    /// ABI's own error signal — e.g. the module refuses the input
    /// arity, or the UDF itself returned an error).
    #[error("module `{code_hash}` declared failure (swath_udf_run answered 0)")]
    GuestFailure {
        /// The module that refused.
        code_hash: String,
    },
    /// The guest's answer violated the ABI framing: an out-of-bounds
    /// allocation or response pointer, or a response buffer that does
    /// not decode as a v1 response. Always a typed error, never UB —
    /// every guest byte is bounds-checked and strictly parsed.
    #[error("malformed UDF response: {detail}")]
    MalformedOutput {
        /// What failed to parse or bounds-check.
        detail: String,
    },
    /// The response header's plane count disagrees with the stage's
    /// pinned `swath_udf_output_planes` answer.
    #[error("UDF answered {actual} output planes, stage pins {declared}")]
    OutputPlanes {
        /// Planes the stage declares (pinned at registration).
        declared: u32,
        /// Planes the response header claimed.
        actual: u32,
    },
}

/// The executor port: runs one [`UdfStage`] over the plan's warped input
/// planes, returning the module's output planes.
///
/// # Contract (checked by the caller)
///
/// [`eval`](crate::ir::eval) verifies the returned planes — exactly
/// [`UdfStage::output_planes`] buffers, each tile-shaped — and enforces
/// the ABI's host post-conditions (`docs/udf-abi/v1.md`): output validity
/// is `ANDed` with input validity, and non-finite values the executor
/// claims valid are canonicalized to invalid. An adapter may enforce them
/// too, but the IR never trusts it to.
///
/// Synchronous by design: render compute runs inline on the calling task
/// (ADR 0012); the fuel/epoch budgets bounding a call are the adapter's
/// job (#203).
pub trait UdfExecutor {
    /// Runs `stage`'s module over `inputs` (one request plane per buffer,
    /// in plan-input order), returning its output planes in order.
    ///
    /// # Errors
    ///
    /// Any [`UdfError`]: the executor could not run the module at all, or
    /// the module failed. Per-pixel data conditions are never errors —
    /// they belong in the returned buffers' validity masks.
    fn run(&self, stage: &UdfStage, inputs: &[WarpedBuffer])
    -> Result<Vec<WarpedBuffer>, UdfError>;
}

/// The default executor: **no UDF support**. Every stage is refused with
/// [`UdfError::NotConfigured`]; plans without UDF stages evaluate exactly
/// as before and never consult it.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoUdf;

impl UdfExecutor for NoUdf {
    fn run(
        &self,
        stage: &UdfStage,
        _inputs: &[WarpedBuffer],
    ) -> Result<Vec<WarpedBuffer>, UdfError> {
        Err(UdfError::NotConfigured {
            code_hash: stage.code_hash.clone(),
        })
    }
}
