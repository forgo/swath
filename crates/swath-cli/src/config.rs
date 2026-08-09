// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Layered `serve` configuration: built-in defaults → optional TOML file
//! (`--config`) → environment/flags (clap's `env` attribute makes
//! `SWATH_BIND`/`SWATH_BASE_URL`/`SWATH_STORE_ROOT` and their flags one
//! surface, so both outrank the file).
//!
//! The surface is deliberately small: bind address, base URL, store root,
//! and layer definitions. Layers are file-only (or `--fixtures`) — a
//! layer is a structure, not a scalar, and encoding structures in
//! environment variables is a misfeature. The layer `kind` enum
//! (`truecolor` | `ndvi`) is the walking-skeleton stand-in the openEO
//! process compiler (issue #32) replaces with real process graphs.
//!
//! Hand-rolled layering (clap + toml + serde) over a config framework:
//! two optional scalars per field is an `or()` chain, and figment's extra
//! dependency tree is deny/supply-chain surface with no work left to do.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use swath_api::{CatalogLayer, Layer, LayerRegistry};
use swath_core::catalog as domain;
use swath_core::raster::AssetRef;
use swath_render::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::{NodataPolicy, Resampling};

use crate::serve::ServeArgs;

/// Default bind address: loopback, the compose service overrides to
/// `0.0.0.0:8080` explicitly — never listen on all interfaces by default.
const DEFAULT_BIND: &str = "127.0.0.1:8080";

/// Where `--fixtures` finds the committed HLS COGs, relative to the
/// working directory (repo root locally, `/app` in the container).
const FIXTURES_ROOT: &str = "./tests/fixtures";

/// Configuration errors, each phrased for the operator reading the log.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigError {
    /// The `--config` file could not be read.
    #[error("cannot read config file `{path}`: {source}")]
    Read {
        /// The path as given.
        path: PathBuf,
        /// The I/O failure.
        source: std::io::Error,
    },
    /// The `--config` file is not valid TOML for this schema.
    #[error("config file `{path}` is invalid: {source}")]
    Parse {
        /// The path as given.
        path: PathBuf,
        /// The TOML/serde failure.
        source: toml::de::Error,
    },
    /// Nothing to serve: no layers configured and `--fixtures` not given.
    #[error("no layers to serve: pass --fixtures, or --config with at least one [[layers]]")]
    NoLayers,
    /// A store root is required when layers come from a config file.
    #[error("store-root is required (config `store-root`, --store-root, or SWATH_STORE_ROOT)")]
    NoStoreRoot,
    /// A layer kind is missing one of its required bands.
    #[error("layer `{layer}`: kind `{kind}` requires band `{band}` in [layers.bands]")]
    MissingBand {
        /// The layer id.
        layer: String,
        /// The kind name as written in the file.
        kind: &'static str,
        /// The required band name.
        band: &'static str,
    },
    /// A layer declares bands its kind does not consume.
    #[error("layer `{layer}`: kind `{kind}` uses bands {expected:?}, but `{band}` is declared")]
    UnknownBand {
        /// The layer id.
        layer: String,
        /// The kind name as written in the file.
        kind: &'static str,
        /// The band names the kind consumes.
        expected: &'static [&'static str],
        /// The unexpected band name.
        band: String,
    },
    /// Catalog mode was requested without any datasets to serve.
    #[error("catalog mode needs at least one [[datasets]] entry (layers are defined per dataset)")]
    NoDatasets,
    /// `[[datasets]]` requires a catalog to live in.
    #[error("[[datasets]] requires catalog mode (config `catalog`, --catalog, or SWATH_CATALOG)")]
    DatasetsNeedCatalog,
    /// A watch directory without a catalog has nowhere to register granules.
    #[error("watch-dir requires catalog mode (config `catalog`, --catalog, or SWATH_CATALOG)")]
    WatchDirNeedsCatalog,
    /// Static `[[layers]]` and catalog mode are mutually exclusive.
    #[error(
        "catalog mode and static [[layers]] are mutually exclusive: define [[datasets.layers]]"
    )]
    MixedLayerSources,
    /// Two layers (across all datasets) share an id — URLs would collide.
    #[error("duplicate layer id `{layer}` across [[datasets]]")]
    DuplicateLayer {
        /// The colliding layer id.
        layer: String,
    },
    /// A dataset id appears twice.
    #[error("duplicate dataset id `{dataset}`")]
    DuplicateDataset {
        /// The colliding dataset id.
        dataset: String,
    },
}

