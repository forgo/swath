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

use std::future::Future;
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
use swath_core::catalog::{Catalog, CatalogError};
use swath_core::events::EventSource;
use swath_events_filedrop::FiledropEvents;
use swath_pyramid_objectstore::PyramidSource;
use swath_reproject_proj4rs::Proj4rsReproject;

use crate::config::{self, CatalogMode, LayerSource, ResolvedConfig};
use crate::source::CompositeSource;

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

    /// Global default for the planner's overview oversampling slack
    /// (issue #37): an overview factor is eligible when `factor <=
    /// desired ratio x this value`. Default 1.2 (GDAL's slack). Per-layer
    /// `[layers.budget]` values override it.
    #[arg(long, value_name = "RATIO", env = "SWATH_OVERVIEW_OVERSAMPLE")]
    pub(crate) overview_oversample: Option<f64>,

    /// Global default for the planner's live-render ceiling (issue #37):
    /// refuse tiles whose estimated live cost exceeds this many bytes
    /// when nothing cheaper can serve. Absent, never refuse. Per-layer
    /// `[layers.budget]` values override it.
    #[arg(long, value_name = "BYTES", env = "SWATH_MAX_ESTIMATED_LIVE_BYTES")]
    pub(crate) max_estimated_live_bytes: Option<u64>,

    /// Serve CORS headers for these origins (comma-separated exact
    /// origins, or `*` for any — cross-origin dev). Default: none — no
    /// CORS headers at all; same-origin serving (the embedded UI, or the
    /// vite dev proxy) needs none (issue #103, ADR 0011).
    #[arg(
        long,
        value_name = "ORIGINS",
        env = "SWATH_CORS_ALLOWED_ORIGINS",
        value_delimiter = ','
    )]
    pub(crate) cors_allowed_origins: Vec<String>,
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
    runtime.block_on(serve(cfg, shutdown_signal()))
}

/// Wires adapters into the router and runs axum until `shutdown` resolves
/// (production passes [`shutdown_signal`]; tests pass a ready future).
async fn serve<F>(cfg: ResolvedConfig, shutdown: F) -> Result<(), ServeError>
where
    F: Future<Output = ()> + Send + 'static,
{
    let ResolvedConfig {
        bind,
        base_url,
        store_root,
        cache,
        cors_allowed_origins,
        layers,
    } = cfg;
    let shared = Shared {
        bind,
        base_url,
        store_root,
        cache,
        cors_allowed_origins,
    };
    match layers {
        LayerSource::Static(registry) => {
            let layer_count = registry.identities().len();
            run_server(&shared, registry, layer_count, None, shutdown).await
        }
        LayerSource::Catalog(mode) => serve_catalog(&shared, mode, shutdown).await,
    }
}

/// The mode-independent scalars of a resolved config.
struct Shared {
    bind: std::net::SocketAddr,
    base_url: String,
    store_root: String,
    cache: Option<String>,
    /// CORS origin allowlist (issue #103, ADR 0011); empty = no CORS
    /// layer at all (the default).
    cors_allowed_origins: Vec<String>,
}

/// Catalog mode: connect to pgstac, then hand over to the generic tail.
async fn serve_catalog<F>(cfg: &Shared, mode: CatalogMode, shutdown: F) -> Result<(), ServeError>
where
    F: Future<Output = ()> + Send + 'static,
{
    let catalog = PgstacCatalog::connect(&mode.url).await?;
    serve_catalog_on(cfg, mode, catalog, shutdown).await
}

