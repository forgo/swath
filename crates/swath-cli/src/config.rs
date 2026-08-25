// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Layered `serve` configuration: built-in defaults → optional TOML file
//! (`--config`) → environment/flags (clap's `env` attribute makes
//! `SWATH_BIND`/`SWATH_BASE_URL`/`SWATH_STORE_ROOT`/`SWATH_CACHE` and
//! their flags one surface, so both outrank the file).
//!
//! The surface is deliberately small: bind address, base URL, store root,
//! optional tile-cache root (#36), optional materialization budgets
//! (#37: a global `[budget]` default — its scalar knobs also reachable as
//! `--overview-oversample`/`SWATH_OVERVIEW_OVERSAMPLE`,
//! `--max-estimated-live-bytes`/`SWATH_MAX_ESTIMATED_LIVE_BYTES`, and
//! `--max-udf-fuel-per-tile`/`SWATH_MAX_UDF_FUEL_PER_TILE` (#205) — with
//! per-layer `[layers.budget]` overrides; see [`BudgetConfig`]), and
//! layer definitions. Layers are file-only (or `--fixtures`) — a
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
use swath_core::planner::Budget;
use swath_core::raster::AssetRef;
use swath_render::ir::Colormap;
use swath_render::{NodataPolicy, PlanSpec, Resampling, ndvi_expr, plan_for};

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
    /// A colormap on a layer kind that renders RGB directly.
    #[error(
        "layer `{layer}`: kind `{kind}` renders RGB directly; `colormap` applies only to `ndvi`"
    )]
    ColormapNotApplicable {
        /// The layer id.
        layer: String,
        /// The kind name as written in the file.
        kind: &'static str,
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
    /// Tile-cache root (`--cache`/`SWATH_CACHE`/`cache`, issue #36):
    /// local directory or `s3://bucket[/prefix]`. `None` = no cache —
    /// serving is byte-for-byte the pre-cache behavior.
    pub(crate) cache: Option<String>,
    /// `run_udf` module-store root (`--udf-store`/`SWATH_UDF_STORE`/
    /// `udf-store`, ADR 0018 / #204): local directory or
    /// `s3://bucket[/prefix]`, where published WASM modules persist by
    /// content hash. `None` = `run_udf` is not offered.
    pub(crate) udf_store: Option<String>,
    /// CORS origin allowlist (issue #103, ADR 0011): exact origins, or
    /// `*` for any. Empty (the default) = no CORS layer at all — the
    /// same-origin story (embedded UI / vite proxy) needs none.
    pub(crate) cors_allowed_origins: Vec<String>,
    /// Read-only serving (#198): write routes unmounted.
    pub(crate) read_only: bool,
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
pub(crate) struct ConfigFile {
    /// Socket address to listen on.
    bind: Option<SocketAddr>,
    /// Base URL minted into OGC links.
    base_url: Option<String>,
    /// Object-store root: local directory or `s3://bucket[/prefix]`.
    store_root: Option<String>,
    /// Tile-cache root: local directory or `s3://bucket[/prefix]`.
    cache: Option<String>,
    /// `run_udf` module-store root: local directory or
    /// `s3://bucket[/prefix]` (absent: `run_udf` not offered).
    udf_store: Option<String>,
    /// Postgres URL of a pgstac database — presence selects catalog mode.
    catalog: Option<String>,
    /// Drop directory watched for granule manifests (catalog mode only).
    watch_dir: Option<PathBuf>,
    /// CORS origin allowlist (issue #103); `["*"]` = any origin.
    cors_allowed_origins: Option<Vec<String>>,
    /// Global default materialization budget (issue #37); per-layer
    /// `[layers.budget]` values override it knob by knob.
    budget: Option<BudgetConfig>,
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
    /// Colormap applied to the gray result — `ndvi` only (`truecolor`
    /// renders RGB directly); `ndvi` defaults to `rdylgn` (issue #94).
    colormap: Option<ColormapConfig>,
    /// Warp kernel; defaults to bilinear (nodata-excluding).
    #[serde(default)]
    resampling: ResamplingConfig,
    /// Tile side length in pixels; defaults to 256.
    tile_size: Option<u32>,
    /// This layer's materialization budget (issue #37): knobs given here
    /// override the resolved global default knob by knob.
    budget: Option<BudgetConfig>,
}

/// The materialization-budget knobs as config spells them (issue #37,
/// `docs/design/materialization-planner.md` §1). Every knob is optional
/// at every level; resolution is knob-by-knob with per-layer values
/// outranking the global default (which is built-in defaults → top-level
/// `[budget]` → `--overview-oversample`/`--max-estimated-live-bytes`
/// flags or their `SWATH_*` variables). Env/flags are inherently global —
/// budgets are per-layer structures, and structures don't belong in
/// environment variables (module docs) — so an explicit `[layers.budget]`
/// value, being more specific, wins over them.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct BudgetConfig {
    /// Consult/fill the tile cache for this layer (default true; only
    /// effective when a cache is configured at all).
    cache_enabled: Option<bool>,
    /// Overview eligibility slack (default 1.2, GDAL's rule).
    overview_oversample: Option<f64>,
    /// Refuse live renders estimated over this many bytes (default:
    /// never refuse). Per-layer values can set or tighten the ceiling,
    /// not clear a global one (set a huge value to effectively disable).
    max_estimated_live_bytes: Option<u64>,
    /// Deterministic fuel a `run_udf` stage may consume per tile
    /// (ADR 0018, #205; default 100 M — the planner crate's documented
    /// calibration point). Only layers with a UDF stage spend any.
    max_udf_fuel_per_tile: Option<u64>,
}

