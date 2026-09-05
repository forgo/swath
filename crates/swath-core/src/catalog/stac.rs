// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Pure `Dataset`/`Granule` ⇄ STAC JSON converters.
//!
//! The lossless-mapping half of the catalog contract
//! (`docs/design/catalog-domain.md` §3): [`dataset_to_stac_collection`] /
//! [`granule_to_stac_item`] emit valid STAC 1.1.0 documents with all
//! swath-owned state under `swath:`-prefixed fields, and the inverses recover
//! the domain values exactly. The normative property — domain → STAC → domain
//! is the identity — is enforced by the proptest suite in
//! `tests/catalog_roundtrip.rs`.
//!
//! These are pure `serde_json::Value` transforms: no I/O, no STAC types in any
//! signature outside this module. Adapters (pgstac first) call them at their
//! storage boundary; nothing above the [`Catalog`](crate::catalog::Catalog)
//! port ever sees the documents.
//!
//! The inverses are **strict**: a document missing swath-required fields (a
//! foreign Collection in a shared database, say) fails loudly with a
//! [`StacError`] naming the JSON path, rather than half-converting.

use serde_json::{Map, Value, json};

use super::{
    AssetKind, Bbox, Dataset, DatasetId, Datetime, Extent, Granule, GranuleAsset, GranuleId, Layer,
    TimeRange,
};
use crate::raster::AssetRef;

/// The STAC version emitted, and the only one accepted back (design doc §3).
pub const STAC_VERSION: &str = "1.1.0";

/// What can go wrong converting a STAC document back to the domain.
///
/// Every variant names the JSON path (dotted, e.g. `properties.datetime`) so
/// a failing document is diagnosable without reproducing the conversion.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StacError {
    /// A required field is absent.
    #[error("missing required field `{path}`")]
    MissingField {
        /// Dotted JSON path of the absent field.
        path: String,
    },

    /// A field exists but holds the wrong JSON type.
    #[error("field `{path}` is not of the expected type ({expected})")]
    WrongType {
        /// Dotted JSON path of the offending field.
        path: String,
        /// The JSON type/shape that was expected.
        expected: &'static str,
    },

    /// A field is well-typed but its value is invalid.
    #[error("field `{path}` has an invalid value: {detail}")]
    InvalidValue {
        /// Dotted JSON path of the offending field.
        path: String,
        /// Why the value was rejected.
        detail: String,
    },
}

/// Serializes a [`Dataset`] as a STAC 1.1.0 Collection document.
///
/// Swath-owned fields ride under `swath:` prefixes (`swath:bands`,
/// `swath:layers`); everything else is plain STAC, so third-party tooling
/// reads the document as an ordinary Collection. `links` is emitted empty —
/// links are catalog plumbing owned by the storage/API layers, not domain
/// state.
#[must_use]
pub fn dataset_to_stac_collection(dataset: &Dataset) -> Value {
    let interval_start = dataset.extent.interval.start.as_ref().map(Datetime::as_str);
    let interval_end = dataset.extent.interval.end.as_ref().map(Datetime::as_str);
    json!({
        "type": "Collection",
        "stac_version": STAC_VERSION,
        "id": dataset.id.as_str(),
        "title": dataset.title,
        "description": dataset.description,
        "license": dataset.license,
        "extent": {
            "spatial": { "bbox": [dataset.extent.bbox.to_array()] },
            "temporal": { "interval": [[interval_start, interval_end]] },
        },
        "links": [],
        "swath:bands": dataset.bands,
        // Infallible: `Layer` is a plain struct tree (string-keyed, no
        // fallible Serialize impls).
        "swath:layers": serde_json::to_value(&dataset.layers)
            .expect("Layer serialization is infallible"),
    })
}

