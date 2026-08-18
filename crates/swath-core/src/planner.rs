// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The cost-aware materialization planner (ARCHITECTURE.md §5/§10,
//! CHARTER.md §7 pillar 2, issue #37) — extracted to the standalone
//! `swath-planner` crate (ADR 0016, issue #189) and re-exported here
//! verbatim, so `swath_core::planner::…` paths keep working across the
//! workspace. The full decision model stays documented in
//! `docs/design/materialization-planner.md` and on the extracted crate.
//!
//! The one piece that knows the Trace model stays home, behind the port
//! boundary: [`PlanTraceExt`](crate::trace::PlanTraceExt) turns a
//! [`Plan`] into the x-ray's [`PlanTrace`](crate::trace::PlanTrace)
//! payload ([`Trace::plan`](crate::trace::Trace::plan)) — the extracted
//! crate is IR- and Trace-free by the standalone rule.

pub use swath_planner::{
    Availability, BandWindow, Budget, CacheProbe, CandidateTrace, DEFAULT_OVERVIEW_OVERSAMPLE,
    Plan, PlanChoice, PlannedStrategy, WARP_COST_WEIGHT, plan,
};
