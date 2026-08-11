// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Layer resolution: how a `{layerId}` becomes a renderable request.
//!
//! [`LayerProvider`] is the seam the walking-skeleton registry docs
//! promised: the OGC handlers consume its `identities`/`resolve` surface and
//! never know whether layers are static ([`LayerRegistry`], `--fixtures` and
//! config-file mode — unchanged) or catalog-backed ([`CatalogLayers`],
//! `swath serve --catalog`, issue #31).
//!
//! # Catalog-backed resolution (the ingest-to-pixel serve half)
//!
//! A [`CatalogLayers`] layer is a compiled render template over a *dataset*;
//! each tile request resolves the plan's band names against the assets of
//! the dataset's **latest granule** — "latest" = maximum acquisition
//! `datetime` at millisecond precision, ties broken by granule id (a total,
//! documented order). The granule's `ingested_at` rides into the
//! [`TileRequest`], where the tiler computes `Trace::ingest_to_pixel_ms` at
//! render completion.
//!
//! Resolution is one `Catalog::find_granules` per tile request — honest
//! walking-skeleton cost, noted for the planner/cache issues to optimize;
//! it is also what makes a freshly ingested granule visible on the *next*
//! tile with no invalidation machinery.
//!
//! A layer whose dataset has no granules yet resolves to 404: the tileset
//! exists (it appears in `/tilesets`), its pixels do not — a poll loop
//! watching for a drop to land observes exactly 404 → 200.

use core::future::Future;

use swath_core::catalog::{Catalog, CatalogError, DatasetId, Datetime, Granule, GranuleQuery};
use swath_core::tile::TileCoord;
use swath_render::TileRequest;
use swath_render::ir::RenderPlan;

use crate::error::ApiError;
use crate::registry::{Layer, LayerRegistry};

/// The human-facing identity of a servable layer — what the tilesets list
/// and tileset metadata expose regardless of how the layer resolves.
#[derive(Debug, Clone)]
pub struct LayerIdentity {
    /// URL-safe identifier — the `{layerId}` path segment.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Short narrative description.
    pub description: String,
}

/// A layer resolved to renderable form: the static template plus, for
/// catalog-backed layers, the ingest timestamp of the granule the assets
/// came from.
#[derive(Debug, Clone)]
pub struct ResolvedLayer {
    /// The renderable layer (identity + bands + plan).
    pub layer: Layer,
    /// When the backing granule was ingested (`None` for static layers).
    pub ingested_at: Option<Datetime>,
    /// The id of the granule the assets resolved from (`None` for static
    /// layers) — the granule half of the cache's `layer_version` (#36,
    /// `swath_core::cache::layer_version`): a new granule is a new
    /// version, which is the whole invalidation story.
    pub granule_id: Option<String>,
}

impl ResolvedLayer {
    /// The [`TileRequest`] rendering `coord`, ingest timestamp included.
    #[must_use]
    pub fn tile_request(&self, coord: TileCoord) -> TileRequest {
        let request = self.layer.tile_request(coord);
        match &self.ingested_at {
            Some(ingested_at) => request.with_ingested_at(ingested_at.clone()),
            None => request,
        }
    }
}

/// The layer-resolution port of the OGC surface (native AFIT, same pattern
/// as the core's ports): who the layers are, and what serving one means
/// right now.
pub trait LayerProvider: Send + Sync {
    /// Every layer's identity, in id order — the tilesets list. Static
    /// knowledge; never does I/O.
    fn identities(&self) -> Vec<LayerIdentity>;

    /// The identity of one layer, or `None` if the id is not served.
    fn identity(&self, id: &str) -> Option<LayerIdentity>;

    /// Resolves a layer to renderable form. Errors are API-shaped: unknown
    /// id → 404, a catalog-backed layer with no granules yet → 404,
    /// catalog failure → 500.
    fn resolve(&self, id: &str) -> impl Future<Output = Result<ResolvedLayer, ApiError>> + Send;
}

/// The static registry is the trivial provider: resolution is a clone, and
/// no layer ever carries an ingest timestamp.
impl LayerProvider for LayerRegistry {
    fn identities(&self) -> Vec<LayerIdentity> {
        self.iter()
            .map(|layer| LayerIdentity {
                id: layer.id.clone(),
                title: layer.title.clone(),
                description: layer.description.clone(),
            })
            .collect()
    }

    fn identity(&self, id: &str) -> Option<LayerIdentity> {
        self.get(id).map(|layer| LayerIdentity {
            id: layer.id.clone(),
            title: layer.title.clone(),
            description: layer.description.clone(),
        })
    }

