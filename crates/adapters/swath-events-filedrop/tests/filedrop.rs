// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Filedrop watcher + ingest orchestrator integration tests: a real
//! filesystem (temp dir), a real poll loop, an in-memory catalog — no
//! pgstac needed for the unit path (the live path joins `just
//! test-catalog`'s gated suite).

use std::path::{Path, PathBuf};
use std::time::Duration;

use swath_core::catalog::{
    AssetKind, Bbox, Catalog, CatalogError, Dataset, DatasetId, Datetime, Extent, GranuleId,
    GranuleQuery, TimeRange,
};
use swath_core::events::{EventError, EventSource as _, GranuleEvent};
use swath_core::ingest::ingest_granule;
use swath_events_filedrop::FiledropEvents;
use swath_testsupport::TempDir;
use swath_testsupport::catalog::MemoryCatalog;

/// Fast-polling watcher over `dir` (tests should not wait real cadences).
fn watcher(dir: &Path) -> FiledropEvents {
    FiledropEvents::new(dir, Duration::from_millis(10))
}

fn manifest_json(dataset: &str, granule: &str) -> String {
    format!(
        r#"{{
            "dataset": "{dataset}",
            "granule": "{granule}",
            "bbox": [-106.1, 39.2, -105.9, 39.4],
            "datetime": "2024-06-06T17:54:00Z",
            "assets": {{
                "b04": "{granule}-b04.tif",
                "b03": "{granule}-b03.tif"
            }}
        }}"#
    )
}

/// Writes per the drop convention: temp name first, rename into place.
fn drop_manifest(dir: &Path, dataset: &str, granule: &str) {
    let staged = dir.join(format!(".{granule}.json"));
    std::fs::write(&staged, manifest_json(dataset, granule)).unwrap();
    std::fs::rename(&staged, dir.join(format!("{granule}.json"))).unwrap();
}

async fn next(source: &mut FiledropEvents) -> Result<Option<GranuleEvent>, EventError> {
    tokio::time::timeout(Duration::from_secs(5), source.next_event())
        .await
        .expect("an event within the test timeout")
}

#[tokio::test]
async fn manifest_appearance_becomes_a_granule_event() {
    let dir = TempDir::new("filedrop-basic");
    let mut source = watcher(dir.path());

    let before = Datetime::from_unix_millis(now_millis()).unwrap();
    drop_manifest(dir.path(), "hls-s30", "g-2024158");
    let event = next(&mut source).await.unwrap().unwrap();

    assert_eq!(event.granule.id, GranuleId::new("g-2024158"));
    assert_eq!(event.granule.dataset, DatasetId::new("hls-s30"));
    assert_eq!(event.granule.datetime.as_str(), "2024-06-06T17:54:00Z");
    assert_eq!(event.granule.assets.len(), 2);
    assert_eq!(
        event.granule.assets["b04"].href.as_str(),
        "g-2024158-b04.tif"
    );
    assert_eq!(
        event.granule.ingested_at, None,
        "the orchestrator stamps it"
    );
    // Observation-time stamp: within [before, now].
    let arrived = event.arrived_at.to_unix_millis();
    assert!(arrived >= before.to_unix_millis() && arrived <= now_millis());
}

#[tokio::test]
async fn watcher_survives_a_directory_created_after_start() {
    let dir = TempDir::new("filedrop-late-dir");
    let missing = dir.path().join("drop");
    let mut source = watcher(&missing);

    // Directory doesn't exist yet; create it and drop mid-watch.
    let waiter = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        std::fs::create_dir_all(&missing).unwrap();
        drop_manifest(&missing, "hls-s30", "late");
    };
    let (event, ()) = tokio::join!(next(&mut source), waiter);
    assert_eq!(event.unwrap().unwrap().granule.id, GranuleId::new("late"));
}

