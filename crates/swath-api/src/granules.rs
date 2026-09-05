// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Granule browsing (issue #107): `GET /datasets/{datasetId}/granules`,
//! a read-only window onto what has been ingested.
//!
//! [`Catalog::find_granules`] has served the tile path since #31; this
//! module exposes the same query over HTTP so the UI and operators can
//! see a dataset's granules without a database session. Like the openEO
//! surface, the router is **merged beside the OGC one in catalog mode
//! only** — static-registry serving has no catalog, so the route simply
//! does not exist there.
//!
//! The response is a typed page: each granule carries its id, footprint
//! (`bbox`, CRS84 `[west, south, east, north]`), acquisition `datetime`,
//! ingest timestamp when known, and its band → asset map (the source
//! refs). Pagination is `limit`/`offset` with `numberMatched` /
//! `numberReturned` and a `next` link while more remain, over a total
//! newest-first order (acquisition datetime descending, ties by id) so
//! pages are stable between requests against an unchanged catalog.
//!
//! Errors are the crate's uniform RFC 7807 taxonomy ([`ApiError`], as the
//! tiles routes): unknown dataset → 404, malformed `bbox` / `datetime` /
//! `limit` / `offset` → 400, catalog backend failure → 500.
//!
//! **No host paths leak here by construction**: every response field is
//! either catalog domain data (ids, degrees, RFC 3339 timestamps) or an
//! asset `href` passed through verbatim — and hrefs are *store keys*
//! resolved against the configured store root (`swath-cli`'s composite
//! source), never absolute filesystem paths of the serving host. The
//! integration suite additionally asserts the fixtures response contains
//! no absolute path.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use swath_core::catalog::{
    AssetKind, Bbox, Catalog, CatalogError, DatasetId, Granule, GranuleQuery, TimeRange,
};

use crate::error::ApiError;
use crate::model::Link;

/// Granules returned when the request names no `limit`.
pub const DEFAULT_LIMIT: usize = 100;

/// Largest admissible `limit` — a browsing page, not a bulk export.
pub const MAX_LIMIT: usize = 1000;

/// Everything the granule handlers need: the catalog queried and the base
/// URL pagination links are minted under (trailing slashes trimmed, as in
/// [`ApiState::new`](crate::ApiState::new)).
#[derive(Debug)]
pub struct GranulesState<C> {
    catalog: C,
    base_url: String,
}

impl<C> GranulesState<C> {
    /// Wires the surface over `catalog`.
    pub fn new(catalog: C, base_url: impl Into<String>) -> Self {
        let mut base_url: String = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        Self { catalog, base_url }
    }
}

/// The granule-browsing router over `state`, to be merged with the OGC
/// tiles router in catalog mode (the wiring `swath serve --catalog` does,
/// beside [`openeo_router`](crate::openeo_router)).
pub fn granules_router<C>(state: Arc<GranulesState<C>>) -> axum::Router
where
    C: Catalog + 'static,
{
    axum::Router::new()
        .route("/datasets/{datasetId}/granules", get(granules))
        .with_state(state)
}

// --- The response shape (contractual, pinned by snapshot test) ---

/// One asset of a granule: the store key the serving path reads, plus
/// what kind of thing it points at.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GranuleAssetItem {
    /// The asset reference — a key under the deployment's store root
    /// (or a full object-store URI), exactly as cataloged. Never a
    /// serving-host filesystem path.
    pub href: String,
    /// `"raster"` (directly readable, COG) or `"virtual_cube"` (a virtual
    /// reference manifest, ADR 0006).
    pub kind: &'static str,
}

