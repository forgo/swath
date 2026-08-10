// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! A fresh, self-deleting temp directory per test — no `tempfile` dep (the
//! supply-chain gate stays untouched), parallel-safe by construction.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Distinguishes concurrent creations within one process (nextest runs one
/// process per test binary, but a binary's tests share the process).
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A uniquely named directory under [`std::env::temp_dir`], removed
/// (recursively) on drop.
///
/// The name is `swath-{tag}-{pid}-{counter}-{nanos}`: the pid separates
/// concurrent test *processes*, the process-global counter separates
/// concurrent tests *within* a process, and the timestamp keeps a recycled
/// pid from colliding with a crashed run's leftovers. Safe at nextest's
/// default (full) parallelism.
pub struct TempDir(PathBuf);

impl TempDir {
    /// Creates the directory. `tag` is a human-readable label for the test
    /// (it only aids post-mortem triage of leaked dirs; uniqueness never
    /// depends on it).
    pub fn new(tag: &str) -> Self {
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
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// A path to `name` inside the directory.
    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
