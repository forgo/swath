// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `swath materialize`: batch pyramid materialization (issue #183 — the
//! "optional overview warm" step of ARCHITECTURE.md §8, closing the
//! overview-generation deferral in `docs/ROADMAP.md`).
//!
//! Resolves the same configuration `swath serve` reads, resolves each
//! layer's backing assets exactly as the serve path would (static layers
//! directly; catalog layers from the dataset's latest ingested granule),
//! and materializes the missing overview ladder for every distinct asset
//! into `pyramids/` under the store root
//! (`swath-pyramid-objectstore`; layout documented in that crate). The
//! aggregation follows the layer's resampling: `nearest` layers get
//! nearest pyramids (categorical/QA data), everything else averages.
//!
//! Idempotent and resumable by construction (the adapter probes before
//! writing); rerunning after new granules arrive materializes only what
//! is missing. The serve path picks the new levels up on its next
//! `describe` — no restart required.

use std::collections::BTreeMap;
use std::path::PathBuf;

use swath_api::{CatalogLayers, LayerProvider as _, LayerRegistry};
use swath_catalog_pgstac::PgstacCatalog;
use swath_core::catalog::CatalogError;
use swath_core::raster::AssetRef;
use swath_pyramid_objectstore::{
    MaterializeError, MaterializeSpec, PyramidResampling, PyramidSource,
};
use swath_render::Resampling;

use crate::config::{self, LayerSource, ResolvedConfig};
use crate::serve::{ServeArgs, ServeError, build_store};
use crate::source::CompositeSource;

/// `swath materialize` arguments: the config surface is `swath serve`'s
/// (same file, same store-root grammar); `--layer` narrows the run.
#[derive(Debug, clap::Args)]
pub(crate) struct MaterializeArgs {
    /// TOML config file (the same file `swath serve` reads).
    #[arg(long, value_name = "PATH")]
    pub(crate) config: Option<PathBuf>,

    /// Object-store root: a local directory or `s3://bucket[/prefix]`
    /// (S3 credentials/endpoint via the standard AWS_* environment).
    /// Overrides the config file's `store-root`.
    #[arg(long, value_name = "ROOT")]
    pub(crate) store_root: Option<String>,

    /// Materialize only this layer's assets (default: every layer).
    #[arg(long, value_name = "ID")]
    pub(crate) layer: Option<String>,

    /// Coarsest-level bound: the ladder stops at the first level whose
    /// larger axis fits this many pixels (default 256, GDAL's own
    /// overview-build default).
    #[arg(long, value_name = "PIXELS")]
    pub(crate) min_dim: Option<u32>,
}

/// Materialize-path errors, each phrased for the operator reading the log.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MaterializeCliError {
    /// Configuration resolution failed.
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    /// The object store or runtime could not be built.
    #[error(transparent)]
    Serve(#[from] ServeError),
    /// The pgstac catalog could not be reached or queried.
    #[error("catalog: {0}")]
    Catalog(#[from] CatalogError),
    /// A layer's assets could not be resolved from the catalog.
    #[error("cannot resolve layer `{layer}`: {detail}")]
    Resolve {
        /// The layer being resolved.
        layer: String,
        /// Why.
        detail: String,
    },
    /// `--layer` named a layer the config does not define.
    #[error("no layer `{layer}` in the configuration")]
    NoSuchLayer {
        /// The unmatched id.
        layer: String,
    },
    /// Two layers back the same asset with different resampling — the
    /// pyramid cannot honor both; make the layers agree (or run per
    /// `--layer`).
    #[error(
        "asset `{asset}` is used with both average and nearest resampling; \
         a pyramid stores one aggregation — align the layers or run --layer"
    )]
    ResamplingConflict {
        /// The contested asset.
        asset: AssetRef,
    },
    /// The pyramid writer refused or failed.
    #[error("materialize {asset}: {source}")]
    Materialize {
        /// The asset being materialized.
        asset: AssetRef,
        /// The underlying failure.
        #[source]
        source: MaterializeError,
    },
}

