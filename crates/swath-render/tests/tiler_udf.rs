// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `run_udf` stage on the tile path (ADR 0018, #205), through the
//! port with a host-side double: the layer budget's
//! `max_udf_fuel_per_tile` reaches the executor and its refusal is a
//! typed tile error; the stage's cost lands on the Trace; the cache key
//! binds the module identity *and* its params; and a cache hit never
//! consults the executor at all. The real wasmtime executor over the
//! committed module runs the same path end to end in swath-api
//! (`udf_tiles.rs`).

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use swath_cache_objectstore::ObjectStoreTileCache;
use swath_core::cache::{TileKey, TileKeyInputs, layer_version};
use swath_core::planner::Budget;
use swath_core::raster::AssetRef;
use swath_core::tile::TileCoord;
use swath_core::trace::Strategy;
use swath_render::ir::{
    BandInput, Colormap, OutputSpec, PixelOp, PlanError, RenderPlan, TileFormat,
};
use swath_render::udf::{UdfError, UdfExecutor, UdfLimits, UdfOutput, UdfStage};
use swath_render::{
    NodataPolicy, Resampling, TileError, TileRequest, WarpedBuffer, ndvi_expr, render_tile,
    render_tile_cached,
};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;

const B8A: &str = "hlss30-t13sdd-2024158-b8a.tif";
const B04: &str = "hlss30-t13sdd-2024158-b04.tif";
const HASH_A: &str = "aaaa000000000000000000000000000000000000000000000000000000000001";
const HASH_B: &str = "bbbb000000000000000000000000000000000000000000000000000000000002";

/// The fuel the double charges per tile — a plausible NDVI-sized cost.
const FUEL: u64 = 3_276_800;

fn cog_source() -> CogSource {
    let store = LocalFileSystem::new_with_prefix(common::fixtures_dir()).expect("fixture dir");
    CogSource::new(Arc::new(store))
}

/// `Udf(code_hash, params) → Rescale(-1..1)` — exactly what the compiler
/// emits for `run_udf → linear_scale_range`.
fn udf_plan(code_hash: &str, params: serde_json::Value) -> RenderPlan {
    RenderPlan::new(
        vec![BandInput::new("b8a"), BandInput::new("b04")],
        vec![
            PixelOp::Udf(UdfStage::new(code_hash, 1, params)),
            PixelOp::Rescale {
                min: -1.0,
                max: 1.0,
            },
        ],
        OutputSpec::new(TileFormat::Png),
    )
}

/// The built-in band-math NDVI the double mirrors.
fn band_math_plan() -> RenderPlan {
    RenderPlan::new(
        vec![BandInput::new("b8a"), BandInput::new("b04")],
        vec![
            PixelOp::BandMath(ndvi_expr("b8a", "b04")),
            PixelOp::Rescale {
                min: -1.0,
                max: 1.0,
            },
            PixelOp::Colormap(Colormap::Grayscale),
        ],
        OutputSpec::new(TileFormat::Png),
    )
}

fn request(plan: RenderPlan) -> TileRequest {
    TileRequest::new(
        BTreeMap::from([
            ("b8a".to_owned(), AssetRef::new(B8A)),
            ("b04".to_owned(), AssetRef::new(B04)),
        ]),
        plan,
        TileCoord::new(12, 848, 1561).expect("valid tile"),
        256,
        Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
    )
}

/// The key the serve wiring computes for `request`.
fn key(request: &TileRequest) -> TileKey {
    let plan_json = serde_json::to_string(&request.plan).expect("plan serializes");
    let version = layer_version(Some("g-2024158"), &plan_json);
    TileKey::compute(&TileKeyInputs {
        layer: "ndvi-udf",
        layer_version: &version,
        plan_json: &plan_json,
        tms: "WebMercatorQuad",
        coord: request.coord,
        tile_size: request.tile_size,
    })
}

/// A host-side NDVI executor double: the guest module's arithmetic
/// (`examples/udf/ndvi`) in Rust, charging [`FUEL`] per tile and refusing
/// — exactly as a fueled runtime would — when the limit is below it.
#[derive(Default)]
struct HostNdvi {
    calls: AtomicUsize,
}

impl UdfExecutor for HostNdvi {
    fn run(
        &self,
        _stage: &UdfStage,
        inputs: &[WarpedBuffer],
        limits: &UdfLimits,
    ) -> Result<UdfOutput, UdfError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if limits.max_fuel < FUEL {
            return Err(UdfError::FuelExhausted {
                budget: limits.max_fuel,
            });
        }
        let (nir, red) = (&inputs[0], &inputs[1]);
        let len = nir.values.len();
        let mut out = WarpedBuffer {
            width: nir.width,
            height: nir.height,
            values: vec![0.0; len],
            valid: vec![false; len],
        };
        for i in 0..len {
            if !nir.valid[i] || !red.valid[i] {
                continue;
            }
            let value = (nir.values[i] - red.values[i]) / (nir.values[i] + red.values[i]);
            if value.is_finite() {
                out.values[i] = value;
                out.valid[i] = true;
            }
        }
        Ok(UdfOutput::new(vec![out]).with_fuel_used(FUEL))
    }
}