    async fn resolve(&self, id: &str) -> Result<ResolvedLayer, ApiError> {
        let layer = self
            .get(id)
            .ok_or_else(|| ApiError::not_found(format!("no layer `{id}`")))?;
        Ok(ResolvedLayer {
            layer: layer.clone(),
            ingested_at: None,
            granule_id: None,
        })
    }
}

/// One catalog-backed layer: the compiled render template (plan inputs are
/// **dataset band names**, resolved against granule assets key-for-key)
/// plus the dataset it serves. Compiled by the binary from its config; the
/// same `PlanKind` lowering also persists to the catalog's `swath:layers`.
#[derive(Debug, Clone)]
pub struct CatalogLayer {
    /// URL-safe identifier — the `{layerId}` path segment.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Short narrative description.
    pub description: String,
    /// The dataset whose latest granule backs each tile.
    pub dataset: DatasetId,
    /// The pixel pipeline; its inputs name dataset bands.
    pub plan: RenderPlan,
    /// Resampling kernel for every band's warp.
    pub resampling: swath_render::Resampling,
    /// Tile side length in pixels.
    pub tile_size: u32,
    /// The layer's materialization budget (#37) — the planner's knobs;
    /// defaults reproduce pre-planner behavior.
    pub budget: swath_core::planner::Budget,
}

/// The catalog-backed [`LayerProvider`]: identities compiled from config
/// (plus any layers the openEO services surface publishes at runtime,
/// ADR 0010), per-request asset resolution from the latest granule.
///
/// The layer set lives behind a shared lock: **clones share it**, which is
/// the seam the openEO surface uses — the services handlers hold a clone
/// of the same provider the tile handlers resolve through, so a `POST`ed
/// service is servable on the very next tile request. The lock is never
/// held across an await (entries are cloned out).
#[derive(Debug, Clone)]
pub struct CatalogLayers<C> {
    catalog: C,
    /// Kept in id order (restored on every mutation) for a stable
    /// tilesets list.
    layers: std::sync::Arc<std::sync::RwLock<Vec<CatalogLayer>>>,
}

impl<C> CatalogLayers<C> {
    /// A provider over `catalog` serving `layers` (sorted by id; a
    /// duplicate id is a config bug the binary rejects upstream).
    #[must_use]
    pub fn new(catalog: C, mut layers: Vec<CatalogLayer>) -> Self {
        layers.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            catalog,
            layers: std::sync::Arc::new(std::sync::RwLock::new(layers)),
        }
    }

    /// The catalog this provider resolves granules from — shared with the
    /// openEO services surface, which persists authored layers through it.
    pub fn catalog(&self) -> &C {
        &self.catalog
    }

    /// Inserts (or replaces, by id) a servable layer at runtime — the
    /// openEO `POST /services` seam. Visible to every clone immediately.
    pub fn insert(&self, layer: CatalogLayer) {
        let mut layers = self.layers.write().expect("layer lock is not poisoned");
        layers.retain(|existing| existing.id != layer.id);
        layers.push(layer);
        layers.sort_by(|a, b| a.id.cmp(&b.id));
    }

    /// Removes a layer by id (openEO `DELETE /services/{id}`); `false`
    /// when no such layer was served.
    pub fn remove(&self, id: &str) -> bool {
        let mut layers = self.layers.write().expect("layer lock is not poisoned");
        let before = layers.len();
        layers.retain(|layer| layer.id != id);
        layers.len() != before
    }

    fn entry(&self, id: &str) -> Option<CatalogLayer> {
        self.layers
            .read()
            .expect("layer lock is not poisoned")
            .iter()
            .find(|layer| layer.id == id)
            .cloned()
    }
}

/// The latest granule: max acquisition datetime (millisecond precision),
/// ties by granule id — a total order, so "which granule backs this layer"
/// has one answer.
fn latest(granules: Vec<Granule>) -> Option<Granule> {
    granules
        .into_iter()
        .max_by_key(|g| (g.datetime.to_unix_millis(), g.id.clone()))
}

impl<C: Catalog> LayerProvider for CatalogLayers<C> {
    fn identities(&self) -> Vec<LayerIdentity> {
        self.layers
            .read()
            .expect("layer lock is not poisoned")
            .iter()
            .map(|layer| LayerIdentity {
                id: layer.id.clone(),
                title: layer.title.clone(),
                description: layer.description.clone(),
            })
            .collect()
    }

