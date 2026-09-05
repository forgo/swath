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
use swath_core::catalog::Datetime;
use swath_core::sources::{ConsentRefusal, Source, SourceEvent, consent_event, may_read};
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
    /// Prove what reading a source costs: one bounded read, reporting
    /// the bytes and requests it actually made (#424). Never a money
    /// figure — Swath does not know your rate card.
    Prove {
        /// The source being proved, as named in the config. Its
        /// requester-pays consent is checked before anything is read.
        id: String,
        /// An `http(s)` URL whose host is on the allowlist.
        url: String,
    },
    /// Record consent to be billed for reading a requester-pays source.
    /// Once, explicitly, per source.
    Consent {
        /// The source consented to.
        id: String,
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
    /// The source bills the reader and no consent is recorded. Nothing
    /// was read.
    #[error(transparent)]
    Consent(#[from] ConsentRefusal),
    /// The named source is not in the config.
    #[error("no source `{id}` in this deployment's config")]
    NoSuchSource {
        /// The id asked for.
        id: String,
    },
    /// Federation is off and the caller asked for a fetch.
    #[error(
        "this deployment's egress allowlist is empty, so nothing is fetchable; \
         add hosts under `egress-allowlist` in the config to turn federation on"
    )]
    FederationOff,
}

/// The source named `id`, and the events this process has for it.
///
/// Config-declared sources are the only ones that exist across a
/// restart, and their consent rides in the config too — so what this
/// returns is what the file says, which is the whole truth available
/// before the auth interlock lifts.
fn find_source(
    config: Option<&std::path::Path>,
    id: &str,
) -> Result<(Source, Vec<SourceEvent>), SourcesError> {
    let sources = config::declared_sources(config)?;
    let (source, consented_by) = sources
        .into_iter()
        .find(|(source, _)| source.id.as_str() == id)
        .ok_or_else(|| SourcesError::NoSuchSource { id: id.to_owned() })?;
    let events = consented_by
        .map(|by| vec![consent_event(&source.id, &by, now())])
        .unwrap_or_default();
    Ok((source, events))
}

/// Who this deployment can say is acting: the OS user running the
/// command. Not an authenticated identity — there is none yet
/// (ADR 0031) — and labelled as what it is rather than as more.
fn operator() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_owned())
}

/// Now, as the domain's instant type.
fn now() -> Datetime {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    Datetime::from_unix_millis(millis)
        .unwrap_or_else(|_| Datetime::new("1970-01-01T00:00:00Z").expect("the epoch"))
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
        SourcesCommand::Consent { id } => {
            let (source, _) = find_source(args.config.as_deref(), &id)?;
            let by = operator();
            let at = now();
            let event = consent_event(&source.id, &by, at);
            // Recorded where every other source fact is recorded. There
            // is no persistence for it yet beyond the running process
            // (ADR 0030's registry), so this prints what would be
            // recorded and the operator puts it in the config — the
            // honest state of a domain whose write path waits for auth.
            tracing::info!(
                "consent recorded for `{id}` by {by} at {at}: add \
                 `requester-pays-consented-by = \"{by}\"` to the source in your config \
                 to make it survive a restart",
                at = event.at,
            );
            Ok(())
        }
        SourcesCommand::Prove { id, url } => {
            let (source, events) = find_source(args.config.as_deref(), &id)?;
            // Checked BEFORE the client exists, so a refused read is not
            // a read that failed — it is one that never happened.
            may_read(&source, &events)?;
            if policy.is_empty() {
                return Err(SourcesError::FederationOff);
            }
            let client = StacClient::new(policy)?;
            let (_, cost) = client.fetch_measured(&url).await?;
            // Bytes and requests, measured. No currency: see `FetchCost`.
            tracing::info!(
                "read `{url}`: {bytes} bytes over {requests} request(s) — \
                 what that costs is between you and your provider",
                bytes = cost.bytes,
                requests = cost.requests,
            );
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

    /// A requester-pays source with no recorded consent is refused, and
    /// **nothing is read**: the listener in this test counts accepts and
    /// expects zero, so the refusal is a read that never happened rather
    /// than one that failed (#424).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_requester_pays_read_without_consent_never_opens_a_socket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let accepts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&accepts);
        tokio::spawn(async move {
            while listener.accept().await.is_ok() {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });

        let dir = TempDir::new("cli-consent");
        let path = dir.path().join("swath.toml");
        std::fs::write(
            &path,
            "store-root = \"/data\"\n\
             catalog = \"postgres://x\"\n\
             egress-allowlist = [\"127.0.0.1\"]\n\
             \n\
             [[sources]]\n\
             id = \"billed\"\n\
             watch-dir = \"/data/billed\"\n\
             requester-pays = true\n",
        )
        .expect("write");

        let error = super::run_async(SourcesArgs {
            config: Some(path.clone()),
            command: SourcesCommand::Prove {
                id: "billed".to_owned(),
                url: format!("http://127.0.0.1:{port}/catalog.json"),
            },
        })
        .await
        .expect_err("nobody agreed to be billed");
        assert!(matches!(error, SourcesError::Consent(_)), "{error:?}");
        assert_eq!(
            accepts.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a refused read is one that never happened"
        );

        // With consent recorded in the config, the same read proceeds —
        // so the refusal above was the gate and not a broken command.
        std::fs::write(
            &path,
            std::fs::read_to_string(&path).expect("read").replace(
                "requester-pays = true\n",
                "requester-pays = true\nrequester-pays-consented-by = \"operator\"\n",
            ),
        )
        .expect("write");
        let error = super::run_async(SourcesArgs {
            config: Some(path),
            command: SourcesCommand::Prove {
                id: "billed".to_owned(),
                url: format!("http://127.0.0.1:{port}/catalog.json"),
            },
        })
        .await
        .expect_err("the socket answers nothing parseable");
        // It got past the gate: whatever failed, it was not consent.
        assert!(!matches!(error, SourcesError::Consent(_)), "{error:?}");
    }

    /// Proving a source the config does not declare is a named refusal,
    /// not a silent no-op.
    #[tokio::test]
    async fn proving_an_unknown_source_says_so() {
        let error = super::run_async(SourcesArgs {
            config: None,
            command: SourcesCommand::Prove {
                id: "nope".to_owned(),
                url: "https://example.org/c.json".to_owned(),
            },
        })
        .await
        .expect_err("no config declares it");
        assert!(
            matches!(error, SourcesError::NoSuchSource { .. }),
            "{error:?}"
        );
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
