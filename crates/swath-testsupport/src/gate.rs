// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Skip semantics for env-gated suites.
//!
//! Gated tests are `#[ignore]` (nextest reports them as SKIP in default
//! runs); their harness recipe runs them with `--run-ignored`. When the
//! gating variable is absent even then, the test must skip cleanly with a
//! notice — never panic (a missing credential is not a failure).

/// Reads the gating environment variable for an `#[ignore]`d test.
///
/// Returns the value when set (and non-empty); otherwise prints a skip
/// notice to stderr and returns `None` — the caller early-returns:
///
/// ```ignore
/// let Some(granule) = swath_testsupport::gated_var("SWATH_VNP09GA") else {
///     return;
/// };
/// ```
#[allow(
    clippy::print_stderr,
    reason = "a gated test's skip notice legitimately goes to stderr"
)]
pub fn gated_var(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => {
            eprintln!("{name} not set; skipping");
            None
        }
    }
}
