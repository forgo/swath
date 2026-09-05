// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The dataset-creation surface (#196, ROADMAP item 6): **register, don't
//! manage** — the write side of the single-pane-of-glass claim.
//!
//! Two routes, catalog mode only, and cleanly unmountable as a unit (the
//! `--read-only` slice, #198, simply never merges this router):
//!
//! - `POST /datasets` — register a dataset definition: the config
//!   schema's dataset identity (`id`, `title`, `description`, `license`,
//!   `bands`), exposed over HTTP. **No layers here by design**: layers
//!   are authored through the openEO services surface (ADR 0010), so
//!   register-then-author is one composable flow, not two vocabularies.
//! - `POST /datasets/{datasetId}/granules` — register a granule: either a
//!   direct asset map (band → COG/manifest URL) or an **inline** STAC
//!   Item document (`{"stac_item": {…}}`, the #30 converter underneath —
//!   STAC stays hidden from Swath's own vocabulary, but STAC-speaking
//!   pipelines register without translation). The server never fetches
//!   remote metadata URLs — the client supplies the document (the #197
//!   panel fetches in-browser), so registration adds no SSRF surface.
//!
//! **Headers are validated before anything is accepted:** every asset is
//! `describe`d through the serving source stack; an unreadable or
//! malformed asset is an RFC 7807 `400` naming the asset and the reason —
//! never a registered-but-unservable granule. When the direct form omits
//! `bbox`, it is derived from the described raster's corner coordinates
//! reprojected to WGS84 — the footprint serving will actually use.
//!
//! **Extents are derived, not declared** (the ROADMAP item-15 deferral's
//! trigger): after every granule registration the dataset's spatial and
//! temporal extent is recomputed as the union over its registered
//! granules, so `/collections` documents state what the catalog actually
//! holds.
//!
//! Scope fence (the issue's words): Swath points at data where it lives.
//! No ETL, no file browser, no versioning UI, no deletes — a mis-registered
//! granule is superseded by re-registering it (upsert semantics).

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use serde::Deserialize;
use serde_json::{Value, json};
use swath_core::catalog::stac::granule_from_stac_item;
use swath_core::catalog::{
    Bbox, Catalog, Dataset, DatasetId, Datetime, Extent, Granule, GranuleAsset, GranuleId,
    GranuleQuery, TimeRange, temporal_interval,
};
use swath_core::crs::Crs;
use swath_core::raster::AssetRef;
use swath_core::reproject::Reproject;
use swath_core::source::RasterSource;

use crate::error::ApiError;
use crate::provider::CatalogLayers;

/// Shared state of the dataset-creation surface: the layer provider's
/// catalog (writes go where serving reads), the serving source stack
/// (asset-header validation reads through the same path tiles will), and
/// the reprojection port (bbox derivation).
pub struct DatasetsState<S, R, C> {
    provider: CatalogLayers<C>,
    source: S,
    reproject: R,
}

impl<S, R, C> DatasetsState<S, R, C> {
    /// Wires the surface over the shared provider and the render ports.
    pub fn new(provider: CatalogLayers<C>, source: S, reproject: R) -> Self {
        Self {
            provider,
            source,
            reproject,
        }
    }
}

/// The dataset-creation router (`POST /datasets`,
/// `POST /datasets/{datasetId}/granules`) — merged by catalog-mode serving,
/// and deliberately a separate router so a read-only deployment simply
/// omits it (#198).
pub fn datasets_router<S, R, C>(state: Arc<DatasetsState<S, R, C>>) -> axum::Router
where
    S: RasterSource + Send + Sync + 'static,
    R: Reproject + 'static,
    C: Catalog + 'static,
{
    axum::Router::new()
        .route("/datasets", post(create_dataset))
        .route("/datasets/{datasetId}/granules", post(create_granule))
        .with_state(state)
}

// --- POST /datasets ---

