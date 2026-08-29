// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Headline-number markers (issue #174): every headline figure quoted in
//! prose — the ingest-to-pixel time, the referencer warm latency and
//! ratio, the load p50/p95/p99s — lives inside an inline
//! `<!-- number:<key> -->…<!-- /number:<key> -->` marker pair whose
//! content `just perf-doc` fills from the committed perf artifacts
//! (`docs/perf/*.json` and the 2-CPU load evidence table). This module is
//! the generator's verifying twin:
//!
//! - every marker's content must equal the value rendered from the
//!   artifacts (a stale marker — artifact changed, `just perf-doc` not
//!   re-run — is red CI);
//! - every document must carry its required marker keys (deleting a
//!   marker is red, not silence);
//! - no naked headline literal may appear outside a marker — the
//!   grep-proof, mechanized: re-typing a headline number by hand anywhere
//!   in the measured docs fails the gate.
//!
//! Rendering rules here must match the `just perf-doc` recipe exactly
//! (both are half of one contract; a divergence fails on the next
//! regeneration). Legitimate historic figures — prototype 0001's quoted
//! claims, which are provenance and must never track current artifacts —
//! are exempted via [`ALLOWLIST`], and a stale allowlist entry is itself
//! a failure.

use std::collections::BTreeMap;

use super::read_repo;

/// The measured documents and the marker keys each must carry. ROADMAP
/// quotes no figure today but stays under the naked-literal scan — a
/// headline number can only enter it through a marker.
const DOCS: [(&str, &[&str]); 10] = [
    ("docs/ROADMAP.md", &[]),
    (
        "README.md",
        &[
            "i2p-ms",
            "hot-p50-approx",
            "cold-p50-approx",
            "ref-warm-ms",
            "ref-ratio",
        ],
    ),
    // The published crate's README (issue #188): its measured claims flow
    // through the same markers as the workspace docs'.
    (
        "crates/swath-referencer/README.md",
        &["ref-warm-ms", "ref-sidecar-warm-ms", "ref-ratio"],
    ),
    ("docs/DEMO.md", &["i2p-ms", "i2p-sha"]),
    ("docs/CHARTER.md", &["i2p-ms"]),
    ("docs/REQUIREMENTS.md", &["i2p-ms", "ref-ratio-approx"]),
    (
        "docs/PERFORMANCE.md",
        &[
            "hot-p50-approx",
            "cold-p50-approx",
            "ref-warm-ms",
            "ref-sidecar-warm-ms",
            "ref-ratio",
            "ref-ratio-approx",
            "frame-cold-p50-approx",
            "frame-hot-p50-approx",
            "ov-live-p50-approx",
            "ov-pyramid-p50-approx",
            "materialize-ms",
            "udf-storm-healthz-p99",
            "udf-fuelbomb-healthz-p99",
        ],
    ),
    // ARCHITECTURE's §16 ledger (#220) links its load evidence instead of
    // quoting figures; it stays under the naked-literal scan.
    ("docs/ARCHITECTURE.md", &[]),
    (
        "docs/COMPARISON.md",
        &[
            "2cpu-hot-p50",
            "2cpu-hot-p95",
            "2cpu-hot-rps",
            "2cpu-cold-p50",
            "2cpu-healthz-p99",
        ],
    ),
    (
        "docs/media/wedge.notes.md",
        &[
            "2cpu-hot-p50",
            "2cpu-hot-p95",
            "2cpu-hot-rps",
            "2cpu-cold-p50",
            "2cpu-healthz-p99",
        ],
    ),
];

/// Naked-literal exemptions: `(doc, snippet, reason)`. Occurrences of a
/// headline literal inside the snippet are legitimate; the snippet must
/// still exist in the doc (a stale entry fails), and the reason is the
/// review record.
const ALLOWLIST: [(&str, &str, &str); 4] = [
    (
        "docs/PERFORMANCE.md",
        "**\"~40× warm\" — reproduces.**",
        "verdict heading quoting prototype 0001's historic claim name, not the current ratio",
    ),
    (
        "docs/PERFORMANCE.md",
        "the historic \"40×\" and \"14 ms\" claims, re-run",
        "section title naming prototype 0001's historic claims, not the current figures",
    ),
    (
        "docs/PERFORMANCE.md",
        "Rust ≤ sidecar by ~40×",
        "verbatim quote of prototype 0001's immutable conclusion (provenance, never regenerated)",
    ),
    (
        "docs/PERFORMANCE.md",
        "in line with the prototype's ~40×",
        "comparison against prototype 0001's historic ratio, not the current artifact's",
    ),
];

