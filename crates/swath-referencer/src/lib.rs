// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

#![doc = include_str!("../README.md")]
//!
//! # In the workspace
//!
//! Standalone by design (ADR 0016): this crate exposes its own
//! [`ReferencerError`] taxonomy and depends only on the manifest
//! vocabulary — Swath's `IngestReferencer` port stays in `swath-core`,
//! adapted by a thin in-tree shim. [`SwathReferencer`] is the entry point;
//! HDF-EOS grid parsing for VNP09GA's sinusoidal grids lives in the `eos` module, and
//! the conformance harness's equivalence check is [`manifest::compare`].

#[cfg(feature = "legacy-hdf5")]
mod eos;
mod grib;
#[cfg(feature = "legacy-hdf5")]
mod hdf;

use std::path::Path;

// The manifest vocabulary this generator emits (the `swath-manifest`
// crate, ADR 0016 §standalone rule), re-exported so one dependency gives
// consumers the generator and its output contract together.
pub use swath_manifest as manifest;

use manifest::VirtualManifest;

/// The generator name stamped into manifests.
pub const GENERATOR: &str = "swath-referencer";

/// What can go wrong generating virtual references for a legacy granule.
///
/// The taxonomy separates "this generator does not do that" from "this
/// granule is broken" from "the machine failed" — consumers route the
/// first to a fallback/conformance story (ADR 0006), log the second per
/// granule, and retry the third.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReferencerError {
    /// The granule is readable but uses something the generator deliberately
    /// does not map (an unrecognized extension, an exotic/big-endian dtype,
    /// an unknown projection). A hard, honest error — never a guessed
    /// manifest (prototype 0001 §7).
    #[error("unsupported by this referencer: {detail}")]
    Unsupported {
        /// What was encountered, naming the offending array/feature.
        detail: String,
    },

    /// The granule could not be understood at all (not a valid container,
    /// corrupt structure, missing required metadata).
    #[error("malformed granule: {detail}")]
    Malformed {
        /// What was wrong, naming the offending granule/structure.
        detail: String,
    },

    /// The underlying filesystem/library machinery failed.
    #[error("referencer backend failure: {detail}")]
    Backend {
        /// What was being attempted.
        detail: String,
        /// The underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

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
    /// here. HDF5/NetCDF4 extensions are only claimed when the crate is
    /// built with the `legacy-hdf5` feature (default ON); a feature-off
    /// build honestly declines them so no adapter routes a granule it
    /// cannot reference.
    #[must_use]
    pub fn handles(path: &Path) -> bool {
        let ext = extension(path);
        let hdf = matches!(ext.as_str(), "h5" | "hdf5" | "nc" | "nc4");
        let grib = matches!(ext.as_str(), "grib2" | "grb2" | "grib");
        (cfg!(feature = "legacy-hdf5") && hdf) || grib
    }

    /// Generates the virtual manifest for one granule file. The manifest's
    /// chunk `path`s reference `granule` as given (the caller controls
    /// whether that is relative or absolute).
    ///
    /// # Errors
    ///
    /// A [`ReferencerError`] per the taxonomy above; a partial manifest is
    /// never returned.
    pub fn generate(&self, granule: &Path) -> Result<VirtualManifest, ReferencerError> {
        match extension(granule).as_str() {
            #[cfg(feature = "legacy-hdf5")]
            "h5" | "hdf5" | "nc" | "nc4" => hdf::generate(granule),
            #[cfg(not(feature = "legacy-hdf5"))]
            ext @ ("h5" | "hdf5" | "nc" | "nc4") => Err(ReferencerError::Unsupported {
                detail: format!(
                    "extension `{ext}` of `{}`: this binary was built without the \
                     `legacy-hdf5` feature — HDF5/NetCDF4 referencing is compiled \
                     out (rebuild with default features)",
                    granule.display()
                ),
            }),
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
    use super::{ReferencerError, SwathReferencer};
    use std::path::Path;

    #[test]
    fn handles_recognizes_the_legacy_extensions() {
        for yes in ["e.grib2", "f.grb2", "g.grib"] {
            assert!(SwathReferencer::handles(Path::new(yes)), "{yes}");
        }
        // HDF5/NetCDF4 extensions are claimed exactly when the feature is in.
        for hdf in ["a.h5", "b.HDF5", "c.nc", "d.nc4"] {
            assert_eq!(
                SwathReferencer::handles(Path::new(hdf)),
                cfg!(feature = "legacy-hdf5"),
                "{hdf}"
            );
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

    /// Feature-off builds must fail loudly on HDF5 granules, naming the
    /// missing feature — never a generic "unsupported extension".
    #[cfg(not(feature = "legacy-hdf5"))]
    #[test]
    fn hdf5_without_the_feature_is_a_loud_unsupported_error() {
        let err = SwathReferencer::new()
            .generate(Path::new("granule.h5"))
            .unwrap_err();
        assert!(matches!(err, ReferencerError::Unsupported { .. }), "{err}");
        assert!(err.to_string().contains("legacy-hdf5"), "{err}");
    }
}
