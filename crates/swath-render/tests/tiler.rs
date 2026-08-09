// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `render_tile` end-to-end tests — the keystone regression for the serve
//! path (issue #26).
//!
//! Golden cases run the full orchestration (describe → window → read →
//! warp → pixel ops → encode) over the committed HLS fixtures and
//! perceptually diff the encoded tile against the same oracle goldens the
//! IR tests use (`just render-goldens`, `compose` subcommand), under the
//! **default** `swath-testkit` policy. Trace assertions are the R4 test:
//! the explanation of a render *is* the data these tests assert against.
//! Timings are asserted for presence and sanity only — they are
//! best-effort wall clock and never part of equality assertions.

#[allow(
    dead_code,
    reason = "shared with golden.rs; not every helper is used here"
)]
mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use object_store::local::LocalFileSystem;
use swath_core::crs::Crs;
use swath_core::raster::{AssetRef, RasterInfo, WindowRequest};
use swath_core::source::{BandSelection, RasterSource, SourceError, WindowData};
use swath_core::tile::TileCoord;
use swath_core::trace::{Strategy, Trace};
use swath_render::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::{EncodedTile, NodataPolicy, Resampling, TileError, TileRequest, render_tile};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;
use swath_testkit::{DiffPolicy, RgbaImage, diff, load_png};

const B02: &str = "hlss30-t13sdd-2024158-b02.tif";
const B03: &str = "hlss30-t13sdd-2024158-b03.tif";
const B04: &str = "hlss30-t13sdd-2024158-b04.tif";
const B8A: &str = "hlss30-t13sdd-2024158-b8a.tif";

/// The reflectance-band kernel, as in the golden suites.
const BILINEAR: Resampling = Resampling::Bilinear(NodataPolicy::ExcludeRenormalize);

fn cog_source() -> CogSource {
    let store = LocalFileSystem::new_with_prefix(common::fixtures_dir()).expect("fixture dir");
    CogSource::new(Arc::new(store))
}

fn tile(z: u8, x: u32, y: u32) -> TileCoord {
    TileCoord::new(z, x, y).expect("valid tile")
}

fn bands(pairs: &[(&str, &str)]) -> BTreeMap<String, AssetRef> {
    pairs
        .iter()
        .map(|(name, fixture)| ((*name).to_owned(), AssetRef::new(*fixture)))
        .collect()
}

/// The true-color plan of the IR golden suite: BGR+red fixtures composited
/// RGB, rescaled 0..3000.
fn truecolor_request(z: u8, x: u32, y: u32) -> TileRequest {
    let plan = RenderPlan::new(
        vec![
            BandInput::new("b04"),
            BandInput::new("b03"),
            BandInput::new("b02"),
        ],
        vec![
            PixelOp::Composite {
                r: "b04".into(),
                g: "b03".into(),
                b: "b02".into(),
            },
            PixelOp::Rescale {
                min: 0.0,
                max: 3000.0,
            },
        ],
        OutputSpec::new(TileFormat::Png),
    );
    TileRequest::new(
        bands(&[("b04", B04), ("b03", B03), ("b02", B02)]),
        plan,
        tile(z, x, y),
        256,
        BILINEAR,
    )
}

/// The NDVI plan of the IR golden suite: `(nir - red) / (nir + red)`,
/// rescaled -1..1, grayscale.
fn ndvi_request(z: u8, x: u32, y: u32) -> TileRequest {
    let plan = RenderPlan::new(
        vec![BandInput::new("b8a"), BandInput::new("b04")],
        vec![
            PixelOp::BandMath(
                (Expr::band("b8a") - Expr::band("b04")) / (Expr::band("b8a") + Expr::band("b04")),
            ),
            PixelOp::Rescale {
                min: -1.0,
                max: 1.0,
            },
            PixelOp::Colormap(Colormap::Grayscale),
        ],
        OutputSpec::new(TileFormat::Png),
    );
    TileRequest::new(
        bands(&[("b8a", B8A), ("b04", B04)]),
        plan,
        tile(z, x, y),
        256,
        BILINEAR,
    )
}

async fn render(request: &TileRequest) -> (EncodedTile, Trace) {
    render_tile(&cog_source(), &Proj4rsReproject, request)
        .await
        .expect("render_tile succeeds")
}

fn decode(encoded: &EncodedTile) -> RgbaImage {
    assert_eq!(encoded.format, TileFormat::Png);
    image::load_from_memory(&encoded.bytes)
        .expect("encoded tile decodes")
        .into_rgba8()
}

// --- Golden tests: full pipeline vs the oracle `compose` renders ---

