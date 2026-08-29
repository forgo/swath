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
//! the dataset's **latest granule within the request's resolution
//! window** (ADR 0015): the tiles route's `datetime` parameter parses to
//! an inclusive, optionally open-ended window, and an absent parameter is
//! the fully open window — plain **latest**, the original behavior,
//! unchanged. "Latest" = maximum acquisition `datetime` at millisecond
//! precision, ties broken by granule id (a total, documented order). The
//! granule's `ingested_at` rides into the [`TileRequest`], where the
//! tiler computes `Trace::ingest_to_pixel_ms` at render completion; its
//! id and acquisition datetime ride out on the [`ResolvedLayer`] for the
//! cache's `layer_version` and the Trace's temporal decision.
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

use swath_core::catalog::{
    Bbox, Catalog, CatalogError, DatasetId, Datetime, Granule, GranuleQuery, TimeRange,
};
use swath_core::tile::TileCoord;
use swath_render::ir::RenderPlan;
use swath_render::{SourceWindow, TileRequest};

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
    /// The dataset whose granules back this layer's frames (`None` for
    /// static layers — a single timeless frame, no time dimension). How
    /// the tileset metadata advertises the layer's granule listing
    /// (`/datasets/{id}/granules`), which is where a client learns the
    /// `datetime=` frames it can ask for (ADR 0015; the web time slider
    /// reads exactly this).
    pub dataset: Option<String>,
    /// The compiled frame-selection window (ADR 0015) — the hull of the
    /// branch windows for a two-source layer (ADR 0022); `None` for
    /// static layers. Advertised on the tileset metadata so a client
    /// bounds the frames it offers.
    pub window: Option<TimeRange>,
    /// The number of `load_collection` branches (ADR 0022): 1 for a
    /// chain or a config layer, 2 for a join, 0 for static layers.
    pub sources: usize,
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
    /// version, which is the whole invalidation story. Under ADR 0015 a
    /// time-parameterized frame keys under the granule it *resolved to*,
    /// so this same field is also the whole temporal cache identity.
    pub granule_id: Option<String>,
    /// The resolved granule's acquisition datetime (`None` for static
    /// layers) — what the Trace's temporal decision reports (ADR 0015).
    pub granule_datetime: Option<Datetime>,
    /// The resolved granule's WGS84 footprint (`None` for static layers)
    /// — where this resolution's pixels actually are, which is what the
    /// openEO preview (ADR 0014) frames when the graph names no
    /// `spatial_extent`: a config-declared dataset advertises a
    /// whole-world placeholder box (ROADMAP row 15), and a preview tile
    /// of the placeholder is one blank root tile, never the granule.
    pub granule_bbox: Option<Bbox>,
    /// Every granule the frame resolved to, one per source in branch
    /// order (ADR 0022) — the singular fields above are the first
    /// (primary) entry, kept for the one-source transition. Empty for
    /// static layers.
    pub granules: Vec<ResolvedGranule>,
}

/// One branch's resolution: the `load_collection` node it serves and the
/// granule that backs it.
#[derive(Debug, Clone)]
pub struct ResolvedGranule {
    /// The `load_collection` node id (empty for config-defined layers,
    /// which have no graph).
    pub node: String,
    /// The granule id.
    pub id: String,
    /// The granule's acquisition datetime.
    pub datetime: Datetime,
    /// The granule's WGS84 footprint.
    pub bbox: Bbox,
    /// When the granule was ingested.
    pub ingested_at: Option<Datetime>,
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

