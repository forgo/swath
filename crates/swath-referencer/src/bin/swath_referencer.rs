// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `swath-referencer <granule>`: the standalone CLI over the library
//! (`cli` feature). Generates the v1 virtual-reference manifest for one
//! legacy granule (HDF5/NetCDF4, GRIB2) and writes the JSON to stdout or
//! `--output` — the same generation Swath's ingest path performs, usable
//! without any of Swath.

// A CLI's output and errors go to stdout/stderr by design; the
// workspace-wide print bans target library/server code.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use swath_referencer::SwathReferencer;

/// Generate a v1 virtual-reference manifest (byte-range references, no
/// pixel data) for a legacy granule: HDF5/NetCDF4 (`.h5`/`.hdf5`/`.nc`/
/// `.nc4`, with the `legacy-hdf5` feature) or GRIB2 (`.grib2`/`.grb2`/
/// `.grib`).
#[derive(Debug, Parser)]
#[command(name = "swath-referencer", version)]
struct Args {
    /// The granule file to reference.
    granule: PathBuf,

    /// Where to write the manifest JSON (default: stdout).
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let manifest = match SwathReferencer::new().generate(&args.granule) {
        Ok(manifest) => manifest,
        Err(err) => {
            eprintln!("swath-referencer: {err}");
            return ExitCode::FAILURE;
        }
    };
    let json = manifest.to_json_string();
    match &args.output {
        None => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Some(path) => match std::fs::write(path, json) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!(
                    "swath-referencer: writing `{path}`: {err}",
                    path = path.display()
                );
                ExitCode::FAILURE
            }
        },
    }
}
