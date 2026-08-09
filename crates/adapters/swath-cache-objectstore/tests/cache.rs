// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for the object_store-backed `TileCache`: put/get
//! round trips on both Phase-1 backends (local filesystem, in-memory),
//! misses on absent keys, and honest errors from a store that cannot be
//! written (the read-only-directory failure the serve path must survive).

use std::path::PathBuf;
use std::sync::Arc;

use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use swath_cache_objectstore::ObjectStoreTileCache;
// CacheError is only consumed by the unix-gated read-only-dir test below;
// an unconditional import is an unused-import error on Windows (-D warnings).
#[cfg(unix)]
use swath_core::cache::CacheError;
use swath_core::cache::{TileCache, TileKey, TileKeyInputs};
use swath_core::tile::TileCoord;

/// A fresh, self-deleting temp directory per test (no tempfile dep —
/// same pattern as the filedrop adapter's tests).
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "swath-cache-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        std::fs::create_dir_all(&dir).expect("temp dir creates");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn key(layer: &str, version: &str) -> TileKey {
    TileKey::compute(&TileKeyInputs {
        layer,
        layer_version: version,
        plan_json: r#"{"inputs":[{"name":"b04"}]}"#,
        tms: "WebMercatorQuad",
        coord: TileCoord::new(12, 848, 1561).expect("valid tile"),
        tile_size: 256,
    })
}

async fn round_trip(cache: &ObjectStoreTileCache) {
    let key = key("truecolor", "g-1@aa");
    assert!(
        cache.get(&key).await.expect("get works").is_none(),
        "absent key must miss"
    );

    cache
        .put(&key, b"png bytes here", "image/png")
        .await
        .expect("put works");

    let hit = cache
        .get(&key)
        .await
        .expect("get works")
        .expect("stored entry hits");
    assert_eq!(hit.bytes, b"png bytes here");
    assert_eq!(hit.content_type, "image/png");

    // A different version is a different key — still a miss.
    assert!(
        cache
            .get(&key2_of_other_version())
            .await
            .expect("get works")
            .is_none()
    );
}

fn key2_of_other_version() -> TileKey {
    key("truecolor", "g-2@aa")
}

#[tokio::test]
async fn round_trips_on_the_local_filesystem() {
    let dir = TempDir::new("fs");
    let store = LocalFileSystem::new_with_prefix(&dir.0).expect("store opens");
    let cache = ObjectStoreTileCache::new(Arc::new(store));
    round_trip(&cache).await;

    // The on-disk layout is the documented sharded scheme.
    let hex = key("truecolor", "g-1@aa").as_str().to_owned();
    let object = dir
        .0
        .join("tiles")
        .join(&hex[..2])
        .join(&hex[2..4])
        .join(&hex);
    assert!(object.is_file(), "entry lives at the sharded path");
}

#[tokio::test]
async fn round_trips_in_memory() {
    let cache = ObjectStoreTileCache::new(Arc::new(InMemory::new()));
    round_trip(&cache).await;
}

/// An unwritable store surfaces `CacheError::Io` from `put` — the error
/// the serve path logs and shrugs off (its test lives with the tiler).
#[cfg(unix)]
#[tokio::test]
async fn put_into_a_read_only_directory_is_an_io_error() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new("ro");
    let mut perms = std::fs::metadata(&dir.0).expect("stat").permissions();
    perms.set_mode(0o555);
    std::fs::set_permissions(&dir.0, perms.clone()).expect("chmod");

    let store = LocalFileSystem::new_with_prefix(&dir.0).expect("store opens");
    let cache = ObjectStoreTileCache::new(Arc::new(store));
    let err = cache
        .put(&key("truecolor", "g-1@aa"), b"png", "image/png")
        .await
        .expect_err("read-only dir cannot be written");
    assert!(matches!(err, CacheError::Io { .. }), "got: {err:?}");

    // Restore write permission so TempDir::drop can clean up.
    perms.set_mode(0o755);
    std::fs::set_permissions(&dir.0, perms).expect("chmod back");
}