/// An executor that must never be consulted.
struct PanicUdf;

impl UdfExecutor for PanicUdf {
    fn run(
        &self,
        _stage: &UdfStage,
        _inputs: &[WarpedBuffer],
        _limits: &UdfLimits,
    ) -> Result<UdfOutput, UdfError> {
        panic!("the executor was consulted by a path that must not reach it")
    }
}

/// The budget's fuel axis is what the executor sees, its cost is what
/// the Trace reports, and the UDF tile is byte-identical to the
/// band-math tile it mirrors (the tiler-level half of the
/// dual-implementation golden).
#[tokio::test]
async fn budget_fuel_reaches_the_executor_and_the_cost_lands_on_the_trace() {
    let source = cog_source();
    let executor = HostNdvi::default();
    let (udf_tile, trace) = render_tile(
        &source,
        &Proj4rsReproject,
        &executor,
        &request(udf_plan(HASH_A, serde_json::Value::Null)),
    )
    .await
    .expect("udf render succeeds");
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(trace.decision, Strategy::Live);
    assert_eq!(trace.udf_fuel_used, Some(FUEL));
    assert!(
        trace.timings.udf_ms <= trace.timings.pixel_ops_ms,
        "udf_ms is the UDF's share of pixel_ops_ms: {:?}",
        trace.timings
    );

    let (band_math_tile, band_math_trace) = render_tile(
        &source,
        &Proj4rsReproject,
        &PanicUdf,
        &request(band_math_plan()),
    )
    .await
    .expect("band-math render succeeds");
    assert_eq!(udf_tile.bytes, band_math_tile.bytes, "dual implementation");
    assert_eq!(band_math_trace.udf_fuel_used, None);
    assert_eq!(band_math_trace.timings.udf_ms, 0);

    // The budget is per layer: a fuel ceiling below the module's cost
    // refuses the tile — a typed error naming the budget, never a
    // degraded render.
    let starved = request(udf_plan(HASH_A, serde_json::Value::Null)).with_budget(Budget {
        max_udf_fuel_per_tile: FUEL - 1,
        ..Budget::default()
    });
    let err = render_tile(&source, &Proj4rsReproject, &executor, &starved)
        .await
        .expect_err("fuel must trip");
    assert!(
        matches!(
            &err,
            TileError::Plan(PlanError::Udf(UdfError::FuelExhausted { budget })) if *budget == FUEL - 1
        ),
        "unexpected error: {err:?}"
    );
}

/// Cache identity (#205): `plan_json` already carries the stage's
/// `code_hash` and `params`, so two plans differing in nothing else
/// produce different keys — and different layer versions — while equal
/// plans produce equal keys. Proven, not assumed.
#[test]
fn cache_key_binds_the_module_identity_and_its_params() {
    let base = request(udf_plan(HASH_A, serde_json::Value::Null));
    let same = request(udf_plan(HASH_A, serde_json::Value::Null));
    let other_module = request(udf_plan(HASH_B, serde_json::Value::Null));
    let with_params = request(udf_plan(HASH_A, serde_json::json!({ "gain": 1.5 })));
    let other_params = request(udf_plan(HASH_A, serde_json::json!({ "gain": 2.0 })));

    assert_eq!(key(&base), key(&same));
    let keys = [
        key(&base),
        key(&other_module),
        key(&with_params),
        key(&other_params),
    ];
    for (i, a) in keys.iter().enumerate() {
        for b in &keys[i + 1..] {
            assert_ne!(a, b, "keys must differ across module/params variants");
        }
    }
    let version = |r: &TileRequest| {
        layer_version(
            Some("g"),
            &serde_json::to_string(&r.plan).expect("plan serializes"),
        )
    };
    assert_ne!(version(&base), version(&other_module));
    assert_ne!(version(&with_params), version(&other_params));
}

/// A cache hit serves the stored bytes and **never runs the UDF**: the
/// executor is not consulted, and the hit Trace carries no UDF cost.
#[tokio::test]
async fn cache_hit_never_runs_the_udf() {
    let source = cog_source();
    let cache = ObjectStoreTileCache::new(Arc::new(InMemory::new()));
    let request = request(udf_plan(HASH_A, serde_json::json!({ "note": "cached" })));
    let key = key(&request);

    let executor = HostNdvi::default();
    let (first, first_trace) = render_tile_cached(
        &source,
        &Proj4rsReproject,
        &executor,
        &cache,
        &key,
        &request,
    )
    .await
    .expect("miss renders and writes through");
    assert_eq!(first_trace.decision, Strategy::Live);
    assert_eq!(first_trace.udf_fuel_used, Some(FUEL));
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);

    let (second, second_trace) = render_tile_cached(
        &source,
        &Proj4rsReproject,
        &PanicUdf,
        &cache,
        &key,
        &request,
    )
    .await
    .expect("hit serves without the executor");
    assert_eq!(second.bytes, first.bytes, "hit is byte-identical");
    assert!(matches!(second_trace.decision, Strategy::CacheHit { .. }));
    assert_eq!(second_trace.udf_fuel_used, None);
    assert_eq!(second_trace.timings.udf_ms, 0);
}
