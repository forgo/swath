// SPDX-License-Identifier: Apache-2.0

//! Swath domain core.
//!
//! The pure-logic center of Swath (ADR 0001, ADR 0002): domain types, port
//! traits, the materialization planner, the process-graph compiler + Render IR,
//! and the [`Trace`] model. This crate performs **no I/O** — everything
//! external enters through port traits implemented by adapter crates.
//!
//! Modules land incrementally per the roadmap (issues #21+); this crate
//! currently establishes the workspace contract (lints, edition, MSRV) only.
//!
//! [`Trace`]: https://github.com/forgo/swath/blob/main/docs/ARCHITECTURE.md#9-trace--observability-model-the-x-ray-keystone

#[cfg(test)]
mod tests {
    /// The workspace builds, tests run, and the lint contract is active.
    #[test]
    fn workspace_contract_smoke() {
        let edition_2024 = 2024_u16;
        assert_eq!(edition_2024, 2024);
    }
}
