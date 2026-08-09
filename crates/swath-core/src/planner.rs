// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The cost-aware materialization planner (ARCHITECTURE.md §5/§10,
//! CHARTER.md §7 pillar 2, issue #37) — full decision model in
//! `docs/design/materialization-planner.md`.
//!
//! [`plan`] chooses, per `(layer, tile)`, one of cache-hit / overview /
//! live under an explicit per-layer [`Budget`], and returns **every**
//! candidate it weighed with its estimate, admissibility, and reason —
//! the x-ray "why did it decide that?" payload
//! ([`Trace::plan`](crate::trace::Trace::plan)).
//!
//! # Purity contract
//!
//! `plan()` performs no I/O and consults no clocks: the **caller**
//! gathers [`Availability`] (the cache probe *result* — never a request
//! to probe, so planning can never double-fetch — plus per-band window
//! geometry from metadata it already holds) and executes the returned
//! choice. Same inputs, same [`Plan`], always.
//!
//! # The cost model (v1: transparent, calibratable, not learned)
//!
//! Costs are **estimated source bytes decoded** — the same quantity the
//! Trace measures as `bytes_read`, so estimates are checkable against
//! reality (and tests check them, loosely). Per strategy:
//!
//! - cache: the stored payload length (already fetched by the probe);
//! - overview at factor `f`: `Σ_bands ceil(cols/f) · ceil(rows/f) ·
//!   bytes_per_sample` (× [`WARP_COST_WEIGHT`]);
//! - live: the same at `f = 1`.
//!
//! Constants are documented calibration points (spec §2), never runtime
//! fits; a learned model over Trace history is recorded future work.

use std::borrow::Cow;

/// Warp-cost weight folded into the byte estimates (spec §2): warp cost
/// scales linearly with source pixels touched, so v1 prices it inside
/// the byte count rather than modeling CPU separately. Calibratable,
/// documented, not learned.
pub const WARP_COST_WEIGHT: f64 = 1.0;

/// GDAL's overview oversampling slack (`GDALBandGetBestOverviewLevel2`),
/// the default of [`Budget::overview_oversample`]. Promoted from the #38
/// constant to a knob; the value is calibrated against the rio-tiler
/// oracle (a z11 tile of a 30 m source — desired ratio ~1.97 — must
/// serve the ×2 overview, as GDAL demonstrably does; 1.0 would refuse
/// it and diverge).
pub const DEFAULT_OVERVIEW_OVERSAMPLE: f64 = 1.2;

/// The v1 per-layer budget: three explicit knobs trading storage against
/// latency (spec §1 documents each; §16.4 resolved — knobs + transparent
/// estimates, no learned model in v1).
///
/// Deliberately **not** `#[non_exhaustive]`: configuration layers build
/// budgets field-by-field (`..Budget::default()`), and a new knob *should*
/// be a visible, reviewed change at every construction site.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Budget {
    /// Consult the tile cache and write fresh renders through (`true`,
    /// the default) — or opt this layer out entirely (`false`: no probe,
    /// no write-through, no storage growth).
    pub cache_enabled: bool,
    /// Overview eligibility slack: a factor is eligible when
    /// `factor <= desired_ratio × overview_oversample`. Larger serves
    /// coarser overviews at more zooms (fewer bytes, softer pixels);
    /// `1.0` demands strict decimation.
    pub overview_oversample: f64,
    /// Refuse live renders whose estimated cost exceeds this many bytes
    /// when nothing cheaper can serve — an explicit error instead of an
    /// unbounded full-res read. `None` (default) never refuses.
    pub max_estimated_live_bytes: Option<u64>,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            cache_enabled: true,
            overview_oversample: DEFAULT_OVERVIEW_OVERSAMPLE,
            max_estimated_live_bytes: None,
        }
    }
}

/// The result of the caller's cache lookup — a **result**, never a
/// request: the serve path performs (or skips) the probe itself, so
/// `plan()` can never cause a second fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CacheProbe {
    /// The server runs without a tile cache.
    NotConfigured,
    /// The layer's budget opted out (`cache_enabled = false`); no probe
    /// was performed.
    Disabled,
    /// Probed, no entry (or an unusable one — the caller's policy).
    Miss,
    /// Probed and hit: the payload is already in hand.
    Hit {
        /// Stored payload length in bytes — the cache candidate's cost.
        payload_bytes: u64,
    },
}

