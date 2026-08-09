// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `swath serve`: resolve config, build the object store, wire the
//! Phase-1 adapters into the API router, and run axum on a multi-thread
//! tokio runtime with graceful SIGINT/SIGTERM shutdown.

use std::path::PathBuf;
use std::sync::Arc;

use object_store::ObjectStore;
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::prefix::PrefixStore;
use swath_api::ApiState;
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;

use crate::config::{self, ResolvedConfig};

/// `swath serve` arguments. Scalars carry both a flag and a `SWATH_*`
/// variable (clap's `env` attribute — `--help` documents both); either
/// outranks the config file.
#[derive(Debug, clap::Args)]
pub(crate) struct ServeArgs {
    /// TOML config file (layers live here; flags/env override scalars).
    #[arg(long, value_name = "PATH", conflicts_with = "fixtures")]
    pub(crate) config: Option<PathBuf>,

    /// Serve the built-in HLS demo layers (truecolor, ndvi) from the
    /// committed fixtures in ./tests/fixtures — zero config.
    #[arg(long)]
    pub(crate) fixtures: bool,

    /// Socket address to listen on.
    #[arg(long, value_name = "ADDR:PORT", env = "SWATH_BIND")]
    pub(crate) bind: Option<std::net::SocketAddr>,

    /// Base URL minted into OGC links (defaults to `http://localhost:<port>`).
    #[arg(long, value_name = "URL", env = "SWATH_BASE_URL")]
    pub(crate) base_url: Option<String>,

    /// Object-store root: a local directory or `s3://bucket[/prefix]`
    /// (S3 credentials/endpoint via the standard AWS_* environment).
    #[arg(long, value_name = "ROOT", env = "SWATH_STORE_ROOT")]
    pub(crate) store_root: Option<String>,
}

/// Serve-path errors, each phrased for the operator reading the log.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ServeError {
    /// Configuration resolution failed.
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    /// The object store could not be built from the store root.
    #[error("cannot open object store at `{root}`: {source}")]
    Store {
        /// The configured store root.
        root: String,
        /// The `object_store` failure.
        source: object_store::Error,
    },
    /// The listener could not bind.
    #[error("cannot bind {bind}: {source}")]
    Bind {
        /// The configured bind address.
        bind: std::net::SocketAddr,
        /// The socket failure.
        source: std::io::Error,
    },
    /// The server itself failed.
    #[error("server error: {0}")]
    Serve(#[source] std::io::Error),
}

/// Resolves config, builds the runtime, and serves until SIGINT/SIGTERM.
pub(crate) fn run(args: &ServeArgs) -> Result<(), ServeError> {
    let cfg = config::resolve(args)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(ServeError::Serve)?;
    runtime.block_on(serve(cfg))
}

/// Wires adapters into the router and runs axum with graceful shutdown.
async fn serve(cfg: ResolvedConfig) -> Result<(), ServeError> {
    let store = build_store(&cfg.store_root)?;
    let layer_count = cfg.registry.iter().count();
    let state = ApiState::new(
        cfg.registry,
        CogSource::new(store),
        Proj4rsReproject,
        &cfg.base_url,
    );
    let app = swath_api::router(Arc::new(state));

    let listener = tokio::net::TcpListener::bind(cfg.bind)
        .await
        .map_err(|source| ServeError::Bind {
            bind: cfg.bind,
            source,
        })?;
    let local = listener.local_addr().map_err(ServeError::Serve)?;
    tracing::info!(
        "serving {layer_count} layer(s) on {local} (store: {root}); traces: {base}/traces",
        root = cfg.store_root,
        base = cfg.base_url,
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(ServeError::Serve)?;
    tracing::info!("shutdown complete");
    Ok(())
}

/// The object store behind the configured root: `s3://bucket[/prefix]`
/// builds an S3 store from the standard AWS_* environment (endpoint,
/// region, credentials, `AWS_ALLOW_HTTP` for `MinIO`); anything else is a
/// local directory.
fn build_store(root: &str) -> Result<Arc<dyn ObjectStore>, ServeError> {
    let store_error = |source| ServeError::Store {
        root: root.to_owned(),
        source,
    };
    if let Some(rest) = root.strip_prefix("s3://") {
        let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        let s3 = AmazonS3Builder::from_env()
            .with_bucket_name(bucket)
            .build()
            .map_err(store_error)?;
        Ok(if prefix.is_empty() {
            Arc::new(s3)
        } else {
            Arc::new(PrefixStore::new(s3, prefix))
        })
    } else {
        LocalFileSystem::new_with_prefix(root)
            .map(|fs| Arc::new(fs) as Arc<dyn ObjectStore>)
            .map_err(store_error)
    }
}

/// Resolves on SIGINT (Ctrl-C) or, on unix, SIGTERM (the container stop
/// signal) — axum then stops accepting and drains in-flight requests.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("SIGINT handler installs");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler installs")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("shutdown signal received; draining in-flight requests");
}