/// The registration body: the config schema's dataset identity, exposed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DatasetDefinition {
    /// URL-safe identifier (the STAC Collection id).
    id: String,
    /// Human-readable title.
    title: String,
    /// Narrative description.
    #[serde(default)]
    description: String,
    /// Data license (SPDX id, or `other`).
    #[serde(default = "default_license")]
    license: String,
    /// The band names granules of this dataset provide.
    bands: Vec<String>,
}

fn default_license() -> String {
    "other".to_owned()
}

/// `POST /datasets` — registers a new dataset. `409` when the id exists
/// (register, don't manage: re-definition is deliberate config territory,
/// never a drive-by overwrite).
async fn create_dataset<S, R, C>(
    State(app): State<Arc<DatasetsState<S, R, C>>>,
    Json(body): Json<DatasetDefinition>,
) -> Result<impl IntoResponse, ApiError>
where
    S: RasterSource + Send + Sync,
    R: Reproject,
    C: Catalog,
{
    if body.id.is_empty()
        || !body
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::bad_request(format!(
            "dataset id `{id}` is not URL-safe (ascii alphanumerics, `-`, `_`)",
            id = body.id
        )));
    }
    if body.bands.is_empty() {
        return Err(ApiError::bad_request(
            "a dataset must declare at least one band",
        ));
    }
    let id = DatasetId::new(&body.id);
    let catalog = self_catalog(&app);
    if catalog
        .get_dataset(&id)
        .await
        .map_err(|e| catalog_error(&e))?
        .is_some()
    {
        return Err(ApiError::conflict(format!(
            "dataset `{id}` already exists — registration never overwrites; \
             re-definition is config territory",
            id = body.id
        )));
    }

    let dataset = Dataset {
        id: id.clone(),
        title: body.title,
        description: body.description,
        license: body.license,
        // Honest empty state: the derived extent (module docs) replaces
        // this the moment the first granule registers; until then the
        // collection document declares a global box and an open interval.
        extent: Extent {
            bbox: Bbox {
                west: -180.0,
                south: -90.0,
                east: 180.0,
                north: 90.0,
            },
            interval: TimeRange::default(),
        },
        bands: body.bands.into_iter().collect(),
        // Layers arrive through the openEO services surface (module docs).
        layers: Vec::new(),
    };
    catalog
        .upsert_dataset(&dataset)
        .await
        .map_err(|e| catalog_error(&e))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": dataset.id.as_str() })),
    ))
}

// --- POST /datasets/{datasetId}/granules ---

/// One asset in the direct registration form.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AssetBody {
    /// Just the URI (a plain raster, the overwhelmingly common case).
    Href(String),
    /// URI plus an explicit kind (`cog` | `virtual`).
    Full {
        /// The asset URI/key, as serving will read it.
        href: String,
        /// What the URI points at.
        #[serde(default)]
        kind: swath_core::catalog::AssetKind,
    },
}

impl AssetBody {
    fn into_asset(self) -> GranuleAsset {
        match self {
            Self::Href(href) => GranuleAsset::raster(href),
            Self::Full { href, kind } => GranuleAsset {
                href: AssetRef::new(href),
                kind,
            },
        }
    }
}

/// The registration body: a direct asset map, or an inline STAC Item.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GranuleBody {
    /// Inline STAC Item document (`{"stac_item": {…}}`); mutually
    /// exclusive with the direct fields.
    #[serde(default)]
    stac_item: Option<Value>,
    /// Granule id (direct form; required there).
    #[serde(default)]
    id: Option<String>,
    /// Acquisition datetime, RFC 3339 UTC (direct form; required there).
    #[serde(default)]
    datetime: Option<String>,
    /// WGS84 footprint (direct form; derived from the first asset's
    /// header when omitted).
    #[serde(default)]
    bbox: Option<[f64; 4]>,
    /// Band name → asset (direct form; required there).
    #[serde(default)]
    assets: Option<BTreeMap<String, AssetBody>>,
}