    fn identity(&self, id: &str) -> Option<LayerIdentity> {
        self.entry(id).map(|layer| LayerIdentity {
            id: layer.id.clone(),
            title: layer.title.clone(),
            description: layer.description.clone(),
        })
    }

    async fn resolve(&self, id: &str) -> Result<ResolvedLayer, ApiError> {
        let entry = self
            .entry(id)
            .ok_or_else(|| ApiError::not_found(format!("no layer `{id}`")))?;
        self.resolve_template(&entry).await
    }
}

impl<C: Catalog> CatalogLayers<C> {
    /// Resolves a layer template — registered or not — against the latest
    /// granule of its dataset: the shared resolution of [`resolve`]
    /// (which looks the template up by id first) and the openEO preview
    /// (ADR 0014), which must resolve a *draft* template without ever
    /// inserting it into the served layer set.
    ///
    /// [`resolve`]: LayerProvider::resolve
    ///
    /// # Errors
    ///
    /// API-shaped like [`resolve`]: no granules yet → 404, catalog
    /// failure → 500, a granule missing a required band → 500.
    pub async fn resolve_template(&self, entry: &CatalogLayer) -> Result<ResolvedLayer, ApiError> {
        let id = &entry.id;
        let granules = self
            .catalog
            .find_granules(&entry.dataset, &GranuleQuery::default())
            .await
            .map_err(|err| catalog_error(&entry.dataset, &err))?;
        let granule = latest(granules).ok_or_else(|| {
            ApiError::not_found(format!(
                "layer `{id}`: no granule of dataset `{dataset}` has been ingested yet",
                dataset = entry.dataset,
            ))
        })?;

        // Plan inputs name dataset bands; the granule must provide each.
        let mut bands = std::collections::BTreeMap::new();
        for input in &entry.plan.inputs {
            let asset = granule.assets.get(&input.name).ok_or_else(|| {
                ApiError::internal(format!(
                    "granule `{granule_id}` of dataset `{dataset}` provides no band \
                     `{band}` required by layer `{id}`",
                    granule_id = granule.id,
                    dataset = entry.dataset,
                    band = input.name,
                ))
            })?;
            // Raster assets and virtual-cube assets both resolve to their
            // href: the binary's composite RasterSource dispatches on the
            // href shape (#39 replaced the honest 500 that stood here
            // since #40 — virtual cubes are servable now).
            bands.insert(input.name.clone(), asset.href.clone());
        }

        Ok(ResolvedLayer {
            layer: Layer {
                id: entry.id.clone(),
                title: entry.title.clone(),
                description: entry.description.clone(),
                bands,
                plan: entry.plan.clone(),
                resampling: entry.resampling,
                tile_size: entry.tile_size,
                budget: entry.budget.clone(),
            },
            ingested_at: granule.ingested_at.clone(),
            granule_id: Some(granule.id.to_string()),
        })
    }
}

