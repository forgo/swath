// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `swath serve`: resolve config, build the object store, wire the
//! Phase-1 adapters into the API router, and run axum on a multi-thread
//! tokio runtime with graceful SIGINT/SIGTERM shutdown.
//!
//! Catalog mode (`--catalog`, issue #31) additionally: connects to pgstac,
//! registers the configured datasets (upsert — the dataset-must-pre-exist
//! half of the ingest contract), serves layers that resolve assets from
//! each dataset's latest granule, and — with `--watch-dir` — runs the
//! filedrop ingest loop as a background task in the same process, so
//! `docker compose up` + a dropped manifest is the whole R1 happy path.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use object_store::ObjectStore;
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::prefix::PrefixStore;
use swath_api::{ApiState, CatalogLayers, LayerProvider};
use swath_cache_objectstore::ObjectStoreTileCache;
use swath_catalog_pgstac::PgstacCatalog;
use swath_core::catalog::{Catalog as _, CatalogError};
use swath_core::events::EventSource as _;
use swath_events_filedrop::FiledropEvents;
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;

use crate::config::{self, CatalogMode, LayerSource, ResolvedConfig};

/// Filedrop scan cadence. A quarter second is well under the noise floor
/// of the ingest-to-pixel budget while keeping the idle cost negligible
/// (one `read_dir` per tick).
const WATCH_POLL: Duration = Duration::from_millis(250);

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

    /// Catalog mode: postgres URL of a pgstac database. Layers then come
    /// from [[datasets]] in the config file and resolve their assets from
    /// each dataset's latest ingested granule.
    #[arg(
        long,
        value_name = "URL",
        env = "SWATH_CATALOG",
        conflicts_with = "fixtures"
    )]
    pub(crate) catalog: Option<String>,

    /// Watch this directory for granule manifests (catalog mode): each
    /// `<granule-id>.json` dropped in is ingested automatically.
    #[arg(
        long,
        value_name = "PATH",
        env = "SWATH_WATCH_DIR",
        conflicts_with = "fixtures"
    )]
    pub(crate) watch_dir: Option<PathBuf>,

    /// Tile cache root: a local directory or `s3://bucket[/prefix]`
    /// (issue #36). Rendered tiles are written through and served from
    /// here on repeat requests; absent, no cache is consulted and serving
    /// behaves exactly as before.
    #[arg(long, value_name = "ROOT", env = "SWATH_CACHE")]
    pub(crate) cache: Option<String>,
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
    /// The pgstac catalog could not be reached or prepared.
    #[error("catalog: {0}")]
    Catalog(#[from] CatalogError),
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
    let ResolvedConfig {
        bind,
        base_url,
        store_root,
        cache,
        layers,
    } = cfg;
    let shared = Shared {
        bind,
        base_url,
        store_root,
        cache,
    };
    match layers {
        LayerSource::Static(registry) => {
            let layer_count = registry.identities().len();
            run_server(&shared, registry, layer_count).await
        }
        LayerSource::Catalog(mode) => serve_catalog(&shared, mode).await,
    }
}

/// The mode-independent scalars of a resolved config.
struct Shared {
    bind: std::net::SocketAddr,
    base_url: String,
    store_root: String,
    cache: Option<String>,
}

/// Catalog mode: connect, register datasets, start the ingest loop, serve.
async fn serve_catalog(cfg: &Shared, mode: CatalogMode) -> Result<(), ServeError> {
    let catalog = PgstacCatalog::connect(&mode.url).await?;
    // Register the configured datasets up front: config is the source of
    // truth for dataset identity + layers, and granules ingested later
    // require their dataset to pre-exist (swath_core::ingest docs).
    for dataset in &mode.datasets {
        catalog.upsert_dataset(dataset).await?;
        tracing::info!(
            "registered dataset {id} ({layers} layer(s))",
            id = dataset.id,
            layers = dataset.layers.len(),
        );
    }
    if let Some(dir) = &mode.watch_dir {
        tracing::info!("watching {} for granule manifests", dir.display());
        let mut events = FiledropEvents::new(dir.clone(), WATCH_POLL);
        // The legacy path (ADR 0006): dropped granules whose assets are
        // legacy files (.h5/.nc/.grib2) get virtual manifests generated and
        // stored alongside, automatically. Referencing reads local bytes,
        // so it lights up only for a local store root; on s3:// the legacy
        // extensions would be refused per granule (an honest Malformed),
        // and we say so up front.
        if cfg.store_root.contains("://") {
            tracing::warn!(
                "store root `{root}` is remote: legacy granule referencing                  is disabled (requires a local store root)",
                root = cfg.store_root,
            );
        } else {
            events = events.with_referencer(
                std::sync::Arc::new(swath_referencer::SwathReferencer::new()),
                PathBuf::from(&cfg.store_root),
            );
        }
        tokio::spawn(ingest_loop(events, catalog.clone()));
    }
    let layer_count = mode.layers.len();
    let provider = CatalogLayers::new(catalog, mode.layers);
    run_server(cfg, provider, layer_count).await
}

/// The mode-independent tail of `serve`: build the store, assemble the
/// state, bind, run until SIGINT/SIGTERM.
async fn run_server<L>(cfg: &Shared, layers: L, layer_count: usize) -> Result<(), ServeError>
where
    L: LayerProvider + 'static,
{
    let store = build_store(&cfg.store_root)?;
    let state = ApiState::new(
        layers,
        CogSource::new(store),
        Proj4rsReproject,
        &cfg.base_url,
    );
    // The cache is just another object store (#36): same root grammar,
    // same builder. Wired through `with_cache` so a cache-less config
    // constructs the exact pre-#36 state type and serve path.
    let app = match &cfg.cache {
        Some(root) => {
            tracing::info!("tile cache enabled (write-through) at {root}");
            let cache = ObjectStoreTileCache::new(build_store(root)?);
            swath_api::router(Arc::new(state.with_cache(cache)))
        }
        None => swath_api::router(Arc::new(state)),
    };

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

/// The ingest loop (ARCHITECTURE.md §8): pull granule arrivals from the
/// filedrop watcher, register each through the core's ingest step, log the
/// outcome with its ingest latency. Errors never stop the loop — one bad
/// manifest must not block the next granule (R1).
async fn ingest_loop(mut source: FiledropEvents, catalog: PgstacCatalog) {
    loop {
        match source.next_event().await {
            Ok(Some(event)) => match swath_core::ingest::ingest_granule(&catalog, &event).await {
                Ok(granule) => {
                    let elapsed =
                        now_unix_millis().saturating_sub(event.arrived_at.to_unix_millis());
                    tracing::info!(
                        "ingested granule {id} into dataset {dataset} \
                             ({bands} band(s)) in {elapsed} ms",
                        id = granule.id,
                        dataset = granule.dataset,
                        bands = granule.assets.len(),
                    );
                }
                Err(err) => tracing::error!(
                    "ingest of granule {id} failed: {err}",
                    id = event.granule.id,
                ),
            },
            Ok(None) => {
                tracing::info!("event source exhausted; ingest loop exiting");
                break;
            }
            Err(err) => tracing::warn!("event source: {err}"),
        }
    }
}

/// Wall-clock milliseconds since the Unix epoch.
fn now_unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
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
