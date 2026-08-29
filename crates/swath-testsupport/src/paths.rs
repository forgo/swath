// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The committed test data, located once. Every path is anchored on this
//! crate's own manifest directory, so a consumer's nesting depth (a
//! top-level crate, an adapter two levels down) no longer changes the
//! relative literal — the reason `fixtures_dir` used to exist in six
//! hand-maintained copies (#348).

use std::path::PathBuf;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The committed HLS fixture directory (`tests/fixtures/README.md`,
/// ADR 0004).
#[must_use]
pub fn fixtures_dir() -> PathBuf {
    crate_dir().join("../../tests/fixtures")
}

/// The oracle-rendered golden tiles (`crates/swath-render/tests/data`).
#[must_use]
pub fn render_goldens_dir() -> PathBuf {
    crate_dir().join("../swath-render/tests/data")
}

/// The referencer's committed data (`crates/swath-referencer/tests/data`):
/// the tiny HDF5 fixture and its expected manifest.
#[must_use]
pub fn referencer_data_dir() -> PathBuf {
    crate_dir().join("../swath-referencer/tests/data")
}

/// The warp kernel's golden captures (`crates/swath-warp/tests/data`).
#[must_use]
pub fn warp_data_dir() -> PathBuf {
    crate_dir().join("../swath-warp/tests/data")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_directories_exist() {
        for dir in [
            fixtures_dir(),
            render_goldens_dir(),
            referencer_data_dir(),
            warp_data_dir(),
        ] {
            assert!(dir.is_dir(), "{} is not a directory", dir.display());
        }
    }
}
