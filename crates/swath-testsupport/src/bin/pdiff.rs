// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `pdiff` — perceptual diff between two PNGs against an explicit policy.
//!
//! ```text
//! pdiff A.png B.png [--tolerance N] [--max-bad-frac F]
//! pdiff --corrupt IN.png OUT.png
//! pdiff --content IMG.png [--left X] [--max-modal-frac F]
//! ```
//!
//! The first form prints a [`DiffReport`] and exits nonzero when the images
//! violate the [`DiffPolicy`] (defaults documented in `swath_testsupport::pdiff`).
//! `--corrupt` is oracle-harness self-test support: it copies `IN.png` with a
//! single channel of a single pixel perturbed by exactly 1, the smallest error
//! a zero-tolerance diff must catch (`just oracle-verify` uses it to prove the
//! gate can fail).
//!
//! `--content` is the screenshot suite's blank-canvas gate (issue #211
//! review): a run-vs-run diff cannot tell two identical blanks from two
//! identical scenes, so this form judges ONE image on its own — the
//! fraction of pixels at or right of column `--left` (the map region past
//! the page's rail) covered by the single most common color. A rendered map
//! never concentrates like that; an unpainted canvas is one color plus a few
//! controls. Above `--max-modal-frac` (default 0.97) the exit code is 1.

// A CLI's contract is its stdout/stderr; the workspace-wide restriction lints
// against printing target library/server code.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

use swath_testsupport::{DiffPolicy, RgbaImage, diff, load_png};

const USAGE: &str = "usage: pdiff A.png B.png [--tolerance N] [--max-bad-frac F]\n       pdiff --corrupt IN.png OUT.png\n       pdiff --content IMG.png [--left X] [--max-modal-frac F]";

/// Default ceiling on the modal color's share of the inspected region: a
/// blank 1280x860 canvas with the viewer's handful of white controls sits
/// above 0.99; the sparsest committed shot (the x-ray time-slider views,
/// a fire tile grid on the page background) measures 0.66.
const DEFAULT_MAX_MODAL_FRAC: f64 = 0.97;

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
    if args.first().is_some_and(|a| a == "--content") {
        let (path, left, max_modal_frac) = parse_content_args(&args[1..])?;
        let img = load_png(Path::new(&path)).map_err(|e| e.to_string())?;
        let modal = modal_fraction(&img, left).ok_or("no pixels at or right of --left")?;
        println!(
            "modal color covers {:.4} of the region x >= {left} ({}x{})",
            modal,
            img.width(),
            img.height()
        );
        if modal > max_modal_frac {
            println!("FAIL: blank canvas (max modal fraction {max_modal_frac})");
            return Ok(1);
        }
        println!("PASS (max modal fraction {max_modal_frac})");
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

fn parse_content_args(args: &[String]) -> Result<(String, u32, f64), String> {
    let mut left = 0u32;
    let mut max_modal_frac = DEFAULT_MAX_MODAL_FRAC;
    let mut positional: Vec<&String> = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--left" => {
                let value = iter.next().ok_or("--left requires a value")?;
                left = value
                    .parse()
                    .map_err(|_| format!("invalid --left {value:?} (expected a column)"))?;
            }
            "--max-modal-frac" => {
                let value = iter.next().ok_or("--max-modal-frac requires a value")?;
                let fraction: f64 = value.parse().map_err(|_| {
                    format!("invalid --max-modal-frac {value:?} (expected 0.0-1.0)")
                })?;
                if !(0.0..=1.0).contains(&fraction) {
                    return Err(format!("--max-modal-frac {fraction} out of range 0.0-1.0"));
                }
                max_modal_frac = fraction;
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {other:?}\n{USAGE}"));
            }
            _ => positional.push(arg),
        }
    }
    let [path] = positional[..] else {
        return Err(USAGE.to_string());
    };
    Ok((path.clone(), left, max_modal_frac))
}

