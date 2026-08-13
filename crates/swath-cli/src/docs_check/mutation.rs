// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Mutation verification (issue #173's acceptance bar): each of the six
//! documentation drifts fixed by the #172 sweep (PR #214) is scripted
//! back into the current doc text — the exact pre-sweep hunks — and the
//! gate must fail on every one. Each test first asserts the unmutated
//! text passes the same check, so a failure is provably caused by the
//! re-introduced drift and not by a broken fixture.

use super::{claims, read_repo, routes, stamps};

/// `text` with `old` swapped in for `new`, panicking if the anchor is
/// gone (a moved anchor must move the fixture, never silently no-op).
fn reintroduce(text: &str, current: &str, pre_sweep: &str) -> String {
    assert!(
        text.contains(current),
        "mutation anchor no longer present — update the fixture: {current:?}"
    );
    text.replace(current, pre_sweep)
}

/// Drift 1 (ENDPOINTS.md): `POST /result` (#170) undocumented.
#[test]
fn reintroducing_the_missing_post_result_row_fails() {
    let doc = read_repo(routes::DOC);
    routes::check(&doc).expect("the unmutated route table must pass");
    let mutated = reintroduce(
        &doc,
        "| POST | `/result` | catalog mode | Preview: one bounded synchronous render of a process graph (PNG) |\n",
        "",
    );
    let err = routes::check(&mutated).expect_err("dropping the POST /result row must fail");
    assert!(err.contains("/result"), "error must name the route: {err}");
}

/// Drift 2 (DEMO.md): the pre-baseline measured numbers (297/801/535 ms,
/// issue #35) in place of the committed 646 ms i2p baseline stamp.
#[test]
fn reintroducing_demos_stale_measured_numbers_fails() {
    let doc = read_repo("docs/DEMO.md");
    claims::demo_quotes_the_i2p_baseline(&doc).expect("the unmutated DEMO table must pass");
    let mutated = reintroduce(
        &doc,
        "| Where                  | ingest-to-pixel | Notes                                                            |\n\
         | ---------------------- | --------------- | ---------------------------------------------------------------- |\n\
         | **Committed baseline** | **646 ms**      | `just perf-i2p`, stamped at `27deca2` — [`docs/perf/i2p-baseline.json`](perf/i2p-baseline.json), method in [`PERFORMANCE.md`](PERFORMANCE.md) §4 |\n\
         | **Asserted budget**    | **10 000 ms**   | ~15x headroom over the committed baseline                        |",
        "| Where               | ingest-to-pixel | Notes                                |\n\
         | ------------------- | --------------- | ------------------------------------ |\n\
         | Local (dev laptop)  | 297 ms, 801 ms  | two runs, issue #35                  |\n\
         | CI (GitHub runner)  | 535 ms          | `just e2e`, issue #35                |\n\
         | **Asserted budget** | **10 000 ms**   | ~20x headroom over the CI number     |",
    );
    claims::demo_quotes_the_i2p_baseline(&mutated)
        .expect_err("the pre-sweep measured-numbers table must fail");
}

/// Drift 3 (COMPARISON.md): quoting the README sentence the README
/// rewrite removed.
#[test]
fn reintroducing_comparisons_stale_readme_quote_fails() {
    let doc = read_repo("docs/COMPARISON.md");
    let readme = read_repo("README.md");
    claims::comparison_quotes_the_readme(&doc, &readme)
        .expect("the unmutated COMPARISON quotes must pass");
    let mutated = reintroduce(
        &doc,
        "which is exactly the README's positioning claim (the wedge-quadrant paragraph):\n\
         \"Swath does both — a standard openEO graph in, live measured tiles out.\" Rows 1–2\n\
         are that sentence's two axes; rows 3–4 are the committed evidence behind\n\
         \"live measured tiles\".",
        "which is exactly the README's claim: \"Nobody compiles a data-scientist's process\n\
         graph into a low-latency dynamic tile service with a cost-aware cache.\"",
    );
    let err = claims::comparison_quotes_the_readme(&mutated, &readme)
        .expect_err("quoting the removed README sentence must fail");
    assert!(
        err.contains("Nobody compiles"),
        "error must name the quote: {err}"
    );
}

/// Drift 4 (README.md): the oracle-history pointer citing CHARTER §7
/// (the pre-renumbering section) instead of §8.
#[test]
fn reintroducing_readmes_stale_charter_citation_fails() {
    let readme = read_repo("README.md");
    let charter = read_repo("docs/CHARTER.md");
    claims::readme_cites_the_charter_oracle_section(&readme, &charter)
        .expect("the unmutated README citation must pass");
    let mutated = reintroduce(&readme, "](docs/CHARTER.md) §8", "](docs/CHARTER.md) §7");
    let err = claims::readme_cites_the_charter_oracle_section(&mutated, &charter)
        .expect_err("citing CHARTER §7 for the oracle history must fail");
    assert!(err.contains("§7"), "error must name the citation: {err}");
}

/// Drift 5 (load-2cpu evidence): the header claiming the 12-core host
/// run when the data is the 2-CPU pinned rerun.
#[test]
fn reintroducing_the_load_2cpu_header_misattribution_fails() {
    let doc = read_repo("docs/perf/load-2cpu-16.7-evidence.md");
    claims::load_2cpu_header_names_the_pinned_run(&doc)
        .expect("the unmutated evidence header must pass");
    let mutated = reintroduce(
        &doc,
        "Generated 2026-08-10T15:15:28Z at `dfaa7f0` — Apple M2 Max host, Darwin 25.5.0 arm64, oha 1.15.0, **server pinned to 2 CPUs** (the constrained-VM shape of ADR 0012's maintainer-requested rerun; this run's numbers are the \"2 CPUs (pinned)\" column of that ADR's decision table). Recipe wall time to this point: 90s.",
        "Generated 2026-08-10T15:15:28Z at `dfaa7f0` — Apple M2 Max (12 cores), Darwin 25.5.0 arm64, oha 1.15.0. Recipe wall time to this point: 90s.",
    );
    claims::load_2cpu_header_names_the_pinned_run(&mutated)
        .expect_err("the pre-sweep 12-core header must fail");
}

/// Drift 6 (ARCHITECTURE.md + EXTENDING.md): the pre-sweep sha stamps
/// (`c944a41` / `32fad75`), under which commits have since touched the
/// stamped sections' referenced sources. Skipped (like the gate itself)
/// where git history is unavailable — CI's docs job requires it.
#[test]
fn reintroducing_the_stale_sha_stamps_fails() {
    if !stamps::history_available().unwrap() {
        return;
    }
    for (doc_label, current_sha, pre_sweep_sha) in [
        ("docs/ARCHITECTURE.md", "576324d", "c944a41"),
        ("docs/EXTENDING.md", "9ab35b8", "32fad75"),
    ] {
        let doc = read_repo(doc_label);
        stamps::check_doc(doc_label, &doc).expect("the unmutated stamps must pass");
        let mutated = reintroduce(&doc, current_sha, pre_sweep_sha);
        let err =
            stamps::check_doc(doc_label, &mutated).expect_err("the pre-sweep stamps must be stale");
        assert!(
            err.contains("stale"),
            "error must say stale ({doc_label}): {err}"
        );
    }
}
