// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Live integration suite against a real pgstac (the compose-stack service).
//!
//! Every test is `#[ignore]`: `just test-catalog` brings up the pgstac
//! container and runs them (`cargo nextest run … --run-ignored all`); CI runs
//! the same recipe in the e2e job. Connection comes from `SWATH_PGSTAC_URL`
//! (default: the docker-compose local-dev credentials).
//!
//! Each test owns one `swath-it-*` dataset id and deletes it up front, so
//! reruns are deterministic and tests never share state.

use std::collections::{BTreeMap, BTreeSet};

use swath_catalog_pgstac::PgstacCatalog;
use swath_core::catalog::{
    Bbox, Catalog, CatalogError, Colormap, Dataset, DatasetId, Datetime, Extent, Granule,
    GranuleId, GranuleQuery, Layer, PlanKind, Resampling, Rescale, TimeRange,
};
use swath_core::raster::AssetRef;

/// Connects to the compose-stack pgstac (or `SWATH_PGSTAC_URL`).
async fn catalog() -> PgstacCatalog {
    let url = std::env::var("SWATH_PGSTAC_URL")
        .unwrap_or_else(|_| "postgres://swath:swath-local-dev@localhost:5432/swath".to_owned());
    PgstacCatalog::connect(&url)
        .await
        .expect("pgstac must be reachable — run via `just test-catalog`")
}

/// Resets the test's dataset (drops it if a previous run left it behind).
async fn reset(catalog: &PgstacCatalog, id: &str) {
    // Raw SQL on purpose: deletion is not (yet) part of the port's surface.
    sqlx::query("select pgstac.delete_collection($1)")
        .bind(id)
        .execute(catalog.pool())
        .await
        .ok();
}

/// The HLS fixture's real-world footprint (T13SDD subset, WGS84) and
/// acquisition day — the same granule the render golden suites serve.
fn hls_bbox() -> Bbox {
    Bbox {
        west: -106.1,
        south: 39.2,
        east: -105.9,
        north: 39.4,
    }
}