/// Recovers a [`Dataset`] from a STAC Collection document — the exact inverse
/// of [`dataset_to_stac_collection`] on documents Swath wrote.
///
/// # Errors
///
/// [`StacError`] when the document is not a `Collection`, is not STAC
/// [`STAC_VERSION`], or lacks/mistypes any mapped field — including the
/// `swath:` fields, whose absence is how foreign Collections are detected and
/// rejected (design doc §3).
pub fn dataset_from_stac_collection(doc: &Value) -> Result<Dataset, StacError> {
    let obj = as_object(doc, "")?;
    expect_const(obj, "type", "Collection")?;
    expect_const(obj, "stac_version", STAC_VERSION)?;

    let extent = as_object(get_at(obj, "extent", "extent")?, "extent")?;
    let spatial = as_object(
        get_at(extent, "spatial", "extent.spatial")?,
        "extent.spatial",
    )?;
    let bbox_value = get_at(spatial, "bbox", "extent.spatial.bbox")?;
    let first_bbox = first_element(bbox_value, "extent.spatial.bbox")?;
    let bbox = bbox_from_value(first_bbox, "extent.spatial.bbox[0]")?;

    let temporal = as_object(
        get_at(extent, "temporal", "extent.temporal")?,
        "extent.temporal",
    )?;
    let interval_value = get_at(temporal, "interval", "extent.temporal.interval")?;
    let first_interval = first_element(interval_value, "extent.temporal.interval")?;
    let interval = time_range_from_value(first_interval, "extent.temporal.interval[0]")?;

    let bands = string_array(get(obj, "swath:bands")?, "swath:bands")?
        .into_iter()
        .collect();
    let layers: Vec<Layer> =
        serde_json::from_value(get(obj, "swath:layers")?.clone()).map_err(|e| {
            StacError::InvalidValue {
                path: "swath:layers".to_owned(),
                detail: e.to_string(),
            }
        })?;

    Ok(Dataset {
        id: DatasetId::new(string(obj, "id")?),
        title: string(obj, "title")?,
        description: string(obj, "description")?,
        license: string(obj, "license")?,
        extent: Extent { bbox, interval },
        bands,
        layers,
    })
}

/// Serializes a [`Granule`] as a STAC 1.1.0 Item document.
///
/// `geometry` is derived from the granule's bbox (the box's closed
/// counterclockwise polygon ring) — deterministically, so the round trip
/// through [`granule_from_stac_item`] (which reads only `bbox`) is exact.
/// Assets carry `href` (plus `swath:kind` for non-raster assets); the asset
/// key is the band name.
#[must_use]
pub fn granule_to_stac_item(granule: &Granule) -> Value {
    let Bbox {
        west,
        south,
        east,
        north,
    } = granule.bbox;
    let assets: Map<String, Value> = granule
        .assets
        .iter()
        .map(|(band, asset)| {
            let mut doc = Map::new();
            doc.insert("href".to_owned(), json!(asset.href.as_str()));
            if asset.kind != AssetKind::Raster {
                // Swath-owned asset metadata rides under a namespaced key
                // (design doc §3); the default kind is omitted so
                // plain-raster documents keep their pre-#40 bytes.
                doc.insert(
                    "swath:kind".to_owned(),
                    serde_json::to_value(asset.kind).expect("AssetKind serializes"),
                );
            }
            (band.clone(), Value::Object(doc))
        })
        .collect();
    let mut properties = Map::new();
    // The passthrough goes out first, so a projected key can never be
    // shadowed by a stale carried copy — `is_projected_property` keeps them
    // out on the way in, and writing ours last makes that belt-and-braces.
    for (key, value) in &granule.properties {
        properties.insert(key.clone(), value.clone());
    }
    properties.insert("datetime".to_owned(), json!(granule.datetime.as_str()));
    if let Some(ingested_at) = &granule.ingested_at {
        // Granule-level swath-owned metadata rides under a namespaced
        // property, exactly as the design doc reserved (§3).
        properties.insert("swath:ingested_at".to_owned(), json!(ingested_at.as_str()));
    }
    json!({
        "type": "Feature",
        "stac_version": STAC_VERSION,
        "id": granule.id.as_str(),
        "collection": granule.dataset.as_str(),
        "geometry": {
            "type": "Polygon",
            "coordinates": [[
                [west, south],
                [east, south],
                [east, north],
                [west, north],
                [west, south],
            ]],
        },
        "bbox": granule.bbox.to_array(),
        "properties": properties,
        "assets": assets,
    })
}