/// Catalog failures during resolution, translated for the operator: a
/// vanished dataset is still a 404 (the layer cannot be served), everything
/// else is an honest 500.
fn catalog_error(dataset: &DatasetId, err: &CatalogError) -> ApiError {
    match err {
        CatalogError::DatasetNotFound { .. } => {
            ApiError::not_found(format!("dataset `{dataset}` is not in the catalog"))
        }
        other => ApiError::internal(format!("catalog lookup for `{dataset}` failed: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use swath_core::catalog::{
        Bbox, Catalog, CatalogError, Dataset, DatasetId, Datetime, Granule, GranuleAsset,
        GranuleId, GranuleQuery,
    };
    use swath_core::tile::TileCoord;
    use swath_render::ir::{BandInput, OutputSpec, PixelOp, RenderPlan, TileFormat};
    use swath_render::{NodataPolicy, Resampling};

    use super::{CatalogLayer, CatalogLayers, LayerProvider};

    /// Granule-serving stub: `find_granules` returns the canned set.
    struct StubCatalog {
        granules: Mutex<Vec<Granule>>,
    }

    impl Catalog for StubCatalog {
        async fn upsert_dataset(&self, _: &Dataset) -> Result<(), CatalogError> {
            unreachable!("serving never writes datasets")
        }

        async fn upsert_granules(&self, _: &[Granule]) -> Result<(), CatalogError> {
            unreachable!("serving never writes granules")
        }

        async fn get_dataset(&self, _: &DatasetId) -> Result<Option<Dataset>, CatalogError> {
            unreachable!("resolution uses find_granules only")
        }

        async fn list_datasets(&self) -> Result<Vec<Dataset>, CatalogError> {
            unreachable!("resolution uses find_granules only")
        }

        async fn find_granules(
            &self,
            _: &DatasetId,
            _: &GranuleQuery,
        ) -> Result<Vec<Granule>, CatalogError> {
            Ok(self.granules.lock().unwrap().clone())
        }
    }

    fn granule(id: &str, datetime: &str, ingested_at: Option<&str>) -> Granule {
        Granule {
            id: GranuleId::new(id),
            dataset: DatasetId::new("hls-s30"),
            bbox: Bbox {
                west: -106.1,
                south: 39.2,
                east: -105.9,
                north: 39.4,
            },
            datetime: Datetime::new(datetime).unwrap(),
            assets: BTreeMap::from([
                (
                    "b04".to_owned(),
                    GranuleAsset::raster(format!("{id}-b04.tif")),
                ),
                (
                    "b03".to_owned(),
                    GranuleAsset::raster(format!("{id}-b03.tif")),
                ),
                (
                    "b02".to_owned(),
                    GranuleAsset::raster(format!("{id}-b02.tif")),
                ),
            ]),
            ingested_at: ingested_at.map(|t| Datetime::new(t).unwrap()),
        }
    }

    fn provider(granules: Vec<Granule>) -> CatalogLayers<StubCatalog> {
        let plan = RenderPlan::new(
            vec![
                BandInput::new("b04"),
                BandInput::new("b03"),
                BandInput::new("b02"),
            ],
            vec![PixelOp::Composite {
                r: "b04".into(),
                g: "b03".into(),
                b: "b02".into(),
            }],
            OutputSpec::new(TileFormat::Png),
        );
        CatalogLayers::new(
            StubCatalog {
                granules: Mutex::new(granules),
            },
            vec![CatalogLayer {
                id: "truecolor".to_owned(),
                title: "True color".to_owned(),
                description: String::new(),
                dataset: DatasetId::new("hls-s30"),
                plan,
                resampling: Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
                tile_size: 256,
                budget: swath_core::planner::Budget::default(),
            }],
        )
    }

    #[tokio::test]
    async fn resolves_assets_from_the_latest_granule() {
        let provider = provider(vec![
            granule(
                "g-old",
                "2024-06-06T17:54:00Z",
                Some("2026-08-08T00:00:00Z"),
            ),
            granule(
                "g-new",
                "2024-06-13T17:54:00Z",
                Some("2026-08-08T01:00:00Z"),
            ),
            granule("g-mid", "2024-06-10T17:54:00Z", None),
        ]);
        let resolved = provider.resolve("truecolor").await.unwrap();
        assert_eq!(resolved.layer.bands["b04"].as_str(), "g-new-b04.tif");
        assert_eq!(
            resolved.ingested_at.as_ref().map(Datetime::as_str),
            Some("2026-08-08T01:00:00Z")
        );

        // The ingest stamp rides into the TileRequest.
        let request = resolved.tile_request(TileCoord::new(12, 848, 1561).unwrap());
        assert_eq!(
            request.ingested_at.as_ref().map(Datetime::as_str),
            Some("2026-08-08T01:00:00Z")
        );
    }

    #[tokio::test]
    async fn same_datetime_ties_break_by_granule_id() {
        let provider = provider(vec![
            granule("g-aaa", "2024-06-06T17:54:00Z", None),
            granule("g-zzz", "2024-06-06T17:54:00Z", None),
        ]);
        let resolved = provider.resolve("truecolor").await.unwrap();
        assert_eq!(resolved.layer.bands["b04"].as_str(), "g-zzz-b04.tif");
    }

    #[tokio::test]
    async fn no_granules_is_404_and_unknown_layer_is_404() {
        let provider = provider(Vec::new());
        let err = provider.resolve("truecolor").await.unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
        let err = provider.resolve("nope").await.unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn granule_missing_a_required_band_is_500() {
        let mut incomplete = granule("g-partial", "2024-06-06T17:54:00Z", None);
        incomplete.assets.remove("b03");
        let provider = provider(vec![incomplete]);
        let err = provider.resolve("truecolor").await.unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn identities_are_stable_and_id_sorted() {
        let provider = provider(Vec::new());
        let ids: Vec<String> = provider.identities().into_iter().map(|i| i.id).collect();
        assert_eq!(ids, ["truecolor"]);
        assert!(provider.identity("truecolor").is_some());
        assert!(provider.identity("nope").is_none());
    }
}
