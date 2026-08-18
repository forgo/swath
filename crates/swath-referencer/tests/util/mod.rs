// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Self-contained test utilities (std-only): the published crate's test
//! suite depends on nothing unpublished (ADR 0016's standalone rule) —
//! these mirror the workspace's `swath-testsupport` helpers.
#![allow(dead_code, reason = "not every test binary uses every helper")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Distinguishes concurrent creations within one process.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A uniquely named directory under [`std::env::temp_dir`], removed
/// (recursively) on drop. Parallel-safe: pid + process-global counter +
/// timestamp.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    /// Creates the directory; `tag` labels leaked dirs for triage.
    pub(crate) fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "swath-{tag}-{}-{}-{nanos}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&dir).expect("temp dir creates");
        Self(dir)
    }

    /// The directory's path.
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    /// A path to `name` inside the directory.
    pub(crate) fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Reads the gating environment variable for an `#[ignore]`d test:
/// `Some(value)` when set and non-empty, otherwise a skip notice to stderr
/// and `None` (a missing credential is a skip, never a failure).
#[allow(dead_code, reason = "not every test binary uses every helper")]
#[allow(
    clippy::print_stderr,
    reason = "a gated test's skip notice legitimately goes to stderr"
)]
pub(crate) fn gated_var(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => {
            eprintln!("{name} not set; skipping");
            None
        }
    }
}