/// Two significant figures, as an integer (23.33 -> 23, 660.61 -> 660).
/// Half rounds away from zero — the twin of `sig2` in `just perf-doc`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "perf figures are small positive reals; the rounded value fits i64"
)]
fn sig2(v: f64) -> i64 {
    let exponent = (v.abs().log10().floor() as i32 - 1).max(0);
    let scale = 10f64.powi(exponent);
    ((v / scale).round() * scale) as i64
}

/// `1277.6` -> `1,277.6` (thousands-grouped, fraction preserved) — the
/// twin of `comma` in `just perf-doc`.
fn comma(figure: &str) -> String {
    let (whole, frac) = figure.split_once('.').unwrap_or((figure, ""));
    let mut grouped = String::new();
    for (i, ch) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    if frac.is_empty() {
        grouped
    } else {
        format!("{grouped}.{frac}")
    }
}

/// A required f64 field out of a parsed perf artifact.
fn f64_at(artifact: &str, value: &serde_json::Value, path: &[&str]) -> Result<f64, String> {
    let mut cursor = value;
    for key in path {
        cursor = &cursor[*key];
    }
    cursor
        .as_f64()
        .ok_or_else(|| format!("{artifact} has no numeric `{path}`", path = path.join(".")))
}

/// One row of the 2-CPU load evidence table, by scenario label: the
/// as-written cell strings (never re-formatted through floats).
fn twocpu_row(evidence: &str, label: &str) -> Result<Vec<String>, String> {
    evidence
        .lines()
        .map(|line| {
            line.trim()
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_owned())
                .collect::<Vec<_>>()
        })
        .find(|cells| cells.len() == 8 && cells[0] == label)
        .ok_or_else(|| {
            format!("docs/perf/load-2cpu-16.7-evidence.md has no `{label}` scenario row")
        })
}