/// Recovers a [`Granule`] from a STAC Item document — the exact inverse of
/// [`granule_to_stac_item`] on documents Swath wrote.
///
/// `geometry` is ignored (`bbox` is the source of truth) and asset fields
/// beyond `href`/`swath:kind` are ignored; both are deterministic
/// emissions/no-ops on swath-written documents, so identity holds (design
/// doc §3).
///
/// # Errors
///
/// [`StacError`] when the document is not a `Feature`, is not STAC
/// [`STAC_VERSION`], or lacks/mistypes `id`, `collection`, `bbox`,
/// `properties.datetime`, or any asset's `href`.
pub fn granule_from_stac_item(doc: &Value) -> Result<Granule, StacError> {
    let obj = as_object(doc, "")?;
    expect_const(obj, "type", "Feature")?;
    expect_const(obj, "stac_version", STAC_VERSION)?;

    let bbox = bbox_from_value(get(obj, "bbox")?, "bbox")?;

    let properties = as_object(get(obj, "properties")?, "properties")?;
    let datetime_str = properties
        .get("datetime")
        .ok_or_else(|| StacError::MissingField {
            path: "properties.datetime".to_owned(),
        })?
        .as_str()
        .ok_or(StacError::WrongType {
            path: "properties.datetime".to_owned(),
            expected: "string",
        })?;
    let datetime = Datetime::new(datetime_str).map_err(|_| StacError::InvalidValue {
        path: "properties.datetime".to_owned(),
        detail: format!("`{datetime_str}` is not an RFC 3339 UTC (Z) timestamp"),
    })?;

    // Optional swath-owned ingest timestamp; when present it must be valid.
    let ingested_at = properties
        .get("swath:ingested_at")
        .map(|value| {
            let path = "properties.swath:ingested_at";
            let text = value.as_str().ok_or(StacError::WrongType {
                path: path.to_owned(),
                expected: "string",
            })?;
            Datetime::new(text).map_err(|_| StacError::InvalidValue {
                path: path.to_owned(),
                detail: format!("`{text}` is not an RFC 3339 UTC (Z) timestamp"),
            })
        })
        .transpose()?;

    let assets_obj = as_object(get(obj, "assets")?, "assets")?;
    let mut assets = std::collections::BTreeMap::new();
    for (band, asset) in assets_obj {
        let path = format!("assets.{band}");
        let fields = as_object(asset, &path)?;
        let href = fields
            .get("href")
            .ok_or_else(|| StacError::MissingField {
                path: format!("{path}.href"),
            })?
            .as_str()
            .ok_or_else(|| StacError::WrongType {
                path: format!("{path}.href"),
                expected: "string",
            })?;
        // Optional swath-owned kind; absent = plain raster, present-but-
        // unknown = loud error (a kind this version cannot serve correctly
        // must not be silently degraded to raster).
        let kind = match fields.get("swath:kind") {
            None => AssetKind::default(),
            Some(value) => {
                serde_json::from_value(value.clone()).map_err(|_| StacError::InvalidValue {
                    path: format!("{path}.swath:kind"),
                    detail: format!("`{value}` is not a known asset kind"),
                })?
            }
        };
        assets.insert(
            band.clone(),
            GranuleAsset {
                href: AssetRef::new(href),
                kind,
            },
        );
    }

    // Everything else the item carried, verbatim (#407). The keys Swath
    // owns or projects onto its own fields are excluded so nothing is
    // stored twice — one authority per fact — and so a round-trip cannot
    // resurrect a stale copy of a field the domain has since changed.
    let carried: std::collections::BTreeMap<String, Value> = properties
        .iter()
        .filter(|(key, _)| !is_projected_property(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();

    Ok(Granule {
        id: GranuleId::new(string(obj, "id")?),
        dataset: DatasetId::new(string(obj, "collection")?),
        bbox,
        datetime,
        assets,
        ingested_at,
        properties: carried,
    })
}

/// Whether a STAC property is one Swath projects onto a domain field, and
/// therefore must not also ride in the opaque passthrough (#407).
///
/// `datetime` becomes `Granule::datetime`; the `swath:` namespace is ours
/// (design doc §3) and every key in it is either projected today or
/// reserved for projection. Keeping a second copy in `properties` would
/// give two answers to one question the moment either changed.
fn is_projected_property(key: &str) -> bool {
    key == "datetime" || key.starts_with("swath:")
}

// --- small strict-access helpers; every failure names the JSON path ---

fn as_object<'a>(v: &'a Value, path: &str) -> Result<&'a Map<String, Value>, StacError> {
    v.as_object().ok_or_else(|| StacError::WrongType {
        path: path.to_owned(),
        expected: "object",
    })
}

