// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The normative catalog round-trip property (docs/design/catalog-domain.md
//! §1): for arbitrary in-bounds `Dataset`s and `Granule`s,
//! domain → STAC → domain is the identity — including a serialize-to-text
//! step, so float formatting and JSON re-parsing are inside the property.
//! Plus the pinned snapshot of one representative Collection + Item: the
//! persisted document shape is contractual, and any change to it must show up
//! as a reviewed snapshot diff.

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use swath_core::catalog::stac::{
    dataset_from_stac_collection, dataset_to_stac_collection, granule_from_stac_item,
    granule_to_stac_item,
};
use swath_core::catalog::{
    Bbox, Colormap, Dataset, DatasetId, Datetime, Extent, Granule, GranuleId, Layer, PlanKind,
    Resampling, Rescale, TimeRange,
};
use swath_core::raster::AssetRef;

// --- strategies: arbitrary values within realistic catalog bounds ---

fn identifier() -> impl Strategy<Value = String> {
    "[a-z0-9][a-z0-9-]{0,31}"
}

fn band_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,7}"
}

/// Free text, unicode included — titles/descriptions must survive JSON.
fn text() -> impl Strategy<Value = String> {
    proptest::string::string_regex(".{0,40}").unwrap()
}

fn finite_f64() -> impl Strategy<Value = f64> {
    -1.0e9..1.0e9
}

fn bbox() -> impl Strategy<Value = Bbox> {
    (
        -180.0..180.0_f64,
        -90.0..90.0_f64,
        -180.0..180.0_f64,
        -90.0..90.0_f64,
    )
        .prop_map(|(west, south, east, north)| Bbox {
            west,
            south,
            east,
            north,
        })
}

fn datetime() -> impl Strategy<Value = Datetime> {
    (
        1970..2100_u32,
        1..=12_u32,
        1..=28_u32, // day 28 cap keeps every (year, month) combination valid
        0..24_u32,
        0..60_u32,
        0..60_u32,
        proptest::option::of(1..=999_999_u32),
    )
        .prop_map(|(y, mo, d, h, mi, s, frac)| {
            let base = format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}");
            let text = match frac {
                Some(f) => format!("{base}.{f}Z"),
                None => format!("{base}Z"),
            };
            Datetime::new(text).expect("generated datetimes are valid by construction")
        })
}

fn time_range() -> impl Strategy<Value = TimeRange> {
    (
        proptest::option::of(datetime()),
        proptest::option::of(datetime()),
    )
        .prop_map(|(start, end)| TimeRange { start, end })
}

fn plan_kind() -> impl Strategy<Value = PlanKind> {
    prop_oneof![
        (band_name(), band_name(), band_name()).prop_map(|(r, g, b)| PlanKind::Composite {
            r,
            g,
            b
        }),
        text().prop_map(|expression| PlanKind::BandMath { expression }),
    ]
}

fn layer() -> impl Strategy<Value = Layer> {
    (
        identifier(),
        text(),
        text(),
        plan_kind(),
        (finite_f64(), finite_f64()).prop_map(|(min, max)| Rescale { min, max }),
        proptest::option::of(Just(Colormap::Grayscale)),
        prop_oneof![Just(Resampling::Nearest), Just(Resampling::Bilinear)],
        1..=1024_u32,
    )
        .prop_map(
            |(id, title, description, plan, rescale, colormap, resampling, tile_size)| Layer {
                id,
                title,
                description,
                plan,
                rescale,
                colormap,
                resampling,
                tile_size,
            },
        )
}

fn dataset() -> impl Strategy<Value = Dataset> {
    (
        identifier(),
        text(),
        text(),
        identifier(), // license: an SPDX-ish token
        (bbox(), time_range()).prop_map(|(bbox, interval)| Extent { bbox, interval }),
        proptest::collection::btree_set(band_name(), 0..6),
        proptest::collection::vec(layer(), 0..4),
    )
        .prop_map(
            |(id, title, description, license, extent, bands, layers)| Dataset {
                id: DatasetId::new(id),
                title,
                description,
                license,
                extent,
                bands: BTreeSet::from_iter(bands),
                layers,
            },
        )
}

fn granule() -> impl Strategy<Value = Granule> {
    (
        identifier(),
        identifier(),
        bbox(),
        datetime(),
        proptest::collection::btree_map(band_name(), text().prop_map(AssetRef::new), 0..6),
    )
        .prop_map(|(id, dataset, bbox, datetime, assets)| Granule {
            id: GranuleId::new(id),
            dataset: DatasetId::new(dataset),
            bbox,
            datetime,
            assets: BTreeMap::from_iter(assets),
        })
}

// --- the normative property ---

proptest! {
    #[test]
    fn dataset_to_stac_and_back_is_identity(d in dataset()) {
        let doc = dataset_to_stac_collection(&d);
        // Through text, not just Value: float formatting and JSON re-parsing
        // are part of the storage path.
        let reparsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
        prop_assert_eq!(dataset_from_stac_collection(&reparsed).unwrap(), d);
    }

    #[test]
    fn granule_to_stac_and_back_is_identity(g in granule()) {
        let doc = granule_to_stac_item(&g);
        let reparsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
        prop_assert_eq!(granule_from_stac_item(&reparsed).unwrap(), g);
    }
}

// --- the pinned representative documents ---

fn hls_dataset() -> Dataset {
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
            },
        ],
    }
}

fn hls_granule() -> Granule {
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
                AssetRef::new("s3://hls/t13sdd/2024158/b04.tif"),
            ),
            (
                "b8a".to_owned(),
                AssetRef::new("s3://hls/t13sdd/2024158/b8a.tif"),
            ),
        ]),
    }
}

#[test]
fn representative_collection_document_is_pinned() {
    insta::assert_json_snapshot!("hls_collection", dataset_to_stac_collection(&hls_dataset()));
}

#[test]
fn representative_item_document_is_pinned() {
    insta::assert_json_snapshot!("hls_item", granule_to_stac_item(&hls_granule()));
}