/// The fully resolved `serve` configuration the server runs from.
pub(crate) struct ResolvedConfig {
    /// Socket address to listen on.
    pub(crate) bind: SocketAddr,
    /// Base URL minted into OGC links (and the startup log).
    pub(crate) base_url: String,
    /// Object-store root: a local directory or `s3://bucket[/prefix]`.
    pub(crate) store_root: String,
    /// Where the layers come from.
    pub(crate) layers: LayerSource,
}

/// The two serving modes: a static in-memory registry (fixtures or
/// `[[layers]]` — the walking-skeleton path, unchanged), or catalog-backed
/// resolution over pgstac (issue #31).
pub(crate) enum LayerSource {
    /// Fixtures / `[[layers]]`: assets fixed at startup.
    Static(LayerRegistry),
    /// `catalog` + `[[datasets]]`: assets resolved per tile from each
    /// dataset's latest granule; optionally with a filedrop ingest loop.
    Catalog(CatalogMode),
}

/// Everything catalog mode needs at startup.
pub(crate) struct CatalogMode {
    /// Postgres URL of the pgstac database.
    pub(crate) url: String,
    /// Drop directory to watch for granule manifests (`None` = serve-only).
    pub(crate) watch_dir: Option<PathBuf>,
    /// The datasets to register (upsert) at startup — config is the source
    /// of truth for dataset identity + serving layers (R2: operators write
    /// TOML, never STAC).
    pub(crate) datasets: Vec<domain::Dataset>,
    /// The compiled serving templates, one per `[[datasets.layers]]`.
    pub(crate) layers: Vec<CatalogLayer>,
}

/// The TOML config file schema (kebab-case keys, unknown keys rejected —
/// a typo must fail loudly, not silently fall back to a default).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ConfigFile {
    /// Socket address to listen on.
    bind: Option<SocketAddr>,
    /// Base URL minted into OGC links.
    base_url: Option<String>,
    /// Object-store root: local directory or `s3://bucket[/prefix]`.
    store_root: Option<String>,
    /// Postgres URL of a pgstac database — presence selects catalog mode.
    catalog: Option<String>,
    /// Drop directory watched for granule manifests (catalog mode only).
    watch_dir: Option<PathBuf>,
    /// Static layer definitions (mutually exclusive with catalog mode).
    #[serde(default)]
    layers: Vec<LayerConfig>,
    /// Dataset definitions (catalog mode only).
    #[serde(default)]
    datasets: Vec<DatasetConfig>,
}

/// One `[[datasets]]` entry: dataset identity plus its serving layers. The
/// dataset is upserted into the catalog at startup; granules arrive later
/// via ingest (the dataset-must-pre-exist contract,
/// `swath_core::ingest`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct DatasetConfig {
    /// Dataset identifier (the catalog collection id).
    id: String,
    /// Human-readable title; defaults to the id.
    title: Option<String>,
    /// Narrative description; defaults to empty.
    description: Option<String>,
    /// Data license (SPDX id); defaults to `other`.
    license: Option<String>,
    /// Serving layers over this dataset. Unlike static `[[layers]]`, the
    /// `bands` values name **dataset bands** (granule asset keys, e.g.
    /// `r = "b04"`), not asset URIs — assets are resolved per tile from
    /// the latest ingested granule.
    #[serde(default)]
    layers: Vec<LayerConfig>,
}