    /// Resolves a layer to renderable form, optionally constrained to a
    /// temporal resolution `window` (ADR 0015: the tiles route's parsed
    /// `datetime`; `None` = the fully open interval = latest — today's
    /// behavior, unchanged). Errors are API-shaped: unknown id → 404, a
    /// catalog-backed layer with no granule in the window (none ingested
    /// yet, or the window selects none) → 404, catalog failure → 500.
    fn resolve(
        &self,
        id: &str,
        window: Option<&TimeRange>,
    ) -> impl Future<Output = Result<ResolvedLayer, ApiError>> + Send;
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
                dataset: None,
                window: None,
                sources: 0,
            })
            .collect()
    }

    fn identity(&self, id: &str) -> Option<LayerIdentity> {
        self.get(id).map(|layer| LayerIdentity {
            id: layer.id.clone(),
            title: layer.title.clone(),
            description: layer.description.clone(),
            dataset: None,
            window: None,
            sources: 0,
        })
    }

    /// A static layer is a single, timeless frame — it has no acquisition
    /// datetime to select on, so every valid `window` resolves to that
    /// one frame (the degenerate latest-at-or-before over a dateless
    /// singleton). The grammar is still validated upstream; only catalog
    /// mode has frames time can distinguish.
    async fn resolve(
        &self,
        id: &str,
        _window: Option<&TimeRange>,
    ) -> Result<ResolvedLayer, ApiError> {
        let layer = self
            .get(id)
            .ok_or_else(|| ApiError::not_found(format!("no layer `{id}`")))?;
        Ok(ResolvedLayer {
            layer: layer.clone(),
            ingested_at: None,
            granule_id: None,
            granule_datetime: None,
            granule_bbox: None,
            granules: Vec::new(),
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
    /// The temporal resolution window (ADR 0015 frame selection): granule
    /// resolution considers only acquisitions inside it. Open on both
    /// sides (the default) resolves against every granule — today's
    /// "latest wins", unchanged. Compiled from an openEO graph's
    /// `temporal_extent` / `filter_temporal`; config-defined layers are
    /// unconstrained. For a layer over more than one source this is the
    /// hull of the branch windows; resolution runs per branch over
    /// [`sources`](Self::sources).
    pub window: TimeRange,
    /// The `load_collection` branches of a compiled graph, each with its
    /// own resolution window (ADR 0022): one for a single-source graph,
    /// two for a `merge_cubes` join. Empty for config-defined layers.
    pub sources: Vec<SourceWindow>,
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

/// The intersection of two inclusive, optionally open-ended windows —
/// the later start, the earlier end (ADR 0015: the request's `datetime`
/// composed with a layer's compiled resolution window). May come out
/// empty (start after end); the caller treats that as selecting nothing.
fn intersect(a: &TimeRange, b: &TimeRange) -> TimeRange {
    let pick = |x: Option<&Datetime>, y: Option<&Datetime>, later: bool| match (x, y) {
        (Some(x), Some(y)) => {
            let x_wins = (x.to_unix_millis() >= y.to_unix_millis()) == later;
            Some(if x_wins { x.clone() } else { y.clone() })
        }
        (bound, None) | (None, bound) => bound.cloned(),
    };
    TimeRange {
        start: pick(a.start.as_ref(), b.start.as_ref(), true),
        end: pick(a.end.as_ref(), b.end.as_ref(), false),
    }
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
                dataset: Some(layer.dataset.to_string()),
                window: Some(layer.window.clone()),
                sources: layer.sources.len().max(1),
            })
            .collect()
    }

    fn identity(&self, id: &str) -> Option<LayerIdentity> {
        self.entry(id).map(|layer| LayerIdentity {
            id: layer.id.clone(),
            title: layer.title.clone(),
            description: layer.description.clone(),
            dataset: Some(layer.dataset.to_string()),
            window: Some(layer.window.clone()),
            sources: layer.sources.len().max(1),
        })
    }

    async fn resolve(
        &self,
        id: &str,
        window: Option<&TimeRange>,
    ) -> Result<ResolvedLayer, ApiError> {
        let entry = self
            .entry(id)
            .ok_or_else(|| ApiError::not_found(format!("no layer `{id}`")))?;
        self.resolve_template(&entry, window).await
    }
}