/// One granule of the page.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct GranuleItem {
    /// Granule identifier, unique within the dataset.
    pub id: String,
    /// WGS84 footprint, CRS84 order: `[west, south, east, north]`.
    pub bbox: [f64; 4],
    /// Acquisition time (RFC 3339 UTC).
    pub datetime: String,
    /// When Swath ingested the granule — absent for granules registered
    /// outside the event path.
    #[serde(rename = "ingestedAt", skip_serializing_if = "Option::is_none")]
    pub ingested_at: Option<String>,
    /// Band name → asset (the source refs a layer's plan resolves
    /// against), in band-name order.
    pub assets: BTreeMap<String, GranuleAssetItem>,
    /// Every other STAC property the item carried, verbatim and opaque
    /// (ADR 0029) — `eo:cloud_cover`, `platform`, `proj:epsg` and the rest.
    ///
    /// Omitted when empty, so a granule without foreign properties serves
    /// exactly the bytes it did before this field existed: the shape change
    /// is additive and no client sees a field disappear.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, serde_json::Value>,
}

/// The granule page: `granules` plus pagination bookkeeping.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct GranuleList {
    /// The page, newest first (acquisition datetime descending, ties
    /// broken by id — the same total order everywhere in this crate).
    pub granules: Vec<GranuleItem>,
    /// Granules matching the filter, across all pages.
    #[serde(rename = "numberMatched")]
    pub number_matched: usize,
    /// Granules in this response.
    #[serde(rename = "numberReturned")]
    pub number_returned: usize,
    /// `self`, and `next` while more pages remain.
    pub links: Vec<Link>,
}

impl From<Granule> for GranuleItem {
    fn from(granule: Granule) -> Self {
        Self {
            id: granule.id.to_string(),
            bbox: granule.bbox.to_array(),
            datetime: granule.datetime.to_string(),
            ingested_at: granule.ingested_at.map(|t| t.to_string()),
            properties: granule.properties,
            assets: granule
                .assets
                .into_iter()
                .map(|(band, asset)| {
                    (
                        band,
                        GranuleAssetItem {
                            href: asset.href.to_string(),
                            kind: match asset.kind {
                                AssetKind::Raster => "raster",
                                AssetKind::VirtualCube => "virtual_cube",
                                // `AssetKind` is non_exhaustive: name new
                                // kinds honestly when they arrive.
                                _ => "unknown",
                            },
                        },
                    )
                })
                .collect(),
        }
    }
}

// --- The handler ---