#[tokio::test]
async fn each_manifest_is_announced_once_in_name_order() {
    let dir = TempDir::new("filedrop-once");
    // Both present before the first scan: yielded in name order.
    drop_manifest(dir.path(), "hls-s30", "a-first");
    drop_manifest(dir.path(), "hls-s30", "b-second");
    let mut source = watcher(dir.path());

    let first = next(&mut source).await.unwrap().unwrap();
    let second = next(&mut source).await.unwrap().unwrap();
    assert_eq!(first.granule.id, GranuleId::new("a-first"));
    assert_eq!(second.granule.id, GranuleId::new("b-second"));

    // No re-announcement: a third pull pends until something NEW arrives.
    let waiter = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop_manifest(dir.path(), "hls-s30", "c-third");
    };
    let (third, ()) = tokio::join!(next(&mut source), waiter);
    assert_eq!(
        third.unwrap().unwrap().granule.id,
        GranuleId::new("c-third")
    );
}

#[tokio::test]
async fn non_manifests_and_staging_files_are_ignored() {
    let dir = TempDir::new("filedrop-ignore");
    std::fs::write(dir.path().join("band-b04.tif"), b"not a manifest").unwrap();
    std::fs::write(dir.path().join(".staged.json"), b"{").unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"hello").unwrap();
    let mut source = watcher(dir.path());

    let waiter = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop_manifest(dir.path(), "hls-s30", "real");
    };
    let (event, ()) = tokio::join!(next(&mut source), waiter);
    assert_eq!(event.unwrap().unwrap().granule.id, GranuleId::new("real"));
}

#[tokio::test]
async fn malformed_manifests_error_once_then_stop_reporting() {
    let dir = TempDir::new("filedrop-malformed");
    std::fs::write(dir.path().join("broken.json"), b"{ not json").unwrap();
    let mut source = watcher(dir.path());

    // Reported exactly once, naming the file...
    let err = next(&mut source).await.unwrap_err();
    assert!(matches!(&err, EventError::Malformed { detail } if detail.contains("broken.json")));

    // ...then consumed: the next pull sees only new arrivals.
    let waiter = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop_manifest(dir.path(), "hls-s30", "good");
    };
    let (event, ()) = tokio::join!(next(&mut source), waiter);
    assert_eq!(event.unwrap().unwrap().granule.id, GranuleId::new("good"));
}

#[tokio::test]
async fn name_mismatch_and_empty_assets_are_malformed() {
    let dir = TempDir::new("filedrop-invalid");
    std::fs::write(
        dir.path().join("wrong-name.json"),
        manifest_json("hls-s30", "other-id"),
    )
    .unwrap();
    let mut source = watcher(dir.path());
    let err = next(&mut source).await.unwrap_err();
    assert!(matches!(&err, EventError::Malformed { detail }
            if detail.contains("wrong-name") && detail.contains("other-id")));

    std::fs::write(
        dir.path().join("empty.json"),
        r#"{"dataset":"d","granule":"empty","bbox":[0,0,1,1],
            "datetime":"2024-06-06T17:54:00Z","assets":{}}"#,
    )
    .unwrap();
    let err = next(&mut source).await.unwrap_err();
    assert!(matches!(&err, EventError::Malformed { detail } if detail.contains("assets")));
}

// --- the orchestrator path: drop -> event -> catalog upsert, end to end ---

fn hls_dataset() -> Dataset {
    Dataset {
        id: DatasetId::new("hls-s30"),
        title: "HLS S30".to_owned(),
        description: String::new(),
        license: "CC0-1.0".to_owned(),
        extent: Extent {
            bbox: Bbox {
                west: -180.0,
                south: -90.0,
                east: 180.0,
                north: 90.0,
            },
            interval: TimeRange::default(),
        },
        bands: ["b03", "b04"].map(str::to_owned).into(),
        layers: Vec::new(),
    }
}

#[tokio::test]
async fn dropped_granule_lands_in_the_catalog_with_ingested_at() {
    let dir = TempDir::new("filedrop-orchestrated");
    let catalog = MemoryCatalog::default();
    catalog.upsert_dataset(&hls_dataset()).await.unwrap();

    let mut source = watcher(dir.path());
    drop_manifest(dir.path(), "hls-s30", "g-2024158");
    let event = next(&mut source).await.unwrap().unwrap();

    let stored = ingest_granule(&catalog, &event).await.unwrap();
    assert_eq!(stored.ingested_at, Some(event.arrived_at.clone()));

    let found = catalog
        .find_granules(&DatasetId::new("hls-s30"), &GranuleQuery::default())
        .await
        .unwrap();
    assert_eq!(found, vec![stored]);
}