impl<C: Catalog> CatalogLayers<C> {
    /// Resolves a layer template — registered or not — against the
    /// **latest granule within the effective window** of its dataset
    /// (ADR 0015: the request's parsed `datetime` in `window`,
    /// intersected with the layer's compiled resolution window
    /// [`CatalogLayer::window`]; both open = latest, the pre-#180
    /// behavior byte-for-byte): the shared
    /// resolution of [`resolve`] (which looks the template up by id
    /// first) and the openEO preview (ADR 0014), which must resolve a
    /// *draft* template without ever inserting it into the served layer
    /// set. Mechanically still one `find_granules` per request — the
    /// query now carries its already-existing `datetime` filter.
    ///
    /// [`resolve`]: LayerProvider::resolve
    ///
    /// One branch's granule (ADR 0015 composition, per branch under ADR
    /// 0022): the request's `datetime` window intersected with the
    /// branch's compiled resolution window, then latest-at-or-before.
    /// `branch` names the `load_collection` node in the 404 when the
    /// layer has more than one.
    async fn resolve_branch(
        &self,
        entry: &CatalogLayer,
        compiled: &TimeRange,
        window: Option<&TimeRange>,
        branch: Option<&str>,
    ) -> Result<Granule, ApiError> {
        let id = &entry.id;
        // ADR 0015 composition: the request's `datetime` window (the
        // tiles route, #180) is intersected with the layer's *compiled*
        // resolution window (`temporal_extent`/`filter_temporal`, #181)
        // before the latest-at-or-before rule runs — later start,
        // earlier end. A layer without a graph window passes the request
        // window through untouched; no window at all keeps the exact
        // open query the provider always sent.
        let compiled = (*compiled != TimeRange::default()).then_some(compiled);
        let effective = match (compiled, window) {
            (None, None) => None,
            (Some(one), None) | (None, Some(one)) => Some(one.clone()),
            (Some(a), Some(b)) => Some(intersect(a, b)),
        };
        // A provably empty intersection (a request datetime outside the
        // layer's window) selects no granule by definition — 404 without
        // asking the catalog to evaluate an inverted interval.
        let empty = effective
            .as_ref()
            .is_some_and(|w| match (&w.start, &w.end) {
                (Some(start), Some(end)) => start.to_unix_millis() > end.to_unix_millis(),
                _ => false,
            });
        let granules = if empty {
            Vec::new()
        } else {
            let query = GranuleQuery {
                bbox: None,
                datetime: effective.clone(),
            };
            self.catalog
                .find_granules(&entry.dataset, &query)
                .await
                .map_err(|err| catalog_error(&entry.dataset, &err))?
        };
        latest(granules).ok_or_else(|| {
            let dataset = &entry.dataset;
            let branch = branch.map_or_else(String::new, |node| format!(" (branch `{node}`)"));
            match &effective {
                None => ApiError::not_found(format!(
                    "layer `{id}`{branch}: no granule of dataset `{dataset}` has been ingested yet",
                )),
                Some(window) => ApiError::not_found(format!(
                    "layer `{id}`{branch}: no granule of dataset `{dataset}` has an acquisition \
                     datetime within [{start}, {end}]",
                    start = window.start.as_ref().map_or("..", Datetime::as_str),
                    end = window.end.as_ref().map_or("..", Datetime::as_str),
                )),
            }
        })
    }

