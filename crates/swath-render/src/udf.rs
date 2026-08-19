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
/// [`PlanError::Udf`](crate::ir::PlanError::Udf). `#[non_exhaustive]`:
/// the wasmtime adapter (#203) adds its trap/fuel/deadline variants here
/// without breaking the port's consumers.
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