/// One `[[layers]]` entry.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct LayerConfig {
    /// URL-safe identifier — the `{layerId}` path segment.
    id: String,
    /// Human-readable title; defaults to the id.
    title: Option<String>,
    /// Tileset-metadata description; defaults to empty.
    description: Option<String>,
    /// Which pixel pipeline renders this layer.
    kind: LayerKind,
    /// Band name → asset URI (relative to the store root). `truecolor`
    /// consumes `r`,`g`,`b`; `ndvi` consumes `nir`,`red`.
    bands: BTreeMap<String, String>,
    /// Linear rescale of pipeline output to 0..255. Optional for
    /// `truecolor` (raw values clamp); `ndvi` defaults to `[-1, 1]`.
    rescale: Option<[f64; 2]>,
    /// Warp kernel; defaults to bilinear (nodata-excluding).
    #[serde(default)]
    resampling: ResamplingConfig,
    /// Tile side length in pixels; defaults to 256.
    tile_size: Option<u32>,
}

/// The built-in plan kinds (openEO compiler stand-in, see module docs).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum LayerKind {
    /// RGB composite of bands `r`,`g`,`b`.
    Truecolor,
    /// Grayscale `(nir - red) / (nir + red)` of bands `nir`,`red`.
    Ndvi,
}

impl LayerKind {
    /// The kind name as written in config (for error messages).
    fn name(self) -> &'static str {
        match self {
            Self::Truecolor => "truecolor",
            Self::Ndvi => "ndvi",
        }
    }

    /// The band names this kind consumes.
    fn bands(self) -> &'static [&'static str] {
        match self {
            Self::Truecolor => &["r", "g", "b"],
            Self::Ndvi => &["nir", "red"],
        }
    }
}

/// Config-file spelling of the warp kernel.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ResamplingConfig {
    /// Bilinear, nodata excluded and weights renormalized (the
    /// continuous-band kernel of the golden suites).
    #[default]
    Bilinear,
    /// Nearest neighbor (categorical bands).
    Nearest,
}

impl From<ResamplingConfig> for Resampling {
    fn from(value: ResamplingConfig) -> Self {
        match value {
            ResamplingConfig::Bilinear => Self::Bilinear(NodataPolicy::ExcludeRenormalize),
            ResamplingConfig::Nearest => Self::Nearest,
        }
    }
}

/// Resolves the full layering: defaults → TOML (`--config`) → env/flags
/// (already merged by clap). `--fixtures` supplies the built-in HLS demo
/// registry and defaults the store root to `./tests/fixtures` (clap
/// rejects `--fixtures --config` up front).
pub(crate) fn resolve(args: &ServeArgs) -> Result<ResolvedConfig, ConfigError> {
    let file = match &args.config {
        Some(path) => load_file(path)?,
        None => ConfigFile::default(),
    };

    let bind = args
        .bind
        .or(file.bind)
        .unwrap_or_else(|| DEFAULT_BIND.parse().expect("default bind address is valid"));
    let store_root = args
        .store_root
        .clone()
        .or(file.store_root)
        .or_else(|| args.fixtures.then(|| FIXTURES_ROOT.to_owned()))
        .ok_or(ConfigError::NoStoreRoot)?;
    let base_url = args
        .base_url
        .clone()
        .or(file.base_url)
        .unwrap_or_else(|| format!("http://localhost:{}", bind.port()));

    let catalog = args.catalog.clone().or(file.catalog);
    let watch_dir = args.watch_dir.clone().or(file.watch_dir);

    let layers = if let Some(url) = catalog {
        if !file.layers.is_empty() {
            return Err(ConfigError::MixedLayerSources);
        }
        if file.datasets.is_empty() {
            return Err(ConfigError::NoDatasets);
        }
        LayerSource::Catalog(compile_catalog_mode(url, watch_dir, &file.datasets)?)
    } else if watch_dir.is_some() {
        return Err(ConfigError::WatchDirNeedsCatalog);
    } else if !file.datasets.is_empty() {
        return Err(ConfigError::DatasetsNeedCatalog);
    } else if args.fixtures {
        LayerSource::Static(LayerRegistry::hls_fixtures())
    } else {
        if file.layers.is_empty() {
            return Err(ConfigError::NoLayers);
        }
        let layers: Vec<Layer> = file
            .layers
            .iter()
            .map(LayerConfig::to_layer)
            .collect::<Result<_, _>>()?;
        LayerSource::Static(LayerRegistry::new(layers))
    };

    Ok(ResolvedConfig {
        bind,
        base_url,
        store_root,
        layers,
    })
}