/// One band's full-resolution source geometry — everything the cost
/// model and overview rule need, all of it already computed by the tiler
/// before any pixel read (describe metadata + the tile-boundary extent).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BandWindow {
    /// Fractional full-resolution source columns the tile boundary
    /// spans (pre-clip).
    pub cols: f64,
    /// Fractional full-resolution source rows the tile boundary spans
    /// (pre-clip).
    pub rows: f64,
    /// Bytes per sample (`DType::size_bytes`).
    pub bytes_per_sample: u64,
    /// Decimation factors of the asset's embedded overviews, as
    /// `describe` reports them.
    pub overview_factors: Vec<u32>,
}

impl BandWindow {
    /// A band spanning `cols × rows` full-res source pixels of
    /// `bytes_per_sample`-byte samples with `overview_factors` available.
    #[must_use]
    pub fn new(cols: f64, rows: f64, bytes_per_sample: u64, overview_factors: Vec<u32>) -> Self {
        Self {
            cols,
            rows,
            bytes_per_sample,
            overview_factors,
        }
    }

    /// GDAL's desired downsampling ratio for this band: the *smaller*
    /// per-axis ratio, so the less-decimating axis is never starved of
    /// resolution (#38's rule, unchanged).
    fn desired_ratio(&self, tile_size: u32) -> f64 {
        if tile_size == 0 {
            return f64::NAN;
        }
        (self.cols / f64::from(tile_size)).min(self.rows / f64::from(tile_size))
    }

    /// Estimated decoded bytes of reading this band's window at
    /// decimation `factor` (spec §2): uncompressed pixels of the
    /// boundary extent, warp weight folded in.
    fn estimated_bytes(&self, factor: u32) -> u64 {
        let f = f64::from(factor.max(1));
        let pixels = (self.cols / f).ceil().max(0.0) * (self.rows / f).ceil().max(0.0);
        saturating_bytes(pixels * WARP_COST_WEIGHT) * self.bytes_per_sample
    }
}

/// What is true about this `(layer, tile)` right now — the planner's
/// whole world, gathered by the caller (spec §1).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Availability {
    /// The cache probe result.
    pub cache: CacheProbe,
    /// Target tile side length in pixels.
    pub tile_size: u32,
    /// One entry per plan input band whose source window intersects its
    /// raster; off-raster bands read nothing and are not listed. Empty
    /// = a fully off-data tile (rendered transparent, chosen `Live`).
    pub bands: Vec<BandWindow>,
}

impl Availability {
    /// Availability of `bands` at `tile_size` given `cache`.
    #[must_use]
    pub fn new(cache: CacheProbe, tile_size: u32, bands: Vec<BandWindow>) -> Self {
        Self {
            cache,
            tile_size,
            bands,
        }
    }
}

/// A candidate strategy as the plan trace names it — the shape of
/// [`Strategy`](crate::trace::Strategy) minus execution details (a
/// cache-hit candidate has no key until it is served).
///
/// Wire form mirrors `Strategy`: `"cache_hit"`, `{"overview":
/// {"factor": 2}}`, `"live"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannedStrategy {
    /// Serve the stored encoded tile.
    CacheHit,
    /// Read from an embedded overview.
    Overview {
        /// The decimation factor to read at. In an **inadmissible**
        /// candidate record, `0` means no factor was selectable (the
        /// reason says why); a chosen strategy always carries a real
        /// factor ≥ 2.
        factor: u32,
    },
    /// Read at full resolution.
    Live,
}

/// One weighed candidate: what it would cost, whether the budget admits
/// it, and why — recorded for **every** candidate, chosen or not (the
/// x-ray "show the work" payload, spec §3).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CandidateTrace {
    /// The candidate strategy.
    pub strategy: PlannedStrategy,
    /// Estimated cost in bytes (spec §2 model; 0 when not estimated —
    /// the reason says so).
    pub estimated_cost_bytes: u64,
    /// Whether the budget admits this candidate.
    pub admissible: bool,
    /// Deterministic, human-legible explanation.
    pub reason: Cow<'static, str>,
}

