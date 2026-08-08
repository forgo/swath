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

#[cfg(feature = "grib")]
fn generate_grib(granule: &Path) -> Result<VirtualManifest, String> {
    // TODO(prototype 0001): implement with the `gribberish` crate.
    // Iterate GRIB2 messages; emit one ChunkRef per message (byte offset + length) plus decoded
    // array metadata (shape, dtype, packing/codec). See README §5.
    let _ = granule;
    Err("referencer-rs[grib]: not yet implemented — wire up `gribberish` here".into())
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
        VirtualManifest::from_str(&text).map_err(|e| format!("sidecar output was not valid manifest JSON: {e}"))
    }
}
