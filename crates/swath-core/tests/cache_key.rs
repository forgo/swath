// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Property tests for the `TileKey` scheme (issue #36): determinism and
//! input-sensitivity over arbitrary plan/coordinate variations. The exact
//! digest for a known input is pinned in the `cache` module's unit tests;
//! these properties cover the space around it.

use proptest::prelude::{ProptestConfig, Strategy, prop_assert_eq, prop_assert_ne, proptest};
use swath_core::cache::{TileKey, TileKeyInputs, layer_version};
use swath_core::tile::TileCoord;

/// Any valid tile at zoom 0..=24 (the served `WebMercatorQuad` range).
fn arb_tile() -> impl Strategy<Value = TileCoord> {
    (0u8..=24).prop_flat_map(|z| {
        let max = 1u32 << z;
        (0..max, 0..max).prop_map(move |(x, y)| TileCoord::new(z, x, y).expect("in range"))
    })
}

/// Plan-JSON-shaped strings: identifier-ish band names embedded in a
/// fixed skeleton, so variations look like real canonical plans.
fn arb_plan_json() -> impl Strategy<Value = String> {
    proptest::collection::vec("[a-z][a-z0-9]{0,7}", 1..4).prop_map(|bands| {
        let inputs: Vec<String> = bands
            .iter()
            .map(|band| format!("{{\"name\":\"{band}\"}}"))
            .collect();
        format!("{{\"inputs\":[{}]}}", inputs.join(","))
    })
}

/// The full input tuple, arbitrary.
fn arb_inputs() -> impl Strategy<Value = (String, Option<String>, String, TileCoord, u32)> {
    (
        "[a-z][a-z0-9-]{0,15}",
        proptest::option::of("[a-z][a-z0-9-]{0,15}"),
        arb_plan_json(),
        arb_tile(),
        proptest::sample::select(vec![256u32, 512]),
    )
}

fn key_of(
    (layer, granule, plan_json, coord, tile_size): &(
        String,
        Option<String>,
        String,
        TileCoord,
        u32,
    ),
) -> TileKey {
    let version = layer_version(granule.as_deref(), plan_json);
    TileKey::compute(&TileKeyInputs {
        layer,
        layer_version: &version,
        plan_json,
        tms: "WebMercatorQuad",
        coord: *coord,
        tile_size: *tile_size,
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Same inputs, same key — computed twice, including the version
    /// derivation (the whole path the serve wiring runs per request).
    #[test]
    fn key_is_deterministic(inputs in arb_inputs()) {
        prop_assert_eq!(key_of(&inputs), key_of(&inputs));
    }

    /// Different input tuples, different keys: any observable difference
    /// in (layer, granule, plan, coord, tile_size) must produce a
    /// different key — an equality here would be either an encoding bug
    /// (two identities folded together) or a SHA-256 collision.
    #[test]
    fn distinct_inputs_yield_distinct_keys(a in arb_inputs(), b in arb_inputs()) {
        if a != b {
            prop_assert_ne!(key_of(&a), key_of(&b));
        }
    }

    /// A new granule (the ingest event) always changes the key for every
    /// tile of the layer — the clean-miss invalidation §10 promises.
    #[test]
    fn new_granule_invalidates(inputs in arb_inputs(), granule in "[a-z][a-z0-9-]{0,15}") {
        let (layer, old_granule, plan_json, coord, tile_size) = &inputs;
        if old_granule.as_deref() != Some(granule.as_str()) {
            let bumped = (
                layer.clone(),
                Some(granule),
                plan_json.clone(),
                *coord,
                *tile_size,
            );
            prop_assert_ne!(key_of(&inputs), key_of(&bumped));
        }
    }
}