#[tokio::test]
async fn dropped_granule_of_unknown_dataset_fails_loudly() {
    let dir = TempDir::new("filedrop-unknown-dataset");
    let catalog = MemoryCatalog::default();

    let mut source = watcher(dir.path());
    drop_manifest(dir.path(), "never-registered", "g1");
    let event = next(&mut source).await.unwrap().unwrap();

    let err = ingest_granule(&catalog, &event).await.unwrap_err();
    assert!(
        matches!(err, CatalogError::DatasetNotFound { id } if id.as_str() == "never-registered")
    );
}

// --- the legacy path: drop with a .h5 asset -> manifest generated,
// --- stored alongside, asset rewritten to a virtual cube (ADR 0006, #40) ---

/// The tiny committed HDF5 fixture from the referencer's known-answer suite.
fn tiny_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../swath-referencer/tests/data/tiny.h5")
        .canonicalize()
        .expect("tiny.h5 fixture exists")
}

/// The test's copy of the in-tree port shim (ADR 0016): the standalone
/// referencer crate does not implement the workspace's `IngestReferencer`
/// port, so the adapter test adapts it exactly as `swath serve` does.
#[derive(Debug, Clone, Copy, Default)]
struct ReferencerShim(swath_referencer::SwathReferencer);

impl swath_core::ingest::IngestReferencer for ReferencerShim {
    fn handles(&self, granule: &Path) -> bool {
        swath_referencer::SwathReferencer::handles(granule)
    }

    fn generate(
        &self,
        granule: &Path,
    ) -> Result<swath_core::manifest::VirtualManifest, swath_core::ingest::ReferencerError> {
        use swath_core::ingest::ReferencerError as Port;
        use swath_referencer::ReferencerError as Lib;
        self.0.generate(granule).map_err(|err| match err {
            Lib::Unsupported { detail } => Port::Unsupported { detail },
            Lib::Malformed { detail } => Port::Malformed { detail },
            Lib::Backend { detail, source } => Port::Backend { detail, source },
            other => Port::Backend {
                detail: "unmapped referencer error".to_owned(),
                source: Box::new(other),
            },
        })
    }
}

/// A watcher with the legacy path enabled, data root = the watch dir.
fn legacy_watcher(dir: &Path) -> FiledropEvents {
    FiledropEvents::new(dir, Duration::from_millis(10))
        .with_referencer(std::sync::Arc::new(ReferencerShim::default()), dir)
}

fn drop_legacy_granule(dir: &Path, dataset: &str, granule: &str, asset_uri: &str) {
    let staged = dir.join(format!(".{granule}.json"));
    std::fs::write(
        &staged,
        format!(
            r#"{{
                "dataset": "{dataset}",
                "granule": "{granule}",
                "bbox": [138.7, -30.0, 152.3, -27.0],
                "datetime": "2012-01-19T00:00:00Z",
                "assets": {{ "cube": "{asset_uri}" }}
            }}"#
        ),
    )
    .unwrap();
    std::fs::rename(&staged, dir.join(format!("{granule}.json"))).unwrap();
}

#[tokio::test]
async fn legacy_asset_is_referenced_stored_and_rewritten() {
    let dir = TempDir::new("filedrop-legacy");
    // Bands-first discipline: the granule file lands before its manifest.
    std::fs::copy(tiny_fixture(), dir.path().join("tiny.h5")).unwrap();
    let mut source = legacy_watcher(dir.path());
    drop_legacy_granule(dir.path(), "vnp09ga", "g-legacy", "tiny.h5");

    let event = next(&mut source).await.unwrap().unwrap();
    let asset = &event.granule.assets["cube"];
    assert_eq!(asset.kind, AssetKind::VirtualCube);
    assert_eq!(asset.href.as_str(), "tiny.h5.vmanifest.json");

    // The manifest was stored alongside the source, parses as schema v1,
    // and its chunk paths are the store-relative asset key (not local
    // absolute paths).
    let stored = dir.path().join("tiny.h5.vmanifest.json");
    let manifest = swath_core::manifest::VirtualManifest::from_json_str(
        &std::fs::read_to_string(&stored).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest.source, "tiny.h5");
    assert!(!manifest.arrays.is_empty());
    assert!(
        manifest
            .arrays
            .iter()
            .flat_map(|a| &a.refs)
            .all(|r| r.path == "tiny.h5")
    );
    // No staging dotfile left behind.
    assert!(!dir.path().join(".tiny.h5.vmanifest.json").exists());
}

