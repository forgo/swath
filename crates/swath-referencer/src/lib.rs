// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The production pure-Rust virtual-reference generator (ADR 0006, issue
//! #40): [`SwathReferencer`] implements the core
//! [`IngestReferencer`](swath_core::ingest::IngestReferencer) port, turning a
//! legacy granule (HDF5/NetCDF4 via `hdf5-metno`, GRIB2 via `gribberish`)
//! into a [`VirtualManifest`](swath_core::manifest::VirtualManifest) —
//! byte-range references into the original file, generated in milliseconds
//! from a metadata walk, no pixel data touched.
//!
//! Productionized from prototype 0001 (referencer-bakeoff), whose generator
//! logic was proven byte-identical to the Python `VirtualiZarr`/kerchunk
//! reference on a real VNP09GA granule (67 arrays, 1,551 chunk refs) and a
//! GFS GRIB2 sample. The prototype itself is immutable; this crate carries
//! the proven logic forward with the v1 schema's georeferencing
//! ([`eos`]: HDF-EOS `StructMetadata.0` grid parsing for VNP09GA's
//! sinusoidal grids), the core error taxonomy, and no printing.
//!
//! The Python sidecar remains the *conformance reference*: the gated
//! equivalence harness (`just test-referencer`) runs both generators on a
//! real VNP09GA granule and asserts byte-range equivalence via
//! [`swath_core::manifest::compare`].

mod eos;
mod grib;
mod hdf;

use std::path::Path;

use swath_core::ingest::{IngestReferencer, ReferencerError};
use swath_core::manifest::VirtualManifest;

/// The generator name stamped into manifests.
pub const GENERATOR: &str = "swath-referencer";

/// The production pure-Rust reference generator. Stateless; dispatches on
/// file extension (the drop conventions guarantee meaningful extensions —
/// content sniffing is not this port's job).
#[derive(Debug, Clone, Copy, Default)]
pub struct SwathReferencer;

impl SwathReferencer {
    /// A new (stateless) generator.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Whether `path`'s extension names a format this generator handles —
    /// the trigger predicate ingest adapters use to route legacy assets
    /// here.
    #[must_use]
    pub fn handles(path: &Path) -> bool {
        matches!(
            extension(path).as_str(),
            "h5" | "hdf5" | "nc" | "nc4" | "grib2" | "grb2" | "grib"
        )
    }
}

impl IngestReferencer for SwathReferencer {
    fn generate(&self, granule: &Path) -> Result<VirtualManifest, ReferencerError> {
        match extension(granule).as_str() {
            "h5" | "hdf5" | "nc" | "nc4" => hdf::generate(granule),
            "grib2" | "grb2" | "grib" => grib::generate(granule),
            other => Err(ReferencerError::Unsupported {
                detail: format!(
                    "extension `{other}` of `{}` (supported: .h5/.hdf5/.nc/.nc4, .grib2/.grb2/.grib)",
                    granule.display()
                ),
            }),
        }
    }
}

/// Lowercased extension of `path` (empty when absent).
fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{IngestReferencer, ReferencerError, SwathReferencer};
    use std::path::Path;

    #[test]
    fn handles_recognizes_the_legacy_extensions() {
        for yes in [
            "a.h5", "b.HDF5", "c.nc", "d.nc4", "e.grib2", "f.grb2", "g.grib",
        ] {
            assert!(SwathReferencer::handles(Path::new(yes)), "{yes}");
        }
        for no in ["a.tif", "b.json", "c", "d.h5.json"] {
            assert!(!SwathReferencer::handles(Path::new(no)), "{no}");
        }
    }

    #[test]
    fn unsupported_extension_is_a_hard_error() {
        let err = SwathReferencer::new()
            .generate(Path::new("granule.tif"))
            .unwrap_err();
        assert!(matches!(err, ReferencerError::Unsupported { .. }), "{err}");
    }
}