/// Resolves config, collects each layer's assets, and materializes them.
pub(crate) fn run(args: &MaterializeArgs) -> Result<(), MaterializeCliError> {
    let cfg = config::resolve(&ServeArgs {
        config: args.config.clone(),
        store_root: args.store_root.clone(),
        fixtures: false,
        bind: None,
        base_url: None,
        catalog: None,
        watch_dir: None,
        cache: None,
        overview_oversample: None,
        max_estimated_live_bytes: None,
        cors_allowed_origins: Vec::new(),
    })?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(ServeError::Serve)?;
    runtime.block_on(materialize_all(cfg, args.layer.as_deref(), args.min_dim))
}

/// The async body: resolve every (asset, resampling) pair, then run the
/// batch writer per asset, logging each report.
async fn materialize_all(
    cfg: ResolvedConfig,
    only_layer: Option<&str>,
    min_dim: Option<u32>,
) -> Result<(), MaterializeCliError> {
    let store = build_store(&cfg.store_root)?;
    let source = PyramidSource::new(
        CompositeSource::new(std::sync::Arc::clone(&store)),
        std::sync::Arc::clone(&store),
    );

    let assets = match cfg.layers {
        LayerSource::Static(registry) => static_assets(&registry, only_layer)?,
        LayerSource::Catalog(mode) => {
            let catalog = PgstacCatalog::connect(&mode.url).await?;
            catalog_assets(CatalogLayers::new(catalog, mode.layers), only_layer).await?
        }
    };
    tracing::info!(
        "materializing pyramids for {count} asset(s) under {root}",
        count = assets.len(),
        root = cfg.store_root,
    );

    for (asset, resampling) in assets {
        let mut spec = MaterializeSpec::with_resampling(resampling);
        if let Some(min_dim) = min_dim {
            spec.min_dim = min_dim;
        }
        let report = source.materialize(&asset, &spec).await.map_err(|source| {
            MaterializeCliError::Materialize {
                asset: asset.clone(),
                source,
            }
        })?;
        if report.factors_completed.is_empty() && report.chunks_written == 0 {
            tracing::info!(
                "{asset}: up to date ({complete} level(s) already complete)",
                complete = report.factors_already_complete.len(),
            );
        } else {
            tracing::info!(
                "{asset}: completed levels {completed:?} ({written} chunk(s) written, \
                 {skipped} skipped) at {root}",
                completed = report.factors_completed,
                written = report.chunks_written,
                skipped = report.chunks_skipped,
                root = report.root,
            );
        }
    }
    Ok(())
}

/// The distinct (asset, resampling) pairs of the static registry.
fn static_assets(
    registry: &LayerRegistry,
    only_layer: Option<&str>,
) -> Result<Vec<(AssetRef, PyramidResampling)>, MaterializeCliError> {
    let mut layers: Vec<_> = registry.iter().collect();
    if let Some(id) = only_layer {
        layers.retain(|layer| layer.id == id);
        if layers.is_empty() {
            return Err(MaterializeCliError::NoSuchLayer {
                layer: id.to_owned(),
            });
        }
    }
    collect_assets(
        layers
            .into_iter()
            .flat_map(|layer| layer.bands.values().map(|a| (a.clone(), layer.resampling))),
    )
}

/// The distinct (asset, resampling) pairs of the catalog templates,
/// resolved from each dataset's latest granule (exactly the serve path's
/// resolution).
async fn catalog_assets<C>(
    provider: CatalogLayers<C>,
    only_layer: Option<&str>,
) -> Result<Vec<(AssetRef, PyramidResampling)>, MaterializeCliError>
where
    C: swath_core::catalog::Catalog + Clone + Send + Sync + 'static,
{
    let mut identities = provider.identities();
    if let Some(id) = only_layer {
        identities.retain(|identity| identity.id == id);
        if identities.is_empty() {
            return Err(MaterializeCliError::NoSuchLayer {
                layer: id.to_owned(),
            });
        }
    }
    let mut pairs = Vec::new();
    for identity in identities {
        let resolved =
            provider
                .resolve(&identity.id)
                .await
                .map_err(|err| MaterializeCliError::Resolve {
                    layer: identity.id.clone(),
                    detail: err.to_string(),
                })?;
        for asset in resolved.layer.bands.values() {
            pairs.push((asset.clone(), resolved.layer.resampling));
        }
    }
    collect_assets(pairs.into_iter())
}