/// The expected content of every marker key, rendered from the committed
/// artifacts exactly as `just perf-doc` renders it.
pub(super) fn expected() -> Result<BTreeMap<&'static str, String>, String> {
    let parse = |rel: &str| -> Result<serde_json::Value, String> {
        serde_json::from_str(&read_repo(rel)).map_err(|err| format!("{rel} is not JSON: {err}"))
    };
    let i2p = parse("docs/perf/i2p-baseline.json")?;
    let load = parse("docs/perf/load-baseline.json")?;
    let referencer = parse("docs/perf/referencer-baseline.json")?;
    let temporal = parse("docs/perf/temporal-baseline.json")?;
    let evidence = read_repo("docs/perf/load-2cpu-16.7-evidence.md");

    let i2p_value = i2p["value"]
        .as_u64()
        .ok_or("i2p-baseline.json has no numeric `value`")?;
    let i2p_sha = i2p["git_sha"]
        .as_str()
        .filter(|sha| sha.len() >= 7)
        .ok_or("i2p-baseline.json has no full `git_sha`")?;
    let hot_p50 = f64_at(
        "load-baseline.json",
        &load,
        &["scenarios", "hot_cache_storm", "p50_ms"],
    )?;
    let cold_p50 = f64_at(
        "load-baseline.json",
        &load,
        &["scenarios", "cold_live_burst", "p50_ms"],
    )?;
    let ref_warm = f64_at(
        "referencer-baseline.json",
        &referencer,
        &["generators", "referencer-rs", "warm_median_ms"],
    )?;
    let sidecar_warm = f64_at(
        "referencer-baseline.json",
        &referencer,
        &["generators", "virtualizarr-sidecar", "warm_median_ms"],
    )?;
    let ratio = f64_at(
        "referencer-baseline.json",
        &referencer,
        &["warm_ratio_rust_advantage"],
    )?;
    let hot2 = twocpu_row(&evidence, "(a) hot-cache tile storm")?;
    let cold2 = twocpu_row(&evidence, "(b) cold live-render burst")?;
    let healthz2 = twocpu_row(&evidence, "(c) healthz UNDER WARPS")?;
    let frame_cold = f64_at(
        "temporal-baseline.json",
        &temporal,
        &["scenarios", "frames_cold", "p50_ms"],
    )?;
    let frame_hot = f64_at(
        "temporal-baseline.json",
        &temporal,
        &["scenarios", "frames_hot", "p50_ms"],
    )?;
    let ov_live = f64_at(
        "temporal-baseline.json",
        &temporal,
        &["scenarios", "overview_live_z12", "p50_ms"],
    )?;
    let ov_pyramid = f64_at(
        "temporal-baseline.json",
        &temporal,
        &["scenarios", "overview_pyramid_z10", "p50_ms"],
    )?;
    let materialize_ms = temporal["materialize"]["wall_ms"]
        .as_u64()
        .ok_or("temporal-baseline.json has no integer `materialize.wall_ms`")?;

    let mut markers = BTreeMap::from([
        ("i2p-ms", format!("{i2p_value} ms")),
        ("i2p-sha", format!("`{sha}`", sha = &i2p_sha[..7])),
        ("hot-p50-approx", format!("~{} ms", sig2(hot_p50))),
        ("cold-p50-approx", format!("~{} ms", sig2(cold_p50))),
        ("ref-warm-ms", format!("{ref_warm} ms")),
        ("ref-sidecar-warm-ms", format!("{sidecar_warm} ms")),
        ("ref-ratio", format!("{ratio}×")),
        ("ref-ratio-approx", format!("~{}×", ratio.round())),
        ("2cpu-hot-p50", format!("{} ms", hot2[4])),
        ("2cpu-hot-p95", format!("{} ms", hot2[5])),
        ("2cpu-hot-rps", format!("{} req/s", comma(&hot2[3]))),
        ("2cpu-cold-p50", format!("{} ms", cold2[4])),
        ("2cpu-healthz-p99", format!("{} ms", healthz2[6])),
        ("frame-cold-p50-approx", format!("~{} ms", sig2(frame_cold))),
        ("frame-hot-p50-approx", format!("~{} ms", sig2(frame_hot))),
        ("ov-live-p50-approx", format!("~{} ms", sig2(ov_live))),
        ("ov-pyramid-p50-approx", format!("~{} ms", sig2(ov_pyramid))),
        ("materialize-ms", format!("{materialize_ms} ms")),
    ]);
    markers.extend(udf_healthz_markers()?);
    Ok(markers)
}

