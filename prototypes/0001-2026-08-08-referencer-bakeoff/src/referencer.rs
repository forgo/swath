//! The `IngestReferencer` port and its two adapters:
//!   - `RustReferencer`      — pure-Rust generation, format support feature-gated.
//!   - `VirtualizarrSidecar` — shells out to the Python VirtualiZarr sidecar.
//!
//! Both emit the same `VirtualManifest` — that is the whole point (ADR 0006): the manifest is the
//! contract, so the generators are interchangeable behind this port.

use crate::manifest::VirtualManifest;
use std::path::{Path, PathBuf};
use std::process::Command;

pub trait IngestReferencer {
    fn name(&self) -> &'static str;
    fn generate(&self, granule: &Path) -> Result<VirtualManifest, String>;
}

/// Pure-Rust reference generator. Dispatches on file extension; each format is feature-gated.
pub struct RustReferencer;

impl IngestReferencer for RustReferencer {
    fn name(&self) -> &'static str {
        "referencer-rs"
    }

    fn generate(&self, granule: &Path) -> Result<VirtualManifest, String> {
        let ext = granule
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "grib2" | "grb2" | "grib" => generate_grib(granule),
            "nc" | "nc4" | "h5" | "hdf5" => generate_hdf5(granule),
            other => Err(format!("referencer-rs: unsupported extension '{other}'")),
        }
    }
}

/// GRIB2 generation via `gribberish`: iterate messages, emit one single-chunk array per message
/// whose ChunkRef spans the whole message (kerchunk-style — offset/length of the complete GRIB2
/// message within the file; the reader decodes the message to get the grid).
///
/// Grouping model (matches the reference sidecar, kerchunk `scan_grib`): one array per message,
/// named cfgrib-style (see `cfgrib_variable_name`); repeated variables get `_1`, `_2`, … suffixes.
#[cfg(feature = "grib")]
fn generate_grib(granule: &Path) -> Result<VirtualManifest, String> {
    use crate::manifest::{ArrayRef, ChunkRef};
    use gribberish::message::read_messages;
    use std::collections::HashMap;

    let data = std::fs::read(granule).map_err(|e| format!("read {}: {e}", granule.display()))?;
    let source = granule.display().to_string();

    let mut arrays: Vec<ArrayRef> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (index, message) in read_messages(&data).enumerate() {
        let ctx = |what: &str| format!("referencer-rs[grib]: message {index}: {what}");
        let offset = message.byte_offset() as u64;
        let length = message.len() as u64;
        if length == 0 {
            return Err(format!(
                "{} has zero total length (corrupt indicator section?)",
                ctx("")
            ));
        }
        let abbrev = message
            .variable_abbrev()
            .map_err(|e| format!("{}: {e}", ctx("variable abbrev")))?;
        let (surface, surface_value) = message
            .first_fixed_surface()
            .map_err(|e| format!("{}: {e}", ctx("first fixed surface")))?;
        let (nj, ni) = message
            .grid_dimensions()
            .map_err(|e| format!("{}: {e}", ctx("grid dimensions")))?;
        let template = message
            .data_template_number()
            .map_err(|e| format!("{}: {e}", ctx("data representation template")))?;

        let base = cfgrib_variable_name(&abbrev, &surface, surface_value);
        let n = seen.entry(base.clone()).or_insert(0);
        let name = if *n == 0 {
            base.clone()
        } else {
            format!("{base}_{n}")
        };
        *n += 1;

        arrays.push(ArrayRef {
            name,
            shape: vec![nj as u64, ni as u64],
            chunks: vec![nj as u64, ni as u64],
            // gribberish decodes GRIB2 grids to f64 (`Message::data` -> Vec<f64>), matching the
            // sidecar's kerchunk GRIB codec dtype.
            dtype: "float64".to_string(),
            codecs: vec![packing_codec(template)],
            refs: vec![ChunkRef {
                key: "0.0".to_string(),
                path: source.clone(),
                offset,
                length,
            }],
        });
    }
    if arrays.is_empty() {
        return Err(format!(
            "referencer-rs[grib]: no GRIB messages found in {source}"
        ));
    }
    Ok(VirtualManifest {
        generator: "referencer-rs".to_string(),
        source,
        arrays,
    })
}

