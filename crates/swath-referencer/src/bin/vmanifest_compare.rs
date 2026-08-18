// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `vmanifest-compare a.json b.json`: the conformance harness's equivalence
//! gate (`just test-referencer`). Loads two v1 manifests, runs
//! [`swath_referencer::manifest::compare`], prints the report, and exits non-zero
//! on any mismatch — the promoted form of prototype 0001's `compare`
//! subcommand.

// A test-harness CLI's report goes to stdout/stderr by design; the
// workspace-wide print bans target library/server code, where tracing is
// the spine.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use swath_referencer::manifest::{VirtualManifest, compare};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [a_path, b_path] = args.as_slice() else {
        eprintln!("usage: vmanifest-compare <a.json> <b.json>");
        return ExitCode::from(2);
    };

    let load = |path: &str| -> Result<VirtualManifest, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("cannot read `{path}`: {e}"))?;
        VirtualManifest::from_json_str(&text).map_err(|e| format!("`{path}`: {e}"))
    };
    let (a, b) = match (load(a_path), load(b_path)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("vmanifest-compare: {e}");
            return ExitCode::from(2);
        }
    };

    let report = compare(&a, &b);
    println!(
        "arrays: A={} B={} matched={}",
        report.arrays_a, report.arrays_b, report.matched_arrays
    );
    for line in &report.grid_mismatches {
        println!("grid mismatch: {line}");
    }
    for line in &report.chunk_mismatches {
        println!("chunk mismatch: {line}");
    }
    let refs: usize = a.arrays.iter().map(|arr| arr.refs.len()).sum();
    if report.equivalent() {
        println!("EQUIVALENT ({} arrays, {refs} chunk refs)", report.arrays_a);
        ExitCode::SUCCESS
    } else {
        println!(
            "NOT EQUIVALENT ({} grid, {} chunk mismatches)",
            report.grid_mismatches.len(),
            report.chunk_mismatches.len()
        );
        ExitCode::FAILURE
    }
}
