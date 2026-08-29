// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `render_tile_cached` end-to-end (issue #36): miss → live render +
//! write-through, hit → byte-identical bytes with an honest `CacheHit`
//! Trace, version bump → clean miss, and a failing cache that never
//! fails a response.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use swath_core::cache::{CacheError, CachedTile, TileCache, TileKey, TileKeyInputs, layer_version};
use swath_core::crs::Crs;
use swath_core::planner::{Budget, PlannedStrategy};
use swath_core::raster::AssetRef;
use swath_core::tile::TileCoord;
use swath_core::trace::Strategy;
use swath_render::ir::{BandInput, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::{NoUdf, NodataPolicy, Resampling, TileRequest, render_tile_cached};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;
use swath_store_objectstore::ObjectStoreTileCache;

const B04: &str = "hlss30-t13sdd-2024158-b04.tif";

fn cog_source() -> CogSource {
    let store = LocalFileSystem::new_with_prefix(common::fixtures_dir()).expect("fixture dir");
    CogSource::new(Arc::new(store))
}

/// A single-band grayscale-ish request over the committed B04 fixture —
/// small and real (actual reads, actual PNG).
fn request() -> TileRequest {
    let plan = RenderPlan::new(
        vec![BandInput::new("b04")],
        vec![
            PixelOp::Composite {
                r: "b04".into(),
                g: "b04".into(),
                b: "b04".into(),
            },
            PixelOp::Rescale {
                min: 0.0,
                max: 3000.0,
            },
        ],
        OutputSpec::new(TileFormat::Png),
    );
    TileRequest::new(
        BTreeMap::from([("b04".to_owned(), AssetRef::new(B04))]),
        plan,
        TileCoord::new(12, 848, 1561).expect("valid tile"),
        256,
        Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
    )
}

/// The key the serve wiring would compute for `request()` under `granule`.
fn key(request: &TileRequest, granule: &str) -> TileKey {
    let plan_json = serde_json::to_string(&request.plan).expect("plan serializes");
    let version = layer_version(Some(granule), &plan_json);
    TileKey::compute(&TileKeyInputs {
        layer: "b04-gray",
        layer_version: &version,
        plan_json: &plan_json,
        tms: "WebMercatorQuad",
        coord: request.coord,
        tile_size: request.tile_size,
    })
}

#[tokio::test]
async fn miss_renders_live_then_hit_serves_identical_bytes() {
    let source = cog_source();
    let cache = ObjectStoreTileCache::new(Arc::new(InMemory::new()));
    let request = request();
    let key = key(&request, "g-2024158");

    // First request: live render (miss), written through.
    let (first, first_trace) =
        render_tile_cached(&source, &Proj4rsReproject, &NoUdf, &cache, &key, &request)
            .await
            .expect("renders");
    assert_eq!(first_trace.decision, Strategy::Live);
    assert!(first_trace.bytes_read > 0, "a live render reads sources");

    // Second request: served from cache, byte-identical.
    let (second, second_trace) =
        render_tile_cached(&source, &Proj4rsReproject, &NoUdf, &cache, &key, &request)
            .await
            .expect("serves from cache");
    assert_eq!(second.bytes, first.bytes, "hit must be byte-identical");
    assert_eq!(second.format, first.format);
    assert_eq!(
        second_trace.decision,
        Strategy::CacheHit {
            key: key.as_str().to_owned()
        }
    );

    // The hit Trace's documented field semantics (render_tile_cached docs).
    assert_eq!(second_trace.bytes_read, 0, "no source bytes on a hit");
    assert!(second_trace.provenance.is_empty());
    assert_eq!(second_trace.source, AssetRef::new(format!("cache://{key}")));
    assert_eq!(second_trace.sources, vec![second_trace.source.clone()]);
    assert_eq!(second_trace.crs_from, Crs::WEB_MERCATOR);
    assert_eq!(second_trace.crs_to, Crs::WEB_MERCATOR);
    assert_eq!(second_trace.timings.read_ms, 0);
    assert_eq!(second_trace.timings.warp_ms, 0);
    assert_eq!(second_trace.timings.pixel_ops_ms, 0);
    assert_eq!(second_trace.timings.encode_ms, 0);
    assert_eq!(second_trace.ingest_to_pixel_ms, None);

    // The plan shows the work on both requests (#37): the miss weighed
    // all three candidates and chose live (no overview decimates at
    // z12); the hit is the planner's terminal cache choice, with the
    // payload length as its estimate and the alternatives honestly
    // marked unestimated (a hit must never trigger source metadata I/O).
    let miss_plan = first_trace.plan.as_ref().expect("miss carries a plan");
    assert_eq!(miss_plan.chosen, PlannedStrategy::Live);
    assert_eq!(miss_plan.considered.len(), 3);
    assert_eq!(miss_plan.considered[0].reason, "cache miss");
    let hit_plan = second_trace.plan.as_ref().expect("hit carries a plan");
    assert_eq!(hit_plan.chosen, PlannedStrategy::CacheHit);
    assert_eq!(hit_plan.considered.len(), 3);
    assert!(hit_plan.considered[0].admissible);
    assert_eq!(
        hit_plan.considered[0].estimated_cost_bytes,
        second.bytes.len() as u64,
        "the cache candidate's estimate is the stored payload length"
    );
    for other in &hit_plan.considered[1..] {
        assert!(!other.admissible);
        assert_eq!(other.reason, "not estimated: cache hit short-circuits");
    }
}

/// The budget's cache knob (#37): `cache_enabled = false` opts the layer
/// out entirely — no probe (the trace says "cache disabled by budget"),
/// no write-through (a later enabled request still misses).
#[tokio::test]
async fn disabled_cache_budget_skips_probe_and_write_through() {
    let source = cog_source();
    let cache = ObjectStoreTileCache::new(Arc::new(InMemory::new()));
    let request = request().with_budget(Budget {
        cache_enabled: false,
        ..Budget::default()
    });
    let key = key(&request, "g-2024158");

    // Two identical requests: both live (nothing is ever stored).
    for _ in 0..2 {
        let (_, trace) =
            render_tile_cached(&source, &Proj4rsReproject, &NoUdf, &cache, &key, &request)
                .await
                .expect("renders");
        assert_eq!(trace.decision, Strategy::Live);
        let plan = trace.plan.expect("planned");
        assert_eq!(plan.considered[0].reason, "cache disabled by budget");
    }

    // The write-through was skipped too: re-enabling the cache still
    // misses under the same key.
    let enabled = request.clone().with_budget(Budget::default());
    let (_, trace) = render_tile_cached(&source, &Proj4rsReproject, &NoUdf, &cache, &key, &enabled)
        .await
        .expect("renders");
    assert_eq!(
        trace.decision,
        Strategy::Live,
        "nothing was stored while the budget had the cache off"
    );
}

#[tokio::test]
async fn a_new_granule_version_misses_cleanly() {
    let source = cog_source();
    let cache = ObjectStoreTileCache::new(Arc::new(InMemory::new()));
    let request = request();

    let (_, trace) = render_tile_cached(
        &source,
        &Proj4rsReproject,
        &NoUdf,
        &cache,
        &key(&request, "g-2024158"),
        &request,
    )
    .await
    .expect("renders");
    assert_eq!(trace.decision, Strategy::Live);

    // Same layer, same tile — but a new granule arrived: new version,
    // new key, honest miss (the §10 invalidation story).
    let (_, trace) = render_tile_cached(
        &source,
        &Proj4rsReproject,
        &NoUdf,
        &cache,
        &key(&request, "g-2024165"),
        &request,
    )
    .await
    .expect("renders");
    assert_eq!(trace.decision, Strategy::Live, "new version must miss");
}

/// A cache whose every operation fails — the disk is full, the bucket
/// is gone, the directory is read-only.
struct BrokenCache {
    puts: AtomicUsize,
}

impl TileCache for BrokenCache {
    async fn get(&self, key: &TileKey) -> Result<Option<CachedTile>, CacheError> {
        Err(CacheError::Io {
            key: key.clone(),
            source: "cache backend unavailable".into(),
        })
    }

    async fn put(&self, key: &TileKey, _: &[u8], _: &str) -> Result<(), CacheError> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        Err(CacheError::Io {
            key: key.clone(),
            source: "cache backend unavailable".into(),
        })
    }
}

