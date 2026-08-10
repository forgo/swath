// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Shared test plumbing for the workspace (issue #97).
//!
//! This crate is consumed exclusively through `[dev-dependencies]` and is
//! never shipped (`publish = false`). It consolidates three patterns that
//! were previously duplicated across test files:
//!
//! - [`truth`]: the GDAL/h5py truth-table schema, loader, and shared
//!   pixel-identity assertions (previously verbatim in three adapter
//!   integration tests);
//! - [`TempDir`]: one parallel-safe, self-deleting temp directory
//!   (previously seven hand-rolled variants, one of them not parallel-safe);
//! - [`gated_var`]: the single skip-semantics for env-gated `#[ignore]`
//!   suites (skip with a stderr notice, never panic on a missing variable).

pub mod gate;
pub mod tempdir;
pub mod truth;

pub use gate::gated_var;
pub use tempdir::TempDir;
