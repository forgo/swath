// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `pdiff` — perceptual diff between two PNGs against an explicit policy.
//!
//! ```text
//! pdiff A.png B.png [--tolerance N] [--max-bad-frac F]
//! pdiff --corrupt IN.png OUT.png
//! ```
//!
//! The first form prints a [`DiffReport`] and exits nonzero when the images
//! violate the [`DiffPolicy`] (defaults documented in `swath-testkit`).
//! `--corrupt` is oracle-harness self-test support: it copies `IN.png` with a
//! single channel of a single pixel perturbed by exactly 1, the smallest error
//! a zero-tolerance diff must catch (`just oracle-verify` uses it to prove the
//! gate can fail).

// A CLI's contract is its stdout/stderr; the workspace-wide restriction lints
// against printing target library/server code.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::Path;
use std::process::ExitCode;

use swath_testkit::{DiffPolicy, diff, load_png};

const USAGE: &str = "usage: pdiff A.png B.png [--tolerance N] [--max-bad-frac F]\n       pdiff --corrupt IN.png OUT.png";

fn main() -> ExitCode {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("pdiff: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    if args.first().is_some_and(|a| a == "--corrupt") {
        let [_, input, output] = args else {
            return Err(USAGE.to_string());
        };
        corrupt(Path::new(input), Path::new(output))?;
        return Ok(ExitCode::SUCCESS);
    }

    let (paths, policy) = parse_compare_args(args)?;
    let a = load_png(Path::new(&paths.0)).map_err(|e| e.to_string())?;
    let b = load_png(Path::new(&paths.1)).map_err(|e| e.to_string())?;
    let report = diff(&a, &b).map_err(|e| e.to_string())?;

    println!("{report}");
    println!(
        "pixels > tolerance {}: {} of {} ({:.4}%)",
        policy.per_channel_tolerance,
        report.pixels_exceeding_tolerance(policy.per_channel_tolerance),
        report.total_pixels(),
        report.pct_pixels_exceeding_tolerance(policy.per_channel_tolerance) * 100.0
    );
    if report.passes(&policy) {
        println!(
            "PASS (tolerance {}, max bad fraction {})",
            policy.per_channel_tolerance, policy.max_bad_pixel_fraction
        );
        Ok(ExitCode::SUCCESS)
    } else {
        println!(
            "FAIL (tolerance {}, max bad fraction {})",
            policy.per_channel_tolerance, policy.max_bad_pixel_fraction
        );
        Ok(ExitCode::FAILURE)
    }
}

fn parse_compare_args(args: &[String]) -> Result<((String, String), DiffPolicy), String> {
    let mut policy = DiffPolicy::default();
    let mut positional: Vec<&String> = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--tolerance" => {
                let value = iter.next().ok_or("--tolerance requires a value")?;
                policy.per_channel_tolerance = value
                    .parse()
                    .map_err(|_| format!("invalid --tolerance {value:?} (expected 0-255)"))?;
            }
            "--max-bad-frac" => {
                let value = iter.next().ok_or("--max-bad-frac requires a value")?;
                let fraction: f64 = value
                    .parse()
                    .map_err(|_| format!("invalid --max-bad-frac {value:?} (expected 0.0-1.0)"))?;
                if !(0.0..=1.0).contains(&fraction) {
                    return Err(format!("--max-bad-frac {fraction} out of range 0.0-1.0"));
                }
                policy.max_bad_pixel_fraction = fraction;
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {other:?}\n{USAGE}"));
            }
            _ => positional.push(arg),
        }
    }
    let [a, b] = positional[..] else {
        return Err(USAGE.to_string());
    };
    Ok(((a.clone(), b.clone()), policy))
}

/// Copy `input` to `output` with pixel (0, 0)'s red channel moved by exactly 1.
fn corrupt(input: &Path, output: &Path) -> Result<(), String> {
    let mut img = load_png(input).map_err(|e| e.to_string())?;
    if img.width() == 0 || img.height() == 0 {
        return Err("cannot corrupt an empty image".to_string());
    }
    let px = img.get_pixel_mut(0, 0);
    px.0[0] = if px.0[0] == u8::MAX {
        u8::MAX - 1
    } else {
        px.0[0] + 1
    };
    img.save(output).map_err(|e| e.to_string())?;
    println!(
        "corrupted 1 channel of 1 pixel: {} -> {}",
        input.display(),
        output.display()
    );
    Ok(())
}
