// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Shared plumbing for the adapter integration tests: an in-memory store
//! holding the tiny committed HDF-EOS fixture (`swath-referencer`'s
//! `tests/data/tiny.h5`) plus its virtual manifest, generated at test time
//! by the production referencer and rewritten to store-relative keys
//! exactly the way the filedrop ingest path does — so what these tests
//! read is what serving reads.

use std::path::PathBuf;
use std::sync::Arc;

use object_store::ObjectStoreExt as _;
use object_store::memory::InMemory;
use object_store::path::Path as StorePath;
use swath_core::ingest::IngestReferencer as _;
use swath_core::manifest::VirtualManifest;
use swath_core::raster::AssetRef;
use swath_referencer::SwathReferencer;
use swath_source_virtual::VirtualSource;

/// The store key of the original granule bytes.
pub(crate) const GRANULE_KEY: &str = "tiny.h5";

/// The store key of the virtual manifest.
pub(crate) const MANIFEST_KEY: &str = "tiny.h5.vmanifest.json";

/// The `TinyGrid` data-field array names (`make_tiny_fixture.py`).
pub(crate) const NIR: &str = "HDFEOS/GRIDS/TinyGrid/Data Fields/nir";
#[allow(dead_code, reason = "each integration-test binary uses a subset")]
pub(crate) const RED: &str = "HDFEOS/GRIDS/TinyGrid/Data Fields/red";

/// The committed tiny fixture (swath-referencer's test data).
pub(crate) fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../swath-referencer/tests/data")
        .join(GRANULE_KEY)
}

/// The `<manifest-key>#<array-name>` asset addressing `array`.
pub(crate) fn asset(array: &str) -> AssetRef {
    AssetRef::new(format!("{MANIFEST_KEY}#{array}"))
}

/// Generates the fixture's manifest with the production referencer,
/// rewritten to store-relative keys (the filedrop convention).
pub(crate) fn generate_manifest() -> VirtualManifest {
    let mut manifest = SwathReferencer::new()
        .generate(&fixture_path())
        .expect("tiny fixture generates");
    GRANULE_KEY.clone_into(&mut manifest.source);
    for array in &mut manifest.arrays {
        for chunk in &mut array.refs {
            GRANULE_KEY.clone_into(&mut chunk.path);
        }
    }
    manifest
}

/// A `VirtualSource` over an in-memory store holding the original granule
/// bytes and the manifest.
pub(crate) async fn memory_source() -> VirtualSource {
    let store = InMemory::new();
    let granule = std::fs::read(fixture_path()).expect("tiny fixture readable");
    store
        .put(&StorePath::from(GRANULE_KEY), granule.into())
        .await
        .expect("put granule");
    store
        .put(
            &StorePath::from(MANIFEST_KEY),
            generate_manifest().to_json_string().into_bytes().into(),
        )
        .await
        .expect("put manifest");
    VirtualSource::new(Arc::new(store))
}

/// Fixture file size in bytes (for provenance range bounds checks).
#[allow(dead_code, reason = "each integration-test binary uses a subset")]
pub(crate) fn granule_len() -> u64 {
    std::fs::metadata(fixture_path())
        .expect("fixture exists")
        .len()
}