    /// # Errors
    ///
    /// API-shaped like [`resolve`]: no granule in the window (none
    /// ingested yet, or a `datetime` that selects none — before the
    /// first acquisition, or a narrowed interval) → 404 of one shape,
    /// catalog failure → 500, a granule missing a required band → 500.
    pub async fn resolve_template(
        &self,
        entry: &CatalogLayer,
        window: Option<&TimeRange>,
    ) -> Result<ResolvedLayer, ApiError> {
        let id = &entry.id;
        // One branch per source (ADR 0022); a config-defined layer, with
        // no graph, is the single branch over its own window.
        let branches: Vec<(&str, &TimeRange)> = if entry.sources.len() > 1 {
            entry
                .sources
                .iter()
                .map(|source| (source.node.as_str(), &source.window))
                .collect()
        } else {
            vec![(
                entry
                    .sources
                    .first()
                    .map_or("", |source| source.node.as_str()),
                &entry.window,
            )]
        };
        let named = branches.len() > 1;
        let mut granules = Vec::with_capacity(branches.len());
        for (node, compiled) in branches {
            let branch = named.then_some(node);
            let granule = self.resolve_branch(entry, compiled, window, branch).await?;
            granules.push((node.to_owned(), granule));
        }

        // Plan inputs name dataset bands — `band@node` in a multi-source
        // plan; each must come from the granule its branch resolved to.
        let mut bands = std::collections::BTreeMap::new();
        for input in &entry.plan.inputs {
            let (node, granule) = match &input.source {
                Some(source) => granules
                    .iter()
                    .find(|(node, _)| node == source)
                    .ok_or_else(|| {
                        ApiError::internal(format!(
                            "layer `{id}`: plan input `{name}` names source `{source}`, which \
                             the layer does not load",
                            name = input.name,
                        ))
                    })?,
                None => &granules[0],
            };
            let asset = granule.assets.get(input.band()).ok_or_else(|| {
                let branch = if named {
                    format!(" (branch `{node}`)")
                } else {
                    String::new()
                };
                ApiError::internal(format!(
                    "granule `{granule_id}` of dataset `{dataset}` provides no band \
                     `{band}` required by layer `{id}`{branch}",
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

        // The frame is as fresh as its newest branch: ingest→pixel reads
        // against the latest arrival.
        let ingested_at = granules
            .iter()
            .filter_map(|(_, g)| g.ingested_at.clone())
            .max_by_key(Datetime::to_unix_millis);
        let primary = &granules[0].1;
        let resolved: Vec<ResolvedGranule> = granules
            .iter()
            .map(|(node, g)| ResolvedGranule {
                node: node.clone(),
                id: g.id.to_string(),
                datetime: g.datetime.clone(),
                bbox: g.bbox,
                ingested_at: g.ingested_at.clone(),
            })
            .collect();

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
            ingested_at,
            granule_id: Some(primary.id.to_string()),
            granule_datetime: Some(primary.datetime.clone()),
            granule_bbox: Some(primary.bbox),
            granules: resolved,
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

    use swath_core::catalog::{
        Bbox, DatasetId, Datetime, Granule, GranuleAsset, GranuleId, TimeRange,
    };
    use swath_core::tile::TileCoord;
    use swath_render::ir::{BandInput, OutputSpec, PixelOp, RenderPlan, TileFormat};
    use swath_render::{NodataPolicy, Resampling, SourceWindow};

    use super::{CatalogLayer, CatalogLayers, LayerProvider};
    use swath_testsupport::catalog::MemoryCatalog;
    use swath_testsupport::fixtures::hls_catalog_dataset;

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

    /// The shared in-memory catalog with the HLS dataset registered and
    /// `granules` seeded — resolution reads through `find_granules`, whose
    /// datetime filter the double honours exactly as the port documents.
    fn seeded(granules: Vec<Granule>) -> MemoryCatalog {
        let catalog = MemoryCatalog::default();
        catalog.seed(hls_catalog_dataset(), granules);
        catalog
    }

    fn provider(granules: Vec<Granule>) -> CatalogLayers<MemoryCatalog> {
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
            seeded(granules),
            vec![CatalogLayer {
                id: "truecolor".to_owned(),
                title: "True color".to_owned(),
                description: String::new(),
                dataset: DatasetId::new("hls-s30"),
                plan,
                resampling: Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
                tile_size: 256,
                budget: swath_core::planner::Budget::default(),
                window: TimeRange::default(),
                sources: Vec::new(),
            }],
        )
    }

    /// A two-source layer (ADR 0022): one granule per branch under its
    /// own window, plan inputs read from the branch they name, the
    /// frame as fresh as its newest branch, and a 404 that names the
    /// branch left without a granule.
    #[tokio::test]
    async fn two_source_layers_resolve_one_granule_per_branch() {
        let two_source = |granules: Vec<Granule>| {
            let plan = RenderPlan::new(
                vec![BandInput::new("b04@after"), BandInput::new("b04@before")],
                vec![PixelOp::BandMath(
                    swath_render::ir::Expr::band("b04@after")
                        - swath_render::ir::Expr::band("b04@before"),
                )],
                OutputSpec::new(TileFormat::Png),
            );
            let window = |start: &str, end: &str| TimeRange {
                start: Some(Datetime::new(start).unwrap()),
                end: Some(Datetime::new(end).unwrap()),
            };
            CatalogLayers::new(
                seeded(granules),
                vec![CatalogLayer {
                    id: "change".to_owned(),
                    title: "Change".to_owned(),
                    description: String::new(),
                    dataset: DatasetId::new("hls-s30"),
                    plan,
                    resampling: Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
                    tile_size: 256,
                    budget: swath_core::planner::Budget::default(),
                    window: window("2024-06-01T00:00:00Z", "2024-06-30T23:59:59.999Z"),
                    sources: vec![
                        SourceWindow {
                            node: "after".to_owned(),
                            window: window("2024-06-10T00:00:00Z", "2024-06-30T23:59:59.999Z"),
                        },
                        SourceWindow {
                            node: "before".to_owned(),
                            window: window("2024-06-01T00:00:00Z", "2024-06-09T23:59:59.999Z"),
                        },
                    ],
                }],
            )
        };
        let provider = two_source(vec![
            granule(
                "g-jun06",
                "2024-06-06T17:54:00Z",
                Some("2026-08-08T00:00:00Z"),
            ),
            granule(
                "g-jun13",
                "2024-06-13T17:54:00Z",
                Some("2026-08-08T01:00:00Z"),
            ),
        ]);
        let resolved = provider.resolve("change", None).await.unwrap();
        assert_eq!(
            resolved.layer.bands["b04@after"].as_str(),
            "g-jun13-b04.tif"
        );
        assert_eq!(
            resolved.layer.bands["b04@before"].as_str(),
            "g-jun06-b04.tif"
        );
        let branches: Vec<(&str, &str)> = resolved
            .granules
            .iter()
            .map(|g| (g.node.as_str(), g.id.as_str()))
            .collect();
        assert_eq!(branches, [("after", "g-jun13"), ("before", "g-jun06")]);
        // The singular fields are the primary (first) branch.
        assert_eq!(resolved.granule_id.as_deref(), Some("g-jun13"));
        assert_eq!(
            resolved.ingested_at.as_ref().map(Datetime::as_str),
            Some("2026-08-08T01:00:00Z")
        );
        // A request instant that empties one branch names it.
        let early = TimeRange {
            start: None,
            end: Some(Datetime::new("2024-06-08T00:00:00Z").unwrap()),
        };
        let err = provider.resolve("change", Some(&early)).await.unwrap_err();
        let problem = err.to_string();
        assert!(problem.contains("(branch `after`)"), "{problem}");
        assert!(problem.contains("2024-06-10T00:00:00Z"), "{problem}");
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
        let resolved = provider.resolve("truecolor", None).await.unwrap();
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

    /// The ADR 0015 resolution-rule table over the catalog port: instants
    /// (latest-at-or-before, inclusive), intervals (latest-within, both
    /// ends inclusive), open ends, and the empty window → 404. Windows
    /// here are the parsed forms `crate::temporal` produces (an instant
    /// `t` arrives as `(.., t]`).
    #[tokio::test]
    async fn temporal_windows_resolve_latest_at_or_before() {
        use swath_core::catalog::TimeRange;
        let dt = |s: &str| Some(swath_core::catalog::Datetime::new(s).unwrap());
        let provider = provider(vec![
            granule("g-old", "2024-06-06T17:54:00Z", None),
            granule("g-mid", "2024-06-10T17:54:00Z", None),
            granule("g-new", "2024-06-13T17:54:00Z", None),
        ]);

        let resolve = |window: TimeRange| {
            let provider = &provider;
            async move {
                provider
                    .resolve("truecolor", Some(&window))
                    .await
                    .map(|resolved| resolved.granule_id.unwrap())
            }
        };
        let cases = [
            // Instant windows `(.., t]`: the granule current at `t`.
            (None, dt("2024-06-11T00:00:00Z"), Ok("g-mid")),
            // At-or-before is inclusive: an instant exactly at an
            // acquisition selects it.
            (None, dt("2024-06-10T17:54:00Z"), Ok("g-mid")),
            (None, dt("2024-06-05T00:00:00Z"), Err("before the first")),
            // Intervals: latest within, both bounds inclusive.
            (
                dt("2024-06-07T00:00:00Z"),
                dt("2024-06-13T17:54:00Z"),
                Ok("g-new"),
            ),
            (
                dt("2024-06-07T00:00:00Z"),
                dt("2024-06-11T00:00:00Z"),
                Ok("g-mid"),
            ),
            // A granule before the interval's start never leaks in.
            (
                dt("2024-06-14T00:00:00Z"),
                None,
                Err("nothing after the last"),
            ),
            // Open-ended interval forms.
            (dt("2024-06-07T00:00:00Z"), None, Ok("g-new")),
            (None, None, Ok("g-new")),
        ];
        for (start, end, expected) in cases {
            let window = TimeRange {
                start: start.clone(),
                end: end.clone(),
            };
            let outcome = resolve(window).await;
            match expected {
                Ok(granule_id) => {
                    assert_eq!(outcome.as_deref(), Ok(granule_id), "[{start:?}, {end:?}]");
                }
                Err(why) => {
                    let err = outcome.expect_err(why);
                    assert_eq!(
                        err.status,
                        axum::http::StatusCode::NOT_FOUND,
                        "[{start:?}, {end:?}]: {why}"
                    );
                    assert!(
                        err.detail.contains("acquisition datetime within"),
                        "the empty-window 404 names the window: {}",
                        err.detail
                    );
                }
            }
        }

        // Absent window (`None`) is plain latest — and carries the
        // no-granules-yet wording only when the dataset is empty.
        let resolved = provider.resolve("truecolor", None).await.unwrap();
        assert_eq!(resolved.granule_id.as_deref(), Some("g-new"));
        assert_eq!(
            resolved.granule_datetime.as_ref().map(Datetime::as_str),
            Some("2024-06-13T17:54:00Z"),
        );
    }

    /// Ties inside a window break by granule id — the same total order as
    /// plain latest, so a time-parameterized frame is deterministic too.
    #[tokio::test]
    async fn temporal_window_ties_break_by_granule_id() {
        use swath_core::catalog::TimeRange;
        let provider = provider(vec![
            granule("g-aaa", "2024-06-06T17:54:00Z", None),
            granule("g-zzz", "2024-06-06T17:54:00Z", None),
        ]);
        let window = TimeRange {
            start: None,
            end: Some(swath_core::catalog::Datetime::new("2024-06-07T00:00:00Z").unwrap()),
        };
        let resolved = provider.resolve("truecolor", Some(&window)).await.unwrap();
        assert_eq!(resolved.granule_id.as_deref(), Some("g-zzz"));
    }

    #[tokio::test]
    async fn same_datetime_ties_break_by_granule_id() {
        let provider = provider(vec![
            granule("g-aaa", "2024-06-06T17:54:00Z", None),
            granule("g-zzz", "2024-06-06T17:54:00Z", None),
        ]);
        let resolved = provider.resolve("truecolor", None).await.unwrap();
        assert_eq!(resolved.layer.bands["b04"].as_str(), "g-zzz-b04.tif");
    }

    #[tokio::test]
    async fn no_granules_is_404_and_unknown_layer_is_404() {
        let provider = provider(Vec::new());
        let err = provider.resolve("truecolor", None).await.unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
        let err = provider.resolve("nope", None).await.unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn granule_missing_a_required_band_is_500() {
        let mut incomplete = granule("g-partial", "2024-06-06T17:54:00Z", None);
        incomplete.assets.remove("b03");
        let provider = provider(vec![incomplete]);
        let err = provider.resolve("truecolor", None).await.unwrap_err();
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
