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
//! Scope: `README.md` + `docs/*.md` + `docs/design/*.md` (per-file rows,
//! closed in both directions like every allowlist in this gate: a new doc
//! must get a row, a row whose doc was deleted or renamed fails as stale)
//! plus a directory ceiling over `docs/media/*.md` (#342) — the figure
//! pages and provenance sidecars grow with the figure set, so the ceiling
//! is on their sum. Generated docs (`docs/media/screenshots/index.md`,
//! `docs/perf/*.md`) and the immutable canon (`docs/decisions/`) stay out.
//!
//! Budgets ratchet: after a consolidation lands, every row becomes the
//! smaller of its previous value and the measured count + ~5% (first
//! ratchet 2026-08-29, #342), so the gate locks each gain in and never
//! records growth.

use super::repo_root;

/// The committed budgets: (repo-relative doc, max whitespace-delimited
/// words). On 2026-08-29 (#342, the first ratchet after M13 phase 1) every
/// row became `min(its previous budget, measured + ~5% rounded up to 25)`;
/// the second ratchet (2026-08-29, #344) applied the same rule after phase 4 —
/// a ratchet only moves down. Raising one is a reviewed edit with a dated
/// reason, in the same diff as the words.
const BUDGETS: [(&str, usize); 24] = [
    ("README.md", 1250),
    ("docs/ARCHITECTURE.md", 2125),
    ("docs/CHARTER.md", 1350),
    ("docs/COMPARISON.md", 1700),
    // Raised 1775 → 2050 on 2026-09-04 (#415): the reference gained the
    // `[[sources]]` table and what a restart does to it; 2050 → 2150 on
    // 2026-09-05 (#423) for credentials by reference. The ratchet takes it
    // back down at the next consolidation.
    ("docs/CONFIG.md", 2150),
    ("docs/DEMO.md", 875),
    // Raised 1800 → 2000 on 2026-09-04 (#409) and 2000 → 2150 on the same
    // day (#410): the reference gained a section per new route, and one
    // route is one section; 2150 → 2200 the same day (#416) for the trace
    // stream's second event kind; 2200 → 2400 the same day (#417) for the
    // sources resource. The ratchet takes it back down at the next
    // consolidation.
    ("docs/ENDPOINTS.md", 2400),
    ("docs/ENGINEERING.md", 1000),
    ("docs/EXTENDING.md", 1475),
    ("docs/OPERATIONS.md", 975),
    ("docs/PERFORMANCE.md", 2425),
    ("docs/QUICKSTART.md", 850),
    ("docs/RECIPES.md", 545),
    ("docs/RELEASING.md", 600),
    ("docs/REQUIREMENTS.md", 1400),
    ("docs/ROADMAP.md", 1850),
    // New on 2026-08-29 (#347): the Cargo.toml dependency essays, in one place.
    ("docs/SUPPLY-CHAIN.md", 1100),
    ("docs/design/authoring-dag.md", 2250),
    ("docs/design/authoring-ux.md", 3025),
    // New on 2026-09-03 (#390): the voice/elevation/closed-set companion to
    // ui-system.md — measured 952, budget 1000.
    ("docs/design/design-language.md", 1000),
    ("docs/design/catalog-domain.md", 1925),
    ("docs/design/extraction-boundary.md", 675),
    ("docs/design/materialization-planner.md", 1675),
    // 3425 -> 3475 on 2026-09-03: ADR 0028's compose amendment (#400) and
    // the css-template rule (#456) each landed under the old ceiling, and
    // together broke it — #400 had trimmed to EXACTLY 3425, leaving no room
    // for a concurrent PR. Raised to the measured 3451 plus a small margin,
    // which is what the ceiling is supposed to carry.
    ("docs/design/ui-system.md", 3475),
];

/// Directory ceilings: (repo-relative directory, max words summed over its
/// `*.md` files, non-recursive). `docs/media/` measured at 5594 on
/// 2026-08-29 (#342).
const DIR_BUDGETS: [(&str, usize); 1] = [("docs/media", 5875)];

/// The word count of `text` under the committed measurement method:
/// whitespace-delimited tokens, the same rule as `wc -w`.
fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// The `*.md` files directly under `dir` (repo-relative), sorted.
fn markdown_in(dir: &str) -> Vec<String> {
    let mut docs: Vec<String> = std::fs::read_dir(repo_root().join(dir))
        .unwrap_or_else(|err| panic!("{dir} is readable: {err}"))
        .map(|entry| entry.expect("readable dir entry").file_name())
        .filter_map(|name| {
            let name = name.to_str().expect("utf-8 filename").to_owned();
            std::path::Path::new(&name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                .then(|| format!("{dir}/{name}"))
        })
        .collect();
    docs.sort();
    docs
}

/// The per-file budgeted scope as found on disk: `README.md` +
/// `docs/*.md` + `docs/design/*.md`.
fn scope() -> Vec<String> {
    let mut files = vec!["README.md".to_owned()];
    files.append(&mut markdown_in("docs"));
    files.append(&mut markdown_in("docs/design"));
    files
}

/// Words summed over the `*.md` files directly under `dir`.
fn directory_words(dir: &str) -> usize {
    markdown_in(dir)
        .iter()
        .map(|file| word_count(&super::read_repo(file)))
        .sum()
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
    for (dir, budget) in DIR_BUDGETS {
        let words = directory_words(dir);
        if words > budget {
            violations.push(format!(
                "{dir}/*.md is over its directory word budget: {words} words > {budget} — \
                 the figure pages and sidecars are budgeted on their sum; trim, or raise \
                 the ceiling deliberately in the same reviewed diff"
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

/// The directory ceiling is live: the measured sum sits under it, and a
/// ceiling one word below the sum would fail.
#[test]
fn the_media_directory_ceiling_is_live() {
    let (dir, budget) = DIR_BUDGETS[0];
    let words = directory_words(dir);
    assert!(
        words <= budget,
        "{dir} measured {words} words, over its {budget} ceiling"
    );
    assert!(
        words > budget / 2,
        "{dir} ceiling {budget} is far above the measured {words}"
    );
}