/// Compiles `[[datasets]]` into the domain datasets the server registers at
/// startup and the serving templates the catalog-backed provider resolves
/// per tile — one config, two synchronized views (the persisted
/// `swath:layers` and the compiled `RenderPlan` come from the same entry).
fn compile_catalog_mode(
    url: String,
    watch_dir: Option<PathBuf>,
    configs: &[DatasetConfig],
) -> Result<CatalogMode, ConfigError> {
    let mut datasets = Vec::with_capacity(configs.len());
    let mut layers = Vec::new();
    let mut dataset_ids = BTreeSet::new();
    let mut layer_ids = BTreeSet::new();

    for config in configs {
        if !dataset_ids.insert(config.id.clone()) {
            return Err(ConfigError::DuplicateDataset {
                dataset: config.id.clone(),
            });
        }
        let mut bands = BTreeSet::new();
        let mut domain_layers = Vec::with_capacity(config.layers.len());
        for layer in &config.layers {
            if !layer_ids.insert(layer.id.clone()) {
                return Err(ConfigError::DuplicateLayer {
                    layer: layer.id.clone(),
                });
            }
            let (template, domain_layer) = layer.to_catalog_layer(&config.id)?;
            bands.extend(template.plan.inputs.iter().map(|input| input.name.clone()));
            layers.push(template);
            domain_layers.push(domain_layer);
        }
        datasets.push(domain::Dataset {
            id: domain::DatasetId::new(config.id.clone()),
            title: config.title.clone().unwrap_or_else(|| config.id.clone()),
            description: config.description.clone().unwrap_or_default(),
            license: config.license.clone().unwrap_or_else(|| "other".to_owned()),
            // A whole-world, open-ended placeholder extent: honest for a
            // dataset whose coverage is defined by whatever granules
            // arrive. Deriving/maintaining real extents from ingested
            // granules is deliberate future work (noted, not built).
            extent: domain::Extent {
                bbox: domain::Bbox {
                    west: -180.0,
                    south: -90.0,
                    east: 180.0,
                    north: 90.0,
                },
                interval: domain::TimeRange::default(),
            },
            bands,
            layers: domain_layers,
        });
    }

    Ok(CatalogMode {
        url,
        watch_dir,
        datasets,
        layers,
    })
}

/// Reads and parses the TOML config file.
fn load_file(path: &Path) -> Result<ConfigFile, ConfigError> {
    let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_owned(),
        source,
    })?;
    toml::from_str(&raw).map_err(|source| ConfigError::Parse {
        path: path.to_owned(),
        source,
    })
}

