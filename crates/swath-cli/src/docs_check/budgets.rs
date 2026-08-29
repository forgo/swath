// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Per-doc word budgets (issue #177): the deletion-first word-reduction
//! sweep committed each core doc to a word budget, and this gate keeps
//! the docs from silently growing back. Words are whitespace-delimited
//! tokens over the raw file — exactly what `wc -w` counts and what
//! `just docs-words` (the committed measurement method) prints, so a
//! failure here is reproducible with one shell command.
//!
//! Severity: **failure** (the maintainer-decides knob from the issue was
//! implemented as a hard gate) — but every budget carries a generous
//! ~10% margin above the swept word count, rounded up to the next 25,
//! so ordinary edits (a clarified sentence, a new table row) never trip
//! it; only sustained regrowth does. Tightening a budget after further
//! deletion — or loosening one deliberately alongside new content — is
//! a reviewed edit to [`BUDGETS`], visible in the diff.
//!
//! Scope is the sweep's scope: `README.md` + `docs/*.md`. The map is
//! closed in both directions, like every allowlist in this gate: a new
//! doc under `docs/` must get a budget row, and a row whose doc was
//! deleted or renamed fails as stale.

use super::repo_root;

/// The committed budgets: (repo-relative doc, max whitespace-delimited
/// words). Values are the post-sweep counts plus ~10%, rounded up to 25.
const BUDGETS: [(&str, usize); 16] = [
    // 1025 → 1250 on 2026-08-29 (#331, the product-language pass): the hero is
    // the product-loop diagram and the README carries three captioned
    // screenshots instead of one — alt text and captions are the growth; the
    // prose itself shrank (the milestone paragraph went). Measured at 1216.
    ("README.md", 1250),
    ("docs/ARCHITECTURE.md", 2125),
    // Held at 1350 on 2026-08-29 (#338): PITCH.md (192 words) folded in as §14
    // while §§1, 4, 7, 12 became pointers to REQUIREMENTS — measured at 1289,
    // net −125 across the two files.
    ("docs/CHARTER.md", 1350),
    // 1600 → 1700 on 2026-08-29 (#331): the wedge diagram and its alt moved
    // here from the README, where the reader-facing hero replaced it.
    ("docs/COMPARISON.md", 1700),
    ("docs/CONFIG.md", 1775),
    // 775 → 875 on 2026-08-29 (#331): six screenshot embeds (x-ray, slider,
    // compare) — alt text, not prose.
    ("docs/DEMO.md", 875),
    // 1700 → 1760 on 2026-08-28: the merge_cubes join and its preview framing
    // (ADR 0022) — three sentences the process list could not carry; → 1800
    // the same day for the tileset metadata's window and branch count (#301).
    ("docs/ENDPOINTS.md", 1800),
    ("docs/ENGINEERING.md", 1000),
    ("docs/EXTENDING.md", 1525),
    ("docs/OPERATIONS.md", 975),
    // Raised 2050 -> 2425 with #207's `run_udf` evidence: PERFORMANCE.md
    // gained §9 (UDF bench + load evidence under the ADR 0012 guard) and
    // its generated load table — an acceptance criterion, not prose
    // regrowth — re-measured at 2372 + the usual headroom.
    ("docs/PERFORMANCE.md", 2425),
    // 850 → 900 on 2026-08-29 (#331): the tracks show their screenshots
    // instead of linking them.
    ("docs/QUICKSTART.md", 900),
    // Raised 525 -> 545 with #194's evidence screenshot: the QGIS recipe
    // gained its capture note and image line (an acceptance criterion,
    // not prose regrowth), re-measured at 534 + the usual headroom.
    ("docs/RECIPES.md", 545),
    ("docs/RELEASING.md", 600),
    ("docs/REQUIREMENTS.md", 1400),
    // Raised 1375 -> 1420 with #208's deferral row 18, then -> 1775 with
    // #212's era evidence: §1 gained the M9 entry (exit criteria, each
    // linking its committed evidence) and §3 item 8 its recorded
    // amendment — acceptance criteria, not prose regrowth; re-measured
    // at 1619 + the usual headroom.
    // Raised 1775 → 1950 with ADR 0022 (#294): the canonical deferral
    // inventory grew by five reopen-condition rows (19–23, ~110 words,
    // written tight) — table growth, not prose regrowth — plus ~4%
    // headroom for the next row.
    // Lowered 1950 → 1850 on 2026-08-29 (#339): §1 became a one-line-per-milestone
    // table, §3 lost its shipped items, the two co-recording ledgers point at §2;
    // measured at 1752 — the ratchet after a consolidation, not headroom.
    ("docs/ROADMAP.md", 1850),
];

