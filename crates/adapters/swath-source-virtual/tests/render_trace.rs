// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The legacy thesis, in CI: a full `render_tile` over a virtual-cube
//! asset — sinusoidal source CRS, NDVI band math, Web Mercator tile out —
//! whose Trace provenance points at the ORIGINAL `.h5` granule, never at
//! the manifest and never at any converted copy (REQUIREMENTS.md R4,
//! ADR 0006). This is the offline miniature of the gated real-VNP09GA
//! validation (`just test-virtual`).

mod common;

use std::collections::BTreeMap;

use swath_core::crs::Crs;
use swath_core::tile::TileCoord;
use swath_core::trace::Strategy;
use swath_render::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::{NoUdf, NodataPolicy, Resampling, TileRequest, render_tile};
use swath_reproject_proj4rs::Proj4rsReproject;

/// The z11 `WebMercatorQuad` tile containing the `TinyGrid` center
/// (~173.3°E, 30.0°S — `make_tiny_fixture.py` places the grid at the
/// VNP09GA h33v12 upper-left corner).
const Z: u8 = 11;
const X: u32 = 2009;
const Y: u32 = 1203;

#[tokio::test]
async fn virtual_ndvi_render_traces_back_to_the_original_granule() {
    let source = common::memory_source().await;

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
    let bands: BTreeMap<String, _> = [
        ("nir".to_owned(), common::asset(common::NIR)),
        ("red".to_owned(), common::asset(common::RED)),
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

    let (tile, trace) = render_tile(&source, &Proj4rsReproject, &NoUdf, &request)
        .await
        .expect("virtual-cube render succeeds");
    assert_eq!(tile.format, TileFormat::Png);
    assert!(!tile.bytes.is_empty());

    // The x-ray payoff: the render's source CRS is the sinusoidal proj
    // string (straight out of StructMetadata), and every byte range in
    // the provenance points at the ORIGINAL granule file.
    assert_eq!(trace.decision, Strategy::Live);
    assert!(
        matches!(&trace.crs_from, Crs::Proj4(s) if s.starts_with("+proj=sinu")),
        "crs_from is the sinusoidal proj string, got {}",
        trace.crs_from
    );
    assert_eq!(trace.crs_to, Crs::WEB_MERCATOR);
    assert!(
        trace.bytes_read > 0,
        "the tile intersects the grid, so real bytes were read"
    );
    assert!(!trace.provenance.is_empty());
    for range in &trace.provenance {
        assert_eq!(
            range.path,
            common::GRANULE_KEY,
            "provenance points at the original .h5, not the manifest"
        );
        assert!(range.offset + range.length <= common::granule_len());
    }
    // The sources list names the manifest-fragment assets (the servable
    // identities), while provenance names the original bytes.
    assert!(
        trace
            .sources
            .iter()
            .all(|a| a.as_str().starts_with(common::MANIFEST_KEY)),
        "sources carry the virtual asset addressing"
    );
}
