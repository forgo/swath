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
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("pdiff: {message}");
            ExitCode::from(2)
        }
    }
}

/// Returns the process exit code: 0 = pass, 1 = policy failure.
fn run(args: &[String]) -> Result<u8, String> {
    if args.first().is_some_and(|a| a == "--corrupt") {
        let [_, input, output] = args else {
            return Err(USAGE.to_string());
        };
        corrupt(Path::new(input), Path::new(output))?;
        return Ok(0);
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
        Ok(0)
    } else {
        println!(
            "FAIL (tolerance {}, max bad fraction {})",
            policy.per_channel_tolerance, policy.max_bad_pixel_fraction
        );
        Ok(1)
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use swath_testkit::RgbaImage;

    use super::{parse_compare_args, run};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    /// A per-test scratch PNG under the target-managed temp dir.
    fn scratch_png(name: &str, image: &RgbaImage) -> PathBuf {
        let path = std::env::temp_dir().join(format!("swath-pdiff-{}-{name}", std::process::id()));
        image
            .save_with_format(&path, image::ImageFormat::Png)
            .expect("write scratch png");
        path
    }

    fn flat_image(value: u8) -> RgbaImage {
        RgbaImage::from_pixel(16, 16, image::Rgba([value, value, value, 255]))
    }

    #[test]
    fn parse_defaults_and_flags() {
        let (paths, policy) = parse_compare_args(&args(&["a.png", "b.png"])).expect("valid args");
        assert_eq!(paths, ("a.png".to_string(), "b.png".to_string()));
        assert_eq!(policy, swath_testkit::DiffPolicy::default());

        let (_, policy) = parse_compare_args(&args(&[
            "a.png",
            "--tolerance",
            "0",
            "b.png",
            "--max-bad-frac",
            "0.25",
        ]))
        .expect("valid args");
        assert_eq!(policy.per_channel_tolerance, 0);
        assert!((policy.max_bad_pixel_fraction - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert!(parse_compare_args(&args(&["a.png"])).is_err());
        assert!(parse_compare_args(&args(&["a.png", "b.png", "--tolerance"])).is_err());
        assert!(parse_compare_args(&args(&["a.png", "b.png", "--tolerance", "256"])).is_err());
        assert!(parse_compare_args(&args(&["a.png", "b.png", "--max-bad-frac", "1.5"])).is_err());
        assert!(parse_compare_args(&args(&["a.png", "b.png", "--frobnicate"])).is_err());
    }

    #[test]
    fn identical_files_pass_and_corruption_fails_at_zero_tolerance() {
        let a = scratch_png("id-a.png", &flat_image(128));
        let b = scratch_png("id-b.png", &flat_image(128));
        let corrupted =
            std::env::temp_dir().join(format!("swath-pdiff-{}-id-corrupt.png", std::process::id()));
        let (a_s, b_s, c_s) = (
            a.display().to_string(),
            b.display().to_string(),
            corrupted.display().to_string(),
        );

        assert_eq!(run(&args(&[&a_s, &b_s])), Ok(0));
        assert_eq!(run(&args(&["--corrupt", &a_s, &c_s])), Ok(0));
        assert_eq!(
            run(&args(&[
                &a_s,
                &c_s,
                "--tolerance",
                "0",
                "--max-bad-frac",
                "0"
            ])),
            Ok(1)
        );
        // The seeded error is exactly 1, so the default tolerance (2) absorbs it.
        assert_eq!(run(&args(&[&a_s, &c_s])), Ok(0));

        for path in [a, b, corrupted] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn missing_file_and_dimension_mismatch_are_errors() {
        assert!(run(&args(&["/nonexistent-a.png", "/nonexistent-b.png"])).is_err());

        let a = scratch_png("dim-a.png", &flat_image(10));
        let b = scratch_png(
            "dim-b.png",
            &RgbaImage::from_pixel(8, 16, image::Rgba([10, 10, 10, 255])),
        );
        let result = run(&args(&[&a.display().to_string(), &b.display().to_string()]));
        assert!(result.is_err_and(|message| message.contains("dimension mismatch")));

        for path in [a, b] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn corrupt_perturbs_a_saturated_channel_downward() {
        let a = scratch_png("sat-a.png", &flat_image(255));
        let out = std::env::temp_dir().join(format!(
            "swath-pdiff-{}-sat-corrupt.png",
            std::process::id()
        ));
        let (a_s, out_s) = (a.display().to_string(), out.display().to_string());
        assert_eq!(run(&args(&["--corrupt", &a_s, &out_s])), Ok(0));
        let corrupted = swath_testkit::load_png(&out).expect("readable output");
        assert_eq!(corrupted.get_pixel(0, 0).0[0], 254);
        assert_eq!(run(&args(&["--corrupt"])), Err(super::USAGE.to_string()));

        for path in [a, out] {
            let _ = std::fs::remove_file(path);
        }
    }
}
