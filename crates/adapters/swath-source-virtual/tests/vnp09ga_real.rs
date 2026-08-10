// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Gated real-data validation (`just test-virtual`, ADR 0008): render a
//! VIIRS NDVI Web Mercator tile from a REAL VNP09GA granule through the
//! full virtual-reference path — manifest → chunk-range reads into the
//! original `.h5` → decode → sinusoidal warp → band math — and leave the
//! PNG for the recipe to perceptually diff against a GDAL oracle render
//! of the SAME tile from the SAME original file.
//!
//! Ignored by default: needs `SWATH_VNP09GA` (the granule path) and
//! `SWATH_VNP09GA_MANIFEST` (the manifest `swath ingest reference` wrote
//! for it); `SWATH_VIRTUAL_OUT` names where the rendered PNG goes.
//! `just test-virtual` orchestrates all three plus the oracle + pdiff.

use std::collections::BTreeMap;
use std::sync::Arc;

use object_store::ObjectStoreExt as _;
use object_store::memory::InMemory;
use object_store::path::Path as StorePath;
use swath_core::crs::Crs;
use swath_core::manifest::VirtualManifest;
use swath_core::raster::AssetRef;
use swath_core::tile::TileCoord;
use swath_render::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::{NodataPolicy, Resampling, TileRequest, render_tile};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_virtual::VirtualSource;

/// The VNP09GA 1-km reflectance arrays NDVI needs (VIIRS NDVI =
/// (M7 − M5) / (M7 + M5); band identity asserted from the manifest).
const M7: &str = "HDFEOS/GRIDS/VIIRS_Grid_1km_2D/Data Fields/SurfReflect_M7_1";
const M5: &str = "HDFEOS/GRIDS/VIIRS_Grid_1km_2D/Data Fields/SurfReflect_M5_1";

/// The z9 `WebMercatorQuad` tile rendered: ~178°E / 31.3°S, inside the
/// h33v12 granule's valid-data (non-fill) northwest region, west of the
/// antimeridian wrap. `just test-virtual` renders the same tile with the
/// GDAL oracle.
const Z: u8 = 9;
const X: u32 = 509;
const Y: u32 = 302;

#[tokio::test]
#[ignore = "needs a real VNP09GA granule + manifest (just test-virtual)"]
#[allow(
    clippy::too_many_lines,
    reason = "one linear gated scenario: stage, render, assert, emit"
)]
async fn real_vnp09ga_ndvi_tile_renders_from_original_bytes() {
    // Belt and braces: even under --ignored, an absent granule skips
    // cleanly (never panics) — `just test-virtual` provides both variables.
    let Some(granule) = swath_testsupport::gated_var("SWATH_VNP09GA") else {
        return;
    };
    let Some(manifest_path) = swath_testsupport::gated_var("SWATH_VNP09GA_MANIFEST") else {
        return;
    };
    let out = std::env::var("SWATH_VIRTUAL_OUT")
        .unwrap_or_else(|_| "target/virtual/swath-ndvi.png".to_owned());

    // Stage granule + manifest into a store under the filedrop naming:
    // store-relative keys, manifest alongside the original file.
    let granule_key = std::path::Path::new(&granule)
        .file_name()
        .and_then(|n| n.to_str())
        .expect("granule file name")
        .to_owned();
    let manifest_key = format!("{granule_key}.vmanifest.json");
    let mut manifest = VirtualManifest::from_json_str(
        &std::fs::read_to_string(&manifest_path).expect("manifest readable"),
    )
    .expect("manifest parses");
    granule_key.clone_into(&mut manifest.source);
    for array in &mut manifest.arrays {
        for chunk in &mut array.refs {
            granule_key.clone_into(&mut chunk.path);
        }
    }

    // Band identity comes from the manifest, not assumption: the M7/M5
    // arrays exist, are georeferenced sinusoidal, and carry their band
    // names.
    for (array_name, band) in [(M7, "SurfReflect_M7_1"), (M5, "SurfReflect_M5_1")] {
        let array = manifest
            .arrays
            .iter()
            .find(|a| a.name == array_name)
            .unwrap_or_else(|| panic!("manifest lacks `{array_name}`"));
        let georef = array.georef.as_ref().expect("reflectance is georeferenced");
        assert_eq!(georef.band.as_deref(), Some(band));
        assert!(
            matches!(&georef.crs, swath_core::manifest::GeorefCrs::Proj4(s)
                if s.starts_with("+proj=sinu")),
            "sinusoidal grid"
        );
    }

    let store = InMemory::new();
    store
        .put(
            &StorePath::from(granule_key.clone()),
            std::fs::read(&granule).expect("granule readable").into(),
        )
        .await
        .expect("stage granule");
    store
        .put(
            &StorePath::from(manifest_key.clone()),
            manifest.to_json_string().into_bytes().into(),
        )
        .await
        .expect("stage manifest");
    let source = VirtualSource::new(Arc::new(store));

    // The NDVI plan, matching the oracle compose invocation exactly:
    // (M7 − M5)/(M7 + M5), rescaled −1..1, grayscale, bilinear.
    let plan = RenderPlan::new(
        vec![BandInput::new("nir"), BandInput::new("red")],
        vec![
            PixelOp::BandMath(
                (Expr::band("nir") - Expr::band("red")) / (Expr::band("nir") + Expr::band("red")),
            ),
            PixelOp::Rescale {
                min: -1.0,
                max: 1.0,
            },
            PixelOp::Colormap(Colormap::Grayscale),
        ],
        OutputSpec::new(TileFormat::Png),
    );
    let bands: BTreeMap<String, AssetRef> = [
        (
            "nir".to_owned(),
            AssetRef::new(format!("{manifest_key}#{M7}")),
        ),
        (
            "red".to_owned(),
            AssetRef::new(format!("{manifest_key}#{M5}")),
        ),
    ]
    .into_iter()
    .collect();
    let request = TileRequest::new(
        bands,
        plan,
        TileCoord::new(Z, X, Y).expect("valid tile"),
        256,
        Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
    );

    let (tile, trace) = render_tile(&source, &Proj4rsReproject, &request)
        .await
        .expect("real-granule virtual render succeeds");

    // The legacy thesis on real data: every byte range this render read
    // points at the ORIGINAL granule file and is one of the manifest's
    // chunk refs — the granule was served, never converted.
    assert!(
        matches!(&trace.crs_from, Crs::Proj4(s) if s.starts_with("+proj=sinu")),
        "crs_from is sinusoidal, got {}",
        trace.crs_from
    );
    assert!(trace.bytes_read > 0, "the tile reads real granule bytes");
    let file_len = std::fs::metadata(&granule).expect("granule metadata").len();
    let all_refs: Vec<(u64, u64)> = manifest
        .arrays
        .iter()
        .flat_map(|a| &a.refs)
        .map(|r| (r.offset, r.length))
        .collect();
    for range in &trace.provenance {
        assert_eq!(range.path, granule_key, "provenance names the original .h5");
        assert!(range.offset + range.length <= file_len);
        assert!(
            all_refs.contains(&(range.offset, range.length)),
            "range {}+{} is not a manifest chunk ref",
            range.offset,
            range.length
        );
    }

    if let Some(parent) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(parent).expect("output dir");
    }
    std::fs::write(&out, &tile.bytes).expect("write rendered tile");
    // The recipe pdiffs this PNG against the GDAL oracle render of the
    // same tile from the same original file.
}
