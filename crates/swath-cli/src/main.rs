// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `swath` binary (ARCHITECTURE.md §12, issue #29): the single
//! self-contained deployable. `swath serve` wires the concrete Phase-1
//! adapters — [`CogSource`](swath_source_cog::CogSource) over an
//! `object_store` root (local directory or S3) and
//! [`Proj4rsReproject`](swath_reproject_proj4rs::Proj4rsReproject) — into
//! the generic OGC API - Tiles surface (`swath-api`) and serves it on
//! tokio/axum with graceful SIGINT/SIGTERM shutdown.
//!
//! Configuration is layered, smallest-surface-first (see [`config`]):
//! built-in defaults → optional TOML file (`--config`) → environment
//! (`SWATH_*`) / flags. `swath serve --fixtures` serves the committed HLS
//! demo layers with zero configuration — the compose/e2e path.
//!
//! Logging is `tracing` with a compact single-line subscriber — the
//! workspace already bans `println!` in favor of structured logging
//! (workspace lints), and a server's startup/shutdown lines belong on the
//! same spine its future request telemetry will use. `SWATH_LOG` selects
//! the max level (`error`..`trace`; default `info`) — a plain level name,
//! not an env-filter DSL, keeping the regex machinery out of the tree.

mod config;
mod ingest;
mod serve;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Post-help notes: the environment surface that isn't per-flag.
const AFTER_HELP: &str = "\
Environment:
  SWATH_LOG    max log level: error|warn|info|debug|trace (default: info)

  With an s3:// store root, credentials and endpoint come from the standard
  object_store AWS environment: AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY,
  AWS_DEFAULT_REGION (or AWS_REGION), AWS_ENDPOINT, and AWS_ALLOW_HTTP=true
  for plain-HTTP endpoints such as local MinIO.

Config file (--config, TOML; flags/env override its scalars):
  bind = \"127.0.0.1:8080\"
  base-url = \"http://localhost:8080\"
  store-root = \"/data\"              # or \"s3://bucket/prefix\"

  [[layers]]
  id = \"truecolor\"                  # URL path segment
  title = \"True color\"
  kind = \"truecolor\"                # truecolor (bands r,g,b) | ndvi (bands nir,red)
  rescale = [0.0, 3000.0]           # optional; ndvi defaults to [-1, 1]
  resampling = \"bilinear\"           # bilinear (default) | nearest
  tile-size = 256                   # default 256
  [layers.bands]                    # band name -> asset URI under store-root
  r = \"granule-b04.tif\"
  g = \"granule-b03.tif\"
  b = \"granule-b02.tif\"

Catalog mode (--catalog / SWATH_CATALOG, or `catalog` in the file): layers
are defined per dataset and resolve their assets from the dataset's latest
ingested granule; a watch-dir ingests dropped `<granule-id>.json` manifests
automatically (the ingest-to-pixel path):
  catalog = \"postgres://user:pass@host:5432/db\"   # pgstac
  watch-dir = \"/data/drop\"          # optional: filedrop ingest
  [[datasets]]
  id = \"hls-s30\"
  title = \"HLS S30\"
  license = \"CC0-1.0\"
  [[datasets.layers]]               # same layer schema, except bands map
  id = \"truecolor\"                  # roles to DATASET BAND NAMES:
  kind = \"truecolor\"
  rescale = [0.0, 3000.0]
  [datasets.layers.bands]
  r = \"b04\"
  g = \"b03\"
  b = \"b02\"

The layer `kind` enum is the walking-skeleton stand-in the openEO process
compiler (issue #32) replaces.";

/// Top-level CLI: `swath <command>`.
#[derive(Parser)]
#[command(
    name = "swath",
    version,
    about = "Swath: live satellite imagery tiles (OGC API - Tiles)",
    after_help = AFTER_HELP
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Subcommands. `register` arrives with issue #31's successors.
#[derive(Subcommand)]
enum Command {
    /// Serve configured layers over OGC API - Tiles (plus /traces SSE
    /// and the /healthz liveness probe).
    Serve(serve::ServeArgs),
    /// Ingest utilities (manual/testing): `reference` generates a legacy
    /// granule's virtual manifest (ADR 0006).
    Ingest(ingest::IngestArgs),
}

fn main() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Command::Serve(args) => report(serve::run(&args)),
        Command::Ingest(args) => report(ingest::run(&args)),
    }
}

/// One exit-code policy for every subcommand: errors are logged, not
/// panicked.
fn report<E: std::fmt::Display>(result: Result<(), E>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!("{err}");
            ExitCode::FAILURE
        }
    }
}

/// Compact single-line `tracing` output on stdout; max level from
/// `SWATH_LOG` (a bare level name — default `info`). An unrecognized
/// value falls back to `info` rather than refusing to start: logging
/// config must never take the server down.
fn init_tracing() {
    let level = std::env::var("SWATH_LOG")
        .ok()
        .and_then(|value| value.parse::<tracing::Level>().ok())
        .unwrap_or(tracing::Level::INFO);
    tracing_subscriber::fmt()
        .compact()
        .with_max_level(level)
        .with_target(false)
        .init();
}
