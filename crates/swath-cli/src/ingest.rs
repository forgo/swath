// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `swath ingest`: manual/testing entrypoints into the ingest machinery.
//!
//! `swath ingest reference <granule>` runs the production referencer
//! ([`SwathReferencer`]) on one legacy granule and writes the
//! `VirtualManifest` JSON next to it (or to `--output`) — the same
//! generation the filedrop legacy path performs automatically, exposed for
//! operators and the conformance harness (`just test-referencer`). The
//! manifest's chunk paths name the granule as given on the command line;
//! ingest-time generation rewrites them to store-relative keys instead
//! (the filedrop adapter's job, not this command's).

use std::path::PathBuf;

use swath_core::ingest::{IngestReferencer as _, ReferencerError};
use swath_referencer::SwathReferencer;

/// `swath ingest <subcommand>` arguments.
#[derive(Debug, clap::Args)]
pub(crate) struct IngestArgs {
    #[command(subcommand)]
    command: IngestCommand,
}

#[derive(Debug, clap::Subcommand)]
enum IngestCommand {
    /// Generate a virtual-reference manifest for a legacy granule
    /// (HDF5/NetCDF4, GRIB2) without registering anything.
    Reference {
        /// The granule file to reference.
        granule: PathBuf,
        /// Where to write the manifest JSON
        /// (default: `<granule>.vmanifest.json`).
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,
    },
}

/// Ingest-path errors, phrased for the operator.
#[derive(Debug, thiserror::Error)]
pub(crate) enum IngestError {
    /// The generator refused or failed.
    #[error("referencing `{granule}`: {source}")]
    Reference {
        /// The granule that was being referenced.
        granule: String,
        /// The generator's error.
        #[source]
        source: ReferencerError,
    },
    /// The manifest could not be written.
    #[error("writing manifest `{path}`: {source}")]
    Write {
        /// The output path.
        path: String,
        /// The filesystem error.
        #[source]
        source: std::io::Error,
    },
}

/// Runs one ingest subcommand.
pub(crate) fn run(args: &IngestArgs) -> Result<(), IngestError> {
    match &args.command {
        IngestCommand::Reference { granule, output } => {
            let manifest = SwathReferencer::new().generate(granule).map_err(|source| {
                IngestError::Reference {
                    granule: granule.display().to_string(),
                    source,
                }
            })?;
            let out = output.clone().unwrap_or_else(|| {
                let mut name = granule.file_name().unwrap_or_default().to_os_string();
                name.push(".vmanifest.json");
                granule.with_file_name(name)
            });
            std::fs::write(&out, manifest.to_json_string()).map_err(|source| {
                IngestError::Write {
                    path: out.display().to_string(),
                    source,
                }
            })?;
            let refs: usize = manifest.arrays.iter().map(|a| a.refs.len()).sum();
            let georeferenced = manifest
                .arrays
                .iter()
                .filter(|a| a.georef.is_some())
                .count();
            tracing::info!(
                "referenced {granule}: {arrays} array(s), {refs} chunk ref(s), \
                 {georeferenced} georeferenced -> {out}",
                granule = granule.display(),
                arrays = manifest.arrays.len(),
                out = out.display(),
            );
            Ok(())
        }
    }
}
