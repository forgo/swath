// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Stages the production web bundle for compile-time embedding (issue
//! #103, feature `embedded-ui`): copies `web/dist` (built by `pnpm build`
//! — `just build-full` orchestrates both halves) into `$OUT_DIR/ui`,
//! where `serve.rs` `include_dir!`s it. Indirection through `OUT_DIR`
//! rather than embedding `web/dist` directly is what keeps a dist-less
//! checkout compiling: `include_dir!` fails on a missing directory, so
//! the script always materializes one — empty when there is no bundle,
//! in which case the server simply has no UI to serve (an honest,
//! documented degradation, not a build break).

use std::path::Path;

fn main() {
    // Rerun when the bundle changes (or first appears). Cargo watches the
    // path even while it doesn't exist.
    println!("cargo:rerun-if-changed=../../web/dist");
    if std::env::var_os("CARGO_FEATURE_EMBEDDED_UI").is_none() {
        return;
    }
    let out_dir = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let staged = Path::new(&out_dir).join("ui");
    // Start from a clean slate so a removed bundle file cannot linger in
    // an incremental build.
    if staged.exists() {
        std::fs::remove_dir_all(&staged).expect("stale staged UI removes");
    }
    std::fs::create_dir_all(&staged).expect("staging dir creates");
    let dist = Path::new(&manifest_dir).join("../../web/dist");
    if dist.is_dir() {
        copy_tree(&dist, &staged);
    }
}

/// Recursive copy, declaring each source file to cargo's change tracking
/// (a directory's own mtime doesn't cover nested edits).
fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).expect("bundle dir reads") {
        let entry = entry.expect("bundle entry reads");
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            std::fs::create_dir_all(&target).expect("bundle subdir creates");
            copy_tree(&source, &target);
        } else {
            println!("cargo:rerun-if-changed={}", source.display());
            std::fs::copy(&source, &target).expect("bundle file copies");
        }
    }
}