/// The share of pixels with `x >= left` that carry the single most common
/// RGBA value; `None` when the region is empty.
fn modal_fraction(img: &RgbaImage, left: u32) -> Option<f64> {
    let mut counts: HashMap<[u8; 4], u64> = HashMap::new();
    let mut total = 0u64;
    for (x, _, px) in img.enumerate_pixels() {
        if x >= left {
            *counts.entry(px.0).or_insert(0) += 1;
            total += 1;
        }
    }
    let modal = counts.values().copied().max()?;
    // Both counts are pixel totals of one image: exactly representable.
    #[allow(clippy::cast_precision_loss)]
    Some(modal as f64 / total as f64)
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

    use swath_testsupport::RgbaImage;
    use swath_testsupport::TempDir;

    use super::{parse_compare_args, run};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    /// A scratch PNG inside the test's self-deleting temp dir.
    fn scratch_png(dir: &TempDir, name: &str, image: &RgbaImage) -> PathBuf {
        let path = dir.join(name);
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
        assert_eq!(policy, swath_testsupport::DiffPolicy::default());

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
        let dir = TempDir::new("pdiff-id");
        let a = scratch_png(&dir, "id-a.png", &flat_image(128));
        let b = scratch_png(&dir, "id-b.png", &flat_image(128));
        let corrupted = dir.join("id-corrupt.png");
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
    }

    #[test]
    fn missing_file_and_dimension_mismatch_are_errors() {
        assert!(run(&args(&["/nonexistent-a.png", "/nonexistent-b.png"])).is_err());

        let dir = TempDir::new("pdiff-dim");
        let a = scratch_png(&dir, "dim-a.png", &flat_image(10));
        let b = scratch_png(
            &dir,
            "dim-b.png",
            &RgbaImage::from_pixel(8, 16, image::Rgba([10, 10, 10, 255])),
        );
        let result = run(&args(&[&a.display().to_string(), &b.display().to_string()]));
        assert!(result.is_err_and(|message| message.contains("dimension mismatch")));
    }

    #[test]
    fn content_gate_fails_a_blank_canvas_and_passes_a_scene() {
        let dir = TempDir::new("pdiff-content");
        // A "page": a 4px rail of one color, then a canvas.
        let mut blank = RgbaImage::from_pixel(32, 16, image::Rgba([15, 23, 42, 255]));
        for x in 0..4 {
            for y in 0..16 {
                blank.put_pixel(x, y, image::Rgba([1, 2, 3, 255]));
            }
        }
        // Two "control" pixels on the blank canvas, like the viewer's buttons.
        blank.put_pixel(30, 1, image::Rgba([255, 255, 255, 255]));
        blank.put_pixel(30, 2, image::Rgba([255, 255, 255, 255]));
        let mut scene = blank.clone();
        for x in 8..24 {
            for y in 2..14 {
                let v = u8::try_from((x * 7 + y * 13) % 256).expect("fits");
                scene.put_pixel(x, y, image::Rgba([v, 255 - v, v / 2, 255]));
            }
        }
        let blank_path = scratch_png(&dir, "blank.png", &blank).display().to_string();
        let scene_path = scratch_png(&dir, "scene.png", &scene).display().to_string();

        assert_eq!(
            run(&args(&["--content", &blank_path, "--left", "4"])),
            Ok(1)
        );
        assert_eq!(
            run(&args(&["--content", &scene_path, "--left", "4"])),
            Ok(0)
        );
        // Why the verifier passes --left: counted from column 0, the rail
        // (here an eighth of the image) dilutes the blank canvas below the
        // ceiling and the same unpainted shot would pass.
        assert_eq!(run(&args(&["--content", &blank_path])), Ok(0));
        // A permissive ceiling lets the blank through — the policy is explicit.
        assert_eq!(
            run(&args(&[
                "--content",
                &blank_path,
                "--max-modal-frac",
                "1.0"
            ])),
            Ok(0)
        );
        assert!(run(&args(&["--content"])).is_err());
        assert!(run(&args(&["--content", &blank_path, "--left", "999"])).is_err());
        assert!(run(&args(&["--content", &blank_path, "--max-modal-frac", "2"])).is_err());
    }

    #[test]
    fn corrupt_perturbs_a_saturated_channel_downward() {
        let dir = TempDir::new("pdiff-sat");
        let a = scratch_png(&dir, "sat-a.png", &flat_image(255));
        let out = dir.join("sat-corrupt.png");
        let (a_s, out_s) = (a.display().to_string(), out.display().to_string());
        assert_eq!(run(&args(&["--corrupt", &a_s, &out_s])), Ok(0));
        let corrupted = swath_testsupport::load_png(&out).expect("readable output");
        assert_eq!(corrupted.get_pixel(0, 0).0[0], 254);
        assert_eq!(run(&args(&["--corrupt"])), Err(super::USAGE.to_string()));
    }
}