fn get<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<&'a Value, StacError> {
    get_at(obj, key, key)
}

/// Like [`get`], but reporting a full dotted `path` for nested lookups.
fn get_at<'a>(obj: &'a Map<String, Value>, key: &str, path: &str) -> Result<&'a Value, StacError> {
    obj.get(key).ok_or_else(|| StacError::MissingField {
        path: path.to_owned(),
    })
}

fn string(obj: &Map<String, Value>, key: &str) -> Result<String, StacError> {
    get(obj, key)?
        .as_str()
        .map(str::to_owned)
        .ok_or(StacError::WrongType {
            path: key.to_owned(),
            expected: "string",
        })
}

fn expect_const(obj: &Map<String, Value>, key: &str, expected: &str) -> Result<(), StacError> {
    let actual = string(obj, key)?;
    if actual == expected {
        Ok(())
    } else {
        Err(StacError::InvalidValue {
            path: key.to_owned(),
            detail: format!("expected `{expected}`, found `{actual}`"),
        })
    }
}

fn first_element<'a>(v: &'a Value, path: &str) -> Result<&'a Value, StacError> {
    v.as_array()
        .ok_or(StacError::WrongType {
            path: path.to_owned(),
            expected: "array",
        })?
        .first()
        .ok_or_else(|| StacError::InvalidValue {
            path: path.to_owned(),
            detail: "array is empty".to_owned(),
        })
}

fn bbox_from_value(v: &Value, path: &str) -> Result<Bbox, StacError> {
    let arr = v.as_array().ok_or(StacError::WrongType {
        path: path.to_owned(),
        expected: "array of 4 numbers",
    })?;
    if arr.len() != 4 {
        return Err(StacError::InvalidValue {
            path: path.to_owned(),
            detail: format!("expected 4 elements, found {}", arr.len()),
        });
    }
    let mut nums = [0.0_f64; 4];
    for (i, value) in arr.iter().enumerate() {
        nums[i] = value.as_f64().ok_or(StacError::WrongType {
            path: path.to_owned(),
            expected: "array of 4 numbers",
        })?;
    }
    Ok(Bbox::from_array(nums))
}