/// The `run_udf` load evidence's two headline markers (issue #207): the
/// `/healthz` p99 held under each storm, rendered exactly as `just
/// perf-doc` renders them.
fn udf_healthz_markers() -> Result<[(&'static str, String); 2], String> {
    let udf =
        serde_json::from_str::<serde_json::Value>(&read_repo("docs/perf/load-udf-baseline.json"))
            .map_err(|err| format!("load-udf-baseline.json is not JSON: {err}"))?;
    let p99 = |scenario: &str| {
        f64_at(
            "load-udf-baseline.json",
            &udf,
            &["scenarios", scenario, "p99_ms"],
        )
    };
    Ok([
        (
            "udf-storm-healthz-p99",
            format!("{} ms", p99("healthz_under_udf_storm")?),
        ),
        (
            "udf-fuelbomb-healthz-p99",
            format!("{} ms", p99("healthz_under_fuelbomb")?),
        ),
    ])
}

/// `text` split at triple-backtick fences, fenced (odd) segments dropped
/// — markers and literals inside fences are examples, not live figures.
fn prose_segments(text: &str) -> Vec<&str> {
    text.split("```").step_by(2).collect()
}

/// The `(key, content)` of every well-formed marker pair in `text`
/// (fences excluded), or an error for a malformed pair.
fn markers(doc_label: &str, text: &str) -> Result<Vec<(String, String)>, String> {
    let mut found = Vec::new();
    for segment in prose_segments(text) {
        let mut rest = segment;
        while let Some(start) = rest.find("<!-- number:") {
            rest = &rest[start + "<!-- number:".len()..];
            let key = rest
                .split(" -->")
                .next()
                .filter(|key| !key.is_empty() && rest.len() > key.len())
                .ok_or_else(|| format!("{doc_label}: unterminated `<!-- number:` marker"))?
                .to_owned();
            rest = &rest[key.len() + " -->".len()..];
            let end = format!("<!-- /number:{key} -->");
            let stop = rest
                .find(&end)
                .ok_or_else(|| format!("{doc_label}: `number:{key}` marker missing `{end}`"))?;
            found.push((key, rest[..stop].to_owned()));
            rest = &rest[stop + end.len()..];
        }
    }
    Ok(found)
}

/// One document's markers vs the expected values: every key known, every
/// content current, every required key present.
pub(super) fn check_doc(
    doc_label: &str,
    text: &str,
    required: &[&str],
    expected: &BTreeMap<&'static str, String>,
) -> Result<(), String> {
    let found = markers(doc_label, text)?;
    for (key, content) in &found {
        let want = expected.get(key.as_str()).ok_or_else(|| {
            format!("{doc_label}: unknown number marker key `{key}` — not a rendered figure")
        })?;
        if content != want {
            return Err(format!(
                "{doc_label}: marker `number:{key}` is stale — doc says {content:?}, the \
                 committed artifacts say {want:?}; run `just perf-doc`"
            ));
        }
    }
    for key in required {
        if !found.iter().any(|(k, _)| k == key) {
            return Err(format!(
                "{doc_label}: required marker `number:{key}` is missing — the headline \
                 figure must be quoted through a generated marker"
            ));
        }
    }
    Ok(())
}

/// True at `text[pos]` for a detector of `len` chars standing alone: the
/// neighbors are not alphanumeric (a hex sha), digits, `.` or `,` (a
/// longer number).
fn stands_alone(text: &str, pos: usize, len: usize) -> bool {
    let boundary =
        |ch: Option<char>| ch.is_none_or(|c| !c.is_ascii_alphanumeric() && c != '.' && c != ',');
    boundary(text[..pos].chars().next_back()) && boundary(text[pos + len..].chars().next())
}

/// The naked-literal detectors: every rendering of a headline figure that
/// must never appear hand-typed. Derived from the artifacts, so a new
/// measurement automatically hunts its own literals.
fn detectors(expected: &BTreeMap<&'static str, String>) -> Result<Vec<String>, String> {
    let mut tokens: Vec<String> = Vec::new();
    for (key, value) in expected {
        if *key == "i2p-sha" {
            // The sha stamp is checked as a marker; `27deca2`-shaped
            // tokens are not number literals to hunt.
            continue;
        }
        if key.ends_with("-approx") {
            // Approx figures are hunted as their full rendered form
            // ("~23 ms", "~40×") plus, for ratios, the tilde-less token
            // ("40×"); a bare "23" would be hopelessly collision-prone.
            tokens.push(value.clone());
            if value.ends_with('×') {
                tokens.push(value.trim_start_matches('~').to_owned());
            }
        } else {
            // Exact figures are hunted by their numeric core ("646",
            // "13.8", "1,277.6"), which any unit or suffix respelling
            // still contains.
            tokens.push(
                value
                    .trim_end_matches(" ms")
                    .trim_end_matches(" req/s")
                    .trim_end_matches('×')
                    .to_owned(),
            );
        }
    }
    // Every published rendering of the load percentiles, not only the
    // marker-quoted ones: the p50/p95/p99 of each tile scenario in both
    // committed load tables, the ungrouped rps, and the once-published
    // one-decimal cold p50 — none may creep back in hand-typed.
    let load =
        serde_json::from_str::<serde_json::Value>(&read_repo("docs/perf/load-baseline.json"))
            .map_err(|err| format!("load-baseline.json is not JSON: {err}"))?;
    for scenario in ["hot_cache_storm", "cold_live_burst", "mixed_tile_storm"] {
        for percentile in ["p50_ms", "p95_ms", "p99_ms"] {
            let value = f64_at(
                "load-baseline.json",
                &load,
                &["scenarios", scenario, percentile],
            )?;
            tokens.push(value.to_string());
        }
    }
    // The temporal + overview scenarios' published percentiles (issue
    // #184), same rule: no p50/p95/p99 of any committed row may reappear
    // hand-typed.
    let temporal =
        serde_json::from_str::<serde_json::Value>(&read_repo("docs/perf/temporal-baseline.json"))
            .map_err(|err| format!("temporal-baseline.json is not JSON: {err}"))?;
    for scenario in [
        "frames_cold",
        "frames_hot",
        "overview_live_z12",
        "overview_embedded_z10",
        "overview_pyramid_z11",
        "overview_pyramid_z10",
    ] {
        for percentile in ["p50_ms", "p95_ms", "p99_ms"] {
            let value = f64_at(
                "temporal-baseline.json",
                &temporal,
                &["scenarios", scenario, percentile],
            )?;
            tokens.push(value.to_string());
        }
    }
    // The run_udf load scenarios' published percentiles (issue #207),
    // same rule: no p50/p95/p99 of any committed row may reappear
    // hand-typed.
    let udf =
        serde_json::from_str::<serde_json::Value>(&read_repo("docs/perf/load-udf-baseline.json"))
            .map_err(|err| format!("load-udf-baseline.json is not JSON: {err}"))?;
    for scenario in [
        "udf_storm",
        "healthz_under_udf_storm",
        "fuelbomb_storm",
        "healthz_under_fuelbomb",
    ] {
        for percentile in ["p50_ms", "p95_ms", "p99_ms"] {
            let value = f64_at(
                "load-udf-baseline.json",
                &udf,
                &["scenarios", scenario, percentile],
            )?;
            tokens.push(value.to_string());
        }
    }
    let evidence = read_repo("docs/perf/load-2cpu-16.7-evidence.md");
    for label in [
        "(a) hot-cache tile storm",
        "(b) cold live-render burst",
        "(c) mixed tile storm",
    ] {
        let row = twocpu_row(&evidence, label)?;
        tokens.extend(row[4..7].iter().cloned());
    }
    let hot2 = twocpu_row(&evidence, "(a) hot-cache tile storm")?;
    tokens.push(hot2[3].clone());
    let cold2_p50: f64 = twocpu_row(&evidence, "(b) cold live-render burst")?[4]
        .parse()
        .map_err(|err| format!("2-CPU evidence cold p50 is not a number: {err}"))?;
    tokens.push(format!("{cold2_p50:.1}"));
    tokens.sort();
    tokens.dedup();
    Ok(tokens)
}

/// The grep-proof, as a permanent check: after removing marker pairs
/// (content included), fenced code, generated `table:` blocks, and the
/// reason-carrying [`ALLOWLIST`] snippets, no headline literal may remain
/// in a measured doc.
pub(super) fn check_naked(
    doc_label: &str,
    text: &str,
    expected: &BTreeMap<&'static str, String>,
) -> Result<(), String> {
    let mut prose = prose_segments(text).join("\n");
    // Remove every marker pair with its content.
    for (key, content) in markers(doc_label, text)? {
        prose = prose.replace(
            &format!("<!-- number:{key} -->{content}<!-- /number:{key} -->"),
            " ",
        );
    }
    // Remove generated table blocks (their bodies are artifact renderings).
    while let Some(start) = prose.find("<!-- table:") {
        let rest = &prose[start..];
        let stop = rest
            .find("<!-- /table:")
            .and_then(|at| rest[at..].find("-->").map(|end| at + end + "-->".len()))
            .ok_or_else(|| format!("{doc_label}: unterminated `<!-- table:` block"))?;
        prose.replace_range(start..start + stop, " ");
    }
    // Remove allowlisted historic snippets — each must still exist.
    for (doc, snippet, reason) in ALLOWLIST {
        if doc != doc_label {
            continue;
        }
        if !prose.contains(snippet) {
            return Err(format!(
                "{doc_label}: stale naked-number allowlist entry — snippet {snippet:?} \
                 (reason: {reason}) is gone; remove or update the entry in \
                 docs_check/numbers.rs"
            ));
        }
        prose = prose.replace(snippet, " ");
    }
    for token in detectors(expected)? {
        let mut from = 0;
        while let Some(at) = prose[from..].find(&token) {
            let pos = from + at;
            if stands_alone(&prose, pos, token.len()) {
                return Err(format!(
                    "{doc_label}: naked headline literal `{token}` outside a generated \
                     marker — quote it through a `number:<key>` marker and run \
                     `just perf-doc`"
                ));
            }
            from = pos + token.len();
        }
    }
    Ok(())
}

#[test]
fn headline_markers_agree_with_the_artifacts() {
    let expected = expected().unwrap();
    for (doc_label, required) in DOCS {
        check_doc(doc_label, &read_repo(doc_label), required, &expected).unwrap();
    }
}

#[test]
fn no_naked_headline_literal_outside_markers() {
    let expected = expected().unwrap();
    for (doc_label, _) in DOCS {
        check_naked(doc_label, &read_repo(doc_label), &expected).unwrap();
    }
}

// --- Mutation verification (the #173 discipline): each drift this gate
// exists to catch is re-introduced in memory and must fail, with the
// unmutated text first proven green. ---

/// A hand-edited (stale) marker body must fail.
#[test]
fn mutating_a_marker_body_fails() {
    let expected = expected().unwrap();
    let readme = read_repo("README.md");
    check_doc("README.md", &readme, &["i2p-ms"], &expected).expect("unmutated README must pass");
    let current = format!(
        "<!-- number:i2p-ms -->{v}<!-- /number:i2p-ms -->",
        v = expected["i2p-ms"]
    );
    assert!(
        readme.contains(&current),
        "anchor gone — update the fixture"
    );
    let mutated = readme.replace(
        &current,
        "<!-- number:i2p-ms -->747 ms<!-- /number:i2p-ms -->",
    );
    let err = check_doc("README.md", &mutated, &["i2p-ms"], &expected)
        .expect_err("a stale marker body must fail");
    assert!(err.contains("stale"), "error must say stale: {err}");
}

/// Stripping the marker tags but keeping the number (the pre-#174 state:
/// a hand-typed headline literal) must fail the naked scan.
#[test]
fn reintroducing_a_hand_typed_headline_number_fails() {
    let expected = expected().unwrap();
    let charter = read_repo("docs/CHARTER.md");
    check_naked("docs/CHARTER.md", &charter, &expected).expect("unmutated CHARTER must pass");
    let current = format!(
        "<!-- number:i2p-ms -->{v}<!-- /number:i2p-ms -->",
        v = expected["i2p-ms"]
    );
    assert!(
        charter.contains(&current),
        "anchor gone — update the fixture"
    );
    let mutated = charter.replace(&current, &expected["i2p-ms"]);
    let err = check_naked("docs/CHARTER.md", &mutated, &expected)
        .expect_err("a hand-typed headline literal must fail");
    assert!(err.contains("naked"), "error must say naked: {err}");
}

/// Deleting a required marker entirely must fail, not fall silent.
#[test]
fn deleting_a_required_marker_fails() {
    let expected = expected().unwrap();
    let demo = read_repo("docs/DEMO.md");
    check_doc("docs/DEMO.md", &demo, &["i2p-sha"], &expected).expect("unmutated DEMO must pass");
    let current = format!(
        "<!-- number:i2p-sha -->{v}<!-- /number:i2p-sha -->",
        v = expected["i2p-sha"]
    );
    assert!(demo.contains(&current), "anchor gone — update the fixture");
    let mutated = demo.replace(&current, "");
    let err = check_doc("docs/DEMO.md", &mutated, &["i2p-sha"], &expected)
        .expect_err("deleting a required marker must fail");
    assert!(err.contains("missing"), "error must say missing: {err}");
}

/// An allowlist entry whose snippet left the doc must fail as stale.
#[test]
fn a_stale_allowlist_entry_fails() {
    let expected = expected().unwrap();
    let perf = read_repo("docs/PERFORMANCE.md");
    check_naked("docs/PERFORMANCE.md", &perf, &expected).expect("unmutated PERFORMANCE must pass");
    let snippet = "in line with the prototype's ~40×";
    assert!(perf.contains(snippet), "anchor gone — update the fixture");
    let mutated = perf.replace(snippet, "in line with the prototype");
    let err = check_naked("docs/PERFORMANCE.md", &mutated, &expected)
        .expect_err("a stale allowlist entry must fail");
    assert!(err.contains("stale"), "error must say stale: {err}");
}