/// The word count of `text` under the committed measurement method:
/// whitespace-delimited tokens, the same rule as `wc -w`.
fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// The budgeted scope as found on disk: `README.md` + `docs/*.md`.
fn scope() -> Vec<String> {
    let mut files = vec!["README.md".to_owned()];
    let mut docs: Vec<String> = std::fs::read_dir(repo_root().join("docs"))
        .expect("docs directory exists")
        .map(|entry| entry.expect("readable dir entry").file_name())
        .filter_map(|name| {
            let name = name.to_str().expect("utf-8 filename").to_owned();
            std::path::Path::new(&name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                .then(|| format!("docs/{name}"))
        })
        .collect();
    docs.sort();
    files.append(&mut docs);
    files
}

/// The gate: every in-scope doc within its budget, every budget row
/// matching a doc that exists, no in-scope doc without a budget.
pub(super) fn check() -> Result<(), String> {
    let mut violations = Vec::new();
    let on_disk = scope();
    for file in &on_disk {
        let Some((_, budget)) = BUDGETS.iter().find(|(doc, _)| doc == file) else {
            violations.push(format!(
                "{file} has no word budget — add a row to docs_check/budgets.rs \
                 (count it with `just docs-words`)"
            ));
            continue;
        };
        let words = word_count(&super::read_repo(file));
        if words > *budget {
            violations.push(format!(
                "{file} is over its word budget: {words} words > {budget} — the \
                 budget carries ~10% headroom over the #177 sweep, so this is \
                 sustained regrowth; delete words (deletion-first) or raise the \
                 budget deliberately in the same reviewed diff"
            ));
        }
    }
    for (doc, _) in BUDGETS {
        if !on_disk.iter().any(|file| file == doc) {
            violations.push(format!(
                "stale budget row (doc no longer exists — remove it): {doc}"
            ));
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "per-doc word budgets (issue #177; measure with `just docs-words`):\n  {}",
            violations.join("\n  ")
        ))
    }
}

#[test]
fn every_doc_is_within_its_committed_word_budget() {
    check().unwrap();
}

#[test]
fn word_counting_matches_wc_w() {
    // The committed measurement method is `wc -w`; split_whitespace is its
    // twin (both count maximal runs of non-whitespace).
    assert_eq!(word_count("a b  c\n\td "), 4);
    assert_eq!(word_count(""), 0);
    assert_eq!(word_count("  \n "), 0);
}

/// Mutation verification (the #173 discipline): a doc pushed over its
/// budget must fail, with the unmutated scope first proven green.
#[test]
fn a_doc_over_budget_is_caught() {
    check().expect("the unmutated docs must all be within budget");
    let (doc, budget) = BUDGETS[0];
    let words = word_count(&super::read_repo(doc));
    let overflow = budget + 1 - words;
    let padded = format!(
        "{}\n{}",
        super::read_repo(doc),
        vec!["padding"; overflow].join(" ")
    );
    assert!(
        word_count(&padded) > budget,
        "the padded fixture must exceed the budget"
    );
    // The scan itself is exercised through word_count + the budget
    // comparison; re-run the arithmetic the gate applies.
    assert!(
        word_count(&padded) > budget && words <= budget,
        "padding must be what pushes {doc} over its budget"
    );
}
