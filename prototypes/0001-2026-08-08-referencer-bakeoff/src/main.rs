//! Prototype 0001 — Referencer Bake-Off harness (std-only, zero dependencies).
//!
//! Subcommands:
//!   gen --with <rust|virtualizarr> <granule> -o <out.json> [--python p] [--script s]
//!   compare <a.json> <b.json>
//!   bakeoff <granule> [--python p] [--script s]

mod json;
mod manifest;
mod referencer;

use referencer::{IngestReferencer, RustReferencer, VirtualizarrSidecar};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const USAGE: &str = "\
swath-referencer-bakeoff — prototype 0001

USAGE:
  gen --with <rust|virtualizarr> <granule> -o <out.json> [--python p] [--script s]
  compare <a.json> <b.json>
  bakeoff <granule> [--python p] [--script s]
";

struct Args {
    positionals: Vec<String>,
    flags: HashMap<String, String>,
}

fn parse_args(rest: &[String]) -> Args {
    let mut positionals = Vec::new();
    let mut flags = HashMap::new();
    let mut i = 0;
    while i < rest.len() {
        let a = &rest[i];
        if let Some(name) = a.strip_prefix("--") {
            let val = rest.get(i + 1).cloned().unwrap_or_default();
            flags.insert(name.to_string(), val);
            i += 2;
        } else if a.len() == 2 && a.starts_with('-') {
            let name = a[1..].to_string();
            let val = rest.get(i + 1).cloned().unwrap_or_default();
            flags.insert(name, val);
            i += 2;
        } else {
            positionals.push(a.clone());
            i += 1;
        }
    }
    Args { positionals, flags }
}

impl Args {
    fn flag(&self, names: &[&str]) -> Option<String> {
        names.iter().find_map(|n| self.flags.get(*n).cloned())
    }
    fn flag_or(&self, names: &[&str], default: &str) -> String {
        self.flag(names).unwrap_or_else(|| default.to_string())
    }
}

fn timed(
    r: &dyn IngestReferencer,
    granule: &Path,
) -> Result<(manifest::VirtualManifest, u128), String> {
    let t = Instant::now();
    let m = r.generate(granule)?;
    Ok((m, t.elapsed().as_millis()))
}

fn read_manifest(p: &Path) -> Result<manifest::VirtualManifest, String> {
    let text = std::fs::read_to_string(p).map_err(|e| format!("read {}: {e}", p.display()))?;
    manifest::VirtualManifest::from_str(&text)
}

fn main() {
    std::process::exit(real_main());
}

fn real_main() -> i32 {
    let argv: Vec<String> = std::env::args().collect();
    let sub = argv.get(1).map(|s| s.as_str());
    let rest: Vec<String> = argv.iter().skip(2).cloned().collect();
    let args = parse_args(&rest);

    match sub {
        Some("gen") => {
            let with = match args.flag(&["with"]) {
                Some(w) => w,
                None => return fail("gen requires --with <rust|virtualizarr>"),
            };
            let granule = match args.positionals.first() {
                Some(g) => PathBuf::from(g),
                None => return fail("gen requires a <granule> path"),
            };
            let out = match args.flag(&["out", "o"]) {
                Some(o) => PathBuf::from(o),
                None => return fail("gen requires -o <out.json>"),
            };
            let python = args.flag_or(&["python"], "python3");
            let script =
                PathBuf::from(args.flag_or(&["script"], "sidecar/referencer_virtualizarr.py"));
            let r: Box<dyn IngestReferencer> = match with.as_str() {
                "rust" => Box::new(RustReferencer),
                "virtualizarr" => Box::new(VirtualizarrSidecar { python, script }),
                other => {
                    return fail(&format!(
                        "unknown generator '{other}' (use rust|virtualizarr)"
                    ));
                }
            };
            match timed(r.as_ref(), &granule) {
                Ok((m, ms)) => {
                    if let Err(e) = std::fs::write(&out, m.to_json_string()) {
                        return fail(&format!("write {}: {e}", out.display()));
                    }
                    println!(
                        "{}: {} arrays in {ms} ms -> {}",
                        r.name(),
                        m.arrays.len(),
                        out.display()
                    );
                    0
                }
                Err(e) => fail(&e),
            }
        }
        Some("compare") => {
            let (a, b) = match (args.positionals.first(), args.positionals.get(1)) {
                (Some(a), Some(b)) => (PathBuf::from(a), PathBuf::from(b)),
                _ => return fail("compare requires <a.json> <b.json>"),
            };
            let (ma, mb) = match (read_manifest(&a), read_manifest(&b)) {
                (Ok(ma), Ok(mb)) => (ma, mb),
                (Err(e), _) | (_, Err(e)) => return fail(&e),
            };
            let rep = manifest::compare(&ma, &mb);
            print_report(&rep);
            if rep.equivalent() { 0 } else { 1 }
        }
        Some("bakeoff") => {
            let granule = match args.positionals.first() {
                Some(g) => PathBuf::from(g),
                None => return fail("bakeoff requires a <granule> path"),
            };
            let python = args.flag_or(&["python"], "python3");
            let script =
                PathBuf::from(args.flag_or(&["script"], "sidecar/referencer_virtualizarr.py"));
            println!("== Referencer bake-off on {} ==", granule.display());
            let vz = timed(&VirtualizarrSidecar { python, script }, &granule);
            let rs = timed(&RustReferencer, &granule);
            match (&vz, &rs) {
                (Ok((mv, tv)), Ok((mr, tr))) => {
                    println!("virtualizarr : {} arrays, {tv} ms", mv.arrays.len());
                    println!("referencer-rs: {} arrays, {tr} ms", mr.arrays.len());
                    print_report(&manifest::compare(mv, mr));
                    0
                }
                _ => {
                    if let Err(e) = &vz {
                        println!("virtualizarr  FAILED: {e}");
                    }
                    if let Err(e) = &rs {
                        println!("referencer-rs FAILED: {e}");
                    }
                    println!(
                        "\n(Expected while scaffolding: implement the generators, then re-run.)"
                    );
                    0
                }
            }
        }
        _ => {
            eprint!("{USAGE}");
            2
        }
    }
}

fn fail(msg: &str) -> i32 {
    eprintln!("error: {msg}");
    2
}

fn print_report(rep: &manifest::EquivalenceReport) {
    println!(
        "arrays: A={} B={} matched={}",
        rep.arrays_a, rep.arrays_b, rep.matched_arrays
    );
    println!("grid/dtype mismatches: {}", rep.grid_mismatches.len());
    for m in &rep.grid_mismatches {
        println!("  - {m}");
    }
    println!("chunk mismatches: {}", rep.chunk_mismatches.len());
    for m in rep.chunk_mismatches.iter().take(20) {
        println!("  - {m}");
    }
    if rep.chunk_mismatches.len() > 20 {
        println!("  … {} more", rep.chunk_mismatches.len() - 20);
    }
    println!(
        "=> {}",
        if rep.equivalent() {
            "EQUIVALENT"
        } else {
            "NOT equivalent"
        }
    );
}