/// Section 5 (data representation) template number -> codec string. The manifest records HOW the
/// chunk bytes decode; the sidecar derives the same strings independently from eccodes'
/// `packingType`, so exact agreement here is part of the contract being validated.
#[cfg(feature = "grib")]
fn packing_codec(template: u16) -> String {
    match template {
        0 => "grib2:simple".to_string(),
        2 => "grib2:complex".to_string(),
        3 => "grib2:complex-spatial-diff".to_string(),
        4 => "grib2:ieee-float".to_string(),
        40 => "grib2:jpeg2000".to_string(),
        41 => "grib2:png".to_string(),
        42 => "grib2:aec".to_string(),
        n => format!("grib2:template{n}"),
    }
}

/// cfgrib-compatible variable name from gribberish's NCEP-style abbreviation.
///
/// The reference sidecar (kerchunk `scan_grib`, backed by cfgrib) names arrays with eccodes
/// `shortName`s made into identifiers ("t", "10u" -> "u10", "prmsl"). gribberish speaks NCEP
/// abbreviations ("TMP", "UGRD", "PRMSL"), so we translate: a small abbrev -> shortName table
/// (prototype scope: the sample's variables plus obvious neighbors; production would carry the
/// full WMO/eccodes table), a level prefix for height-above-ground fields ("10u", "2t"), then
/// cfgrib's identifier rule (leading digits rotate to the end). Unknown abbreviations fall back
/// to the lowercased abbreviation, which the equivalence harness would flag.
#[cfg(feature = "grib")]
fn cfgrib_variable_name(
    abbrev: &str,
    surface: &gribberish::templates::product::tables::FixedSurfaceType,
    surface_value: Option<f64>,
) -> String {
    use gribberish::templates::product::tables::FixedSurfaceType;

    let base = match abbrev {
        "TMP" => "t",
        "UGRD" => "u",
        "VGRD" => "v",
        "RH" => "r",
        "HGT" => "gh",
        "SPFH" => "q",
        "DPT" => "dpt",
        "PRMSL" => "prmsl",
        other => return other.to_lowercase(),
    };

    // eccodes gives height-above-ground surface fields level-qualified shortNames (2t, 10u);
    // cfgrib then rewrites them into identifiers by moving the digits to the end (t2m has an
    // extra 'm' by eccodes convention; wind components are plain u10/v10).
    if matches!(surface, FixedSurfaceType::SpecifiedHeightLevelAboveGround)
        && let Some(level) = surface_value
    {
        let suffix = if base == "t" { "m" } else { "" };
        return format!("{base}{level:.0}{suffix}");
    }
    base.to_string()
}
#[cfg(not(feature = "grib"))]
fn generate_grib(_granule: &Path) -> Result<VirtualManifest, String> {
    Err("referencer-rs built without GRIB support; rebuild with `--features grib`".into())
}

#[cfg(feature = "hdf5")]
fn generate_hdf5(granule: &Path) -> Result<VirtualManifest, String> {
    // TODO(prototype 0001): implement with the `hdf5-metno` crate.
    // For each chunked dataset: walk the chunk index via H5Dchunk_iter / H5Dget_chunk_info to
    // collect (byte offset, size, filter_mask) per chunk; read dtype/shape/fill/filters for meta.
    // This is the correctness-critical path validated against the VirtualiZarr sidecar.
    let _ = granule;
    Err("referencer-rs[hdf5]: not yet implemented — wire up `hdf5-metno` here".into())
}
#[cfg(not(feature = "hdf5"))]
fn generate_hdf5(_granule: &Path) -> Result<VirtualManifest, String> {
    Err("referencer-rs built without HDF5 support; rebuild with `--features hdf5`".into())
}

/// Adapter that runs the Python VirtualiZarr sidecar, which prints a `VirtualManifest` as JSON.
pub struct VirtualizarrSidecar {
    pub python: String,
    pub script: PathBuf,
}

impl IngestReferencer for VirtualizarrSidecar {
    fn name(&self) -> &'static str {
        "virtualizarr"
    }

    fn generate(&self, granule: &Path) -> Result<VirtualManifest, String> {
        let out = Command::new(&self.python)
            .arg(&self.script)
            .arg(granule)
            .output()
            .map_err(|e| format!("failed to launch sidecar '{}': {e}", self.python))?;
        if !out.status.success() {
            return Err(format!(
                "sidecar exited with error:\n{}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        VirtualManifest::from_str(&text)
            .map_err(|e| format!("sidecar output was not valid manifest JSON: {e}"))
    }
}