/// `POST /datasets/{datasetId}/granules` — registers (or re-registers:
/// upsert) one granule, headers validated first, extents re-derived after.
async fn create_granule<S, R, C>(
    State(app): State<Arc<DatasetsState<S, R, C>>>,
    Path(dataset_id): Path<String>,
    Json(body): Json<GranuleBody>,
) -> Result<impl IntoResponse, ApiError>
where
    S: RasterSource + Send + Sync,
    R: Reproject,
    C: Catalog,
{
    let id = DatasetId::new(&dataset_id);
    let catalog = self_catalog(&app);
    let Some(dataset) = catalog
        .get_dataset(&id)
        .await
        .map_err(|e| catalog_error(&e))?
    else {
        return Err(ApiError::not_found(format!(
            "dataset `{dataset_id}` is not registered"
        )));
    };

    let mut granule = match (&body.stac_item, &body.assets) {
        (Some(item), None) => granule_from_stac_item(item).map_err(|e| {
            ApiError::bad_request(format!("stac_item does not describe a granule: {e}"))
        })?,
        (None, Some(_)) => direct_granule(&app, &id, body).await?,
        _ => {
            return Err(ApiError::bad_request(
                "provide exactly one of `stac_item` (inline document) or the \
                 direct fields (`id`, `datetime`, `assets`)",
            ));
        }
    };
    if granule.dataset != id {
        // The inline item's `collection` must agree with the path.
        return Err(ApiError::bad_request(format!(
            "stac_item collection `{item}` does not match dataset `{dataset_id}`",
            item = granule.dataset.as_str(),
        )));
    }

    // Band vocabulary: every asset key must be a declared dataset band —
    // a granule whose assets no layer could ever resolve is a mistake
    // worth refusing loudly at the door.
    for band in granule.assets.keys() {
        if !dataset.bands.contains(band) {
            return Err(ApiError::bad_request(format!(
                "asset band `{band}` is not in dataset `{dataset_id}`'s declared \
                 bands {bands:?}",
                bands = dataset.bands,
            )));
        }
    }

    // Header validation through the serving source stack: what registers
    // is what serves.
    for (band, asset) in &granule.assets {
        app.source.describe(&asset.href).await.map_err(|e| {
            ApiError::bad_request(format!(
                "asset `{band}` ({href}) failed header validation: {e}",
                href = asset.href,
            ))
        })?;
    }

    granule.ingested_at = None; // registered outside the event path (docs)
    catalog
        .upsert_granules(std::slice::from_ref(&granule))
        .await
        .map_err(|e| catalog_error(&e))?;

    // Derived extents (module docs): the union over registered granules.
    derive_extent(catalog, &dataset).await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": granule.id.as_str(),
            "dataset": granule.dataset.as_str(),
        })),
    ))
}

/// Builds a granule from the direct form, deriving `bbox` from the first
/// asset's header when omitted.
async fn direct_granule<S, R, C>(
    app: &DatasetsState<S, R, C>,
    dataset: &DatasetId,
    body: GranuleBody,
) -> Result<Granule, ApiError>
where
    S: RasterSource + Send + Sync,
    R: Reproject,
{
    let missing = |field: &str| ApiError::bad_request(format!("direct form requires `{field}`"));
    let id = body.id.ok_or_else(|| missing("id"))?;
    let datetime = body.datetime.ok_or_else(|| missing("datetime"))?;
    let datetime =
        Datetime::new(&datetime).map_err(|e| ApiError::bad_request(format!("`datetime`: {e}")))?;
    let assets: BTreeMap<String, GranuleAsset> = body
        .assets
        .ok_or_else(|| missing("assets"))?
        .into_iter()
        .map(|(band, asset)| (band, asset.into_asset()))
        .collect();
    if assets.is_empty() {
        return Err(ApiError::bad_request(
            "`assets` must name at least one band",
        ));
    }

    let bbox = if let Some([west, south, east, north]) = body.bbox {
        Bbox {
            west,
            south,
            east,
            north,
        }
    } else {
        let first = assets.values().next().expect("non-empty checked above");
        derived_bbox(app, &first.href).await?
    };

    Ok(Granule {
        id: GranuleId::new(id),
        dataset: dataset.clone(),
        bbox,
        datetime,
        assets,
        ingested_at: None,
        properties: BTreeMap::new(),
    })
}

