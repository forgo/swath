// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The openEO surface's error vocabulary (#354): the crate's one error
//! type rendered as the spec's `{"code","message"}`, the compiler
//! diagnostics in registry codes, and the preview's refusal-over-
//! degradation mapping.

// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

use axum::http::StatusCode;
use swath_render::ir::PlanError;
use swath_render::{CompileError, TileError, UdfError};

/// The openEO rendering of the crate's one error type (`crate::error`).
use crate::error::OpenEo as OpenEoError;

/// Maps compiler diagnostics onto standardized openEO error codes — the
/// #32 diagnostics, spoken in the standard's vocabulary. The message is
/// the compiler's own (it names the offending node); the shapes are
/// pinned by snapshot tests.
impl From<CompileError> for OpenEoError {
    fn from(err: CompileError) -> Self {
        let (status, code) = match &err {
            // `run_udf` where nothing is wired is exactly a process this
            // deployment does not offer (its process list omits it too).
            CompileError::UnsupportedProcess { .. } | CompileError::UdfUnavailable { .. } => {
                (StatusCode::BAD_REQUEST, "ProcessUnsupported")
            }
            CompileError::UnknownCollection { .. } => (StatusCode::NOT_FOUND, "CollectionNotFound"),
            CompileError::MissingArgument { .. } | CompileError::MissingResolver { .. } => {
                (StatusCode::BAD_REQUEST, "ProcessParameterRequired")
            }
            // A rejected or mis-typed module is a bad `udf` parameter on
            // the named node.
            CompileError::InvalidArgument { .. }
            | CompileError::UnknownBand { .. }
            | CompileError::EmptyTemporalWindow { .. }
            | CompileError::DimensionNotAvailable { .. }
            | CompileError::UdfModule { .. }
            | CompileError::UdfOutputPlanes { .. } => {
                (StatusCode::BAD_REQUEST, "ProcessParameterInvalid")
            }
            _ => (StatusCode::BAD_REQUEST, "ProcessGraphInvalid"),
        };
        Self::new(status, code, err.to_string())
    }
}

pub(super) fn service_not_found(id: &str) -> OpenEoError {
    OpenEoError::new(
        StatusCode::NOT_FOUND,
        "ServiceNotFound",
        format!("Service '{id}' does not exist."),
    )
}

/// Preview resolution failures in the openEO vocabulary: a 404 (the
/// collection has no ingested granule yet — there is nothing to render)
/// keeps its status under the registry's generic `NotFound`; everything
/// else is an `Internal` backend failure.
pub(super) fn preview_resolution_error(err: crate::error::ApiError) -> OpenEoError {
    if err.status == StatusCode::NOT_FOUND {
        OpenEoError(err.with_code("NotFound"))
    } else {
        OpenEoError::internal(err.detail)
    }
}

/// Preview render failures in the spec's registry vocabulary — refusal
/// over degradation (ADR 0014), and the preview as the `run_udf`
/// validation loop (ADR 0018, #206): a module's failure is the author's
/// to fix, so it answers a 400 that says what happened in plain words,
/// never a 500.
///
/// - The planner's refusal (the live estimate exceeds the preview budget
///   and nothing cheaper can serve) is `ProcessGraphComplexity`.
/// - A UDF that runs out of its per-tile fuel, or trips the epoch
///   backstop, is a graph too heavy for the bound — the same
///   `ProcessGraphComplexity`, in fuel terms.
/// - A UDF that traps, declares failure, answers malformed or
///   mis-counted planes, or overruns its memory cap is a bad module: the
///   `udf` argument is invalid, `ProcessParameterInvalid`, with the
///   executor's own diagnosis as the detail.
/// - Host-side UDF failures that a validated plan cannot reach (an
///   unregistered hash, an unencodable request) and every non-UDF
///   failure stay an honest `Internal`.
pub(super) fn preview_render_error(err: TileError) -> OpenEoError {
    match err {
        TileError::BudgetExceeded {
            estimated_live_bytes,
            limit,
        } => OpenEoError::new(
            StatusCode::BAD_REQUEST,
            "ProcessGraphComplexity",
            format!(
                "The process is too complex for synchronous processing: the preview would \
                 read an estimated {estimated_live_bytes} bytes at full resolution (budget: \
                 {limit} bytes) and no overview can serve it. Narrow the spatial extent."
            ),
        ),
        TileError::Plan(PlanError::Udf(udf)) => preview_udf_error(&udf),
        other => OpenEoError::internal(format!("preview render failed: {other}")),
    }
}

/// The `run_udf` half of [`preview_render_error`].
pub(super) fn preview_udf_error(udf: &UdfError) -> OpenEoError {
    match udf {
        UdfError::FuelExhausted { budget } => OpenEoError::new(
            StatusCode::BAD_REQUEST,
            "ProcessGraphComplexity",
            format!(
                "The process is too complex for synchronous processing: the UDF exceeded the \
                 per-tile fuel budget ({budget} fuel) — simplify or narrow it."
            ),
        ),
        UdfError::EpochDeadline { deadline_ms } => OpenEoError::new(
            StatusCode::BAD_REQUEST,
            "ProcessGraphComplexity",
            format!(
                "The process is too complex for synchronous processing: the UDF exceeded the \
                 per-tile fuel budget's {deadline_ms} ms wall-clock backstop — simplify or \
                 narrow it."
            ),
        ),
        UdfError::Trap { .. }
        | UdfError::MalformedOutput { .. }
        | UdfError::GuestFailure { .. }
        | UdfError::OutputPlanes { .. }
        | UdfError::MemoryLimit { .. } => OpenEoError::new(
            StatusCode::BAD_REQUEST,
            "ProcessParameterInvalid",
            format!("The value passed for parameter 'udf' in process 'run_udf' is invalid: {udf}"),
        ),
        other => OpenEoError::internal(format!("preview render failed: UDF stage failed: {other}")),
    }
}