#[allow(clippy::print_stdout, reason = "diff metrics are the test's report")]
async fn assert_matches_oracle(request: &TileRequest, golden: &str) {
    let (encoded, trace) = render(request).await;
    let ours = decode(&encoded);
    let reference = load_png(&common::goldens_dir().join(golden)).expect("golden loads");

    let report = diff(&ours, &reference).expect("dimensions match");
    let policy = DiffPolicy::default();
    let bad_pct = report.pct_pixels_exceeding_tolerance(policy.per_channel_tolerance) * 100.0;
    println!(
        "{golden}: max |diff| {max}, mean |diff| {mean:.4}, bad pixels {bad_pct:.4}% \
         (bytes_read {bytes})",
        max = report.max_abs_channel_diff,
        mean = report.mean_abs_diff,
        bytes = trace.bytes_read,
    );
    assert!(
        report.passes(&policy),
        "{golden}: fails default policy — max |diff| {}, {bad_pct:.4}% pixels over tolerance {}",
        report.max_abs_channel_diff,
        policy.per_channel_tolerance,
    );
}

#[tokio::test]
async fn truecolor_interior_tile_matches_oracle() {
    assert_matches_oracle(
        &truecolor_request(12, 848, 1561),
        "truecolor-12-848-1561.png",
    )
    .await;
}

#[tokio::test]
async fn truecolor_swath_edge_tile_matches_oracle() {
    assert_matches_oracle(
        &truecolor_request(12, 848, 1562),
        "truecolor-12-848-1562.png",
    )
    .await;
}

#[tokio::test]
async fn ndvi_interior_tile_matches_oracle() {
    assert_matches_oracle(&ndvi_request(12, 848, 1561), "ndvi-12-848-1561.png").await;
}

#[tokio::test]
async fn ndvi_swath_edge_tile_matches_oracle() {
    assert_matches_oracle(&ndvi_request(12, 848, 1562), "ndvi-12-848-1562.png").await;
}

// --- Trace assertions: the R4 keystone ---

#[tokio::test]
async fn trace_explains_the_true_color_render() {
    let (_, trace) = render(&truecolor_request(12, 848, 1561)).await;

    // Strategy: no planner yet (#37) — always a live full-res render.
    assert_eq!(trace.decision, Strategy::Live);

    // Sources: primary is the first declared input's asset; the full list
    // covers every distinct asset, in declaration order.
    assert_eq!(trace.source, AssetRef::new(B04));
    assert_eq!(
        trace.sources,
        vec![AssetRef::new(B04), AssetRef::new(B03), AssetRef::new(B02)]
    );

    // CRSs, for real: fixture UTM zone in, Web Mercator out.
    assert_eq!(trace.crs_from, Crs::from_epsg(32613));
    assert_eq!(trace.crs_to, Crs::WEB_MERCATOR);

    // Provenance: non-empty, every range within its object's real size,
    // and bytes_read is exactly the sum of range lengths.
    assert!(
        !trace.provenance.is_empty(),
        "live render must report reads"
    );
    for range in &trace.provenance {
        let size = std::fs::metadata(common::fixtures_dir().join(&range.path))
            .expect("provenance path is a fixture file")
            .len();
        assert!(
            range.offset + range.length <= size,
            "range {}+{} exceeds {} ({} bytes)",
            range.offset,
            range.length,
            range.path,
            size,
        );
    }
    let sum: u64 = trace.provenance.iter().map(|p| p.length).sum();
    assert_eq!(trace.bytes_read, sum);

    // Timings: best-effort wall clock — presence and sanity only, never
    // equality (parts need not sum to total once stages overlap).
    assert!(trace.timings.total_ms > 0, "a real render takes time");

    // Ingest-to-pixel only exists for granule-backed requests (#31): a
    // plain request leaves it unset.
    assert_eq!(trace.ingest_to_pixel_ms, None);
}

/// The north-star timer (#31): a request carrying `ingested_at` yields a
/// Trace whose `ingest_to_pixel_ms` is elapsed-since-ingest — bounded below
/// by the known offset and sane above; a future `ingested_at` (clock skew)
/// clamps to 0 rather than going negative.
#[tokio::test]
async fn trace_carries_ingest_to_pixel_for_granule_backed_requests() {
    use swath_core::catalog::Datetime;

    let now_ms = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("present after epoch")
            .as_millis(),
    )
    .expect("fits i64");

    // Ingested 10 seconds "ago": the number must be >= that offset.
    let ingested = Datetime::from_unix_millis(now_ms - 10_000).expect("in range");
    let request = truecolor_request(12, 848, 1561).with_ingested_at(ingested);
    let (_, trace) = render(&request).await;
    let i2p = trace.ingest_to_pixel_ms.expect("granule-backed => timed");
    assert!(
        (10_000..600_000).contains(&i2p),
        "elapsed-since-ingest should be >= the 10s offset and sane, got {i2p}"
    );

    // Skew guard: an ingest stamp in the future clamps to 0.
    let future = Datetime::from_unix_millis(now_ms + 3_600_000).expect("in range");
    let request = truecolor_request(12, 848, 1561).with_ingested_at(future);
    let (_, trace) = render(&request).await;
    assert_eq!(trace.ingest_to_pixel_ms, Some(0));
}