/// The connection-independent body of catalog mode: register datasets
/// (carrying over any layers the openEO services surface published in
/// earlier runs), start the ingest loop, serve — with the openEO authoring
/// router merged in (ADR 0010). Generic over the [`Catalog`] so tests can
/// drive it against an in-memory catalog.
async fn serve_catalog_on<C, F>(
    cfg: &Shared,
    mut mode: CatalogMode,
    catalog: C,
    shutdown: F,
) -> Result<(), ServeError>
where
    C: Catalog + Clone + Send + Sync + 'static,
    F: Future<Output = ()> + Send + 'static,
{
    // Register the configured datasets up front: config is the source of
    // truth for dataset identity + config-defined layers, and granules
    // ingested later require their dataset to pre-exist
    // (swath_core::ingest docs). Layers authored through the openEO
    // services surface (ADR 0010) live only in the catalog — carry them
    // over before the upsert and recompile them into serving templates,
    // so published products survive a restart.
    for dataset in &mut mode.datasets {
        if let Some(existing) = catalog.get_dataset(&dataset.id).await? {
            for layer in existing.layers {
                let is_service = layer.process.is_some();
                let conflicts = dataset.layers.iter().any(|own| own.id == layer.id);
                if !is_service || conflicts {
                    continue;
                }
                match swath_api::compile_service_layer(dataset, &layer) {
                    Ok(template) => {
                        tracing::info!(
                            "restored openEO service {id} on dataset {dataset}",
                            id = layer.id,
                            dataset = dataset.id,
                        );
                        dataset.layers.push(layer);
                        mode.layers.push(template);
                    }
                    // Honest degradation: a graph that no longer compiles
                    // (e.g. the dataset's band vocabulary changed under
                    // it) is dropped loudly, not served wrongly or
                    // crashed on.
                    Err(err) => tracing::warn!(
                        "dropping persisted openEO service {id}: its process graph no \
                         longer compiles against dataset {dataset}: {err}",
                        id = layer.id,
                        dataset = dataset.id,
                    ),
                }
            }
        }
        // Derived temporal extent (ADR 0015): the config compiles an
        // open "no granule yet" interval; the served truth is the
        // min/max acquisition datetime of what has actually been
        // ingested. Re-deriving from all granules at registration also
        // heals any drift the incremental per-ingest widening (the
        // core's `ingest_granule`) could leave behind.
        let granules = catalog
            .find_granules(&dataset.id, &swath_core::catalog::GranuleQuery::default())
            .await?;
        dataset.extent.interval = swath_core::catalog::temporal_interval(&granules);
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
    // The granule browsing surface (issue #107): read-only
    // `GET /datasets/{datasetId}/granules` over the same catalog.
    let granules = swath_api::granules_router(Arc::new(swath_api::GranulesState::new(
        catalog.clone(),
        &cfg.base_url,
    )));
    let provider = CatalogLayers::new(catalog, mode.layers);
    // The openEO authoring surface (ADR 0010) over the same provider:
    // clones share the layer set, so a POSTed service serves on the next
    // tile request. The preview endpoint (ADR 0014's POST /result)
    // renders inline through the same composite source and reprojection
    // adapters the tile handlers use — same store root, same pixels
    // (pyramid overlay included, so previews benefit from materialized
    // overviews exactly as tiles do).
    let openeo_store = build_store(&cfg.store_root)?;
    let openeo = swath_api::openeo_router(Arc::new(swath_api::OpenEoState::new(
        provider.clone(),
        PyramidSource::new(
            CompositeSource::new(Arc::clone(&openeo_store)),
            openeo_store,
        ),
        Proj4rsReproject,
        &cfg.base_url,
    )));
    run_server(
        cfg,
        provider,
        layer_count,
        Some(openeo.merge(granules)),
        shutdown,
    )
    .await
}

/// The mode-independent tail of `serve`: build the store, assemble the
/// state, bind, run until SIGINT/SIGTERM. `extra` merges an additional
/// router into the OGC one — catalog mode passes the openEO authoring
/// surface (ADR 0010), which also switches the landing page into its
/// dual OGC + openEO-capabilities form.
async fn run_server<L, F>(
    cfg: &Shared,
    layers: L,
    layer_count: usize,
    extra: Option<axum::Router>,
    shutdown: F,
) -> Result<(), ServeError>
where
    L: LayerProvider + 'static,
    F: Future<Output = ()> + Send + 'static,
{
    let store = build_store(&cfg.store_root)?;
    // The composite source (crate::source): COG assets and virtual-cube
    // manifests (#39) served from the same store root, dispatched per
    // asset — legacy granules render from byte ranges into their
    // original files. Wrapped in the pyramid overlay (#183): levels
    // materialized by `swath materialize` under `pyramids/` in the same
    // root are advertised to the planner and served from stored chunks;
    // with no pyramid present the overlay is a per-describe existence
    // probe and nothing more.
    let mut state = ApiState::new(
        layers,
        PyramidSource::new(CompositeSource::new(Arc::clone(&store)), store),
        Proj4rsReproject,
        &cfg.base_url,
    );
    if extra.is_some() {
        state = state.with_openeo();
    }
    // The embedded UI (issue #103, ADR 0011): browsers get index.html at
    // `/`, hashed assets serve from the router fallback, API clients see
    // no change. Compiled in by default (feature `embedded-ui`); an empty
    // embed (a build without web/dist) degrades honestly to no UI.
    #[cfg(feature = "embedded-ui")]
    {
        let ui = embedded_ui();
        if ui.is_empty() {
            tracing::warn!(
                "no web bundle was embedded at build time (web/dist was absent — build via \
                 `just build-full`); serving without a UI"
            );
        } else {
            tracing::info!("embedded UI at {base}/", base = cfg.base_url);
            state = state.with_ui(ui);
        }
    }
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
    let app = match extra {
        Some(extra) => app.merge(extra),
        None => app,
    };
    // Opt-in CORS (issue #103, ADR 0011), layered over the WHOLE merged
    // router (openEO included). Absent origins = absent layer: the
    // default same-origin story serves byte-identical responses.
    let app = match swath_api::cors_layer(&cfg.cors_allowed_origins) {
        Some(cors) => {
            tracing::info!(
                "CORS enabled for origins: {origins}",
                origins = cfg.cors_allowed_origins.join(", "),
            );
            app.layer(cors)
        }
        None => app,
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
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(ServeError::Serve)?;
    tracing::info!("shutdown complete");
    Ok(())
}

/// The ingest loop (ARCHITECTURE.md §8): pull granule arrivals from the
/// filedrop watcher, register each through the core's ingest step, log the
/// outcome with its ingest latency. Errors never stop the loop — one bad
/// manifest must not block the next granule (R1). Generic over the ports so
/// the arrive→register flow is testable against in-memory fakes.
async fn ingest_loop<S: EventSource, C: Catalog>(mut source: S, catalog: C) {
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

/// The web bundle staged by the build script (`$OUT_DIR/ui`, a copy of
/// `web/dist` — empty when the checkout had no bundle at build time),
/// flattened into the API crate's [`swath_api::UiAssets`].
#[cfg(feature = "embedded-ui")]
fn embedded_ui() -> swath_api::UiAssets {
    static UI_DIR: include_dir::Dir<'_> = include_dir::include_dir!("$OUT_DIR/ui");
    fn collect<'a>(dir: &include_dir::Dir<'a>, files: &mut Vec<(String, &'a [u8])>) {
        for entry in dir.entries() {
            match entry {
                include_dir::DirEntry::Dir(sub) => collect(sub, files),
                include_dir::DirEntry::File(file) => {
                    files.push((file.path().to_string_lossy().into_owned(), file.contents()));
                }
            }
        }
    }
    let mut files = Vec::new();
    collect(&UI_DIR, &mut files);
    swath_api::UiAssets::from_files(files)
}

/// The object store behind the configured root: `s3://bucket[/prefix]`
/// builds an S3 store from the standard AWS_* environment (endpoint,
/// region, credentials, `AWS_ALLOW_HTTP` for `MinIO`); anything else is a
/// local directory.
pub(crate) fn build_store(root: &str) -> Result<Arc<dyn ObjectStore>, ServeError> {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::future::ready;
    use std::sync::{Arc, Mutex};

    use object_store::ObjectStoreExt as _;

    use swath_core::catalog::{
        Bbox, Catalog, CatalogError, Dataset, DatasetId, Datetime, Granule, GranuleId,
        GranuleQuery, TimeRange,
    };
    use swath_core::events::{EventError, EventSource, GranuleEvent};
    use swath_testsupport::TempDir;

    use super::{
        ServeArgs, ServeError, Shared, build_store, ingest_loop, now_unix_millis, run, serve,
        serve_catalog_on,
    };
    use crate::config::{self, ConfigError, LayerSource};

    /// A minimal in-memory [`Catalog`] enforcing the dataset-must-pre-exist
    /// contract, shared by clones like pgstac in production.
    #[derive(Debug, Clone, Default)]
    struct MemoryCatalog {
        datasets: Arc<Mutex<BTreeMap<String, Dataset>>>,
        granules: Arc<Mutex<Vec<Granule>>>,
    }

    impl Catalog for MemoryCatalog {
        async fn upsert_dataset(&self, dataset: &Dataset) -> Result<(), CatalogError> {
            self.datasets
                .lock()
                .unwrap()
                .insert(dataset.id.as_str().to_owned(), dataset.clone());
            Ok(())
        }

        async fn upsert_granules(&self, granules: &[Granule]) -> Result<(), CatalogError> {
            for granule in granules {
                if !self
                    .datasets
                    .lock()
                    .unwrap()
                    .contains_key(granule.dataset.as_str())
                {
                    return Err(CatalogError::DatasetNotFound {
                        id: granule.dataset.clone(),
                    });
                }
                self.granules.lock().unwrap().push(granule.clone());
            }
            Ok(())
        }

        async fn get_dataset(&self, id: &DatasetId) -> Result<Option<Dataset>, CatalogError> {
            Ok(self.datasets.lock().unwrap().get(id.as_str()).cloned())
        }

        async fn list_datasets(&self) -> Result<Vec<Dataset>, CatalogError> {
            Ok(self.datasets.lock().unwrap().values().cloned().collect())
        }

        async fn find_granules(
            &self,
            dataset: &DatasetId,
            _query: &GranuleQuery,
        ) -> Result<Vec<Granule>, CatalogError> {
            Ok(self
                .granules
                .lock()
                .unwrap()
                .iter()
                .filter(|granule| granule.dataset == *dataset)
                .cloned()
                .collect())
        }
    }

    /// A finite replay [`EventSource`]: yields the scripted results, then
    /// reports exhaustion (`Ok(None)`).
    struct ScriptedEvents(Vec<Result<Option<GranuleEvent>, EventError>>);

    impl EventSource for ScriptedEvents {
        async fn next_event(&mut self) -> Result<Option<GranuleEvent>, EventError> {
            if self.0.is_empty() {
                Ok(None)
            } else {
                self.0.remove(0)
            }
        }
    }

    fn granule_event(dataset: &str) -> GranuleEvent {
        GranuleEvent {
            granule: Granule {
                id: GranuleId::new("g1"),
                dataset: DatasetId::new(dataset),
                bbox: Bbox {
                    west: -106.1,
                    south: 39.2,
                    east: -105.9,
                    north: 39.4,
                },
                datetime: Datetime::new("2024-06-06T17:54:00Z").expect("valid datetime"),
                assets: BTreeMap::new(),
                ingested_at: None,
            },
            arrived_at: Datetime::new("2026-08-08T12:00:00Z").expect("valid datetime"),
        }
    }

    fn dataset(id: &str) -> Dataset {
        Dataset {
            id: DatasetId::new(id),
            title: id.to_owned(),
            description: String::new(),
            license: "other".to_owned(),
            extent: swath_core::catalog::Extent {
                bbox: Bbox {
                    west: -180.0,
                    south: -90.0,
                    east: 180.0,
                    north: 90.0,
                },
                interval: TimeRange::default(),
            },
            bands: std::collections::BTreeSet::new(),
            layers: Vec::new(),
        }
    }

    fn args() -> ServeArgs {
        ServeArgs {
            config: None,
            fixtures: false,
            bind: None,
            base_url: None,
            store_root: None,
            catalog: None,
            watch_dir: None,
            cache: None,
            overview_oversample: None,
            max_estimated_live_bytes: None,
            cors_allowed_origins: Vec::new(),
        }
    }

    #[tokio::test]
    async fn build_store_local_directory_roots_the_store() {
        let dir = TempDir::new("cli-store-local");
        let store =
            build_store(dir.path().to_str().expect("utf-8 temp path")).expect("local store builds");
        store
            .put(
                &object_store::path::Path::from("probe.txt"),
                object_store::PutPayload::from_static(b"tile bytes"),
            )
            .await
            .expect("put succeeds");
        assert_eq!(
            std::fs::read(dir.join("probe.txt")).expect("object lands under the root"),
            b"tile bytes"
        );
    }

    #[test]
    fn build_store_s3_scheme_builds_with_and_without_prefix() {
        // Credentials are lazy (resolved per request), so building from a
        // bare bucket URL succeeds without any AWS_* environment.
        assert!(build_store("s3://tiles").is_ok(), "bare bucket");
        assert!(build_store("s3://tiles/some/prefix").is_ok(), "with prefix");
    }

    #[test]
    fn build_store_unsupported_scheme_is_a_store_error() {
        // Any non-s3 scheme falls through to the local branch, where the
        // pseudo-path honestly fails to open.
        let err = build_store("memory://tiles").expect_err("unsupported scheme");
        assert!(matches!(&err, ServeError::Store { root, .. } if root == "memory://tiles"));
        assert!(
            err.to_string()
                .starts_with("cannot open object store at `memory://tiles`: "),
            "got: {err}"
        );
    }

    /// One test per `ServeError` variant, pinning the exact operator-facing
    /// rendering (issue #96 AC).
    #[test]
    fn serve_error_config_message() {
        let err = ServeError::from(ConfigError::NoLayers);
        assert_eq!(
            err.to_string(),
            "no layers to serve: pass --fixtures, or --config with at least one [[layers]]"
        );
    }

    #[test]
    fn serve_error_store_message() {
        let err = ServeError::Store {
            root: "s3://tiles".to_owned(),
            source: object_store::Error::Generic {
                store: "S3",
                source: "bucket vanished".into(),
            },
        };
        assert_eq!(
            err.to_string(),
            "cannot open object store at `s3://tiles`: Generic S3 error: bucket vanished"
        );
    }

    #[test]
    fn serve_error_bind_message() {
        let err = ServeError::Bind {
            bind: "127.0.0.1:8080".parse().expect("socket addr"),
            source: std::io::Error::other("address in use"),
        };
        assert_eq!(
            err.to_string(),
            "cannot bind 127.0.0.1:8080: address in use"
        );
    }

    #[test]
    fn serve_error_serve_message() {
        let err = ServeError::Serve(std::io::Error::other("connection reset"));
        assert_eq!(err.to_string(), "server error: connection reset");
    }

    #[test]
    fn serve_error_catalog_message() {
        let err = ServeError::from(CatalogError::DatasetNotFound {
            id: DatasetId::new("hls-s30"),
        });
        assert_eq!(err.to_string(), "catalog: dataset not found: hls-s30");
    }

    #[test]
    fn now_unix_millis_reads_the_wall_clock() {
        let first = now_unix_millis();
        let second = now_unix_millis();
        // After mid-2025 in real time, and monotone across two reads.
        assert!(first > 1_750_000_000_000, "got {first}");
        assert!(second >= first);
    }

    #[tokio::test]
    async fn ingest_loop_registers_arrivals_and_survives_bad_ones() {
        let catalog = MemoryCatalog::default();
        catalog
            .upsert_dataset(&dataset("hls-s30"))
            .await
            .expect("seed dataset");
        let events = ScriptedEvents(vec![
            Ok(Some(granule_event("hls-s30"))),
            // A granule of an unknown dataset: logged, never blocks the loop.
            Ok(Some(granule_event("nope"))),
            // A bad announcement: logged, never blocks the loop.
            Err(EventError::Malformed {
                detail: "not a manifest".to_owned(),
            }),
        ]);
        // The scripted source then reports exhaustion, so the loop exits.
        ingest_loop(events, catalog.clone()).await;
        let stored = catalog
            .find_granules(&DatasetId::new("hls-s30"), &GranuleQuery::default())
            .await
            .expect("query succeeds");
        assert_eq!(stored.len(), 1, "exactly the good granule registered");
        assert_eq!(
            stored[0].ingested_at,
            Some(Datetime::new("2026-08-08T12:00:00Z").expect("valid datetime")),
            "ingested_at is stamped from the event's arrival time"
        );
    }

    #[tokio::test]
    async fn serve_static_mode_binds_serves_and_drains() {
        let store = TempDir::new("cli-serve-store");
        let cache = TempDir::new("cli-serve-cache");
        let cfg = config::resolve(&ServeArgs {
            fixtures: true,
            bind: Some("127.0.0.1:0".parse().expect("socket addr")),
            store_root: Some(store.path().display().to_string()),
            cache: Some(cache.path().display().to_string()),
            ..args()
        })
        .expect("fixtures config resolves");
        // An already-resolved shutdown: the server binds, serves zero
        // requests, drains, and returns cleanly.
        serve(cfg, ready(())).await.expect("serves and shuts down");
    }

    #[tokio::test]
    async fn serve_reports_bind_conflicts() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("listener binds");
        let addr = taken.local_addr().expect("local addr");
        let store = TempDir::new("cli-serve-bindfail");
        let mut cfg = config::resolve(&ServeArgs {
            fixtures: true,
            store_root: Some(store.path().display().to_string()),
            ..args()
        })
        .expect("fixtures config resolves");
        cfg.bind = addr;
        let err = serve(cfg, ready(())).await.expect_err("port is taken");
        assert!(matches!(&err, ServeError::Bind { bind, .. } if *bind == addr));
        assert!(
            err.to_string()
                .starts_with(&format!("cannot bind {addr}: ")),
            "got: {err}"
        );
    }

    #[test]
    fn run_surfaces_config_errors_before_serving() {
        let err = run(&args()).expect_err("no store root configured");
        assert!(matches!(err, ServeError::Config(ConfigError::NoStoreRoot)));
    }

    /// A `[[datasets]]` config over a temp store/drop dir, with the bind
    /// port ephemeral. Returns the TOML text. Paths are written as TOML
    /// *literal* (single-quoted) strings — in a basic string a Windows
    /// path's backslashes read as escape sequences (`C:\Users` trips on
    /// `\U`), exactly as they would for an operator authoring the file.
    fn catalog_toml(store_root: &std::path::Path, watch_dir: &std::path::Path) -> String {
        format!(
            r#"
            bind = "127.0.0.1:0"
            store-root = '{store}'
            catalog = "postgres://unused@localhost/unused"
            watch-dir = '{drop}'

            [[datasets]]
            id = "hls-s30"
            title = "HLS S30"
            license = "CC0-1.0"

            [[datasets.layers]]
            id = "truecolor"
            kind = "truecolor"
            rescale = [0.0, 3000.0]
            [datasets.layers.bands]
            r = "b04"
            g = "b03"
            b = "b02"

            [[datasets.layers]]
            id = "ndvi"
            kind = "ndvi"
            [datasets.layers.bands]
            nir = "b8a"
            red = "b04"
            "#,
            store = store_root.display(),
            drop = watch_dir.display(),
        )
    }

    /// A persisted openEO service layer (ADR 0010) as the services surface
    /// would have written it.
    fn service_layer(id: &str, process: serde_json::Value) -> swath_core::catalog::Layer {
        swath_core::catalog::Layer {
            id: id.to_owned(),
            title: id.to_owned(),
            description: String::new(),
            plan: swath_core::catalog::PlanKind::BandMath {
                expression: "(b8a - b04) / (b8a + b04)".to_owned(),
            },
            rescale: swath_core::catalog::Rescale {
                min: -1.0,
                max: 1.0,
            },
            colormap: None,
            resampling: swath_core::catalog::Resampling::Bilinear,
            tile_size: 256,
            process: Some(process),
        }
    }

    /// An NDVI process graph that compiles against the config dataset's
    /// band vocabulary (b02/b03/b04/b8a).
    fn ndvi_graph() -> serde_json::Value {
        serde_json::json!({ "process_graph": {
            "load": { "process_id": "load_collection", "arguments": {
                "id": "hls-s30", "spatial_extent": null, "temporal_extent": null,
                "bands": ["b8a", "b04"],
            }},
            "ndvi": { "process_id": "ndvi", "arguments": {
                "data": { "from_node": "load" }, "nir": "b8a", "red": "b04",
            }},
            "save": { "process_id": "save_result", "arguments": {
                "data": { "from_node": "ndvi" }, "format": "png",
            }, "result": true },
        }})
    }

    #[tokio::test]
    async fn serve_catalog_mode_registers_datasets_and_restores_services() {
        let store = TempDir::new("cli-catalog-store");
        let drop_dir = TempDir::new("cli-catalog-drop");
        let config_dir = TempDir::new("cli-catalog-config");
        let config_path = config_dir.join("swath.toml");
        std::fs::write(&config_path, catalog_toml(store.path(), drop_dir.path()))
            .expect("config writes");
        let cfg = config::resolve(&ServeArgs {
            config: Some(config_path),
            ..args()
        })
        .expect("catalog config resolves");
        let LayerSource::Catalog(mode) = cfg.layers else {
            panic!("catalog config resolves to catalog mode");
        };

        // An earlier run's catalog document: a published service (restored),
        // a broken service (dropped loudly), a service colliding with a
        // config layer id (config wins), and a plain config layer (owned by
        // config, not carried over).
        let catalog = MemoryCatalog::default();
        let mut existing = dataset("hls-s30");
        existing.layers = vec![
            service_layer("xyz-restored", ndvi_graph()),
            service_layer(
                "xyz-broken",
                serde_json::json!({ "process_graph": {
                    "bad": { "process_id": "no_such_process", "arguments": {}, "result": true },
                }}),
            ),
            service_layer("truecolor", ndvi_graph()),
            swath_core::catalog::Layer {
                process: None,
                ..service_layer("stale-config-layer", ndvi_graph())
            },
        ];
        catalog
            .upsert_dataset(&existing)
            .await
            .expect("seed dataset");
        // Granules from earlier runs: registration must derive the
        // dataset's temporal extent from them (ADR 0015) rather than
        // re-registering the config's open placeholder interval.
        for (id, datetime) in [
            ("g-jun", "2024-06-07T19:03:00Z"),
            ("g-oct", "2024-10-15T19:03:00Z"),
            ("g-aug", "2024-08-16T19:03:00Z"),
        ] {
            let mut granule = granule_event("hls-s30").granule;
            granule.id = GranuleId::new(id);
            granule.datetime = Datetime::new(datetime).expect("valid datetime");
            catalog
                .upsert_granules(std::slice::from_ref(&granule))
                .await
                .expect("seed granule");
        }

        let shared = Shared {
            bind: cfg.bind,
            base_url: cfg.base_url,
            store_root: cfg.store_root,
            cache: cfg.cache,
            cors_allowed_origins: cfg.cors_allowed_origins,
        };
        serve_catalog_on(&shared, mode, catalog.clone(), ready(()))
            .await
            .expect("catalog mode serves and shuts down");

        let stored = catalog
            .get_dataset(&DatasetId::new("hls-s30"))
            .await
            .expect("query succeeds")
            .expect("dataset registered");
        let ids: Vec<&str> = stored.layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(
            ids,
            ["truecolor", "ndvi", "xyz-restored"],
            "config layers plus the restored service; broken/colliding/stale dropped"
        );
        assert_eq!(
            stored.extent.interval,
            TimeRange {
                start: Some(Datetime::new("2024-06-07T19:03:00Z").expect("valid datetime")),
                end: Some(Datetime::new("2024-10-15T19:03:00Z").expect("valid datetime")),
            },
            "registration derives the temporal extent from ingested granules (ADR 0015)"
        );
    }

    #[tokio::test]
    async fn serve_catalog_mode_remote_root_disables_referencing_and_store_errors_surface() {
        let drop_dir = TempDir::new("cli-catalog-remote-drop");
        let config_dir = TempDir::new("cli-catalog-remote-config");
        let config_path = config_dir.join("swath.toml");
        // `memory://` contains a scheme, so the legacy-referencing branch
        // is skipped with a warning — and the same root then fails to open
        // as an object store, surfacing as `ServeError::Store`.
        std::fs::write(
            &config_path,
            catalog_toml(std::path::Path::new("memory://tiles"), drop_dir.path()),
        )
        .expect("config writes");
        let cfg = config::resolve(&ServeArgs {
            config: Some(config_path),
            ..args()
        })
        .expect("catalog config resolves");
        let LayerSource::Catalog(mode) = cfg.layers else {
            panic!("catalog config resolves to catalog mode");
        };
        let shared = Shared {
            bind: cfg.bind,
            base_url: cfg.base_url,
            store_root: cfg.store_root,
            cache: cfg.cache,
            cors_allowed_origins: cfg.cors_allowed_origins,
        };
        let err = serve_catalog_on(&shared, mode, MemoryCatalog::default(), ready(()))
            .await
            .expect_err("pseudo-remote root cannot open");
        assert!(matches!(&err, ServeError::Store { root, .. } if root == "memory://tiles"));
    }

    /// The route-table half of the issue #103 collision AC, run against
    /// whatever bundle THIS build embedded: no embedded path may start
    /// with a segment the API routers own — such a file would be
    /// unreachable (API routes structurally outrank the UI fallback; the
    /// priority itself is pinned in swath-api's router tests). With no
    /// web/dist at build time the set is empty and the check is vacuous.
    #[cfg(feature = "embedded-ui")]
    #[test]
    fn embedded_bundle_paths_stay_off_api_routes() {
        let ui = super::embedded_ui();
        for path in ui.paths() {
            assert!(
                !swath_api::ui::collides_with_api_routes(path),
                "embedded UI file `{path}` collides with an API route prefix \
                 ({:?})",
                swath_api::ui::API_ROUTE_PREFIXES,
            );
        }
    }

    /// SIGTERM (the container stop signal) resolves the shutdown future.
    /// Unix-gated: Windows has neither SIGTERM nor `kill` — there the
    /// terminate arm is `pending()` and only Ctrl-C applies.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_signal_resolves_on_sigterm() {
        // Multi-thread flavor: the blocking sleep below must not starve
        // the spawned future of its first poll (which installs the
        // handlers — an uninstalled SIGTERM would be fatal).
        let waiter = tokio::spawn(super::shutdown_signal());
        std::thread::sleep(std::time::Duration::from_millis(300));
        let status = std::process::Command::new("kill")
            .args(["-s", "TERM", &std::process::id().to_string()])
            .status()
            .expect("kill runs");
        assert!(status.success(), "kill exits zero");
        waiter.await.expect("shutdown future resolves");
    }
}
