// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The wasmtime-free-forever guard (issue #188, ADR 0016): this published
//! crate's dependency tree must never contain wasmtime — not today, and
//! not when M9 brings wasmtime into other parts of the workspace. A
//! kerchunk-style referencer is a metadata walk; a WebAssembly runtime in
//! its tree would be a boundary failure, caught here mechanically rather
//! than in a review.

use std::process::Command;

/// `cargo tree` over this crate's full feature surface (normal + build
/// edges — dev-deps reach back into the workspace by design and are never
/// published) must not name wasmtime or any of its `wasmtime-*` satellites.
#[test]
fn dependency_tree_is_wasmtime_free() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo)
        .args([
            "tree",
            "--manifest-path",
            concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
            "--all-features",
            "--edges",
            "normal,build",
            "--prefix",
            "none",
        ])
        .output()
        .expect("cargo tree runs");
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8(output.stdout).expect("cargo tree emits UTF-8");
    // Sanity: the tree is really this crate's (an empty or foreign tree
    // would make the guard vacuous).
    assert!(
        tree.lines()
            .next()
            .is_some_and(|first| first.starts_with("swath-referencer ")),
        "unexpected cargo tree root:\n{tree}"
    );
    let offending: Vec<&str> = tree
        .lines()
        .filter(|line| {
            line.split_whitespace()
                .next()
                .is_some_and(|name| name.starts_with("wasmtime"))
        })
        .collect();
    assert!(
        offending.is_empty(),
        "wasmtime entered swath-referencer's dependency tree — the crate is \
         wasmtime-free forever (ADR 0016):\n{}",
        offending.join("\n")
    );
}