/// The serialized Trace is the SSE/UI contract: assert the *schema* (key
/// sets), not the values — values are covered above and timings are
/// non-deterministic. `serde_json` orders object keys alphabetically, so
/// key sets are compared sorted.
#[allow(clippy::print_stdout, reason = "the trace is the test's report")]
#[tokio::test]
async fn trace_json_schema_matches_the_pinned_contract() {
    fn sorted_keys(value: &serde_json::Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value
            .as_object()
            .expect("a JSON object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }

    let (_, trace) = render(&truecolor_request(12, 848, 1561)).await;
    let json = serde_json::to_value(&trace).expect("trace serializes");

    assert_eq!(
        sorted_keys(&json),
        [
            "bytes_read",
            "crs_from",
            "crs_to",
            "decision",
            "ingest_to_pixel_ms",
            "provenance",
            "source",
            "sources",
            "timings",
        ],
        "Trace JSON keys drifted from the pinned contract (swath-core trace.rs)"
    );

    assert_eq!(
        sorted_keys(&json["timings"]),
        [
            "encode_ms",
            "pixel_ops_ms",
            "read_ms",
            "total_ms",
            "warp_ms"
        ],
        "every timing stage must be recorded"
    );

    assert_eq!(
        sorted_keys(&json["provenance"][0]),
        ["length", "offset", "path"]
    );

    // The full record, for eyeballing what the x-ray actually says.
    println!(
        "{}",
        serde_json::to_string_pretty(&json).expect("pretty-prints")
    );
}

#[tokio::test]
async fn rendering_is_deterministic() {
    let request = truecolor_request(12, 848, 1561);
    let (tile_a, trace_a) = render(&request).await;
    let (tile_b, trace_b) = render(&request).await;
    assert_eq!(tile_a.bytes, tile_b.bytes, "PNG bytes must be identical");
    assert_eq!(
        trace_a.provenance, trace_b.provenance,
        "provenance must be identical, order included"
    );
    assert_eq!(trace_a.sources, trace_b.sources);
    assert_eq!(trace_a.bytes_read, trace_b.bytes_read);
}

// --- Edge behaviors ---

#[tokio::test]
async fn tile_outside_the_raster_is_transparent_and_explained() {
    // ~78 km west of the fixture footprint: valid tile, no source data.
    let (encoded, trace) = render(&truecolor_request(12, 840, 1561)).await;

    let image = decode(&encoded);
    assert_eq!(image.dimensions(), (256, 256));
    assert!(
        image.pixels().all(|p| p.0 == [0, 0, 0, 0]),
        "an off-raster tile must be fully transparent"
    );

    // A served empty tile is still explained (R4): nothing was read, and
    // the trace says so.
    assert_eq!(trace.decision, Strategy::Live);
    assert!(trace.provenance.is_empty());
    assert_eq!(trace.bytes_read, 0);
    assert_eq!(trace.crs_from, Crs::from_epsg(32613));
}

#[tokio::test]
async fn missing_band_asset_is_an_error() {
    let mut request = truecolor_request(12, 848, 1561);
    request.bands.remove("b02");
    let err = render_tile(&cog_source(), &Proj4rsReproject, &request)
        .await
        .expect_err("unmapped band must fail");
    assert!(
        matches!(&err, TileError::MissingBand { band } if band == "b02"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn plan_without_inputs_is_an_error() {
    let request = TileRequest::new(
        BTreeMap::new(),
        RenderPlan::new(vec![], vec![], OutputSpec::new(TileFormat::Png)),
        tile(12, 848, 1561),
        256,
        BILINEAR,
    );
    let err = render_tile(&cog_source(), &Proj4rsReproject, &request)
        .await
        .expect_err("empty plan must fail");
    assert!(matches!(err, TileError::NoBands), "unexpected error: {err}");
}

/// A source wrapper that lies about one asset's CRS — the fixtures are all
/// single-CRS, so mixed-CRS inputs are simulated at the port boundary.
struct CrsPatch {
    inner: CogSource,
    patched: AssetRef,
}

impl RasterSource for CrsPatch {
    async fn describe(&self, asset: &AssetRef) -> Result<RasterInfo, SourceError> {
        let mut info = self.inner.describe(asset).await?;
        if *asset == self.patched {
            info.crs = Crs::from_epsg(32614);
        }
        Ok(info)
    }

    async fn read_window(
        &self,
        asset: &AssetRef,
        window: WindowRequest,
        band: BandSelection,
    ) -> Result<WindowData, SourceError> {
        self.inner.read_window(asset, window, band).await
    }
}

#[tokio::test]
async fn mixed_source_crs_is_a_clear_unsupported_error() {
    let source = CrsPatch {
        inner: cog_source(),
        patched: AssetRef::new(B02),
    };
    let err = render_tile(
        &source,
        &Proj4rsReproject,
        &truecolor_request(12, 848, 1561),
    )
    .await
    .expect_err("mixed CRSs must fail loudly, never silently pick one");
    match &err {
        TileError::MixedCrs {
            expected,
            found,
            band,
            ..
        } => {
            assert_eq!(*expected, Crs::from_epsg(32613));
            assert_eq!(*found, Crs::from_epsg(32614));
            assert_eq!(band, "b02");
        }
        other => panic!("unexpected error: {other}"),
    }
}
