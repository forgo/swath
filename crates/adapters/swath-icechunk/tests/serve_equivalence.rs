// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Read-back serving equivalence (#193), credential-free over the tiny
//! committed fixture: the SAME asset served through the manifest path
//! (`VirtualSource`) and through an Icechunk commit (`IcechunkSource`)
//! must be **byte-identical** at the port boundary — identical
//! `RasterInfo`, identical pixel bytes, provenance carving the same byte
//! ranges out of the original `.h5`. Everything downstream of the port
//! (warp, IR, encode) is a pure function of these, so port equality *is*
//! tile equality; the gated real-granule test (`vnp09ga_serve.rs`,
//! `just test-virtual`) additionally pins the rendered-PNG equality on a
//! real VNP09GA NDVI tile.

use std::sync::Arc;
use swath_testsupport::paths::referencer_data_dir as data_dir;

use object_store::ObjectStoreExt as _;
use object_store::memory::InMemory;
use object_store::path::Path as StorePath;
use swath_core::raster::{AssetRef, WindowRequest};
use swath_core::source::{BandSelection, RasterSource as _, ReadLevel};
use swath_icechunk::{IcechunkSource, commit_manifest};
use swath_referencer::SwathReferencer;
use swath_source_virtual::VirtualSource;

const NIR: &str = "HDFEOS/GRIDS/TinyGrid/Data Fields/nir";

#[tokio::test]
async fn icechunk_and_manifest_paths_serve_identical_bytes() {
    let tmp = swath_testsupport::TempDir::new("icechunk-serve-eq");
    let source_root = data_dir().canonicalize().expect("fixture dir");
    let granule = source_root.join("tiny.h5");
    let manifest = SwathReferencer::new()
        .generate(&granule)
        .expect("tiny fixture references");

    // Manifest path: in-memory store with granule + manifest, the
    // filedrop storage convention (store-relative chunk keys).
    let store = Arc::new(InMemory::new());
    let mut stored = manifest.clone();
    "tiny.h5".clone_into(&mut stored.source);
    for array in &mut stored.arrays {
        for chunk in &mut array.refs {
            "tiny.h5".clone_into(&mut chunk.path);
        }
    }
    store
        .put(
            &StorePath::from("tiny.h5"),
            std::fs::read(&granule).expect("fixture readable").into(),
        )
        .await
        .expect("granule stored");
    store
        .put(
            &StorePath::from("tiny.h5.vmanifest.json"),
            stored.to_json_string().into_bytes().into(),
        )
        .await
        .expect("manifest stored");
    let via_manifest = VirtualSource::new(store);

    // Icechunk path: commit the SAME manifest, serve from the commit —
    // pinned by snapshot id (the reproducible form) rather than a branch.
    let repo_dir = tmp.join("repo");
    let outcome = commit_manifest(&repo_dir, &manifest, &source_root, "serve equivalence")
        .await
        .expect("commit succeeds");
    let via_icechunk = IcechunkSource::new(&repo_dir);

    let manifest_asset = AssetRef::new(format!("tiny.h5.vmanifest.json#{NIR}"));
    let icechunk_asset = AssetRef::new(format!("{}#{NIR}", outcome.snapshot_id));

    // describe: identical RasterInfo (CRS, grid, transform, dtype, nodata).
    let info_a = via_manifest
        .describe(&manifest_asset)
        .await
        .expect("manifest describe");
    let info_b = via_icechunk
        .describe(&icechunk_asset)
        .await
        .expect("icechunk describe");
    assert_eq!(info_a, info_b, "RasterInfo identical through both paths");

    // read_window: full grid and a chunk-straddling interior window.
    for window in [
        WindowRequest {
            col_off: 0,
            row_off: 0,
            width: info_a.width,
            height: info_a.height,
        },
        WindowRequest {
            col_off: 2,
            row_off: 1,
            width: 4,
            height: 5,
        },
    ] {
        let a = via_manifest
            .read_window(
                &manifest_asset,
                window,
                BandSelection::Single(0),
                ReadLevel::FullRes,
            )
            .await
            .expect("manifest read");
        let b = via_icechunk
            .read_window(
                &icechunk_asset,
                window,
                BandSelection::Single(0),
                ReadLevel::FullRes,
            )
            .await
            .expect("icechunk read");
        assert_eq!(a.window, b.window, "clipped window identical");
        assert_eq!(a.pixels, b.pixels, "pixel bytes identical ({window:?})");
        assert_eq!(a.nodata, b.nodata);
        // Provenance: same byte ranges out of the same original granule
        // (paths differ only in store rooting: key vs absolute key).
        let ranges = |p: &[swath_core::trace::Provenance]| -> Vec<(u64, u64)> {
            p.iter().map(|r| (r.offset, r.length)).collect()
        };
        assert_eq!(
            ranges(&a.provenance),
            ranges(&b.provenance),
            "identical byte ranges carved from the original file"
        );
        assert!(
            b.provenance.iter().all(|r| r.path.ends_with("tiny.h5")),
            "icechunk-path provenance points at the original .h5"
        );
    }

    // The branch-tip form serves the same commit.
    let by_branch = AssetRef::new(format!("main#{NIR}"));
    let info_c = via_icechunk
        .describe(&by_branch)
        .await
        .expect("branch-tip describe");
    assert_eq!(info_a, info_c);
}

#[tokio::test]
async fn fragmentless_and_unknown_versions_are_refused() {
    let tmp = swath_testsupport::TempDir::new("icechunk-serve-err");
    let source_root = data_dir().canonicalize().expect("fixture dir");
    let manifest = SwathReferencer::new()
        .generate(&source_root.join("tiny.h5"))
        .expect("references");
    let repo_dir = tmp.join("repo");
    commit_manifest(&repo_dir, &manifest, &source_root, "err cases")
        .await
        .expect("commit succeeds");
    let source = IcechunkSource::new(&repo_dir);

    let err = source
        .describe(&AssetRef::new("no-fragment"))
        .await
        .expect_err("fragment-less asset refused");
    assert!(err.to_string().contains("format"), "{err}");

    let err = source
        .describe(&AssetRef::new(format!("not-a-version#{NIR}")))
        .await
        .expect_err("unknown version refused");
    assert!(
        err.to_string().contains("not-a-version") || err.to_string().contains("format"),
        "{err}"
    );
}