fn time_range_from_value(v: &Value, path: &str) -> Result<TimeRange, StacError> {
    let arr = v.as_array().ok_or(StacError::WrongType {
        path: path.to_owned(),
        expected: "array of 2 (string or null)",
    })?;
    if arr.len() != 2 {
        return Err(StacError::InvalidValue {
            path: path.to_owned(),
            detail: format!("expected 2 elements, found {}", arr.len()),
        });
    }
    let bound = |v: &Value, which: &str| -> Result<Option<Datetime>, StacError> {
        let sub_path = format!("{path}.{which}");
        match v {
            Value::Null => Ok(None),
            Value::String(s) => {
                Datetime::new(s.clone())
                    .map(Some)
                    .map_err(|_| StacError::InvalidValue {
                        path: sub_path,
                        detail: format!("`{s}` is not an RFC 3339 UTC (Z) timestamp"),
                    })
            }
            _ => Err(StacError::WrongType {
                path: sub_path,
                expected: "string or null",
            }),
        }
    };
    Ok(TimeRange {
        start: bound(&arr[0], "start")?,
        end: bound(&arr[1], "end")?,
    })
}

fn string_array(v: &Value, path: &str) -> Result<Vec<String>, StacError> {
    v.as_array()
        .ok_or(StacError::WrongType {
            path: path.to_owned(),
            expected: "array of strings",
        })?
        .iter()
        .map(|e| {
            e.as_str().map(str::to_owned).ok_or(StacError::WrongType {
                path: path.to_owned(),
                expected: "array of strings",
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::json;

    use super::super::{
        AssetKind, Bbox, Colormap, Dataset, DatasetId, Datetime, Extent, Granule, GranuleAsset,
        GranuleId, Layer, PlanKind, Resampling, Rescale, TimeRange,
    };
    use super::{
        StacError, dataset_from_stac_collection, dataset_to_stac_collection,
        granule_from_stac_item, granule_to_stac_item,
    };

    /// The HLS-shaped example the snapshot suite also pins.
    pub(crate) fn hls_dataset() -> Dataset {
        Dataset {
            id: DatasetId::new("hls-s30"),
            title: "HLS Sentinel-2 (S30)".to_owned(),
            description: "Harmonized Landsat Sentinel-2, S30 product.".to_owned(),
            license: "CC0-1.0".to_owned(),
            extent: Extent {
                bbox: Bbox {
                    west: -106.1,
                    south: 39.2,
                    east: -105.9,
                    north: 39.4,
                },
                interval: TimeRange {
                    start: Some(Datetime::new("2024-06-01T00:00:00Z").unwrap()),
                    end: None,
                },
            },
            bands: BTreeSet::from([
                "b02".to_owned(),
                "b03".to_owned(),
                "b04".to_owned(),
                "b8a".to_owned(),
            ]),
            layers: vec![
                Layer {
                    id: "truecolor".to_owned(),
                    title: "HLS true color".to_owned(),
                    description: "B04/B03/B02 composite.".to_owned(),
                    plan: PlanKind::Composite {
                        r: "b04".to_owned(),
                        g: "b03".to_owned(),
                        b: "b02".to_owned(),
                    },
                    rescale: Rescale {
                        min: 0.0,
                        max: 3000.0,
                    },
                    colormap: None,
                    resampling: Resampling::Bilinear,
                    tile_size: 256,
                    process: None,
                },
                Layer {
                    id: "ndvi".to_owned(),
                    title: "HLS NDVI".to_owned(),
                    description: "(B8A - B04) / (B8A + B04), grayscale.".to_owned(),
                    plan: PlanKind::BandMath {
                        expression: "(b8a - b04) / (b8a + b04)".to_owned(),
                    },
                    rescale: Rescale {
                        min: -1.0,
                        max: 1.0,
                    },
                    colormap: Some(Colormap::Grayscale),
                    resampling: Resampling::Bilinear,
                    tile_size: 256,
                    process: None,
                },
            ],
        }
    }

    pub(crate) fn hls_granule() -> Granule {
        Granule {
            id: GranuleId::new("hlss30-t13sdd-2024158"),
            dataset: DatasetId::new("hls-s30"),
            bbox: Bbox {
                west: -106.1,
                south: 39.2,
                east: -105.9,
                north: 39.4,
            },
            datetime: Datetime::new("2024-06-06T17:54:00Z").unwrap(),
            assets: BTreeMap::from([
                (
                    "b04".to_owned(),
                    GranuleAsset::raster("s3://hls/t13sdd/2024158/b04.tif"),
                ),
                (
                    "b8a".to_owned(),
                    GranuleAsset::raster("s3://hls/t13sdd/2024158/b8a.tif"),
                ),
            ]),
            ingested_at: Some(Datetime::new("2024-06-06T18:00:00Z").unwrap()),
            properties: BTreeMap::new(),
        }
    }

    #[test]
    fn dataset_round_trips_through_stac() {
        let d = hls_dataset();
        let doc = dataset_to_stac_collection(&d);
        assert_eq!(dataset_from_stac_collection(&doc).unwrap(), d);
    }

    #[test]
    fn granule_round_trips_through_stac() {
        let g = hls_granule();
        let doc = granule_to_stac_item(&g);
        assert_eq!(granule_from_stac_item(&doc).unwrap(), g);
    }

    #[test]
    fn emitted_collection_carries_all_stac_required_fields() {
        let doc = dataset_to_stac_collection(&hls_dataset());
        // STAC 1.1 Collection required fields.
        assert_eq!(doc["type"], "Collection");
        assert_eq!(doc["stac_version"], "1.1.0");
        assert!(doc["id"].is_string());
        assert!(doc["description"].is_string());
        assert!(doc["license"].is_string());
        assert!(doc["extent"]["spatial"]["bbox"][0].is_array());
        assert!(doc["extent"]["temporal"]["interval"][0].is_array());
        assert!(doc["links"].is_array());
    }

    #[test]
    fn emitted_item_carries_all_stac_required_fields() {
        let doc = granule_to_stac_item(&hls_granule());
        // STAC 1.1 Item required fields.
        assert_eq!(doc["type"], "Feature");
        assert_eq!(doc["stac_version"], "1.1.0");
        assert!(doc["id"].is_string());
        assert!(doc["geometry"]["coordinates"].is_array());
        assert!(doc["bbox"].is_array());
        assert!(doc["properties"]["datetime"].is_string());
        assert!(doc["assets"].is_object());
        assert!(doc["collection"].is_string());
        // The derived geometry ring is closed.
        let ring = &doc["geometry"]["coordinates"][0];
        assert_eq!(ring[0], ring[4]);
    }

    #[test]
    fn ingested_at_is_optional_and_validated() {
        // Absent in the domain -> absent in the document (a plain STAC Item).
        let mut g = hls_granule();
        g.ingested_at = None;
        let doc = granule_to_stac_item(&g);
        assert!(
            doc["properties"]
                .as_object()
                .unwrap()
                .get("swath:ingested_at")
                .is_none()
        );
        assert_eq!(granule_from_stac_item(&doc).unwrap(), g);

        // Present -> namespaced property, and round-trips.
        let doc = granule_to_stac_item(&hls_granule());
        assert_eq!(
            doc["properties"]["swath:ingested_at"],
            "2024-06-06T18:00:00Z"
        );
        assert_eq!(granule_from_stac_item(&doc).unwrap(), hls_granule());

        // Malformed values are rejected loudly, naming the path.
        let mut doc = granule_to_stac_item(&hls_granule());
        doc["properties"]["swath:ingested_at"] = json!("yesterday");
        assert!(matches!(
            granule_from_stac_item(&doc).unwrap_err(),
            StacError::InvalidValue { path, .. } if path == "properties.swath:ingested_at"
        ));
        let mut doc = granule_to_stac_item(&hls_granule());
        doc["properties"]["swath:ingested_at"] = json!(12);
        assert!(matches!(
            granule_from_stac_item(&doc).unwrap_err(),
            StacError::WrongType { path, .. } if path == "properties.swath:ingested_at"
        ));
    }

    #[test]
    fn asset_kind_is_optional_namespaced_and_validated() {
        // Raster assets emit exactly the pre-#40 shape: href only.
        let doc = granule_to_stac_item(&hls_granule());
        assert_eq!(
            doc["assets"]["b04"],
            json!({ "href": "s3://hls/t13sdd/2024158/b04.tif" })
        );

        // A virtual-cube asset carries the namespaced kind and round-trips.
        let mut g = hls_granule();
        g.assets.insert(
            "cube".to_owned(),
            GranuleAsset::virtual_cube("vnp09ga/granule.h5.vmanifest.json"),
        );
        let doc = granule_to_stac_item(&g);
        assert_eq!(doc["assets"]["cube"]["swath:kind"], "virtual_cube");
        assert_eq!(granule_from_stac_item(&doc).unwrap(), g);

        // A kind this version does not know is a loud error, not a silent
        // raster.
        let mut doc = granule_to_stac_item(&g);
        doc["assets"]["cube"]["swath:kind"] = json!("hologram");
        assert!(matches!(
            granule_from_stac_item(&doc).unwrap_err(),
            StacError::InvalidValue { path, .. } if path == "assets.cube.swath:kind"
        ));

        // Absent kind reads as raster (pre-#40 documents stay valid).
        let mut doc = granule_to_stac_item(&hls_granule());
        assert!(doc["assets"]["b04"].get("swath:kind").is_none());
        let read = granule_from_stac_item(&doc).unwrap();
        assert_eq!(read.assets["b04"].kind, AssetKind::Raster);
        // And unknown foreign asset fields are still ignored on read.
        doc["assets"]["b04"]["title"] = json!("someone else's title");
        assert!(granule_from_stac_item(&doc).is_ok());
    }

    #[test]
    fn foreign_collection_without_swath_fields_is_rejected_loudly() {
        let mut doc = dataset_to_stac_collection(&hls_dataset());
        doc.as_object_mut().unwrap().remove("swath:bands");
        assert_eq!(
            dataset_from_stac_collection(&doc).unwrap_err(),
            StacError::MissingField {
                path: "swath:bands".to_owned()
            }
        );
    }

    #[test]
    fn wrong_document_type_and_version_are_rejected() {
        let mut doc = dataset_to_stac_collection(&hls_dataset());
        doc["type"] = json!("Catalog");
        assert!(matches!(
            dataset_from_stac_collection(&doc).unwrap_err(),
            StacError::InvalidValue { path, .. } if path == "type"
        ));

        let mut doc = granule_to_stac_item(&hls_granule());
        doc["stac_version"] = json!("1.0.0");
        assert!(matches!(
            granule_from_stac_item(&doc).unwrap_err(),
            StacError::InvalidValue { path, .. } if path == "stac_version"
        ));
    }

    #[test]
    fn item_error_paths_are_precise() {
        let g = hls_granule();

        let mut doc = granule_to_stac_item(&g);
        doc["properties"] = json!({});
        assert_eq!(
            granule_from_stac_item(&doc).unwrap_err(),
            StacError::MissingField {
                path: "properties.datetime".to_owned()
            }
        );

        let mut doc = granule_to_stac_item(&g);
        doc["bbox"] = json!([1.0, 2.0]);
        assert!(matches!(
            granule_from_stac_item(&doc).unwrap_err(),
            StacError::InvalidValue { path, .. } if path == "bbox"
        ));

        let mut doc = granule_to_stac_item(&g);
        doc["assets"]["b04"] = json!({ "title": "no href" });
        assert_eq!(
            granule_from_stac_item(&doc).unwrap_err(),
            StacError::MissingField {
                path: "assets.b04.href".to_owned()
            }
        );
    }

    #[test]
    fn unknown_layer_fields_are_rejected() {
        let mut doc = dataset_to_stac_collection(&hls_dataset());
        doc["swath:layers"][0]["surprise"] = json!(true);
        assert!(matches!(
            dataset_from_stac_collection(&doc).unwrap_err(),
            StacError::InvalidValue { path, .. } if path == "swath:layers"
        ));
    }
}
