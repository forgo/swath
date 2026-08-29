// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Shared test plumbing for the workspace (issue #97; one crate since #348).
//!
//! Never published and never in the product binary: a `[dev-dependencies]`
//! entry everywhere except `swath-e2e`, the CI harness binary, which uses
//! [`pdiff`] at run time. It consolidates the patterns that were previously
//! duplicated across test files:
//!
//! - [`pdiff`]: the perceptual image diff against the GDAL/rio-tiler oracle
//!   (formerly the `swath-testkit` crate), the golden-tile assertion, and
//!   the `pdiff` CLI;
//! - [`catalog`]: the one in-memory `Catalog` double (five copies before);
//! - [`fixtures`]: the committed HLS and Park Fire fixtures in catalog form;
//! - [`paths`]: where the committed test data lives, anchored once;
//! - [`http`]: in-process requests against an axum router;
//! - [`truth`]: the GDAL/h5py truth-table schema, loader, and shared
//!   pixel-identity assertions (previously verbatim in three adapter
//!   integration tests);
//! - [`TempDir`]: one parallel-safe, self-deleting temp directory
//!   (previously seven hand-rolled variants, one of them not parallel-safe);
//! - [`gated_var`]: the single skip-semantics for env-gated `#[ignore]`
//!   suites (skip with a stderr notice, never panic on a missing variable).

pub mod catalog;
pub mod fixtures;
pub mod gate;
pub mod http;
pub mod paths;
pub mod pdiff;
pub mod tempdir;
pub mod truth;

pub use gate::gated_var;
pub use pdiff::{DiffError, DiffPolicy, DiffReport, RgbaImage, diff, load_png};
pub use tempdir::TempDir;