impl LayerConfig {
    /// Compiles this entry into a servable [`Layer`], validating that the
    /// declared bands are exactly the set the kind consumes.
    fn to_layer(&self) -> Result<Layer, ConfigError> {
        let expected = self.kind.bands();
        for band in self.bands.keys() {
            if !expected.contains(&band.as_str()) {
                return Err(ConfigError::UnknownBand {
                    layer: self.id.clone(),
                    kind: self.kind.name(),
                    expected,
                    band: band.clone(),
                });
            }
        }
        let mut bands = BTreeMap::new();
        for name in expected {
            let uri = self
                .bands
                .get(*name)
                .ok_or_else(|| ConfigError::MissingBand {
                    layer: self.id.clone(),
                    kind: self.kind.name(),
                    band: name,
                })?;
            bands.insert((*name).to_owned(), AssetRef::new(uri.clone()));
        }

        let inputs = expected.iter().map(|name| BandInput::new(*name)).collect();
        let mut ops = Vec::new();
        match self.kind {
            LayerKind::Truecolor => {
                ops.push(PixelOp::Composite {
                    r: "r".into(),
                    g: "g".into(),
                    b: "b".into(),
                });
                if let Some([min, max]) = self.rescale {
                    ops.push(PixelOp::Rescale { min, max });
                }
            }
            LayerKind::Ndvi => {
                ops.push(PixelOp::BandMath(
                    (Expr::band("nir") - Expr::band("red"))
                        / (Expr::band("nir") + Expr::band("red")),
                ));
                let [min, max] = self.rescale.unwrap_or([-1.0, 1.0]);
                ops.push(PixelOp::Rescale { min, max });
                ops.push(PixelOp::Colormap(Colormap::Grayscale));
            }
        }

        Ok(Layer {
            id: self.id.clone(),
            title: self.title.clone().unwrap_or_else(|| self.id.clone()),
            description: self.description.clone().unwrap_or_default(),
            bands,
            plan: RenderPlan::new(inputs, ops, OutputSpec::new(TileFormat::Png)),
            resampling: self.resampling.into(),
            tile_size: self.tile_size.unwrap_or(256),
        })
    }