#[tokio::test]
async fn cache_failures_never_fail_the_response() {
    let source = cog_source();
    let cache = BrokenCache {
        puts: AtomicUsize::new(0),
    };
    let request = request();
    let key = key(&request, "g-2024158");

    let (encoded, trace) =
        render_tile_cached(&source, &Proj4rsReproject, &NoUdf, &cache, &key, &request)
            .await
            .expect("a broken cache must not break serving");
    assert_eq!(trace.decision, Strategy::Live);
    assert!(!encoded.bytes.is_empty());
    assert_eq!(
        cache.puts.load(Ordering::SeqCst),
        1,
        "the write-through was attempted (and its failure swallowed)"
    );
}

/// The read-only-directory shape of the same guarantee, with the real
/// filesystem adapter: the first render succeeds and stays `Live`, the
/// second stays `Live` too (nothing was ever stored).
#[cfg(unix)]
#[tokio::test]
async fn read_only_cache_directory_still_serves_live() {
    use std::os::unix::fs::PermissionsExt;

    let dir = swath_testsupport::TempDir::new("ro-cache");
    let mut perms = std::fs::metadata(dir.path()).expect("stat").permissions();
    perms.set_mode(0o555);
    std::fs::set_permissions(dir.path(), perms.clone()).expect("chmod");

    let source = cog_source();
    let store = LocalFileSystem::new_with_prefix(dir.path()).expect("store opens");
    let cache = ObjectStoreTileCache::new(Arc::new(store));
    let request = request();
    let key = key(&request, "g-2024158");

    for _ in 0..2 {
        let (encoded, trace) =
            render_tile_cached(&source, &Proj4rsReproject, &NoUdf, &cache, &key, &request)
                .await
                .expect("read-only cache must not break serving");
        assert_eq!(trace.decision, Strategy::Live);
        assert!(!encoded.bytes.is_empty());
    }

    // Restore write permission so TempDir::drop can clean up.
    perms.set_mode(0o755);
    std::fs::set_permissions(dir.path(), perms).expect("chmod back");
}