/// The planner's choice for one tile.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanChoice {
    /// Serve the stored payload the probe already fetched.
    CacheHit,
    /// Read every band at this overview factor.
    Overview {
        /// The decimation factor to read at.
        factor: u32,
    },
    /// Read every band at full resolution.
    Live,
    /// Nothing is admissible: the live estimate exceeds the budget's
    /// ceiling and no cheaper strategy can serve. The caller surfaces
    /// an explicit error — never an unbounded read.
    Refuse {
        /// The live estimate that broke the ceiling.
        estimated_live_bytes: u64,
        /// The ceiling (`Budget::max_estimated_live_bytes`).
        limit: u64,
    },
}

/// The full planning result: the choice plus every candidate weighed.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Plan {
    /// The chosen strategy (or the refusal).
    pub strategy: PlanChoice,
    /// Every candidate, in the fixed evaluation order `cache_hit`,
    /// `overview`, `live`.
    pub considered: Vec<CandidateTrace>,
}

impl Plan {
    /// The Trace payload for a plan that chose a servable strategy
    /// (`None` for a refusal — a refused render errors and emits no
    /// Trace).
    #[must_use]
    pub fn trace(&self) -> Option<crate::trace::PlanTrace> {
        let chosen = match self.strategy {
            PlanChoice::CacheHit => PlannedStrategy::CacheHit,
            PlanChoice::Overview { factor } => PlannedStrategy::Overview { factor },
            PlanChoice::Live => PlannedStrategy::Live,
            PlanChoice::Refuse { .. } => return None,
        };
        Some(crate::trace::PlanTrace {
            chosen,
            considered: self.considered.clone(),
        })
    }
}

/// `f64` byte/pixel counts to `u64`, saturating; negatives and NaN
/// clamp to 0 (never produced by the model, but the conversion is total).
fn saturating_bytes(value: f64) -> u64 {
    if value.is_nan() || value <= 0.0 {
        0
    } else if value >= 1.844_674_407_370_955_2e19 {
        u64::MAX
    } else {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "range-checked above: value is finite, non-negative, < 2^64"
        )]
        {
            value as u64
        }
    }
}

/// The coarsest overview factor eligible for **every** band under the
/// budget's oversample slack (spec §3): factor `f > 1` is eligible for a
/// band when `f` is among its overview factors and `f <= desired_ratio ×
/// oversample`. Requiring a *common* factor keeps "one tile, one honest
/// decision" — and makes execution match the Trace by construction.
fn common_overview_factor(bands: &[BandWindow], tile_size: u32, oversample: f64) -> Option<u32> {
    let mut common: Option<Vec<u32>> = None;
    for band in bands {
        let bound = band.desired_ratio(tile_size) * oversample;
        if !bound.is_finite() {
            return None;
        }
        let eligible: Vec<u32> = band
            .overview_factors
            .iter()
            .copied()
            .filter(|&f| f > 1 && f64::from(f) <= bound)
            .collect();
        common = Some(match common {
            None => eligible,
            Some(prev) => prev.into_iter().filter(|f| eligible.contains(f)).collect(),
        });
    }
    common.and_then(|factors| factors.into_iter().max())
}

/// The terminal cache-hit plan (spec §3 step 1): the payload fetch was
/// already paid by the probe, so re-rendering could only add cost on
/// top; the other candidates are recorded unestimated and the hot hit
/// path never needs source metadata I/O.
fn cache_hit_plan(payload_bytes: u64) -> Plan {
    let mut considered = vec![CandidateTrace {
        strategy: PlannedStrategy::CacheHit,
        estimated_cost_bytes: payload_bytes,
        admissible: true,
        reason: Cow::Borrowed("stored payload already fetched by the probe"),
    }];
    for strategy in [
        PlannedStrategy::Overview { factor: 0 },
        PlannedStrategy::Live,
    ] {
        considered.push(CandidateTrace {
            strategy,
            estimated_cost_bytes: 0,
            admissible: false,
            reason: Cow::Borrowed("not estimated: cache hit short-circuits"),
        });
    }
    Plan {
        strategy: PlanChoice::CacheHit,
        considered,
    }
}

