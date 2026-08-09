// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Truth-table agreement with PROJ (issue #23).
//!
//! The truth table is generated once by `tests/oracle/reproject_truth.py`
//! (pinned pyproj/PROJ — ADR 0002: real PROJ lives only in the test suite)
//! and committed under `tests/data/`. The suite itself lives in
//! `tests/common/mod.rs` and is adapter-agnostic; see there for how a
//! future PROJ C-binding adapter reuses it.
//!
//! # Tolerance justification (proj4rs, measured 2026-08 vs PROJ 9.5.1)
//!
//! Measured worst-case deviation across the whole table: `3.8e-9` m for
//! projected targets (a few nanometers — double-precision noise on
//! ~1e7-meter coordinates) and `2.9e-14`° for geographic targets (~3 nm on
//! the ground). Asserted tolerances are set just above measured + margin,
//! NOT at the mm/1e-9° bar the port would minimally need — so any future
//! regression in proj4rs (or in our boundary conversion) that costs even
//! micrometers fails loudly:
//!
//! * meters (projected targets): assert `1e-8` m ≈ 2.6× measured worst;
//! * degrees (geographic targets): assert `1e-12`° ≈ 35× measured worst,
//!   still well below one micrometer of ground distance.

mod common;

use common::Tolerances;
use swath_reproject_proj4rs::Proj4rsReproject;

/// The proj4rs adapter's asserted accuracy vs PROJ (see module docs).
const PROJ4RS_TOLERANCES: Tolerances = Tolerances {
    degrees: 1e-12,
    meters: 1e-8,
};

#[test]
// The whole point of this print is the measured-accuracy report a human
// reads with `--nocapture`; the print_stdout lint targets library/server
// code, not test diagnostics.
#[allow(clippy::print_stdout)]
fn agrees_with_proj_within_documented_tolerances() {
    let report = common::run_truth_suite(&Proj4rsReproject::new(), PROJ4RS_TOLERANCES);
    assert_eq!(report.len(), 18, "truth table shrank unexpectedly");
    // Keep the measured numbers inspectable: `--nocapture` prints the
    // per-case worst deviation that backs the crate-level accuracy table.
    for (name, worst) in &report {
        println!("{name}: worst deviation {worst:e}");
    }
}

#[test]
fn batch_path_is_observably_identical_to_per_point() {
    common::assert_batch_matches_per_point(&Proj4rsReproject::new());
}
