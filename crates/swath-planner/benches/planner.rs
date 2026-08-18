// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `plan()` must be trivially cheap (issue #37, spec §5): the decision
//! sits on every tile request, so its cost has to be free relative to
//! any strategy it picks — nanoseconds against milliseconds of I/O.
//! Three shapes: the hot cache hit, the single-band z11 overview case,
//! and a 3-band miss (the truecolor shape).

// criterion's group/main macros generate undocumented items.
#![allow(missing_docs)]

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use swath_planner::{Availability, BandWindow, Budget, CacheProbe, plan};

fn band() -> BandWindow {
    BandWindow::new(505.0, 505.0, 2, vec![2])
}

fn bench_plan(c: &mut Criterion) {
    let budget = Budget::default();

    let hit = Availability::new(
        CacheProbe::Hit {
            payload_bytes: 24_117,
        },
        256,
        Vec::new(),
    );
    c.bench_function("plan_cache_hit", |b| {
        b.iter(|| plan(black_box(&budget), black_box(&hit)));
    });

    let z11 = Availability::new(CacheProbe::NotConfigured, 256, vec![band()]);
    c.bench_function("plan_overview_single_band", |b| {
        b.iter(|| plan(black_box(&budget), black_box(&z11)));
    });

    let truecolor = Availability::new(CacheProbe::Miss, 256, vec![band(), band(), band()]);
    c.bench_function("plan_miss_three_bands", |b| {
        b.iter(|| plan(black_box(&budget), black_box(&truecolor)));
    });
}

criterion_group!(benches, bench_plan);
criterion_main!(benches);
