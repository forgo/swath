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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[cfg(feature = "legacy-hdf5")]
    use swath_core::manifest::{VirtualManifest, compare};
    use swath_testsupport::TempDir;

    use super::{IngestArgs, IngestCommand, IngestError, run};

    /// The committed tiny HDF5 fixture (and its h5py-derived truth) from
    /// the referencer's conformance data. Only the `legacy-hdf5` tests
    /// (default profile — `just test` always runs them) touch it; the
    /// feature-off fast profile compiles them out (#99).
    #[cfg(feature = "legacy-hdf5")]
    fn data(file: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../swath-referencer/tests/data")
            .join(file)
    }

    fn reference(granule: PathBuf, output: Option<PathBuf>) -> IngestArgs {
        IngestArgs {
            command: IngestCommand::Reference { granule, output },
        }
    }

    #[cfg(feature = "legacy-hdf5")]
    #[test]
    fn reference_writes_the_known_answer_manifest() {
        let dir = TempDir::new("cli-ingest-known-answer");
        let out = dir.join("tiny.vmanifest.json");
        run(&reference(data("tiny.h5"), Some(out.clone()))).expect("referencing succeeds");

        let written = VirtualManifest::from_json_str(
            &std::fs::read_to_string(&out).expect("manifest written"),
        )
        .expect("written manifest parses as schema v1");
        let expected = VirtualManifest::from_json_str(
            &std::fs::read_to_string(data("tiny.expected.json")).expect("expected json"),
        )
        .expect("expected json parses");
        let report = compare(&written, &expected);
        assert!(
            report.equivalent(),
            "CLI manifest disagrees with the h5py-derived truth: {report:#?}"
        );
    }

    #[cfg(feature = "legacy-hdf5")]
    #[test]
    fn reference_defaults_the_output_beside_the_granule() {
        let dir = TempDir::new("cli-ingest-default-out");
        let granule = dir.join("tiny.h5");
        std::fs::copy(data("tiny.h5"), &granule).expect("fixture copies");
        run(&reference(granule, None)).expect("referencing succeeds");
        let default_out = dir.join("tiny.h5.vmanifest.json");
        assert!(
            VirtualManifest::from_json_str(
                &std::fs::read_to_string(&default_out).expect("default-named manifest written"),
            )
            .is_ok(),
            "default output parses as a manifest"
        );
    }

    #[test]
    fn reference_failures_name_the_granule() {
        let dir = TempDir::new("cli-ingest-badfile");
        let granule = dir.join("not-a-granule.txt");
        std::fs::write(&granule, "plain text").expect("file writes");
        let err = run(&reference(granule.clone(), None)).expect_err("unsupported extension");
        assert!(matches!(&err, IngestError::Reference { granule: g, .. }
            if *g == granule.display().to_string()));
        assert!(
            err.to_string()
                .starts_with(&format!("referencing `{}`: ", granule.display())),
            "got: {err}"
        );
    }

    #[cfg(feature = "legacy-hdf5")]
    #[test]
    fn write_failures_name_the_output_path() {
        let dir = TempDir::new("cli-ingest-badout");
        let out = dir.join("missing-subdir/manifest.json");
        let err =
            run(&reference(data("tiny.h5"), Some(out.clone()))).expect_err("unwritable output");
        assert!(matches!(&err, IngestError::Write { path, .. }
            if *path == out.display().to_string()));
        assert!(
            err.to_string()
                .starts_with(&format!("writing manifest `{}`: ", out.display())),
            "got: {err}"
        );
    }
}
