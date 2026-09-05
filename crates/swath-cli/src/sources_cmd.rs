// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `swath sources` (issue #419): the operator actions on the sources
//! domain.
//!
//! # Why a subcommand and not a route
//!
//! ADR 0030 §5 says a server-side fetch happens **only on an operator
//! action**, and ADR 0031 says the mutating surface is absent until there
//! is anyone to authorise it. A subcommand satisfies both by
//! construction: the operator is whoever can run the binary, which is the
//! same person who wrote the config, and there is no handler for an
//! anonymous caller to reach.
//!
//! `fetch` is deliberately a *read*: it retrieves a document under the
//! egress policy and prints it. Registering what it found is #420's
//! guided import, and that is also an operator action.

use std::io::Write as _;

use clap::{Args, Subcommand};
use swath_sources_stac::{FetchError, StacClient};

use crate::config;

/// `swath sources …`.
#[derive(Args)]
pub struct SourcesArgs {
    /// Config file the egress allowlist is read from — the same file
    /// `swath serve` reads, because the policy belongs to the deployment
    /// and not to the invocation.
    #[arg(long, value_name = "PATH", env = "SWATH_CONFIG", global = true)]
    pub config: Option<std::path::PathBuf>,
    /// Which sources action to run.
    #[command(subcommand)]
    pub command: SourcesCommand,
}

/// The sources actions an operator can take from the shell.
#[derive(Subcommand)]
pub enum SourcesCommand {
    /// Print the hosts this deployment may fetch from.
    Allowlist,
    /// Fetch one STAC document under the egress policy and print it.
    Fetch {
        /// An `http(s)` URL whose host is on the allowlist.
        url: String,
    },
}

/// What can go wrong running `swath sources`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SourcesError {
    /// The config could not be read.
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    /// The fetch was refused, or failed.
    #[error(transparent)]
    Fetch(#[from] FetchError),
    /// Federation is off and the caller asked for a fetch.
    #[error(
        "this deployment's egress allowlist is empty, so nothing is fetchable; \
         add hosts under `egress-allowlist` in the config to turn federation on"
    )]
    FederationOff,
}

/// Runs the subcommand, owning its runtime as `serve` does.
///
/// # Errors
///
/// [`SourcesError`]: an unreadable config, an empty allowlist when a
/// fetch was asked for, or the fetch's own refusal.
pub fn run(args: SourcesArgs) -> Result<(), SourcesError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| {
            SourcesError::Fetch(FetchError::Transport {
                detail: err.to_string(),
            })
        })?;
    runtime.block_on(run_async(args))
}

async fn run_async(args: SourcesArgs) -> Result<(), SourcesError> {
    let policy = config::egress_policy(args.config.as_deref())?;
    match args.command {
        SourcesCommand::Allowlist => {
            if policy.is_empty() {
                // Not an error: federation off is the default and a
                // working configuration.
                tracing::info!("egress allowlist: empty — federation is off");
            } else {
                tracing::info!(
                    "egress allowlist: {hosts} (caps: {bytes} bytes, {secs}s)",
                    hosts = policy.hosts().collect::<Vec<_>>().join(", "),
                    bytes = policy.max_bytes,
                    secs = policy.timeout_secs,
                );
            }
            Ok(())
        }
        SourcesCommand::Fetch { url } => {
            if policy.is_empty() {
                return Err(SourcesError::FederationOff);
            }
            let client = StacClient::new(policy)?;
            let document = client.fetch_json(&url).await?;
            // The document goes to stdout, not to the log: it is the
            // command's output, and an operator pipes it.
            let rendered =
                serde_json::to_string_pretty(&document).unwrap_or_else(|_| document.to_string());
            writeln!(std::io::stdout(), "{rendered}").map_err(|err| {
                SourcesError::Fetch(FetchError::Transport {
                    detail: err.to_string(),
                })
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use swath_core::sources::EgressPolicy;
    use swath_testsupport::TempDir;

    use super::{SourcesArgs, SourcesCommand, SourcesError};

    /// No config file is federation off, and so is a config that lists no
    /// hosts. The default is the pre-#419 behaviour: the server reaches
    /// nothing.
    #[test]
    fn federation_is_off_until_the_config_names_a_host() {
        assert!(
            crate::config::egress_policy(None)
                .expect("no file")
                .is_empty()
        );

        let dir = TempDir::new("cli-egress");
        let path = dir.path().join("swath.toml");
        std::fs::write(&path, "store-root = \"/data\"\n").expect("write");
        assert!(
            crate::config::egress_policy(Some(&path))
                .expect("parses")
                .is_empty()
        );

        std::fs::write(
            &path,
            "store-root = \"/data\"\negress-allowlist = [\"stac.example.org\"]\n",
        )
        .expect("write");
        let policy = crate::config::egress_policy(Some(&path)).expect("parses");
        assert!(policy.allows("stac.example.org"));
        assert!(!policy.allows("example.org"));
    }

    /// Asking to fetch with federation off is refused with the sentence
    /// that says how to turn it on — never a silent empty answer.
    #[test]
    fn a_fetch_with_an_empty_allowlist_says_how_to_turn_it_on() {
        let error = super::run(SourcesArgs {
            config: None,
            command: SourcesCommand::Fetch {
                url: "https://stac.example.org/catalog.json".to_owned(),
            },
        })
        .expect_err("federation is off");
        assert!(matches!(error, SourcesError::FederationOff));
        let said = error.to_string();
        assert!(said.contains("egress-allowlist"), "{said}");
    }

    /// Listing the allowlist is never an error, even when it is empty:
    /// "nothing" is a true and useful answer.
    #[test]
    fn listing_an_empty_allowlist_is_not_an_error() {
        super::run(SourcesArgs {
            config: None,
            command: SourcesCommand::Allowlist,
        })
        .expect("an empty allowlist is a working configuration");
    }

    /// The policy's caps are the domain's defaults — the subcommand does
    /// not invent its own.
    #[test]
    fn the_caps_come_from_the_domain() {
        let policy = EgressPolicy::default();
        assert_eq!(
            policy.max_bytes,
            swath_core::sources::DEFAULT_MAX_FETCH_BYTES
        );
        assert_eq!(
            policy.timeout_secs,
            swath_core::sources::DEFAULT_FETCH_TIMEOUT_SECS
        );
    }
}
