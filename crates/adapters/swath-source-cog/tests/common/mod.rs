// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Shared plumbing for the adapter integration tests: fixture paths and
//! `CogSource` construction over local-filesystem and in-memory stores.

use std::sync::Arc;

use object_store::ObjectStoreExt as _;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::path::Path as StorePath;
use swath_core::raster::DType;
use swath_source_cog::CogSource;
pub(crate) use swath_testsupport::paths::fixtures_dir;

/// A `CogSource` over the real fixture files on the local filesystem.
pub(crate) fn local_source() -> CogSource {
    let store = LocalFileSystem::new_with_prefix(fixtures_dir()).expect("fixture dir exists");
    CogSource::new(Arc::new(store))
}

/// A `CogSource` over an in-memory store preloaded with every fixture, so
/// local-file and object-storage reads exercise the same adapter code path.
pub(crate) async fn memory_source() -> CogSource {
    let store = InMemory::new();
    for entry in std::fs::read_dir(fixtures_dir()).expect("fixture dir readable") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().into_string().expect("utf-8 file name");
        if std::path::Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("tif"))
        {
            let bytes = std::fs::read(entry.path()).expect("fixture readable");
            store
                .put(&StorePath::from(name), bytes.into())
                .await
                .expect("put fixture into memory store");
        }
    }
    CogSource::new(Arc::new(store))
}

/// Fixture file size in bytes (for provenance range bounds checks).
#[allow(
    dead_code,
    reason = "each integration-test binary uses a subset of common"
)]
pub(crate) fn file_len(name: &str) -> u64 {
    std::fs::metadata(fixtures_dir().join(name))
        .expect("fixture exists")
        .len()
}

/// Maps the manifest/numpy dtype strings to the port's `DType`.
pub(crate) fn dtype_from_str(s: &str) -> DType {
    match s {
        "uint8" => DType::UInt8,
        "int16" => DType::Int16,
        "uint16" => DType::UInt16,
        "int32" => DType::Int32,
        "float32" => DType::Float32,
        "float64" => DType::Float64,
        other => panic!("unexpected dtype in test data: {other}"),
    }
}