impl BudgetConfig {
    /// `base` with this config's explicit knobs applied on top.
    fn overlay(self, base: &Budget) -> Budget {
        Budget {
            cache_enabled: self.cache_enabled.unwrap_or(base.cache_enabled),
            overview_oversample: self.overview_oversample.unwrap_or(base.overview_oversample),
            max_estimated_live_bytes: self
                .max_estimated_live_bytes
                .or(base.max_estimated_live_bytes),
            max_udf_fuel_per_tile: self
                .max_udf_fuel_per_tile
                .unwrap_or(base.max_udf_fuel_per_tile),
        }
    }
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

/// Config-file spelling of the colormap applied to gray (ndvi) output.
/// The names match the persisted catalog vocabulary and the openEO
/// `save_result` colormap option.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ColormapConfig {
    /// The identity map: gray in, gray out.
    Grayscale,
    /// Matplotlib's perceptually uniform sequential `viridis`.
    Viridis,
    /// Matplotlib's perceptually uniform sequential `magma`.
    Magma,
    /// The `ColorBrewer` diverging red–yellow–green map — the NDVI default.
    Rdylgn,
}

impl ColormapConfig {
    /// The Render IR variant this spelling selects.
    fn to_ir(self) -> Colormap {
        match self {
            Self::Grayscale => Colormap::Grayscale,
            Self::Viridis => Colormap::Viridis,
            Self::Magma => Colormap::Magma,
            Self::Rdylgn => Colormap::RdYlGn,
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

    let cache = args.cache.clone().or(file.cache);
    let udf_store = args.udf_store.clone().or(file.udf_store);
    let catalog = args.catalog.clone().or(file.catalog);
    let watch_dir = args.watch_dir.clone().or(file.watch_dir);
    // Flag/env (a non-empty list) outranks the file, like every scalar;
    // absent everywhere resolves to an empty list — CORS off (ADR 0011).
    let cors_allowed_origins = if args.cors_allowed_origins.is_empty() {
        file.cors_allowed_origins.unwrap_or_default()
    } else {
        args.cors_allowed_origins.clone()
    };

    // The resolved global default budget (#37): built-in defaults →
    // top-level [budget] → flags/env. Per-layer [layers.budget] overlays
    // this knob by knob (BudgetConfig docs carry the precedence story).
    let mut default_budget = file
        .budget
        .map_or_else(Budget::default, |b| b.overlay(&Budget::default()));
    if let Some(oversample) = args.overview_oversample {
        default_budget.overview_oversample = oversample;
    }
    if let Some(limit) = args.max_estimated_live_bytes {
        default_budget.max_estimated_live_bytes = Some(limit);
    }
    if let Some(fuel) = args.max_udf_fuel_per_tile {
        default_budget.max_udf_fuel_per_tile = fuel;
    }

    let layers = if let Some(url) = catalog {
        if !file.layers.is_empty() {
            return Err(ConfigError::MixedLayerSources);
        }
        if file.datasets.is_empty() {
            return Err(ConfigError::NoDatasets);
        }
        LayerSource::Catalog(compile_catalog_mode(
            url,
            watch_dir,
            &file.datasets,
            &default_budget,
        )?)
    } else if watch_dir.is_some() {
        return Err(ConfigError::WatchDirNeedsCatalog);
    } else if !file.datasets.is_empty() {
        return Err(ConfigError::DatasetsNeedCatalog);
    } else if args.fixtures {
        // Fixture layers ship default budgets; the resolved global
        // default (flags/env) still applies to them.
        let mut layers: Vec<Layer> = LayerRegistry::hls_fixtures().iter().cloned().collect();
        for layer in &mut layers {
            layer.budget = default_budget.clone();
        }
        LayerSource::Static(LayerRegistry::new(layers))
    } else {
        if file.layers.is_empty() {
            return Err(ConfigError::NoLayers);
        }
        let layers: Vec<Layer> = file
            .layers
            .iter()
            .map(|layer| layer.to_layer(&default_budget))
            .collect::<Result<_, _>>()?;
        LayerSource::Static(LayerRegistry::new(layers))
    };

    Ok(ResolvedConfig {
        bind,
        base_url,
        store_root,
        cache,
        udf_store,
        cors_allowed_origins,
        read_only: args.read_only,
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
    default_budget: &Budget,
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
            let (template, domain_layer) = layer.to_catalog_layer(&config.id, default_budget)?;
            bands.extend(template.plan.inputs.iter().map(|input| input.name.clone()));
            layers.push(template);
            domain_layers.push(domain_layer);
        }
        datasets.push(domain::Dataset {
            id: domain::DatasetId::new(config.id.clone()),
            title: config.title.clone().unwrap_or_else(|| config.id.clone()),
            description: config.description.clone().unwrap_or_default(),
            license: config.license.clone().unwrap_or_else(|| "other".to_owned()),
            // The compiled extent is a starting point, not the served
            // truth. Temporal: an open interval meaning "no granule
            // recorded yet" — serve registration re-derives it from
            // ingested granules (min/max acquisition datetime) and each
            // ingest widens it (ADR 0015; the temporal half of ROADMAP
            // deferral row 15). Spatial: a whole-world placeholder,
            // honest for a dataset whose coverage is whatever granules
            // arrive — deriving real spatial extents stays deferred
            // (docs/ROADMAP.md row 15, Records trigger).
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
    /// The effective colormap of a gray (`ndvi`) layer: the configured
    /// map, defaulting to the diverging `RdYlGn` (issue #94).
    fn colormap(&self) -> ColormapConfig {
        self.colormap.unwrap_or(ColormapConfig::Rdylgn)
    }

    /// Rejects an explicit `colormap` on kinds that render RGB directly.
    fn reject_colormap(&self) -> Result<(), ConfigError> {
        match self.colormap {
            None => Ok(()),
            Some(_) => Err(ConfigError::ColormapNotApplicable {
                layer: self.id.clone(),
                kind: self.kind.name(),
            }),
        }
    }

    /// Role (`r`/`g`/`b` or `nir`/`red`) → the configured `[layers.bands]`
    /// value, validating the declared set is exactly what the kind
    /// consumes.
    fn role_bands(&self) -> Result<BTreeMap<&'static str, &str>, ConfigError> {
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
        let mut roles = BTreeMap::new();
        for name in expected {
            let value = self
                .bands
                .get(*name)
                .ok_or_else(|| ConfigError::MissingBand {
                    layer: self.id.clone(),
                    kind: self.kind.name(),
                    band: name,
                })?;
            roles.insert(*name, value.as_str());
        }
        Ok(roles)
    }

    /// This entry's [`PlanSpec`] — the one plan-kind vocabulary
    /// [`plan_for`] lowers ([issue #95]) — with each band role resolved
    /// through `band` (identity in static mode, role → dataset band in
    /// catalog mode). `materialize_rescale`: catalog mode always writes
    /// the truecolor rescale (the persisted record and the compiled ops
    /// must describe the same rendering, default `[0, 255]`); static mode
    /// omits it when unset (raw values clamp at quantization).
    ///
    /// [issue #95]: https://github.com/forgo/swath/issues/95
    fn plan_spec(
        &self,
        band: impl Fn(&'static str) -> String,
        materialize_rescale: bool,
    ) -> Result<PlanSpec, ConfigError> {
        Ok(match self.kind {
            LayerKind::Truecolor => {
                self.reject_colormap()?;
                let rescale = self
                    .rescale
                    .map(|[min, max]| (min, max))
                    .or_else(|| materialize_rescale.then_some((0.0, 255.0)));
                PlanSpec::Composite {
                    r: band("r"),
                    g: band("g"),
                    b: band("b"),
                    rescale,
                }
            }
            LayerKind::Ndvi => {
                let [min, max] = self.rescale.unwrap_or([-1.0, 1.0]);
                PlanSpec::BandMath {
                    expr: ndvi_expr(band("nir"), band("red")),
                    rescale: Some((min, max)),
                    colormap: self.colormap().to_ir(),
                }
            }
        })
    }

    /// Compiles this entry into a servable [`Layer`]: plan inputs are the
    /// role names themselves, and each role's configured value is the
    /// asset URI backing it. `default_budget` is the resolved global
    /// default the entry's own `[layers.budget]` overlays.
    fn to_layer(&self, default_budget: &Budget) -> Result<Layer, ConfigError> {
        let roles = self.role_bands()?;
        let spec = self.plan_spec(str::to_owned, false)?;
        let bands = roles
            .iter()
            .map(|(role, uri)| ((*role).to_owned(), AssetRef::new((*uri).to_owned())))
            .collect();
        Ok(Layer {
            id: self.id.clone(),
            title: self.title.clone().unwrap_or_else(|| self.id.clone()),
            description: self.description.clone().unwrap_or_default(),
            bands,
            plan: plan_for(&spec).0,
            resampling: self.resampling.into(),
            tile_size: self.tile_size.unwrap_or(256),
            budget: self.budget.map_or_else(
                || default_budget.clone(),
                |budget| budget.overlay(default_budget),
            ),
        })
    }

    /// Compiles a `[[datasets.layers]]` entry, where `bands` values name
    /// **dataset bands** rather than asset URIs, into the serving template
    /// (plan inputs = dataset band names, resolved against granule assets
    /// per tile) *and* the domain [`domain::Layer`] persisted on the
    /// dataset's catalog document — same entry, both views, one
    /// [`plan_for`] call producing both so they cannot disagree.
    fn to_catalog_layer(
        &self,
        dataset: &str,
        default_budget: &Budget,
    ) -> Result<(CatalogLayer, domain::Layer), ConfigError> {
        let roles = self.role_bands()?;
        let spec = self.plan_spec(|role| roles[role].to_owned(), true)?;
        let (plan, meta) = plan_for(&spec);

        let title = self.title.clone().unwrap_or_else(|| self.id.clone());
        let description = self.description.clone().unwrap_or_default();
        let tile_size = self.tile_size.unwrap_or(256);
        let template = CatalogLayer {
            id: self.id.clone(),
            title: title.clone(),
            description: description.clone(),
            dataset: domain::DatasetId::new(dataset),
            plan,
            resampling: self.resampling.into(),
            tile_size,
            budget: self.budget.map_or_else(
                || default_budget.clone(),
                |budget| budget.overlay(default_budget),
            ),
            // Config-defined layers are temporally unconstrained: latest
            // wins, exactly as before ADR 0015.
            window: swath_core::catalog::TimeRange::default(),
        };
        let domain_layer = domain::Layer {
            id: self.id.clone(),
            title,
            description,
            plan: meta.kind,
            rescale: meta.rescale,
            colormap: meta.colormap,
            resampling: match self.resampling {
                ResamplingConfig::Bilinear => domain::Resampling::Bilinear,
                ResamplingConfig::Nearest => domain::Resampling::Nearest,
            },
            tile_size,
            // Operator-config layers carry no openEO process record; only
            // the services surface (ADR 0010) authors layers with one.
            process: None,
        };
        Ok((template, domain_layer))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use swath_core::planner::Budget;

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
            cache: None,
            udf_store: None,
            overview_oversample: None,
            max_estimated_live_bytes: None,
            max_udf_fuel_per_tile: None,
            cors_allowed_origins: Vec::new(),
            read_only: false,
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

    /// The tile-cache root (#36) layers exactly like the other scalars:
    /// absent everywhere = None (no cache), file value used, flag/env
    /// outranks the file.
    #[test]
    fn cache_root_layers_and_defaults_to_none() {
        let cfg = resolve(&ServeArgs {
            fixtures: true,
            ..args()
        })
        .expect("resolves");
        assert!(cfg.cache.is_none(), "no cache unless configured");

        let file: ConfigFile = toml::from_str(r#"cache = "/var/cache/swath""#).expect("parses");
        assert_eq!(file.cache.as_deref(), Some("/var/cache/swath"));

        let cfg = resolve(&ServeArgs {
            fixtures: true,
            cache: Some("s3://tiles/cache".to_owned()),
            ..args()
        })
        .expect("resolves");
        assert_eq!(cfg.cache.as_deref(), Some("s3://tiles/cache"));
    }

    /// The `run_udf` module-store root (ADR 0018, #204) layers like the
    /// cache and defaults to absent — `run_udf` is not offered until an
    /// operator names where modules persist.
    #[test]
    fn udf_store_layers_like_the_cache_and_defaults_to_absent() {
        let cfg = resolve(&ServeArgs {
            fixtures: true,
            ..args()
        })
        .expect("resolves");
        assert!(cfg.udf_store.is_none(), "no module store unless configured");

        let file: ConfigFile =
            toml::from_str(r#"udf-store = "/var/lib/swath/udf""#).expect("parses");
        assert_eq!(file.udf_store.as_deref(), Some("/var/lib/swath/udf"));

        let cfg = resolve(&ServeArgs {
            fixtures: true,
            udf_store: Some("s3://tiles/udf".to_owned()),
            ..args()
        })
        .expect("resolves");
        assert_eq!(cfg.udf_store.as_deref(), Some("s3://tiles/udf"));
    }

    /// CORS (issue #103, ADR 0011) layers like the other scalars and —
    /// the decision — defaults to OFF (an empty allowlist).
    #[test]
    fn cors_origins_default_off_and_flags_outrank_the_file() {
        let cfg = resolve(&ServeArgs {
            fixtures: true,
            ..args()
        })
        .expect("resolves");
        assert!(
            cfg.cors_allowed_origins.is_empty(),
            "CORS is off unless configured"
        );

        let file: ConfigFile =
            toml::from_str(r#"cors-allowed-origins = ["http://localhost:5173"]"#).expect("parses");
        assert_eq!(
            file.cors_allowed_origins.as_deref(),
            Some(&["http://localhost:5173".to_owned()][..])
        );

        let cfg = resolve(&ServeArgs {
            fixtures: true,
            cors_allowed_origins: vec!["*".to_owned()],
            ..args()
        })
        .expect("resolves");
        assert_eq!(cfg.cors_allowed_origins, ["*"]);
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
        let layer = file.layers[0]
            .to_layer(&Budget::default())
            .expect("compiles");
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
            file.layers[0].to_layer(&Budget::default()),
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
            file.layers[0].to_layer(&Budget::default()),
            Err(ConfigError::UnknownBand { .. })
        ));
    }

    /// The materialization budget (#37) layers as documented: built-in
    /// defaults, a global `[budget]` default, flags/env over that, and an
    /// explicit `[layers.budget]` winning knob by knob.
    #[test]
    fn budget_layers_knob_by_knob() {
        // Absent everywhere: pure defaults on every layer.
        let file: ConfigFile = toml::from_str(
            r#"
            store-root = "/data"
            [[layers]]
            id = "tc"
            kind = "truecolor"
            [layers.bands]
            r = "b04.tif"
            g = "b03.tif"
            b = "b02.tif"
        "#,
        )
        .expect("parses");
        let layer = file.layers[0]
            .to_layer(&Budget::default())
            .expect("compiles");
        assert_eq!(layer.budget, Budget::default());

        // Global [budget] + per-layer override: the layer's explicit
        // knob wins, unspecified knobs inherit the global default.
        let file: ConfigFile = toml::from_str(
            r#"
            store-root = "/data"
            [budget]
            overview-oversample = 1.5
            max-estimated-live-bytes = 50000000
            max-udf-fuel-per-tile = 5000000
            [[layers]]
            id = "tc"
            kind = "truecolor"
            [layers.bands]
            r = "b04.tif"
            g = "b03.tif"
            b = "b02.tif"
            [layers.budget]
            cache-enabled = false
            overview-oversample = 1.0
        "#,
        )
        .expect("parses");
        let global = file
            .budget
            .expect("global budget parsed")
            .overlay(&Budget::default());
        assert!((global.overview_oversample - 1.5).abs() < f64::EPSILON);
        assert_eq!(global.max_estimated_live_bytes, Some(50_000_000));
        assert_eq!(global.max_udf_fuel_per_tile, 5_000_000);
        assert!(global.cache_enabled, "unset knob keeps its default");
        let layer = file.layers[0].to_layer(&global).expect("compiles");
        assert!(!layer.budget.cache_enabled);
        assert!((layer.budget.overview_oversample - 1.0).abs() < f64::EPSILON);
        assert_eq!(
            layer.budget.max_estimated_live_bytes,
            Some(50_000_000),
            "unset per-layer knob inherits the global default"
        );
        assert_eq!(
            layer.budget.max_udf_fuel_per_tile, 5_000_000,
            "unset per-layer fuel inherits the global default"
        );

        // Flags/env act as the global default (resolve() wiring): they
        // override the file's [budget] scalars.
        let cfg = resolve(&ServeArgs {
            fixtures: true,
            overview_oversample: Some(2.0),
            max_estimated_live_bytes: Some(123),
            max_udf_fuel_per_tile: Some(456),
            ..args()
        })
        .expect("resolves");
        let LayerSource::Static(registry) = &cfg.layers else {
            panic!("fixtures mode is static");
        };
        let budget = &registry.get("truecolor").expect("layer").budget;
        assert!((budget.overview_oversample - 2.0).abs() < f64::EPSILON);
        assert_eq!(budget.max_estimated_live_bytes, Some(123));
        assert_eq!(budget.max_udf_fuel_per_tile, 456);
        // Absent everywhere: the planner crate's documented default.
        let cfg = resolve(&ServeArgs {
            fixtures: true,
            ..args()
        })
        .expect("resolves");
        let LayerSource::Static(registry) = &cfg.layers else {
            panic!("fixtures mode is static");
        };
        assert_eq!(
            registry
                .get("truecolor")
                .expect("layer")
                .budget
                .max_udf_fuel_per_tile,
            swath_core::planner::DEFAULT_MAX_UDF_FUEL_PER_TILE
        );

        // A typo inside [layers.budget] fails loudly like every other key.
        assert!(
            toml::from_str::<ConfigFile>(
                r"
                [budget]
                oversample = 1.5
            "
            )
            .is_err()
        );
    }

    /// The per-layer colormap key (issue #94): ndvi defaults to the
    /// diverging `RdYlGn`, an explicit map wins, and RGB kinds reject the
    /// key loudly.
    #[test]
    fn ndvi_colormap_defaults_to_rdylgn_and_is_selectable() {
        use swath_render::ir::{Colormap, PixelOp};

        let ndvi = |extra: &str| -> super::ConfigFile {
            toml::from_str(&format!(
                r#"
                [[layers]]
                id = "veg"
                kind = "ndvi"
                {extra}
                [layers.bands]
                nir = "b8a.tif"
                red = "b04.tif"
            "#
            ))
            .expect("parses")
        };

        // Unset: the NDVI default is the diverging map.
        let layer = ndvi("").layers[0]
            .to_layer(&Budget::default())
            .expect("compiles");
        assert_eq!(
            layer.plan.ops.last(),
            Some(&PixelOp::Colormap(Colormap::RdYlGn))
        );

        // Explicit choice wins — including opting back into grayscale.
        for (spelling, expected) in [
            ("grayscale", Colormap::Grayscale),
            ("viridis", Colormap::Viridis),
            ("magma", Colormap::Magma),
            ("rdylgn", Colormap::RdYlGn),
        ] {
            let file = ndvi(&format!("colormap = \"{spelling}\""));
            let layer = file.layers[0]
                .to_layer(&Budget::default())
                .expect("compiles");
            assert_eq!(layer.plan.ops.last(), Some(&PixelOp::Colormap(expected)));
        }

        // An unknown spelling fails at parse, like every enum key.
        assert!(
            toml::from_str::<super::ConfigFile>(
                r#"
                [[layers]]
                id = "veg"
                kind = "ndvi"
                colormap = "jet"
                [layers.bands]
                nir = "b8a.tif"
                red = "b04.tif"
            "#
            )
            .is_err()
        );

        // truecolor renders RGB directly: an explicit colormap is an error.
        let file: super::ConfigFile = toml::from_str(
            r#"
            [[layers]]
            id = "tc"
            kind = "truecolor"
            colormap = "viridis"
            [layers.bands]
            r = "b04.tif"
            g = "b03.tif"
            b = "b02.tif"
        "#,
        )
        .expect("parses");
        assert!(matches!(
            file.layers[0].to_layer(&Budget::default()),
            Err(ConfigError::ColormapNotApplicable { .. })
        ));
    }

    /// Catalog mode persists the same default: the ndvi layer's domain
    /// record carries the diverging map (issue #94).
    #[test]
    fn catalog_ndvi_layer_persists_the_rdylgn_colormap() {
        let file: ConfigFile = toml::from_str(CATALOG_TOML).expect("parses");
        let mode = super::compile_catalog_mode(
            file.catalog.clone().unwrap(),
            None,
            &file.datasets,
            &Budget::default(),
        )
        .expect("compiles");
        assert_eq!(
            mode.datasets[0].layers[1].colormap,
            Some(super::domain::Colormap::RdYlGn)
        );
        assert_eq!(
            mode.layers[1].plan.ops.last(),
            Some(&swath_render::ir::PixelOp::Colormap(
                swath_render::ir::Colormap::RdYlGn
            ))
        );
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
            &Budget::default(),
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
            super::compile_catalog_mode(
                "postgres://x".to_owned(),
                None,
                &file.datasets,
                &Budget::default(),
            ),
            Err(ConfigError::DuplicateLayer { layer }) if layer == "truecolor"
        ));

        // A dataset id appearing twice is refused outright.
        let mut file: ConfigFile = toml::from_str(CATALOG_TOML).expect("parses");
        let twin: ConfigFile = toml::from_str(CATALOG_TOML).expect("parses");
        let mut twin_dataset = twin.datasets.into_iter().next().unwrap();
        twin_dataset.layers.clear();
        file.datasets.push(twin_dataset);
        let err = super::compile_catalog_mode(
            "postgres://x".to_owned(),
            None,
            &file.datasets,
            &Budget::default(),
        )
        .err()
        .expect("duplicate dataset id");
        assert!(matches!(&err, ConfigError::DuplicateDataset { dataset } if dataset == "hls-s30"));
        assert_eq!(err.to_string(), "duplicate dataset id `hls-s30`");

        // Static [[layers]] and catalog mode are mutually exclusive.
        let mixed = format!(
            "{CATALOG_TOML}\n\
             [[layers]]\n\
             id = \"static\"\n\
             kind = \"ndvi\"\n\
             [layers.bands]\n\
             nir = \"b8a.tif\"\n\
             red = \"b04.tif\"\n"
        );
        let dir = swath_testsupport::TempDir::new("cli-config-mixed");
        let path = dir.join("swath.toml");
        std::fs::write(&path, mixed).expect("config writes");
        let err = resolve(&ServeArgs {
            config: Some(path),
            ..args()
        })
        .err()
        .expect("mixed layer sources");
        assert!(matches!(err, ConfigError::MixedLayerSources));
        assert_eq!(
            err.to_string(),
            "catalog mode and static [[layers]] are mutually exclusive: \
             define [[datasets.layers]]"
        );

        // [[datasets]] without a catalog has nowhere to live.
        let datasets_only = r#"
            store-root = "/data"
            [[datasets]]
            id = "hls-s30"
        "#;
        let path = dir.join("datasets-only.toml");
        std::fs::write(&path, datasets_only).expect("config writes");
        let err = resolve(&ServeArgs {
            config: Some(path),
            ..args()
        })
        .err()
        .expect("datasets need catalog mode");
        assert!(matches!(err, ConfigError::DatasetsNeedCatalog));
        assert_eq!(
            err.to_string(),
            "[[datasets]] requires catalog mode (config `catalog`, --catalog, or SWATH_CATALOG)"
        );
    }

    /// The two file-level failures (issue #96: previously unasserted
    /// variants): an unreadable path and invalid TOML, each naming the
    /// file as given.
    #[test]
    fn config_file_read_and_parse_failures_name_the_path() {
        let dir = swath_testsupport::TempDir::new("cli-config-file-errors");

        let missing = dir.join("nowhere.toml");
        let err = resolve(&ServeArgs {
            config: Some(missing.clone()),
            ..args()
        })
        .err()
        .expect("missing config file");
        assert!(matches!(&err, ConfigError::Read { path, .. } if *path == missing));
        assert!(
            err.to_string().starts_with(&format!(
                "cannot read config file `{}`: ",
                missing.display()
            )),
            "got: {err}"
        );

        let invalid = dir.join("invalid.toml");
        std::fs::write(&invalid, "bind = ").expect("file writes");
        let err = resolve(&ServeArgs {
            config: Some(invalid.clone()),
            ..args()
        })
        .err()
        .expect("invalid TOML");
        assert!(matches!(&err, ConfigError::Parse { path, .. } if *path == invalid));
        assert!(
            err.to_string()
                .starts_with(&format!("config file `{}` is invalid: ", invalid.display())),
            "got: {err}"
        );
    }
}