#[tokio::test]
async fn legacy_asset_fragments_select_arrays_and_share_one_manifest() {
    // #39's addressing: a multi-band legacy granule maps each band to
    // `<file>#<array-name>`. The watcher references the shared file ONCE
    // and rewrites each band to `<file>.vmanifest.json#<array-name>` —
    // exactly what the virtual RasterSource reads.
    let dir = TempDir::new("filedrop-legacy-fragments");
    std::fs::copy(tiny_fixture(), dir.path().join("tiny.h5")).unwrap();
    let mut source = legacy_watcher(dir.path());
    let staged = dir.path().join(".g-frag.json");
    std::fs::write(
        &staged,
        r#"{
            "dataset": "vnp09ga",
            "granule": "g-frag",
            "bbox": [138.7, -30.0, 152.3, -27.0],
            "datetime": "2012-01-19T00:00:00Z",
            "assets": {
                "nir": "tiny.h5#HDFEOS/GRIDS/TinyGrid/Data Fields/nir",
                "red": "tiny.h5#HDFEOS/GRIDS/TinyGrid/Data Fields/red"
            }
        }"#,
    )
    .unwrap();
    std::fs::rename(&staged, dir.path().join("g-frag.json")).unwrap();

    let event = next(&mut source).await.unwrap().unwrap();
    let nir = &event.granule.assets["nir"];
    assert_eq!(nir.kind, AssetKind::VirtualCube);
    assert_eq!(
        nir.href.as_str(),
        "tiny.h5.vmanifest.json#HDFEOS/GRIDS/TinyGrid/Data Fields/nir"
    );
    let red = &event.granule.assets["red"];
    assert_eq!(red.kind, AssetKind::VirtualCube);
    assert_eq!(
        red.href.as_str(),
        "tiny.h5.vmanifest.json#HDFEOS/GRIDS/TinyGrid/Data Fields/red"
    );
    // One shared manifest, stored once, alongside the file.
    assert!(dir.path().join("tiny.h5.vmanifest.json").exists());
}

#[tokio::test]
async fn legacy_referencing_end_to_end_registers_the_virtual_granule() {
    // Drop -> manifest generated -> granule registered: the full R1 loop
    // for a legacy granule, against the in-memory catalog.
    let dir = TempDir::new("filedrop-legacy-e2e");
    std::fs::copy(tiny_fixture(), dir.path().join("tiny.h5")).unwrap();
    let catalog = MemoryCatalog::default();
    let mut vnp = hls_dataset();
    vnp.id = DatasetId::new("vnp09ga");
    catalog.upsert_dataset(&vnp).await.unwrap();

    let mut source = legacy_watcher(dir.path());
    drop_legacy_granule(dir.path(), "vnp09ga", "g-e2e", "tiny.h5");
    let event = next(&mut source).await.unwrap().unwrap();
    let stored = ingest_granule(&catalog, &event).await.unwrap();

    let found = catalog
        .find_granules(&DatasetId::new("vnp09ga"), &GranuleQuery::default())
        .await
        .unwrap();
    assert_eq!(found, vec![stored.clone()]);
    assert_eq!(found[0].assets["cube"].kind, AssetKind::VirtualCube);
    assert!(stored.ingested_at.is_some());
}

#[tokio::test]
async fn broken_legacy_granule_is_malformed_and_does_not_stop_the_loop() {
    let dir = TempDir::new("filedrop-legacy-broken");
    // A .h5 asset that is not an HDF5 file at all.
    std::fs::write(dir.path().join("junk.h5"), b"not hdf5").unwrap();
    let mut source = legacy_watcher(dir.path());
    drop_legacy_granule(dir.path(), "vnp09ga", "g-bad", "junk.h5");

    let err = next(&mut source).await.unwrap_err();
    assert!(matches!(&err, EventError::Malformed { detail }
            if detail.contains("junk.h5")));

    // The loop keeps going: a good plain-raster drop still announces.
    let waiter = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop_manifest(dir.path(), "hls-s30", "still-good");
    };
    let (event, ()) = tokio::join!(next(&mut source), waiter);
    assert_eq!(
        event.unwrap().unwrap().granule.id,
        GranuleId::new("still-good")
    );
}

