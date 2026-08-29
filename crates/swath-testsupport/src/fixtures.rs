// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The committed fixtures in catalog form (#348): the HLS T13SDD granule
//! the walking skeleton serves, and the Park Fire T10TFK series the
//! temporal, change-detection and datetime suites share. One definition
//! each; the asset names are the bare fixture file names the local store
//! root resolves (`tests/fixtures/README.md`).

use swath_core::catalog::{
    Bbox, Colormap, Dataset, DatasetId, Datetime, Extent, Granule, GranuleAsset, GranuleId, Layer,
    PlanKind, Resampling, Rescale, TimeRange,
};

/// The T13SDD fixture footprint.
const HLS_BBOX: Bbox = Bbox {
    west: -105.537,
    south: 39.1954,
    east: -105.3581,
    north: 39.3345,
};

/// The Park Fire T10TFK footprint.
const FIRE_BBOX: Bbox = Bbox {
    west: -121.7388,
    south: 39.9866,
    east: -121.6474,
    north: 40.0549,
};

/// The Park Fire series: sensing dates of the six committed T10TFK
/// acquisitions (`tests/fixtures/README.md`, "Fire-event series").
pub const FIRE_DAYS: [(&str, &str); 6] = [
    ("2024159", "2024-06-07T19:03:00Z"),
    ("2024204", "2024-07-22T19:03:00Z"),
    ("2024229", "2024-08-16T19:03:00Z"),
    ("2024249", "2024-09-05T19:03:00Z"),
    ("2024274", "2024-09-30T19:03:00Z"),
    ("2024289", "2024-10-15T19:03:00Z"),
];

fn datetime(value: &str) -> Datetime {
    Datetime::new(value).expect("fixture datetime")
}

/// The HLS fixture dataset: the same band vocabulary and serving layers as
/// `LayerRegistry::hls_fixtures`, persisted the way `[[datasets]]` config
/// would persist them (`PlanKind` + rescale).
#[must_use]
pub fn hls_catalog_dataset() -> Dataset {
    Dataset {
        id: DatasetId::new("hls-s30"),
        title: "HLS Sentinel-2 (S30)".to_owned(),
        description: "Harmonized Landsat Sentinel-2, S30 product.".to_owned(),
        license: "CC0-1.0".to_owned(),
        extent: Extent {
            bbox: HLS_BBOX,
            interval: TimeRange {
                start: Some(datetime("2024-06-01T00:00:00Z")),
                end: None,
            },
        },
        bands: ["b02", "b03", "b04", "b8a"]
            .map(str::to_owned)
            .into_iter()
            .collect(),
        layers: vec![
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
        ],
    }
}

/// The committed HLS fixture granule, catalog form.
#[must_use]
pub fn hls_catalog_granule() -> Granule {
    let asset = |name: &str| GranuleAsset::raster(format!("hlss30-t13sdd-2024158-{name}.tif"));
    Granule {
        id: GranuleId::new("hlss30-t13sdd-2024158"),
        dataset: DatasetId::new("hls-s30"),
        bbox: HLS_BBOX,
        datetime: datetime("2024-06-06T17:54:00Z"),
        assets: [
            ("b02".to_owned(), asset("b02")),
            ("b03".to_owned(), asset("b03")),
            ("b04".to_owned(), asset("b04")),
            ("b8a".to_owned(), asset("b8a")),
        ]
        .into(),
        ingested_at: Some(datetime("2024-06-06T18:00:00Z")),
    }
}

/// A Park Fire dataset under `id` (NDVI bands, no config layers, the
/// series' full interval).
#[must_use]
pub fn fire_dataset(id: &str) -> Dataset {
    Dataset {
        id: DatasetId::new(id),
        title: "HLS S30 Park Fire series".to_owned(),
        description: "T10TFK acquisitions across the 2024 Park Fire.".to_owned(),
        license: "CC0-1.0".to_owned(),
        extent: Extent {
            bbox: FIRE_BBOX,
            interval: TimeRange {
                start: Some(datetime(FIRE_DAYS[0].1)),
                end: Some(datetime(FIRE_DAYS[FIRE_DAYS.len() - 1].1)),
            },
        },
        bands: ["b04", "b8a"].map(str::to_owned).into_iter().collect(),
        layers: Vec::new(),
    }
}

/// One committed Park Fire granule of `dataset`: MGRS `tile` (`t10tfk`)
/// on `day` (`2024204`), whose assets are the fixture files
/// `hlss30-{tile}-{day}-{band}.tif`.
#[must_use]
pub fn fire_granule(dataset: &str, tile: &str, day: &str, at: &str) -> Granule {
    let asset = |band: &str| GranuleAsset::raster(format!("hlss30-{tile}-{day}-{band}.tif"));
    Granule {
        id: GranuleId::new(format!("hlss30-{tile}-{day}")),
        dataset: DatasetId::new(dataset),
        bbox: FIRE_BBOX,
        datetime: datetime(at),
        assets: [
            ("b04".to_owned(), asset("b04")),
            ("b8a".to_owned(), asset("b8a")),
        ]
        .into(),
        ingested_at: Some(datetime("2024-11-01T00:00:00Z")),
    }
}

/// The Park Fire dataset (`park-fire`) with one T10TFK granule per
/// `(day, datetime)` — the fixture the temporal and two-source suites share.
#[must_use]
pub fn park_fire(days: &[(&str, &str)]) -> (Dataset, Vec<Granule>) {
    let granules = days
        .iter()
        .map(|&(day, at)| fire_granule("park-fire", "t10tfk", day, at))
        .collect();
    (fire_dataset("park-fire"), granules)
}