/// Chooses the materialization strategy for one tile under `budget`
/// given `availability`, recording every candidate (module docs; spec
/// §3 is the normative procedure).
#[must_use]
pub fn plan(budget: &Budget, availability: &Availability) -> Plan {
    // 1. The cache short-circuit: a hit with the cache enabled is
    //    terminal (see `cache_hit_plan`).
    if let (CacheProbe::Hit { payload_bytes }, true) = (availability.cache, budget.cache_enabled) {
        return cache_hit_plan(payload_bytes);
    }
    let mut considered = Vec::with_capacity(3);
    let (cache_reason, cache_estimate): (&'static str, u64) = match availability.cache {
        CacheProbe::NotConfigured => ("no cache configured", 0),
        CacheProbe::Disabled => ("cache disabled by budget", 0),
        CacheProbe::Miss => ("cache miss", 0),
        // Hit with cache_enabled = false (the budget opted out after an
        // external probe): treated as disabled — never served.
        CacheProbe::Hit { payload_bytes } => ("cache disabled by budget", payload_bytes),
    };
    considered.push(CandidateTrace {
        strategy: PlannedStrategy::CacheHit,
        estimated_cost_bytes: cache_estimate,
        admissible: false,
        reason: Cow::Borrowed(cache_reason),
    });

    // 2. The overview candidate: coarsest factor eligible for every band.
    let factor = common_overview_factor(
        &availability.bands,
        availability.tile_size,
        budget.overview_oversample,
    );
    let overview = match factor {
        Some(factor) => {
            let estimate = availability
                .bands
                .iter()
                .map(|b| b.estimated_bytes(factor))
                .sum();
            CandidateTrace {
                strategy: PlannedStrategy::Overview { factor },
                estimated_cost_bytes: estimate,
                admissible: true,
                reason: Cow::Borrowed("coarsest overview within the oversample threshold"),
            }
        }
        None => CandidateTrace {
            strategy: PlannedStrategy::Overview { factor: 0 },
            estimated_cost_bytes: 0,
            admissible: false,
            reason: Cow::Borrowed(if availability.bands.is_empty() {
                "no source window"
            } else if availability
                .bands
                .iter()
                .all(|b| b.overview_factors.iter().all(|&f| f <= 1))
            {
                "source has no overviews"
            } else {
                "no overview factor eligible at this zoom"
            }),
        },
    };
    considered.push(overview);

    // 3. The live candidate, gated by the ceiling.
    let live_estimate: u64 = availability
        .bands
        .iter()
        .map(|b| b.estimated_bytes(1))
        .sum();
    let live_admissible = budget
        .max_estimated_live_bytes
        .is_none_or(|limit| live_estimate <= limit);
    considered.push(CandidateTrace {
        strategy: PlannedStrategy::Live,
        estimated_cost_bytes: live_estimate,
        admissible: live_admissible,
        reason: Cow::Borrowed(if live_admissible {
            "full-resolution read"
        } else {
            "estimated bytes exceed max_estimated_live_bytes"
        }),
    });

    // 4. Cheapest admissible; ties break by the fixed evaluation order
    //    (earlier wins), so the decision is fully deterministic.
    let strategy = considered
        .iter()
        .filter(|c| c.admissible)
        .min_by_key(|c| c.estimated_cost_bytes)
        .map_or_else(
            || PlanChoice::Refuse {
                estimated_live_bytes: live_estimate,
                limit: budget
                    .max_estimated_live_bytes
                    .expect("refusal only arises from the live ceiling"),
            },
            |cheapest| match cheapest.strategy {
                PlannedStrategy::CacheHit => PlanChoice::CacheHit,
                PlannedStrategy::Overview { factor } => PlanChoice::Overview { factor },
                PlannedStrategy::Live => PlanChoice::Live,
            },
        );

    Plan {
        strategy,
        considered,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Availability, BandWindow, Budget, CacheProbe, DEFAULT_OVERVIEW_OVERSAMPLE, Plan,
        PlanChoice, PlannedStrategy, plan,
    };

    /// A band whose window matches the ratio a `SourceExtent` of
    /// `cols × rows` full-res pixels produces (the #38 test geometries,
    /// re-homed with the selection rule).
    fn band(cols: f64, rows: f64, factors: &[u32]) -> BandWindow {
        BandWindow::new(cols, rows, 2, factors.to_vec())
    }

    fn avail(cache: CacheProbe, bands: Vec<BandWindow>) -> Availability {
        Availability::new(cache, 256, bands)
    }

    fn choice(p: &Plan) -> &PlanChoice {
        &p.strategy
    }

    /// The #38 selection-rule truth table, unchanged behavior at the
    /// default oversample (window.rs's `select_overview` test, re-homed).
    #[test]
    fn overview_selection_follows_the_gdal_rule() {
        let budget = Budget::default();
        assert!((budget.overview_oversample - DEFAULT_OVERVIEW_OVERSAMPLE).abs() < f64::EPSILON);

        // z11-like: ~505 px per 256-px axis (ratio ~1.97) — inside the
        // 1.2 slack for the x2 overview.
        let p = plan(
            &budget,
            &avail(CacheProbe::NotConfigured, vec![band(505.0, 505.0, &[2])]),
        );
        assert_eq!(*choice(&p), PlanChoice::Overview { factor: 2 });
        // Coarsest eligible wins.
        let p = plan(
            &budget,
            &avail(
                CacheProbe::NotConfigured,
                vec![band(2048.0, 2048.0, &[2, 4, 8])],
            ),
        );
        assert_eq!(*choice(&p), PlanChoice::Overview { factor: 8 });
        // z12-like non-decimating warp (~0.99) stays full-res.
        let p = plan(
            &budget,
            &avail(CacheProbe::NotConfigured, vec![band(254.0, 254.0, &[2])]),
        );
        assert_eq!(*choice(&p), PlanChoice::Live);
        // Just inside the slack (1.7 × 1.2 = 2.04 ≥ 2)…
        let p = plan(
            &budget,
            &avail(
                CacheProbe::NotConfigured,
                vec![band(1.7 * 256.0, 1.7 * 256.0, &[2])],
            ),
        );
        assert_eq!(*choice(&p), PlanChoice::Overview { factor: 2 });
        // …and just outside it (1.6 × 1.2 = 1.92 < 2).
        let p = plan(
            &budget,
            &avail(
                CacheProbe::NotConfigured,
                vec![band(1.6 * 256.0, 1.6 * 256.0, &[2])],
            ),
        );
        assert_eq!(*choice(&p), PlanChoice::Live);
        // No overviews: live.
        let p = plan(
            &budget,
            &avail(CacheProbe::NotConfigured, vec![band(2048.0, 2048.0, &[])]),
        );
        assert_eq!(*choice(&p), PlanChoice::Live);
        // The limiting axis is the less-decimating one: a ~1 ratio on
        // one axis vetoes the overview even if the other decimates 4x.
        let p = plan(
            &budget,
            &avail(CacheProbe::NotConfigured, vec![band(256.0, 1024.0, &[2])]),
        );
        assert_eq!(*choice(&p), PlanChoice::Live);
        // Degenerate target: full resolution.
        let p = plan(
            &budget,
            &Availability::new(
                CacheProbe::NotConfigured,
                0,
                vec![band(2048.0, 2048.0, &[2])],
            ),
        );
        assert_eq!(*choice(&p), PlanChoice::Live);
    }

    /// The oversample knob moves the eligibility boundary.
    #[test]
    fn oversample_knob_is_live() {
        let strict = Budget {
            overview_oversample: 1.0,
            ..Budget::default()
        };
        // Ratio ~1.97 < 2: strict oversampling refuses the x2 overview
        // GDAL's 1.2 slack would serve.
        let a = avail(CacheProbe::NotConfigured, vec![band(505.0, 505.0, &[2])]);
        assert_eq!(*choice(&plan(&strict, &a)), PlanChoice::Live);
        assert_eq!(
            *choice(&plan(&Budget::default(), &a)),
            PlanChoice::Overview { factor: 2 }
        );
    }

    /// A hit with the cache enabled is terminal; the other candidates
    /// are recorded unestimated (the hot path stays free of source I/O).
    #[test]
    fn cache_hit_short_circuits() {
        let p = plan(
            &Budget::default(),
            &avail(
                CacheProbe::Hit {
                    payload_bytes: 20_000,
                },
                vec![],
            ),
        );
        assert_eq!(*choice(&p), PlanChoice::CacheHit);
        assert_eq!(p.considered.len(), 3);
        assert!(p.considered[0].admissible);
        assert_eq!(p.considered[0].estimated_cost_bytes, 20_000);
        assert!(!p.considered[1].admissible);
        assert!(!p.considered[2].admissible);
    }

    /// `cache_enabled = false` never serves a hit.
    #[test]
    fn disabled_cache_never_serves() {
        let a = avail(
            CacheProbe::Hit { payload_bytes: 1 },
            vec![band(254.0, 254.0, &[2])],
        );
        let p = plan(
            &Budget {
                cache_enabled: false,
                ..Budget::default()
            },
            &a,
        );
        assert_eq!(*choice(&p), PlanChoice::Live);
        assert_eq!(p.considered[0].reason, "cache disabled by budget");
    }

    /// The ceiling refuses a live render nothing cheaper can replace,
    /// and admits one under it.
    #[test]
    fn live_ceiling_refuses_and_admits() {
        // 505×505 × 2 bytes ≈ 512 KB estimated.
        let a = avail(CacheProbe::NotConfigured, vec![band(505.0, 505.0, &[])]);
        let p = plan(
            &Budget {
                max_estimated_live_bytes: Some(100_000),
                ..Budget::default()
            },
            &a,
        );
        match choice(&p) {
            PlanChoice::Refuse {
                estimated_live_bytes,
                limit,
            } => {
                assert!(*estimated_live_bytes > 100_000);
                assert_eq!(*limit, 100_000);
            }
            other => panic!("expected refusal, got {other:?}"),
        }
        assert_eq!(p.trace(), None, "a refusal has no Trace payload");
        // A generous ceiling admits the same render.
        let p = plan(
            &Budget {
                max_estimated_live_bytes: Some(1_000_000),
                ..Budget::default()
            },
            &a,
        );
        assert_eq!(*choice(&p), PlanChoice::Live);
        // With an eligible overview, the ceiling on live doesn't refuse:
        // the overview serves.
        let a = avail(CacheProbe::NotConfigured, vec![band(505.0, 505.0, &[2])]);
        let p = plan(
            &Budget {
                max_estimated_live_bytes: Some(100_000),
                ..Budget::default()
            },
            &a,
        );
        assert_eq!(*choice(&p), PlanChoice::Overview { factor: 2 });
    }

    /// An off-data tile (no band windows) is a zero-cost Live — the
    /// transparent-tile path, still explained.
    #[test]
    fn off_data_tile_is_live_with_zero_estimate() {
        let p = plan(&Budget::default(), &avail(CacheProbe::Miss, vec![]));
        assert_eq!(*choice(&p), PlanChoice::Live);
        assert_eq!(p.considered[2].estimated_cost_bytes, 0);
        assert_eq!(p.considered[1].reason, "no source window");
    }

    /// Multi-band: the factor must be eligible for every band. A band
    /// without the coarser factor pulls the choice down to the common
    /// one.
    #[test]
    fn common_factor_across_bands() {
        let a = avail(
            CacheProbe::NotConfigured,
            vec![
                band(2048.0, 2048.0, &[2, 4, 8]),
                band(2048.0, 2048.0, &[2, 4]),
            ],
        );
        let p = plan(&Budget::default(), &a);
        assert_eq!(*choice(&p), PlanChoice::Overview { factor: 4 });
        // One band with no overviews at all vetoes the strategy.
        let a = avail(
            CacheProbe::NotConfigured,
            vec![band(2048.0, 2048.0, &[2, 4, 8]), band(2048.0, 2048.0, &[])],
        );
        assert_eq!(*choice(&plan(&Budget::default(), &a)), PlanChoice::Live);
    }

    /// Every plan records all three candidates in the fixed order.
    #[test]
    fn all_three_candidates_are_always_recorded() {
        for cache in [
            CacheProbe::NotConfigured,
            CacheProbe::Disabled,
            CacheProbe::Miss,
            CacheProbe::Hit { payload_bytes: 9 },
        ] {
            let p = plan(
                &Budget::default(),
                &avail(cache, vec![band(505.0, 505.0, &[2])]),
            );
            assert_eq!(p.considered.len(), 3);
            assert_eq!(p.considered[0].strategy, PlannedStrategy::CacheHit);
            assert!(matches!(
                p.considered[1].strategy,
                PlannedStrategy::Overview { .. }
            ));
            assert_eq!(p.considered[2].strategy, PlannedStrategy::Live);
        }
    }
}
