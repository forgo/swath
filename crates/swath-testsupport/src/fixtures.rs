// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The committed fixtures in catalog form (#348): the HLS T13SDD granule
//! the walking skeleton serves, and the Park Fire T10TFK series the
//! temporal, change-detection and datetime suites share. One definition
//! each; the asset names are the bare fixture file names the local store
//! root resolves (`tests/fixtures/README.md`).

use std::collections::BTreeMap;
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
        properties: BTreeMap::new(),
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
        properties: BTreeMap::new(),
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

/// The scale fixture (#414): `count` granules of `dataset`, generated from
/// the **already committed** T10TFK COGs — rows, not bytes. Nothing here
/// reads a clock or a random number: every id, instant and box is derived
/// from the index, so two runs produce byte-identical granules.
///
/// The series starts at `2024-01-01T00:00:00Z` and steps six hours per
/// granule (so a day holds four, and a month's bucket is a round number),
/// and each footprint is the Park Fire box walked east and north on a
/// 16-column lattice in hundredths of a degree — enough distinct cells to
/// exercise density bucketing without leaving the tile's neighbourhood.
#[must_use]
pub fn scale_granules(dataset: &str, count: usize) -> Vec<Granule> {
    (0..count).map(|i| scale_granule(dataset, i)).collect()
}

/// One granule of the scale fixture, addressed by index.
#[must_use]
pub fn scale_granule(dataset: &str, index: usize) -> Granule {
    let index = i64::try_from(index).expect("the scale fixture is indexed in range");
    let hours = index * 6;
    let (day, hour) = (hours / 24, hours % 24);
    // 2024 is a leap year; the fixture never runs past it at the sizes the
    // suites use, and the assertion below says so rather than wrapping
    // silently into a wrong year.
    assert!(
        day < 366,
        "the scale fixture spans one year: {index} is past it"
    );
    let (month, day_of_month) = month_and_day(day);
    let at = format!("2024-{month:02}-{day_of_month:02}T{hour:02}:00:00Z");

    let column = f64::from(u32::try_from(index % 16).expect("a lattice column"));
    let row = f64::from(u32::try_from(index / 16).expect("a lattice row"));
    let (dx, dy) = (column * 0.01, row * 0.01);
    let asset = |band: &str| GranuleAsset::raster(format!("hlss30-t10tfk-2024204-{band}.tif"));
    Granule {
        id: GranuleId::new(format!("scale-{index:05}")),
        dataset: DatasetId::new(dataset),
        bbox: Bbox {
            west: FIRE_BBOX.west + dx,
            south: FIRE_BBOX.south + dy,
            east: FIRE_BBOX.east + dx,
            north: FIRE_BBOX.north + dy,
        },
        datetime: datetime(&at),
        assets: [
            ("b04".to_owned(), asset("b04")),
            ("b8a".to_owned(), asset("b8a")),
        ]
        .into(),
        ingested_at: Some(datetime("2024-12-31T00:00:00Z")),
        properties: BTreeMap::new(),
    }
}

/// The 1-based calendar month and day of a 0-based day of 2024.
fn month_and_day(day_of_year: i64) -> (i64, i64) {
    const LENGTHS: [i64; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut remaining = day_of_year;
    for (index, length) in LENGTHS.iter().enumerate() {
        if remaining < *length {
            return (
                i64::try_from(index).expect("twelve months") + 1,
                remaining + 1,
            );
        }
        remaining -= *length;
    }
    unreachable!("day_of_year is bounded by the caller")
}

/// The scale fixture's dataset: the Park Fire bands, the lattice's full
/// extent, no config layers.
#[must_use]
pub fn scale_dataset(id: &str, count: usize) -> Dataset {
    let last = scale_granule(id, count.saturating_sub(1));
    Dataset {
        id: DatasetId::new(id),
        title: "Scale fixture".to_owned(),
        description: "Synthetic rows over the committed T10TFK COGs (#414).".to_owned(),
        license: "CC0-1.0".to_owned(),
        extent: Extent {
            bbox: Bbox {
                west: FIRE_BBOX.west,
                south: FIRE_BBOX.south,
                east: last.bbox.east,
                north: last.bbox.north,
            },
            interval: TimeRange {
                start: Some(datetime("2024-01-01T00:00:00Z")),
                end: Some(last.datetime.clone()),
            },
        },
        bands: ["b04", "b8a"].map(str::to_owned).into_iter().collect(),
        layers: Vec::new(),
    }
}