/// `GET /datasets/{datasetId}/granules` — the granules of a dataset,
/// newest first, as a typed page (issue #107). Query parameters:
/// `bbox=west,south,east,north` (CRS84 degrees, footprint intersection),
/// `datetime=instant | start/end | ../end | start/..` (RFC 3339 UTC,
/// inclusive), `limit` (1..=[`MAX_LIMIT`], default [`DEFAULT_LIMIT`]),
/// `offset` (default 0). Unknown parameters are ignored.
///
/// Taxonomy (matching the tiles routes): a dataset the catalog does not
/// contain addresses a resource that does not exist → 404; a parameter
/// that does not parse is a malformed request → 400; both carry the
/// RFC 7807 exception body. An existing dataset with no matching
/// granules is an empty 200 page, not an error.
async fn granules<C>(
    State(app): State<Arc<GranulesState<C>>>,
    Path(dataset_id): Path<String>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<GranuleList>, ApiError>
where
    C: Catalog + 'static,
{
    let params = GranulesParams::parse(&raw)?;
    let id = DatasetId::new(&dataset_id);

    // Existence first: `find_granules` on an unknown dataset may be an
    // empty set on some backends, and "no such dataset" must be 404, not
    // an empty page.
    app.catalog
        .get_dataset(&id)
        .await
        .map_err(|err| catalog_error(&id, &err))?
        .ok_or_else(|| ApiError::not_found(format!("no dataset `{dataset_id}`")))?;

    let mut granules = app
        .catalog
        .find_granules(&id, &params.filter)
        .await
        .map_err(|err| catalog_error(&id, &err))?;
    // Newest first, ties by id — a total order, so pages are stable.
    granules.sort_by(|a, b| {
        (b.datetime.to_unix_millis(), &b.id).cmp(&(a.datetime.to_unix_millis(), &a.id))
    });

    let number_matched = granules.len();
    let page: Vec<GranuleItem> = granules
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .map(GranuleItem::from)
        .collect();
    let number_returned = page.len();

    let mut links = vec![
        Link::new(
            params.page_url(&app.base_url, &dataset_id, params.offset),
            "self",
        )
        .media_type("application/json")
        .title(format!("Granules of dataset {dataset_id}")),
    ];
    if params.offset + number_returned < number_matched {
        links.push(
            Link::new(
                params.page_url(&app.base_url, &dataset_id, params.offset + params.limit),
                "next",
            )
            .media_type("application/json")
            .title("Next page"),
        );
    }

    Ok(Json(GranuleList {
        granules: page,
        number_matched,
        number_returned,
        links,
    }))
}

/// Catalog failures, translated exactly as layer resolution translates
/// them: a missing dataset is 404, everything else an honest 500.
fn catalog_error(dataset: &DatasetId, err: &CatalogError) -> ApiError {
    match err {
        CatalogError::DatasetNotFound { .. } => {
            ApiError::not_found(format!("no dataset `{dataset}`"))
        }
        other => ApiError::internal(format!("catalog lookup for `{dataset}` failed: {other}")),
    }
}

// --- Parameter parsing (translation only — no domain logic) ---

/// The parsed request: the domain filter plus the page window, and the
/// raw filter strings kept verbatim for reconstructing page links.
#[derive(Debug, Clone, PartialEq)]
struct GranulesParams {
    filter: GranuleQuery,
    limit: usize,
    offset: usize,
    bbox_raw: Option<String>,
    datetime_raw: Option<String>,
}

impl GranulesParams {
    /// Parses the query map; every failure is a 400 naming the parameter.
    fn parse(raw: &HashMap<String, String>) -> Result<Self, ApiError> {
        let bbox = raw.get("bbox").map(|s| parse_bbox(s)).transpose()?;
        let datetime = raw.get("datetime").map(|s| parse_datetime(s)).transpose()?;
        let limit = match raw.get("limit") {
            None => DEFAULT_LIMIT,
            Some(s) => s
                .parse::<usize>()
                .ok()
                .filter(|limit| (1..=MAX_LIMIT).contains(limit))
                .ok_or_else(|| {
                    ApiError::bad_request(format!(
                        "limit `{s}` is not an integer in 1..={MAX_LIMIT}"
                    ))
                })?,
        };
        let offset = match raw.get("offset") {
            None => 0,
            Some(s) => s.parse::<usize>().map_err(|_| {
                ApiError::bad_request(format!("offset `{s}` is not a non-negative integer"))
            })?,
        };
        Ok(Self {
            filter: GranuleQuery { bbox, datetime },
            limit,
            offset,
            bbox_raw: raw.get("bbox").cloned(),
            datetime_raw: raw.get("datetime").cloned(),
        })
    }

    /// The URL of the page starting at `offset`, filter echoed verbatim.
    fn page_url(&self, base: &str, dataset_id: &str, offset: usize) -> String {
        let mut params: Vec<String> = Vec::new();
        if let Some(bbox) = &self.bbox_raw {
            params.push(format!("bbox={bbox}"));
        }
        if let Some(datetime) = &self.datetime_raw {
            params.push(format!("datetime={datetime}"));
        }
        params.push(format!("limit={limit}", limit = self.limit));
        params.push(format!("offset={offset}"));
        format!(
            "{base}/datasets/{dataset_id}/granules?{query}",
            query = params.join("&"),
        )
    }
}

/// Parses `bbox=west,south,east,north`: four finite CRS84 degrees, south
/// not above north. `west > east` is allowed (the STAC/GeoJSON
/// antimeridian convention the domain [`Bbox`] documents).
fn parse_bbox(raw: &str) -> Result<Bbox, ApiError> {
    let malformed = || {
        ApiError::bad_request(format!(
            "bbox `{raw}` is not `west,south,east,north` (four finite CRS84 degrees)"
        ))
    };
    let mut values = [0.0_f64; 4];
    let mut parts = raw.split(',');
    for value in &mut values {
        *value = parts
            .next()
            .and_then(|part| part.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .ok_or_else(malformed)?;
    }
    if parts.next().is_some() {
        return Err(malformed());
    }
    let bbox = Bbox::from_array(values);
    if bbox.south > bbox.north {
        return Err(ApiError::bad_request(format!(
            "bbox `{raw}` has south ({south}) above north ({north})",
            south = bbox.south,
            north = bbox.north,
        )));
    }
    Ok(bbox)
}

/// Parses `datetime` through the shared OGC grammar ([`crate::temporal`],
/// also the tiles route's `datetime`): here an instant is an inclusive
/// point-range `[t, t]` — "granules acquired at `t`" — a *filter*, unlike
/// the tiles route's latest-at-or-before *resolution* of the same form.
fn parse_datetime(raw: &str) -> Result<TimeRange, ApiError> {
    Ok(match crate::temporal::parse_datetime_param(raw)? {
        crate::temporal::DatetimeParam::Instant(point) => TimeRange {
            start: Some(point.clone()),
            end: Some(point),
        },
        crate::temporal::DatetimeParam::Interval(range) => range,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::http::StatusCode;

    use super::{DEFAULT_LIMIT, GranulesParams, parse_bbox, parse_datetime};

    fn params(pairs: &[(&str, &str)]) -> Result<GranulesParams, crate::ApiError> {
        let raw: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        GranulesParams::parse(&raw)
    }

    #[test]
    fn bbox_taxonomy() {
        let bbox = parse_bbox("-106.1,39.2,-105.9,39.4").unwrap();
        // Bitwise-exact float comparison is the point: the values must
        // round-trip the parse untouched.
        assert!(
            bbox.to_array()
                .iter()
                .zip([-106.1_f64, 39.2, -105.9, 39.4])
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "parsed {parsed:?}",
            parsed = bbox.to_array(),
        );
        // Antimeridian crossing (west > east) is a valid box...
        assert!(parse_bbox("179.5,-1,-179.5,1").is_ok());
        // ...malformed shapes are 400.
        for bad in [
            "",
            "1,2,3",
            "1,2,3,4,5",
            "a,2,3,4",
            "1,2,3,inf",
            "1,2,3,NaN",
            "0,10,1,-10", // south above north
        ] {
            assert_eq!(
                parse_bbox(bad).unwrap_err().status,
                StatusCode::BAD_REQUEST,
                "bbox `{bad}`"
            );
        }
    }

    #[test]
    fn datetime_forms() {
        let single = parse_datetime("2024-06-06T17:54:00Z").unwrap();
        assert_eq!(single.start, single.end);
        let range = parse_datetime("2024-06-01T00:00:00Z/2024-06-30T23:59:59Z").unwrap();
        assert!(range.start.is_some() && range.end.is_some());
        assert!(
            parse_datetime("../2024-06-30T23:59:59Z")
                .unwrap()
                .start
                .is_none()
        );
        assert!(
            parse_datetime("2024-06-01T00:00:00Z/..")
                .unwrap()
                .end
                .is_none()
        );
        for bad in [
            "../..",
            "yesterday",
            "2024-06-06",
            "2024-06-06T17:54:00+00:00",
        ] {
            assert_eq!(
                parse_datetime(bad).unwrap_err().status,
                StatusCode::BAD_REQUEST,
                "datetime `{bad}`"
            );
        }
    }

    #[test]
    fn limit_and_offset_taxonomy() {
        assert_eq!(params(&[]).unwrap().limit, DEFAULT_LIMIT);
        assert_eq!(params(&[]).unwrap().offset, 0);
        assert_eq!(params(&[("limit", "1000")]).unwrap().limit, 1000);
        for bad in [
            ("limit", "0"),
            ("limit", "1001"),
            ("limit", "x"),
            ("offset", "-1"),
        ] {
            assert_eq!(
                params(&[bad]).unwrap_err().status,
                StatusCode::BAD_REQUEST,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn page_urls_echo_the_filter() {
        let parsed = params(&[("bbox", "-106,39,-105,40"), ("limit", "2")]).unwrap();
        assert_eq!(
            parsed.page_url("http://localhost", "hls-s30", 2),
            "http://localhost/datasets/hls-s30/granules?bbox=-106,39,-105,40&limit=2&offset=2"
        );
    }
}