/// Dedups (asset, resampling) pairs, refusing cross-layer disagreement.
fn collect_assets(
    pairs: impl Iterator<Item = (AssetRef, Resampling)>,
) -> Result<Vec<(AssetRef, PyramidResampling)>, MaterializeCliError> {
    let mut assets: BTreeMap<String, (AssetRef, PyramidResampling)> = BTreeMap::new();
    for (asset, resampling) in pairs {
        let resampling = match resampling {
            Resampling::Nearest => PyramidResampling::Nearest,
            _ => PyramidResampling::Average,
        };
        match assets.get(asset.as_str()) {
            Some((_, existing)) if *existing != resampling => {
                return Err(MaterializeCliError::ResamplingConflict { asset });
            }
            Some(_) => {}
            None => {
                assets.insert(asset.as_str().to_owned(), (asset, resampling));
            }
        }
    }
    Ok(assets.into_values().collect())
}

#[cfg(test)]
mod tests {
    use swath_testsupport::TempDir;

    use super::{MaterializeArgs, MaterializeCliError, run};

    fn fixtures_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
    }

    /// A static NDVI config over a temp store preloaded with the two
    /// committed HLS fixture bands the layer needs.
    fn config_over_temp_store() -> (TempDir, TempDir, std::path::PathBuf) {
        let store = TempDir::new("cli-materialize-store");
        for name in [
            "hlss30-t13sdd-2024158-b04.tif",
            "hlss30-t13sdd-2024158-b8a.tif",
        ] {
            std::fs::copy(fixtures_dir().join(name), store.join(name)).expect("fixture copies");
        }
        let config_dir = TempDir::new("cli-materialize-config");
        let config_path = config_dir.join("swath.toml");
        std::fs::write(
            &config_path,
            format!(
                r"
                store-root = '{store}'

                [[layers]]
                id = 'ndvi'
                kind = 'ndvi'
                [layers.bands]
                nir = 'hlss30-t13sdd-2024158-b8a.tif'
                red = 'hlss30-t13sdd-2024158-b04.tif'
                ",
                store = store.path().display(),
            ),
        )
        .expect("config writes");
        // The dirs must outlive the run; the caller holds them.
        (store, config_dir, config_path)
    }

    fn args(config: std::path::PathBuf, layer: Option<&str>) -> MaterializeArgs {
        MaterializeArgs {
            config: Some(config),
            store_root: None,
            layer: layer.map(str::to_owned),
            // The fixtures are 512 px with an embedded x2 overview; a
            // 64-px bound forces real levels (x4, x8) to materialize.
            min_dim: Some(64),
        }
    }

    /// The subcommand materializes every layer asset into `pyramids/`
    /// under the store root, and a rerun is a clean no-op (idempotence at
    /// the CLI surface; the adapter's own tests pin the byte-level
    /// behavior).
    #[test]
    fn materializes_static_layers_and_reruns_cleanly() {
        let (store, _config_dir, config_path) = config_over_temp_store();
        run(&args(config_path.clone(), None)).expect("materializes");
        let pyramids = store.join("pyramids");
        assert!(pyramids.is_dir(), "pyramids/ created under the store root");
        let group_docs: Vec<_> = walk(&pyramids)
            .into_iter()
            .filter(|p| p.file_name().is_some_and(|n| n == ".zattrs"))
            .collect();
        assert_eq!(group_docs.len(), 2, "one pyramid per distinct asset");

        run(&args(config_path, None)).expect("rerun is a no-op");
    }

    /// `--layer` narrows the run and refuses unknown ids loudly.
    #[test]
    fn layer_filter_selects_and_refuses() {
        let (store, _config_dir, config_path) = config_over_temp_store();
        run(&args(config_path.clone(), Some("ndvi"))).expect("filtered run");
        assert!(store.join("pyramids").is_dir());

        let err = run(&args(config_path, Some("nope"))).expect_err("unknown layer");
        assert!(matches!(err, MaterializeCliError::NoSuchLayer { layer } if layer == "nope"));
    }

    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir).expect("readable dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                files.extend(walk(&path));
            } else {
                files.push(path);
            }
        }
        files
    }
}