#[tokio::test]
async fn absolute_or_remote_legacy_assets_are_refused() {
    let dir = TempDir::new("filedrop-legacy-remote");
    let mut source = legacy_watcher(dir.path());
    drop_legacy_granule(dir.path(), "vnp09ga", "g-remote", "s3://bucket/granule.h5");
    let err = next(&mut source).await.unwrap_err();
    assert!(matches!(&err, EventError::Malformed { detail }
            if detail.contains("store-relative")));
}

#[tokio::test]
async fn without_a_referencer_legacy_assets_pass_through_untouched() {
    // The pre-#40 behavior is preserved when no referencer is configured:
    // opaque URIs, kind raster, nothing opened.
    let dir = TempDir::new("filedrop-legacy-off");
    let mut source = watcher(dir.path());
    drop_legacy_granule(dir.path(), "vnp09ga", "g-off", "tiny.h5");
    let event = next(&mut source).await.unwrap().unwrap();
    let asset = &event.granule.assets["cube"];
    assert_eq!(asset.kind, AssetKind::Raster);
    assert_eq!(asset.href.as_str(), "tiny.h5");
    assert!(!dir.path().join("tiny.h5.vmanifest.json").exists());
}

fn now_millis() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

#[tokio::test]
async fn seen_before_start_is_still_announced() {
    // A manifest already present when the watcher starts is an arrival too:
    // restart-safety (module docs) means the directory's current contents
    // are announced, idempotently re-upserted downstream.
    let dir = TempDir::new("filedrop-preexisting");
    drop_manifest(dir.path(), "hls-s30", "old");
    let mut source = watcher(dir.path());
    let event = next(&mut source).await.unwrap().unwrap();
    assert_eq!(event.granule.id, GranuleId::new("old"));
}

/// A dropped manifest may carry STAC properties, and they reach the catalog
/// verbatim (#408). Before ADR 0029 there was nowhere for them to go.
#[tokio::test]
async fn dropped_properties_reach_the_catalog_verbatim() {
    let dir = TempDir::new("filedrop-properties");
    let catalog = MemoryCatalog::default();
    catalog.upsert_dataset(&hls_dataset()).await.unwrap();

    let granule = "g-2024158";
    let staged = dir.path().join(format!(".{granule}.json"));
    std::fs::write(
        &staged,
        format!(
            r#"{{
                "dataset": "hls-s30",
                "granule": "{granule}",
                "bbox": [-106.1, 39.2, -105.9, 39.4],
                "datetime": "2024-06-06T17:54:00Z",
                "assets": {{ "b04": "{granule}-b04.tif" }},
                "properties": {{
                    "eo:cloud_cover": 12.5,
                    "platform": "sentinel-2a",
                    "nested": {{ "a": [1, 2, 3] }}
                }}
            }}"#
        ),
    )
    .unwrap();
    std::fs::rename(&staged, dir.path().join(format!("{granule}.json"))).unwrap();

    let mut source = watcher(dir.path());
    let event = next(&mut source).await.unwrap().unwrap();
    let stored = ingest_granule(&catalog, &event).await.unwrap();

    // Verbatim, whatever the shape — a passthrough that kept numbers and
    // flattened objects would still be data loss.
    assert_eq!(stored.properties["eo:cloud_cover"], serde_json::json!(12.5));
    assert_eq!(
        stored.properties["platform"],
        serde_json::json!("sentinel-2a")
    );
    assert_eq!(
        stored.properties["nested"],
        serde_json::json!({ "a": [1, 2, 3] })
    );
}

/// A manifest written before properties existed still parses, and produces
/// exactly the granule it did before.
#[tokio::test]
async fn a_manifest_without_properties_is_unchanged() {
    let dir = TempDir::new("filedrop-no-properties");
    let mut source = watcher(dir.path());
    drop_manifest(dir.path(), "hls-s30", "g-2024158");
    let event = next(&mut source).await.unwrap().unwrap();
    assert!(event.granule.properties.is_empty());
}