/// The WGS84 bounding box of an asset, from its described header: the
/// four raster corners reprojected to EPSG:4326 — the footprint serving
/// will actually intersect against.
async fn derived_bbox<S, R, C>(
    app: &DatasetsState<S, R, C>,
    asset: &AssetRef,
) -> Result<Bbox, ApiError>
where
    S: RasterSource + Send + Sync,
    R: Reproject,
{
    let info = app.source.describe(asset).await.map_err(|e| {
        ApiError::bad_request(format!("bbox derivation: asset {asset} failed: {e}"))
    })?;
    let transform = app
        .reproject
        .transformer(&info.crs, &Crs::WGS84)
        .map_err(|e| {
            ApiError::bad_request(format!(
                "bbox derivation: no transform {crs} -> EPSG:4326: {e}",
                crs = info.crs,
            ))
        })?;
    #[allow(
        clippy::cast_precision_loss,
        reason = "raster dims far below 2^52; corner coordinates"
    )]
    let (w, h) = (info.width as f64, info.height as f64);
    let mut bbox = Bbox {
        west: f64::INFINITY,
        south: f64::INFINITY,
        east: f64::NEG_INFINITY,
        north: f64::NEG_INFINITY,
    };
    for (col, row) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
        let (x, y) = info.transform.pixel_to_crs(col, row);
        let (lon, lat) = transform.transform(x, y).map_err(|e| {
            ApiError::bad_request(format!(
                "bbox derivation: corner ({col}, {row}) does not reproject: {e}"
            ))
        })?;
        bbox.west = bbox.west.min(lon);
        bbox.south = bbox.south.min(lat);
        bbox.east = bbox.east.max(lon);
        bbox.north = bbox.north.max(lat);
    }
    Ok(bbox)
}

/// Recomputes the dataset's extent as the union over its granules and
/// persists it — the derived-extent contract (module docs).
async fn derive_extent<C: Catalog>(catalog: &C, dataset: &Dataset) -> Result<(), ApiError> {
    let granules = catalog
        .find_granules(&dataset.id, &GranuleQuery::default())
        .await
        .map_err(|e| catalog_error(&e))?;
    let mut bbox: Option<Bbox> = None;
    for granule in &granules {
        bbox = Some(match bbox {
            None => granule.bbox,
            Some(b) => Bbox {
                west: b.west.min(granule.bbox.west),
                south: b.south.min(granule.bbox.south),
                east: b.east.max(granule.bbox.east),
                north: b.north.max(granule.bbox.north),
            },
        });
    }
    let Some(bbox) = bbox else {
        return Ok(()); // no granules: leave the declared empty state
    };
    let mut updated = dataset.clone();
    updated.extent = Extent {
        bbox,
        // The same derivation serve-start re-runs (the shared helper).
        interval: temporal_interval(&granules),
    };
    catalog
        .upsert_dataset(&updated)
        .await
        .map_err(|e| catalog_error(&e))
}

/// The provider's catalog handle (writes land where serving reads).
fn self_catalog<S, R, C>(app: &DatasetsState<S, R, C>) -> &C {
    app.provider.catalog()
}

/// Catalog failures are 500s: the request was well-formed; the machinery
/// was not.
fn catalog_error(err: &swath_core::catalog::CatalogError) -> ApiError {
    ApiError::internal(format!("catalog failure: {err}"))
}