fn dataset(id: &str) -> Dataset {
    Dataset {
        id: DatasetId::new(id),
        title: "HLS Sentinel-2 (S30)".to_owned(),
        description: "Harmonized Landsat Sentinel-2, S30 product.".to_owned(),
        license: "CC0-1.0".to_owned(),
        extent: Extent {
            bbox: hls_bbox(),
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

fn granule(dataset: &str, id: &str, bbox: Bbox, datetime: &str) -> Granule {
    Granule {
        id: GranuleId::new(id),
        dataset: DatasetId::new(dataset),
        bbox,
        datetime: Datetime::new(datetime).unwrap(),
        assets: BTreeMap::from([
            (
                "b04".to_owned(),
                AssetRef::new(format!("s3://hls/{id}/b04.tif")),
            ),
            (
                "b8a".to_owned(),
                AssetRef::new(format!("s3://hls/{id}/b8a.tif")),
            ),
        ]),
    }
}

fn ids(granules: &[Granule]) -> Vec<&str> {
    let mut ids: Vec<&str> = granules.iter().map(|g| g.id.as_str()).collect();
    ids.sort_unstable();
    ids
}

#[tokio::test]
#[ignore = "needs live pgstac (just test-catalog)"]
async fn dataset_upsert_get_round_trip_is_identity() {
    let catalog = catalog().await;
    let id = "swath-it-roundtrip";
    reset(&catalog, id).await;

    let d = dataset(id);
    catalog.upsert_dataset(&d).await.unwrap();

    // Read back through pgstac: the stored STAC document maps to the exact
    // same Dataset, layers and all.
    let read = catalog.get_dataset(&d.id).await.unwrap().unwrap();
    assert_eq!(read, d);

    // Upsert is idempotent-replace: a changed title comes back changed.
    let mut d2 = d.clone();
    d2.title = "HLS S30 (renamed)".to_owned();
    catalog.upsert_dataset(&d2).await.unwrap();
    assert_eq!(catalog.get_dataset(&d.id).await.unwrap().unwrap(), d2);

    // And the dataset appears in the listing.
    let listed = catalog.list_datasets().await.unwrap();
    assert!(
        listed.contains(&d2),
        "list_datasets must include the upsert"
    );

    reset(&catalog, id).await;
}

#[tokio::test]
#[ignore = "needs live pgstac (just test-catalog)"]
async fn missing_dataset_is_none_and_granule_writes_fail_loudly() {
    let catalog = catalog().await;
    let id = DatasetId::new("swath-it-does-not-exist");

    assert!(catalog.get_dataset(&id).await.unwrap().is_none());

    let err = catalog
        .upsert_granules(&[granule(
            id.as_str(),
            "g1",
            hls_bbox(),
            "2024-06-06T17:54:00Z",
        )])
        .await
        .unwrap_err();
    assert!(
        matches!(&err, CatalogError::DatasetNotFound { id: missing } if *missing == id),
        "expected DatasetNotFound({id}), got {err:?}"
    );

    let err = catalog
        .find_granules(&id, &GranuleQuery::default())
        .await
        .unwrap_err();
    assert!(matches!(&err, CatalogError::DatasetNotFound { id: missing } if *missing == id));
}

#[tokio::test]
#[ignore = "needs live pgstac (just test-catalog)"]
async fn find_granules_filters_by_bbox_and_datetime() {
    let catalog = catalog().await;
    let id = "swath-it-filters";
    reset(&catalog, id).await;
    catalog.upsert_dataset(&dataset(id)).await.unwrap();

    // Two granules on the HLS footprint (different days), one disjoint
    // granule far away (next UTM zone over), same day as the first.
    let on_a1 = granule(id, "t13sdd-2024158", hls_bbox(), "2024-06-06T17:54:00Z");
    let on_a2 = granule(id, "t13sdd-2024165", hls_bbox(), "2024-06-13T17:54:00Z");
    let far = granule(
        id,
        "t12sxx-2024158",
        Bbox {
            west: -112.3,
            south: 39.2,
            east: -112.1,
            north: 39.4,
        },
        "2024-06-06T18:03:00Z",
    );
    catalog
        .upsert_granules(&[on_a1.clone(), on_a2.clone(), far.clone()])
        .await
        .unwrap();

    // Unfiltered: everything, and each granule round-trips identically.
    let all = catalog
        .find_granules(&DatasetId::new(id), &GranuleQuery::default())
        .await
        .unwrap();
    assert_eq!(
        ids(&all),
        ["t12sxx-2024158", "t13sdd-2024158", "t13sdd-2024165"]
    );
    for g in [&on_a1, &on_a2, &far] {
        assert!(all.contains(g), "granule {} must round-trip exactly", g.id);
    }

    // Bbox overlap keeps only the HLS-footprint granules.
    let near = catalog
        .find_granules(
            &DatasetId::new(id),
            &GranuleQuery {
                bbox: Some(Bbox {
                    west: -106.0,
                    south: 39.25,
                    east: -105.95,
                    north: 39.3,
                }),
                datetime: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(ids(&near), ["t13sdd-2024158", "t13sdd-2024165"]);

    // Datetime range around June 6 keeps both same-day granules.
    let june6 = catalog
        .find_granules(
            &DatasetId::new(id),
            &GranuleQuery {
                bbox: None,
                datetime: Some(TimeRange {
                    start: Some(Datetime::new("2024-06-06T00:00:00Z").unwrap()),
                    end: Some(Datetime::new("2024-06-07T00:00:00Z").unwrap()),
                }),
            },
        )
        .await
        .unwrap();
    assert_eq!(ids(&june6), ["t12sxx-2024158", "t13sdd-2024158"]);

    // Open-started range from June 10 onward keeps only the later granule.
    let later = catalog
        .find_granules(
            &DatasetId::new(id),
            &GranuleQuery {
                bbox: None,
                datetime: Some(TimeRange {
                    start: Some(Datetime::new("2024-06-10T00:00:00Z").unwrap()),
                    end: None,
                }),
            },
        )
        .await
        .unwrap();
    assert_eq!(ids(&later), ["t13sdd-2024165"]);

    // Combined bbox + datetime narrows to exactly one.
    let one = catalog
        .find_granules(
            &DatasetId::new(id),
            &GranuleQuery {
                bbox: Some(hls_bbox()),
                datetime: Some(TimeRange {
                    start: Some(Datetime::new("2024-06-06T00:00:00Z").unwrap()),
                    end: Some(Datetime::new("2024-06-07T00:00:00Z").unwrap()),
                }),
            },
        )
        .await
        .unwrap();
    assert_eq!(ids(&one), ["t13sdd-2024158"]);

    reset(&catalog, id).await;
}

#[tokio::test]
#[ignore = "needs live pgstac (just test-catalog)"]
async fn find_granules_pages_past_the_search_limit() {
    let catalog = catalog().await;
    let id = "swath-it-paging";
    reset(&catalog, id).await;
    catalog.upsert_dataset(&dataset(id)).await.unwrap();

    // More granules than one internal search page (1000), so exhaustive
    // paging is actually exercised.
    let granules: Vec<Granule> = (0..1010)
        .map(|i| {
            granule(
                id,
                &format!("g{i:04}"),
                hls_bbox(),
                &format!("2024-06-06T00:{:02}:{:02}Z", i / 60, i % 60),
            )
        })
        .collect();
    catalog.upsert_granules(&granules).await.unwrap();

    let found = catalog
        .find_granules(&DatasetId::new(id), &GranuleQuery::default())
        .await
        .unwrap();
    assert_eq!(found.len(), granules.len());
    let mut found_ids: Vec<&str> = found.iter().map(|g| g.id.as_str()).collect();
    found_ids.sort_unstable();
    assert_eq!(found_ids, ids(&granules));

    reset(&catalog, id).await;
}

/// The R2/R5 bridge: Swath hides STAC, but what it persists is a valid STAC
/// catalog a plain STAC client (here: raw `pgstac.search`, no swath code in
/// the read path) can consume.
#[tokio::test]
#[ignore = "needs live pgstac (just test-catalog)"]
async fn plain_stac_clients_see_a_valid_catalog() {
    let catalog = catalog().await;
    let id = "swath-it-visibility";
    reset(&catalog, id).await;
    catalog.upsert_dataset(&dataset(id)).await.unwrap();
    catalog
        .upsert_granules(&[granule(
            id,
            "t13sdd-2024158",
            hls_bbox(),
            "2024-06-06T17:54:00Z",
        )])
        .await
        .unwrap();

    // The Collection, as any STAC client would fetch it.
    let sqlx::types::Json(collection): sqlx::types::Json<serde_json::Value> =
        sqlx::query_scalar("select pgstac.get_collection($1)")
            .bind(id)
            .fetch_one(catalog.pool())
            .await
            .unwrap();
    assert_eq!(collection["type"], "Collection");
    assert_eq!(collection["stac_version"], "1.1.0");
    for field in ["id", "description", "license", "extent", "links"] {
        assert!(
            !collection[field].is_null(),
            "Collection must carry `{field}`"
        );
    }
    // Swath-owned state rides along namespaced, invisible to STAC semantics.
    assert!(collection["swath:layers"].is_array());

    // The Items, via plain STAC search.
    let sqlx::types::Json(page): sqlx::types::Json<serde_json::Value> =
        sqlx::query_scalar("select pgstac.search($1)")
            .bind(sqlx::types::Json(
                serde_json::json!({ "collections": [id] }),
            ))
            .fetch_one(catalog.pool())
            .await
            .unwrap();
    assert_eq!(page["type"], "FeatureCollection");
    let features = page["features"].as_array().unwrap();
    assert_eq!(features.len(), 1);
    let item = &features[0];
    assert_eq!(item["type"], "Feature");
    assert_eq!(item["stac_version"], "1.1.0");
    assert_eq!(item["collection"], id);
    assert_eq!(item["geometry"]["type"], "Polygon");
    assert!(item["bbox"].as_array().is_some_and(|b| b.len() == 4));
    assert!(item["properties"]["datetime"].is_string());
    assert!(item["assets"]["b04"]["href"].is_string());

    reset(&catalog, id).await;
}