    /// Compiles a `[[datasets.layers]]` entry, where `bands` values name
    /// **dataset bands** rather than asset URIs, into the serving template
    /// (plan inputs = dataset band names, resolved against granule assets
    /// per tile) *and* the domain [`domain::Layer`] persisted on the
    /// dataset's catalog document — same entry, both views.
    ///
    /// Unlike static mode, the rescale always materializes (default
    /// `[0, 255]` for truecolor, `[-1, 1]` for ndvi): the persisted
    /// `PlanKind` + `Rescale` and the compiled pixel ops must describe the
    /// same rendering.
    fn to_catalog_layer(
        &self,
        dataset: &str,
    ) -> Result<(CatalogLayer, domain::Layer), ConfigError> {
        let expected = self.kind.bands();
        for band in self.bands.keys() {
            if !expected.contains(&band.as_str()) {
                return Err(ConfigError::UnknownBand {
                    layer: self.id.clone(),
                    kind: self.kind.name(),
                    expected,
                    band: band.clone(),
                });
            }
        }
        // Role (`r`/`g`/`b`/`nir`/`red`) -> dataset band name.
        let mut role = BTreeMap::new();
        for name in expected {
            let band = self
                .bands
                .get(*name)
                .ok_or_else(|| ConfigError::MissingBand {
                    layer: self.id.clone(),
                    kind: self.kind.name(),
                    band: name,
                })?;
            role.insert(*name, band.clone());
        }

        let inputs = expected
            .iter()
            .map(|name| BandInput::new(&role[name]))
            .collect();
        let mut ops = Vec::new();
        let (plan_kind, rescale, colormap) = match self.kind {
            LayerKind::Truecolor => {
                ops.push(PixelOp::Composite {
                    r: role["r"].clone(),
                    g: role["g"].clone(),
                    b: role["b"].clone(),
                });
                let [min, max] = self.rescale.unwrap_or([0.0, 255.0]);
                ops.push(PixelOp::Rescale { min, max });
                (
                    domain::PlanKind::Composite {
                        r: role["r"].clone(),
                        g: role["g"].clone(),
                        b: role["b"].clone(),
                    },
                    domain::Rescale { min, max },
                    None,
                )
            }
            LayerKind::Ndvi => {
                let (nir, red) = (&role["nir"], &role["red"]);
                ops.push(PixelOp::BandMath(
                    (Expr::band(nir.clone()) - Expr::band(red.clone()))
                        / (Expr::band(nir.clone()) + Expr::band(red.clone())),
                ));
                let [min, max] = self.rescale.unwrap_or([-1.0, 1.0]);
                ops.push(PixelOp::Rescale { min, max });
                ops.push(PixelOp::Colormap(Colormap::Grayscale));
                (
                    domain::PlanKind::BandMath {
                        expression: format!("({nir} - {red}) / ({nir} + {red})"),
                    },
                    domain::Rescale { min, max },
                    Some(domain::Colormap::Grayscale),
                )
            }
        };

        let title = self.title.clone().unwrap_or_else(|| self.id.clone());
        let description = self.description.clone().unwrap_or_default();
        let tile_size = self.tile_size.unwrap_or(256);
        let template = CatalogLayer {
            id: self.id.clone(),
            title: title.clone(),
            description: description.clone(),
            dataset: domain::DatasetId::new(dataset),
            plan: RenderPlan::new(inputs, ops, OutputSpec::new(TileFormat::Png)),
            resampling: self.resampling.into(),
            tile_size,
        };
        let domain_layer = domain::Layer {
            id: self.id.clone(),
            title,
            description,
            plan: plan_kind,
            rescale,
            colormap,
            resampling: match self.resampling {
                ResamplingConfig::Bilinear => domain::Resampling::Bilinear,
                ResamplingConfig::Nearest => domain::Resampling::Nearest,
            },
            tile_size,
        };
        Ok((template, domain_layer))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{ConfigError, ConfigFile, LayerSource, resolve};
    use crate::serve::ServeArgs;

    fn args() -> ServeArgs {
        ServeArgs {
            config: None,
            fixtures: false,
            bind: None,
            base_url: None,
            store_root: None,
            catalog: None,
            watch_dir: None,
        }
    }

    #[test]
    fn fixtures_mode_needs_zero_config() {
        let cfg = resolve(&ServeArgs {
            fixtures: true,
            ..args()
        })
        .expect("fixtures mode resolves");
        assert_eq!(cfg.bind.to_string(), "127.0.0.1:8080");
        assert_eq!(cfg.base_url, "http://localhost:8080");
        assert_eq!(cfg.store_root, "./tests/fixtures");
        assert!(
            matches!(&cfg.layers, LayerSource::Static(registry) if registry.iter().count() == 2)
        );
    }

    #[test]
    fn without_fixtures_layers_and_store_root_are_required() {
        assert!(matches!(resolve(&args()), Err(ConfigError::NoStoreRoot)));
        let with_root = ServeArgs {
            store_root: Some("/data".to_owned()),
            ..args()
        };
        assert!(matches!(resolve(&with_root), Err(ConfigError::NoLayers)));
    }

    #[test]
    fn flags_override_file_scalars_and_base_url_follows_bind() {
        let file: ConfigFile = toml::from_str(r#"bind = "0.0.0.0:9999""#).expect("parses");
        assert_eq!(file.bind.unwrap().port(), 9999);
        // Flag wins over the file value; derived base-url follows the
        // winning bind port.
        let cfg = resolve(&ServeArgs {
            fixtures: true,
            bind: Some("127.0.0.1:7070".parse().expect("addr")),
            ..args()
        })
        .expect("resolves");
        assert_eq!(cfg.bind.port(), 7070);
        assert_eq!(cfg.base_url, "http://localhost:7070");
    }

    #[test]
    fn truecolor_layer_compiles_and_band_typos_fail_loudly() {
        let good = r#"
            store-root = "/data"
            [[layers]]
            id = "tc"
            kind = "truecolor"
            rescale = [0.0, 3000.0]
            [layers.bands]
            r = "b04.tif"
            g = "b03.tif"
            b = "b02.tif"
        "#;
        let file: ConfigFile = toml::from_str(good).expect("parses");
        let layer = file.layers[0].to_layer().expect("compiles");
        assert_eq!(layer.id, "tc");
        assert_eq!(layer.title, "tc");
        assert_eq!(layer.tile_size, 256);
        assert_eq!(layer.bands.len(), 3);

        let missing = r#"
            [[layers]]
            id = "tc"
            kind = "truecolor"
            [layers.bands]
            r = "b04.tif"
        "#;
        let file: ConfigFile = toml::from_str(missing).expect("parses");
        assert!(matches!(
            file.layers[0].to_layer(),
            Err(ConfigError::MissingBand { band: "g", .. })
        ));

        let extra = r#"
            [[layers]]
            id = "veg"
            kind = "ndvi"
            [layers.bands]
            nir = "b8a.tif"
            red = "b04.tif"
            blue = "b02.tif"
        "#;
        let file: ConfigFile = toml::from_str(extra).expect("parses");
        assert!(matches!(
            file.layers[0].to_layer(),
            Err(ConfigError::UnknownBand { .. })
        ));
    }

    #[test]
    fn unknown_keys_are_rejected_not_defaulted() {
        assert!(toml::from_str::<ConfigFile>("bindd = \"127.0.0.1:1\"").is_err());
    }

    /// The catalog-mode config of the compose stack, in miniature.
    const CATALOG_TOML: &str = r#"
        store-root = "/data"
        catalog = "postgres://swath@localhost/swath"
        watch-dir = "/data/drop"

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
    "#;

    #[test]
    fn catalog_mode_compiles_datasets_and_serving_templates() {
        let file: ConfigFile = toml::from_str(CATALOG_TOML).expect("parses");
        let mode = super::compile_catalog_mode(
            file.catalog.clone().unwrap(),
            file.watch_dir.clone(),
            &file.datasets,
        )
        .expect("compiles");

        // The domain dataset: band vocabulary is the union of layer bands,
        // layers persist as PlanKind over dataset band names.
        assert_eq!(mode.datasets.len(), 1);
        let dataset = &mode.datasets[0];
        assert_eq!(dataset.id.as_str(), "hls-s30");
        assert_eq!(dataset.license, "CC0-1.0");
        let bands: Vec<&str> = dataset.bands.iter().map(String::as_str).collect();
        assert_eq!(bands, ["b02", "b03", "b04", "b8a"]);
        assert_eq!(dataset.layers.len(), 2);
        assert!(matches!(
            &dataset.layers[0].plan,
            super::domain::PlanKind::Composite { r, g, b }
                if r == "b04" && g == "b03" && b == "b02"
        ));
        assert!(matches!(
            &dataset.layers[1].plan,
            super::domain::PlanKind::BandMath { expression }
                if expression == "(b8a - b04) / (b8a + b04)"
        ));

        // The serving templates: plan inputs name dataset bands, so
        // resolution maps granule assets key-for-key.
        assert_eq!(mode.layers.len(), 2);
        let truecolor = &mode.layers[0];
        assert_eq!(truecolor.id, "truecolor");
        assert_eq!(truecolor.dataset.as_str(), "hls-s30");
        let inputs: Vec<&str> = truecolor
            .plan
            .inputs
            .iter()
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(inputs, ["b04", "b03", "b02"]);
        assert_eq!(mode.watch_dir.as_deref(), Some(Path::new("/data/drop")));
    }

    #[test]
    fn catalog_mode_validation_fails_loudly() {
        // Catalog without datasets.
        let with_catalog = ServeArgs {
            catalog: Some("postgres://x".to_owned()),
            store_root: Some("/data".to_owned()),
            ..args()
        };
        assert!(matches!(
            resolve(&with_catalog),
            Err(ConfigError::NoDatasets)
        ));

        // Watch dir without catalog.
        let watch_only = ServeArgs {
            watch_dir: Some(PathBuf::from("/drop")),
            store_root: Some("/data".to_owned()),
            ..args()
        };
        assert!(matches!(
            resolve(&watch_only),
            Err(ConfigError::WatchDirNeedsCatalog)
        ));

        // Duplicate layer ids across datasets collide in URL space.
        let mut file: ConfigFile = toml::from_str(CATALOG_TOML).expect("parses");
        let mut clash: ConfigFile = toml::from_str(CATALOG_TOML).expect("parses");
        let mut second = clash.datasets.pop().unwrap();
        second.id = "hls-l30".to_owned();
        file.datasets.push(second);
        assert!(matches!(
            super::compile_catalog_mode("postgres://x".to_owned(), None, &file.datasets),
            Err(ConfigError::DuplicateLayer { layer }) if layer == "truecolor"
        ));
    }
}
