// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The committed-fixture round trip (#191): reference the tiny HDF-EOS
//! fixture, commit its manifest to a fresh Icechunk repository, then read
//! it back through a **new** repository handle — metadata parsed, every
//! virtual chunk fetched through Icechunk's own container resolution and
//! compared **byte-identical** against the original file's declared range.
//!
//! This is the credential-free CI half of the conformance story; the
//! icechunk-python/xarray gate over a real VNP09GA granule runs in
//! `just test-referencer` (`tests/referencer/icechunk_check.py`).

use std::collections::HashMap;
use std::sync::Arc;
use swath_testsupport::paths::referencer_data_dir as data_dir;

use icechunk::config::Credentials;
use icechunk::format::ByteRange;
use icechunk::repository::VersionInfo;
use icechunk::store::Store;
use icechunk::{Repository, new_local_filesystem_storage};
use swath_icechunk::{CommitError, commit_manifest};
use swath_referencer::SwathReferencer;

const NIR: &str = "HDFEOS/GRIDS/TinyGrid/Data Fields/nir";

#[tokio::test]
async fn tiny_fixture_round_trips_byte_identical() {
    let tmp = swath_testsupport::TempDir::new("icechunk-roundtrip");
    let repo_dir = tmp.join("repo");
    let source_root = data_dir().canonicalize().expect("fixture dir exists");
    let granule = source_root.join("tiny.h5");

    let manifest = SwathReferencer::new()
        .generate(&granule)
        .expect("tiny fixture references");

    let outcome = commit_manifest(&repo_dir, &manifest, &source_root, "tiny fixture commit")
        .await
        .expect("commit succeeds");

    // The science arrays commit; the byte-string metadata blob is skipped
    // with an honest reason, never silently dropped.
    assert!(
        outcome.committed.iter().any(|name| name == NIR),
        "nir committed: {:?}",
        outcome.committed
    );
    assert!(
        outcome
            .skipped
            .iter()
            .any(|s| s.name == "meta" && s.reason.contains("dtype")),
        "meta skipped for its dtype: {:?}",
        outcome.skipped
    );

    // Read back through a FRESH repository handle: nothing carried over
    // from the writing session, exactly what an external reader does.
    let storage = new_local_filesystem_storage(&repo_dir)
        .await
        .expect("storage reopens");
    let prefix = format!(
        "{}/",
        url::Url::from_file_path(&source_root)
            .expect("absolute path")
            .to_string()
            .trim_end_matches('/')
    );
    let authorized: HashMap<String, Option<Credentials>> =
        HashMap::from([(prefix, Some(Credentials::LocalFileSystemAccess))]);
    let repo = Repository::open(None, storage, authorized)
        .await
        .expect("repository reopens");
    let session = repo
        .readonly_session(&VersionInfo::BranchTipRef("main".to_owned()))
        .await
        .expect("main branch has the commit");
    let store = Store::from_session(Arc::new(tokio::sync::RwLock::new(session))).await;

    // Array metadata round-trips with the fixture's known grid and the
    // numcodecs codec chain in encode order.
    let metadata = store
        .get(&format!("{NIR}/zarr.json"), &ByteRange::ALL)
        .await
        .expect("array metadata readable");
    let metadata: serde_json::Value = serde_json::from_slice(&metadata).expect("metadata is JSON");
    assert_eq!(metadata["shape"], serde_json::json!([8, 7]));
    assert_eq!(metadata["data_type"], serde_json::json!("int16"));
    assert_eq!(
        metadata["chunk_grid"]["configuration"]["chunk_shape"],
        serde_json::json!([3, 4])
    );
    let codec_names: Vec<&str> = metadata["codecs"]
        .as_array()
        .expect("codec list")
        .iter()
        .map(|c| c["name"].as_str().expect("codec name"))
        .collect();
    assert_eq!(
        codec_names,
        ["bytes", "numcodecs.shuffle", "numcodecs.zlib"]
    );

    // Every nir chunk fetched THROUGH Icechunk equals the original file's
    // declared byte range, byte for byte — the virtual refs point where
    // the manifest said, and the container resolves them.
    let file = std::fs::read(&granule).expect("fixture readable");
    let nir = manifest
        .arrays
        .iter()
        .find(|a| a.name == NIR)
        .expect("nir in manifest");
    assert!(!nir.refs.is_empty(), "nir has chunk refs");
    for chunk in &nir.refs {
        let key = format!("{NIR}/c/{}", chunk.key.replace('.', "/"));
        let via_icechunk = store
            .get(&key, &ByteRange::ALL)
            .await
            .unwrap_or_else(|err| panic!("chunk {key} fetches: {err}"));
        let expected = &file[usize::try_from(chunk.offset).expect("offset fits")..]
            [..usize::try_from(chunk.length).expect("length fits")];
        assert_eq!(
            via_icechunk.as_ref(),
            expected,
            "chunk {key}: bytes through Icechunk == bytes in the file"
        );
    }
}

#[tokio::test]
async fn all_unrepresentable_is_a_loud_error_not_an_empty_commit() {
    let tmp = swath_testsupport::TempDir::new("icechunk-empty");
    let manifest = swath_manifest::VirtualManifest::from_json_str(
        r#"{
            "manifest_version": 1,
            "generator": "test",
            "source": "none.h5",
            "arrays": [{
                "name": "meta", "shape": [], "chunks": [], "dtype": "|S8",
                "codecs": [], "refs": []
            }]
        }"#,
    )
    .expect("manifest parses");
    let err = commit_manifest(&tmp.join("repo"), &manifest, tmp.path(), "empty")
        .await
        .expect_err("nothing representable must not commit");
    assert!(matches!(err, CommitError::NothingToCommit { skipped: 1 }));
}
