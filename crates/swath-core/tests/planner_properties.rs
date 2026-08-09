// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Property tests (proptest, per ENGINEERING.md §2) for the
//! materialization planner (issue #37,
//! `docs/design/materialization-planner.md` §5): the decision invariants
//! that make the budget trustworthy — cheapest-admissible, cache
//! priority, overview-beats-live, determinism, and ceiling admissibility.

use proptest::prelude::{Just, Strategy, prop_assert, prop_assert_eq, prop_oneof, proptest};
use swath_core::planner::{
    Availability, BandWindow, Budget, CacheProbe, PlanChoice, PlannedStrategy, plan,
};

/// Any cache probe result.
fn arb_probe() -> impl Strategy<Value = CacheProbe> {
    prop_oneof![
        Just(CacheProbe::NotConfigured),
        Just(CacheProbe::Disabled),
        Just(CacheProbe::Miss),
        (1u64..10_000_000).prop_map(|payload_bytes| CacheProbe::Hit { payload_bytes }),
    ]
}

/// A band window over realistic source geometry: up to ~16k source pixels
/// per axis, real dtype sizes, and 0–3 overview factors from the
/// power-of-two ladder real COGs carry.
fn arb_band() -> impl Strategy<Value = BandWindow> {
    (
        1.0..16_384.0f64,
        1.0..16_384.0f64,
        prop_oneof![Just(1u64), Just(2), Just(4), Just(8)],
        proptest::collection::vec(prop_oneof![Just(2u32), Just(4), Just(8), Just(16)], 0..4),
    )
        .prop_map(|(cols, rows, bytes_per_sample, mut factors)| {
            factors.sort_unstable();
            factors.dedup();
            BandWindow::new(cols, rows, bytes_per_sample, factors)
        })
}

/// Any availability: a probe, a plausible tile size, and 0–4 bands.
fn arb_availability() -> impl Strategy<Value = Availability> {
    (
        arb_probe(),
        prop_oneof![Just(256u32), Just(512)],
        proptest::collection::vec(arb_band(), 0..4),
    )
        .prop_map(|(cache, tile_size, bands)| Availability::new(cache, tile_size, bands))
}

/// Any budget: both cache polarities, oversample around the practical
/// range, ceiling absent or anywhere from tiny to huge.
fn arb_budget() -> impl Strategy<Value = Budget> {
    (
        proptest::bool::ANY,
        1.0..2.0f64,
        prop_oneof![Just(None), (1u64..2_000_000_000).prop_map(Some)],
    )
        .prop_map(
            |(cache_enabled, overview_oversample, max_estimated_live_bytes)| Budget {
                cache_enabled,
                overview_oversample,
                max_estimated_live_bytes,
            },
        )
}

/// The candidate record matching a choice.
fn chosen_candidate(p: &swath_core::planner::Plan) -> Option<&swath_core::planner::CandidateTrace> {
    let strategy = match p.strategy {
        PlanChoice::CacheHit => PlannedStrategy::CacheHit,
        PlanChoice::Overview { factor } => PlannedStrategy::Overview { factor },
        PlanChoice::Live => PlannedStrategy::Live,
        PlanChoice::Refuse { .. } | _ => return None,
    };
    p.considered.iter().find(|c| c.strategy == strategy)
}

proptest! {
    /// Cheapest-admissible: the chosen candidate is admissible and its
    /// estimate is <= every admissible candidate's estimate; a refusal
    /// happens exactly when nothing is admissible.
    #[test]
    fn chosen_is_cheapest_admissible(
        budget in arb_budget(),
        availability in arb_availability(),
    ) {
        let p = plan(&budget, &availability);
        prop_assert_eq!(p.considered.len(), 3, "all three candidates recorded");
        if let PlanChoice::Refuse { .. } = p.strategy {
            prop_assert!(
                p.considered.iter().all(|c| !c.admissible),
                "refusal only when nothing is admissible"
            );
        } else {
            let chosen = chosen_candidate(&p).expect("chosen candidate recorded");
            prop_assert!(chosen.admissible, "the choice must be admissible");
            for c in p.considered.iter().filter(|c| c.admissible) {
                prop_assert!(
                    chosen.estimated_cost_bytes <= c.estimated_cost_bytes,
                    "chosen {} > admissible {}",
                    chosen.estimated_cost_bytes,
                    c.estimated_cost_bytes,
                );
            }
        }
    }

    /// Cache always wins when available and enabled — the terminal hit.
    #[test]
    fn cache_wins_when_hit_and_enabled(
        mut budget in arb_budget(),
        mut availability in arb_availability(),
        payload in 1u64..10_000_000,
    ) {
        budget.cache_enabled = true;
        availability.cache = CacheProbe::Hit { payload_bytes: payload };
        let p = plan(&budget, &availability);
        prop_assert_eq!(p.strategy, PlanChoice::CacheHit);
    }

    /// Live is never chosen while an admissible overview exists: an
    /// overview at factor >= 2 always estimates cheaper than full res.
    #[test]
    fn overview_beats_live_when_admissible(
        budget in arb_budget(),
        availability in arb_availability(),
    ) {
        let p = plan(&budget, &availability);
        let overview = p
            .considered
            .iter()
            .find(|c| matches!(c.strategy, PlannedStrategy::Overview { .. }))
            .expect("overview candidate recorded");
        if overview.admissible {
            let live = p
                .considered
                .iter()
                .find(|c| c.strategy == PlannedStrategy::Live)
                .expect("live candidate recorded");
            prop_assert!(
                overview.estimated_cost_bytes <= live.estimated_cost_bytes,
                "an eligible overview never estimates above live"
            );
            prop_assert!(
                p.strategy != PlanChoice::Live,
                "live chosen while an admissible overview existed"
            );
        }
    }

    /// Determinism: same inputs, same plan — estimates, reasons, order,
    /// everything.
    #[test]
    fn planning_is_deterministic(
        budget in arb_budget(),
        availability in arb_availability(),
    ) {
        prop_assert_eq!(plan(&budget, &availability), plan(&budget, &availability));
    }

    /// The ceiling is respected: live is admissible iff its estimate is
    /// within `max_estimated_live_bytes`, and an over-ceiling live is
    /// never chosen.
    #[test]
    fn live_ceiling_admissibility_is_respected(
        budget in arb_budget(),
        availability in arb_availability(),
    ) {
        let p = plan(&budget, &availability);
        let live = p
            .considered
            .iter()
            .find(|c| c.strategy == PlannedStrategy::Live)
            .expect("live candidate recorded");
        // On the cache-hit short-circuit the live candidate is not
        // estimated; the ceiling contract applies to evaluated plans.
        if p.strategy != PlanChoice::CacheHit {
            match budget.max_estimated_live_bytes {
                Some(limit) => prop_assert_eq!(
                    live.admissible,
                    live.estimated_cost_bytes <= limit,
                    "live admissibility must mirror the ceiling"
                ),
                None => prop_assert!(live.admissible, "no ceiling: live always admissible"),
            }
            if !live.admissible {
                prop_assert!(p.strategy != PlanChoice::Live);
            }
        }
    }
}
