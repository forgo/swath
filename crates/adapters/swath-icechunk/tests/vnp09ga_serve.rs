// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Gated real-data demo (#193, `just test-virtual`): the SAME VIIRS NDVI
//! Web Mercator tile rendered from a REAL VNP09GA granule twice — through
//! the manifest path (`VirtualSource`) and from an **Icechunk commit**
//! (`IcechunkSource`, snapshot-pinned) — must be **byte-identical PNGs**,
//! and the Icechunk render's Trace must carve its provenance out of the
//! original `.h5`, never the repository.
//!
//! Ignored by default: needs `SWATH_VNP09GA` (granule path) and
//! `SWATH_VNP09GA_MANIFEST` (its manifest); `just test-virtual`
//! orchestrates both after its oracle pdiff.

use std::collections::BTreeMap;
use std::sync::Arc;

use object_store::ObjectStoreExt as _;
use object_store::memory::InMemory;
use object_store::path::Path as StorePath;
use swath_core::crs::Crs;
use swath_core::raster::AssetRef;
use swath_core::tile::TileCoord;
use swath_icechunk::{IcechunkSource, commit_manifest};
use swath_manifest::VirtualManifest;
use swath_render::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::{NodataPolicy, Resampling, TileRequest, render_tile};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_virtual::VirtualSource;

/// The VNP09GA 1-km reflectance arrays NDVI needs (same as the manifest
/// path's gated test).
const M7: &str = "HDFEOS/GRIDS/VIIRS_Grid_1km_2D/Data Fields/SurfReflect_M7_1";
const M5: &str = "HDFEOS/GRIDS/VIIRS_Grid_1km_2D/Data Fields/SurfReflect_M5_1";

/// The same z9 tile `just test-virtual` oracle-checks.
const Z: u8 = 9;
const X: u32 = 509;
const Y: u32 = 302;

fn ndvi_request(m7: AssetRef, m5: AssetRef) -> TileRequest {
    let plan = RenderPlan::new(
        vec![BandInput::new("m7"), BandInput::new("m5")],
        vec![
            PixelOp::BandMath(
                (Expr::band("m7") - Expr::band("m5")) / (Expr::band("m7") + Expr::band("m5")),
            ),
            PixelOp::Rescale {
                min: -1.0,
                max: 1.0,
            },
            PixelOp::Colormap(Colormap::Grayscale),
        ],
        OutputSpec::new(TileFormat::Png),
    );
    let bands: BTreeMap<String, _> = [("m7".to_owned(), m7), ("m5".to_owned(), m5)]
        .into_iter()
        .collect();
    TileRequest::new(
        bands,
        plan,
        TileCoord::new(Z, X, Y).expect("valid tile"),
        256,
        Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
    )
}

#[tokio::test]
#[ignore = "needs a real VNP09GA granule + manifest (just test-virtual)"]
#[allow(
    clippy::too_many_lines,
    reason = "one linear gated scenario: stage both paths, render both, assert"
)]
async fn icechunk_commit_serves_the_ndvi_tile_byte_identical() {
    let Some(granule) = swath_testsupport::gated_var("SWATH_VNP09GA") else {
        return;
    };
    let Some(manifest_path) = swath_testsupport::gated_var("SWATH_VNP09GA_MANIFEST") else {
        return;
    };
    let granule_path = std::path::Path::new(&granule)
        .canonicalize()
        .expect("granule path resolves");
    let manifest = VirtualManifest::from_json_str(
        &std::fs::read_to_string(&manifest_path).expect("manifest readable"),
    )
    .expect("manifest parses");

    // Manifest path: in-memory store, filedrop naming.
    let granule_key = granule_path
        .file_name()
        .and_then(|n| n.to_str())
        .expect("granule file name")
        .to_owned();
    let manifest_key = format!("{granule_key}.vmanifest.json");
    let mut stored = manifest.clone();
    granule_key.clone_into(&mut stored.source);
    for array in &mut stored.arrays {
        for chunk in &mut array.refs {
            granule_key.clone_into(&mut chunk.path);
        }
    }
    let store = InMemory::new();
    store
        .put(
            &StorePath::from(granule_key.as_str()),
            std::fs::read(&granule_path)
                .expect("granule readable")
                .into(),
        )
        .await
        .expect("granule stored");
    store
        .put(
            &StorePath::from(manifest_key.as_str()),
            stored.to_json_string().into_bytes().into(),
        )
        .await
        .expect("manifest stored");
    let via_manifest = VirtualSource::new(Arc::new(store));

    // Icechunk path: commit, then serve snapshot-pinned.
    let tmp = swath_testsupport::TempDir::new("icechunk-vnp09ga-serve");
    let repo_dir = tmp.join("repo");
    let source_root = granule_path.parent().expect("granule has a directory");
    let outcome = commit_manifest(&repo_dir, &manifest, source_root, "vnp09ga serve demo")
        .await
        .expect("commit succeeds");
    let via_icechunk = IcechunkSource::new(&repo_dir);

    let (tile_a, trace_a) = render_tile(
        &via_manifest,
        &Proj4rsReproject,
        &ndvi_request(
            AssetRef::new(format!("{manifest_key}#{M7}")),
            AssetRef::new(format!("{manifest_key}#{M5}")),
        ),
    )
    .await
    .expect("manifest-path render succeeds");

    let snapshot = &outcome.snapshot_id;
    let (tile_b, trace_b) = render_tile(
        &via_icechunk,
        &Proj4rsReproject,
        &ndvi_request(
            AssetRef::new(format!("{snapshot}#{M7}")),
            AssetRef::new(format!("{snapshot}#{M5}")),
        ),
    )
    .await
    .expect("icechunk-path render succeeds");

    // The loop, closed: byte-identical tiles from the two routes.
    assert_eq!(tile_a.format, tile_b.format);
    assert_eq!(
        tile_a.bytes, tile_b.bytes,
        "the Icechunk-served NDVI tile is byte-identical to the manifest path's"
    );

    // Trace-visible: the Icechunk render reads real bytes from the
    // ORIGINAL granule (same total as the manifest path), with the
    // sinusoidal source CRS intact.
    assert_eq!(trace_a.bytes_read, trace_b.bytes_read);
    assert!(!trace_b.provenance.is_empty());
    assert!(
        trace_b
            .provenance
            .iter()
            .all(|r| r.path.ends_with(&granule_key)),
        "icechunk-path provenance points at the original .h5"
    );
    assert!(
        matches!(&trace_b.crs_from, Crs::Proj4(s) if s.starts_with("+proj=sinu")),
        "crs_from stays the sinusoidal proj string, got {}",
        trace_b.crs_from
    );
    #[allow(
        clippy::print_stdout,
        reason = "the gated demo's summary is its report"
    )]
    {
        println!(
            "icechunk serve demo PASS: z{Z}/{X}/{Y} NDVI byte-identical \
             ({} bytes PNG, {} provenance ranges, snapshot {snapshot})",
            tile_b.bytes.len(),
            trace_b.provenance.len(),
        );
    }
}
